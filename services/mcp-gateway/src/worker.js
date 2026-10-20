import { DurableObject } from 'cloudflare:workers';
import OAuthProvider from '@cloudflare/workers-oauth-provider';
import localTools from './tools.json';
import { rejectSocketPending } from './socket-state.js';

const SCOPES = ['files:read', 'files:write', 'commands:execute'];
const MUTATIONS = new Set(['file_create', 'file_edit', 'command_run']);
const TOOL_NAMES = new Set(localTools.map(t => t.name));
const UUID = /^[a-f0-9]{8}-(?:[a-f0-9]{4}-){3}[a-f0-9]{12}$/;
const MAX_BODY = 131072;
const MAX_JOBS = 128;
const JOB_TTL = 10 * 60 * 1000;
const json = (data, status = 200) => Response.json(data, { status, headers: { 'cache-control': 'no-store' } });
const escape = value => String(value).replace(/[&<>"']/g, c => `&#${c.charCodeAt(0)};`);
const randomSecret = () => Array.from(crypto.getRandomValues(new Uint8Array(32)), n => n.toString(16).padStart(2, '0')).join('');
const hash = async text => Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text))), n => n.toString(16).padStart(2, '0')).join('');
function equal(a, b) { if (typeof a !== 'string' || typeof b !== 'string' || a.length !== b.length) return false; let n = 0; for (let i = 0; i < a.length; i++) n |= a.charCodeAt(i) ^ b.charCodeAt(i); return n === 0; }
function scopeFor(tool) { return tool === 'command_run' ? 'commands:execute' : MUTATIONS.has(tool) ? 'files:write' : 'files:read'; }
function canonical(value) { return value && typeof value === 'object' ? Array.isArray(value) ? '[' + value.map(canonical).join(',') + ']' : '{' + Object.keys(value).sort().map(k => JSON.stringify(k) + ':' + canonical(value[k])).join(',') + '}' : JSON.stringify(value); }
function stub(env, owner) { if (!UUID.test(owner)) throw new Error('invalid_owner'); return env.OWNERS.getByName(owner); }
function internal(path, body, headers = {}) { return new Request('https://internal' + path, { method: 'POST', headers: { 'content-type': 'application/json', ...headers }, body: JSON.stringify(body) }); }
async function boundedBytes(request, limit = MAX_BODY) {
  const reader = request.body?.getReader();
  if (!reader) return new Uint8Array();
  const chunks = []; let size = 0;
  for (;;) { const part = await reader.read(); if (part.done) break; size += part.value.byteLength; if (size > limit) { await reader.cancel(); throw new Error('body_too_large'); } chunks.push(part.value); }
  const bytes = new Uint8Array(size); let offset = 0; for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
  return bytes;
}
async function boundedJson(request, limit = MAX_BODY) {
  const bytes = await boundedBytes(request, limit);
  const value = bytes.length ? JSON.parse(new TextDecoder().decode(bytes)) : {};
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('invalid_request');
  return value;
}
async function ownerIdentity(request, env) {
  const token = request.headers.get('authorization')?.match(/^Bearer ([a-f0-9-]+\.[a-f0-9]{64})$/)?.[1];
  const owner = token?.split('.')[0];
  if (!owner || !UUID.test(owner)) return null;
  const checked = await stub(env, owner).fetch(internal('/auth', {}, { authorization: `Bearer ${token}` }));
  return checked.ok ? checked.json() : null;
}

export class RateLimit extends DurableObject {
  async fetch() {
    const now = Date.now();
    const allowed = await this.ctx.storage.transaction(async tx => {
      let state = await tx.get('rate');
      if (!state || state.until < now) state = { count: 0, until: now + 600000 };
      state.count++; await tx.put('rate', state); await tx.setAlarm(state.until);
      return state.count <= 30;
    });
    return json({ allowed }, allowed ? 200 : 429);
  }
  async alarm() { await this.ctx.storage.deleteAll(); }
}

// One Durable Object per authenticated owner. No client-supplied owner selector
// is accepted on the MCP route. Only the Worker can call internal DO endpoints.
export class Owner extends DurableObject {
  constructor(ctx, env) { super(ctx, env); this.pending = new Map(); }
  async fetch(request) {
    const path = new URL(request.url).pathname;
    if (path === '/init') {
      if (await this.ctx.storage.get('device')) return json({ error: 'already_registered' }, 409);
      const device = await boundedJson(request); await this.ctx.storage.put('device', device); return json({ ok: true }, 201);
    }
    const device = await this.ctx.storage.get('device');
    if (!device || device.revoked) return json({ error: 'device_revoked' }, 403);
    if (path === '/auth' || path === '/connect' || path === '/revoke') {
      const supplied = request.headers.get('authorization')?.replace(/^Bearer /, '') || '';
      if (!equal(await hash(supplied), device.token_hash)) return json({ error: 'unauthorized' }, 401);
      if (path === '/auth') return json({ owner_id: device.owner_id, device_id: device.device_id });
      if (path === '/revoke') {
        device.revoked = true; await this.ctx.storage.put('device', device);
        for (const ws of this.ctx.getWebSockets()) ws.close(1008, 'revoked');
        this.failPending(); return json({ revoked: true });
      }
      if (request.headers.get('upgrade')?.toLowerCase() !== 'websocket') return json({ error: 'websocket_required' }, 426);
      const generation = crypto.randomUUID();
      await this.ctx.storage.put('generation', generation);
      for (const ws of this.ctx.getWebSockets()) ws.close(1012, 'replaced');
      this.failPending();
      const pair = new WebSocketPair();
      this.ctx.acceptWebSocket(pair[1]);
      pair[1].serializeAttachment({ device_id: device.device_id, generation });
      return new Response(null, { status: 101, webSocket: pair[0] });
    }
    if (path === '/state') return json({ online: this.ctx.getWebSockets().some(s => s.readyState === 1) });
    if (path === '/job') {
      const { operation_id, scopes } = await boundedJson(request);
      if (!UUID.test(operation_id)) return json({ error: 'invalid_operation' }, 400);
      let job = await this.ctx.storage.get('job:' + operation_id);
      if (!job) return json({ error: 'operation_not_found' }, 404);
      if (!scopes.includes(scopeFor(job.tool))) return json({ error: 'insufficient_scope' }, 403);
      if (job.expires_at < Date.now() && ['dispatching', 'pending_approval'].includes(job.status)) {
        job = { ...job, status: 'expired' }; await this.ctx.storage.put('job:' + operation_id, job);
      }
      return json(job);
    }
    if (path !== '/invoke') return json({ error: 'not_found' }, 404);
    const { tool, args, request_id } = await boundedJson(request);
    if (!TOOL_NAMES.has(tool)) return json({ error: 'unknown_tool' }, 400);
    const id = MUTATIONS.has(tool) ? request_id : crypto.randomUUID();
    if (!UUID.test(id || '')) return json({ error: 'request_id_required' }, 400);
    const request_digest = await hash(canonical({ tool, args }));
    if (MUTATIONS.has(tool)) {
      const previous = await this.ctx.storage.get('job:' + id);
      if (previous) return previous.request_digest === request_digest ? json(previous) : json({ error: 'request_id_conflict' }, 409);
    }
    const socket = this.ctx.getWebSockets().find(s => s.readyState === 1);
    if (!socket) return json({ error: 'device_offline' }, 409);
    let operation;
    if (MUTATIONS.has(tool)) {
      const existing = await this.ctx.storage.list({ prefix: 'job:' });
      const cutoff = Date.now() - JOB_TTL;
      const old = [...existing].filter(([, job]) => job.expires_at < cutoff).map(([key]) => key);
      if (old.length) await this.ctx.storage.delete(old);
      if (existing.size - old.length >= MAX_JOBS) return json({ error: 'operation_capacity' }, 429);
      operation = { operation_id: id, tool, arguments: args, expires_at: Date.now() + JOB_TTL };
      // Persist only metadata until the agent acknowledges durable local acceptance.
      await this.ctx.storage.put('job:' + id, { operation_id: id, tool, request_digest, expires_at: operation.expires_at, status: 'dispatching' });
      await this.ctx.storage.setAlarm(Date.now() + JOB_TTL * 2);
    }
    try {
      const result = await new Promise((resolve, reject) => {
        const timer = setTimeout(() => { this.pending.delete(id); reject(new Error('device_timeout')); }, 15000);
        this.pending.set(id, { socket, resolve, reject, timer });
        socket.send(JSON.stringify({ type: 'request', id, tool, arguments: args, ...(operation ? { operation } : {}) }));
      });
      return json(result);
    } catch {
      if (operation) {
        const job = await this.ctx.storage.get('job:' + id);
        if (job?.status === 'dispatching') await this.ctx.storage.put('job:' + id, { ...job, status: 'indeterminate' });
        return json({ operation_id: id, status: 'indeterminate', next_tool: 'operation_get' });
      }
      return json({ error: 'device_unavailable' }, 409);
    }
  }
  async webSocketMessage(ws, raw) {
    if (ws.deserializeAttachment()?.generation !== await this.ctx.storage.get('generation')) return;
    if (typeof raw !== 'string' || new TextEncoder().encode(raw).length > MAX_BODY * 2) { ws.close(1009, 'frame_too_large'); return; }
    let message; try { message = JSON.parse(raw); } catch { ws.close(1008, 'invalid_frame'); return; }
    if (message.type === 'response') {
      const pending = this.pending.get(message.id);
      if (!pending || pending.socket !== ws) return;
      clearTimeout(pending.timer); this.pending.delete(message.id); pending.resolve(message.result);
    } else if (message.type === 'operation_update') {
      const job = message.job;
      if (!job || !UUID.test(job.operation_id)) return;
      const previous = await this.ctx.storage.get('job:' + job.operation_id);
      if (!previous || previous.tool !== job.tool || previous.expires_at !== job.expires_at) return;
      const terminal = new Set(['completed', 'failed', 'denied', 'expired', 'indeterminate']);
      if (terminal.has(previous.status) && previous.status !== 'indeterminate') return;
      if (!['pending_approval', 'running', ...terminal].includes(job.status)) return;
      const clean = { operation_id: previous.operation_id, tool: previous.tool, request_digest: previous.request_digest,
        expires_at: previous.expires_at, status: job.status,
        ...(job.result !== undefined ? { result: job.result } : {}), ...(job.error ? { error: 'execution_failed' } : {}) };
      await this.ctx.storage.put('job:' + job.operation_id, clean);
    }
  }
  failPending(socket) { rejectSocketPending(this.pending, socket); }
  webSocketClose(ws) { this.failPending(ws); }
  webSocketError(ws) { this.failPending(ws); }
  async alarm() {
    const jobs = await this.ctx.storage.list({ prefix: 'job:' });
    const now = Date.now();
    for (const [key, job] of jobs) if (job.expires_at + JOB_TTL < now) await this.ctx.storage.delete(key);
    if ((await this.ctx.storage.list({ prefix: 'job:', limit: 1 })).size) await this.ctx.storage.setAlarm(now + JOB_TTL);
  }
}

export class Login extends DurableObject {
  async fetch(request) {
    const path = new URL(request.url).pathname;
    const body = await boundedJson(request);
    if (path === '/create') {
      if (await this.ctx.storage.get('login')) return json({ error: 'exists' }, 409);
      await this.ctx.storage.put('login', body); await this.ctx.storage.setAlarm(body.expires_at); return json({ ok: true });
    }
    const login = await this.ctx.storage.get('login');
    if (!login || login.expires_at < Date.now() || login.used) return json({ error: 'login_expired' }, 410);
    if (path === '/preview') return json({ ...login.details, expires_at: login.expires_at });
    if (path === '/approve') {
      if (login.owner_id) return json({ error: 'already_paired' }, 409);
      login.owner_id = body.owner_id; login.device_id = body.device_id;
      await this.ctx.storage.put('login', login); return json({ approved: true });
    }
    if (!equal(await hash(body.browser_secret || ''), login.browser_hash)) return json({ error: 'browser_binding' }, 403);
    if (path === '/status') return json({ paired: Boolean(login.owner_id) });
    if (path !== '/consume' || !login.owner_id) return json({ error: 'mac_approval_required' }, 403);
    if (!equal(await hash(body.handle || ''), login.consent_hash)) return json({ error: 'consent_binding' }, 403);
    login.used = true; await this.ctx.storage.put('login', login);
    return json({ owner_id: login.owner_id, device_id: login.device_id });
  }
  async alarm() { await this.ctx.storage.deleteAll(); }
}

function tools() {
  const list = localTools.map(tool => ({ ...tool,
    annotations: { ...tool.annotations, readOnlyHint: !MUTATIONS.has(tool.name),
      destructiveHint: tool.name === 'file_edit' || tool.name === 'command_run', openWorldHint: tool.name === 'command_run' },
    ...(MUTATIONS.has(tool.name) ? { inputSchema: { ...tool.inputSchema, properties: { ...tool.inputSchema.properties,
      request_id: { type: 'string', format: 'uuid', description: 'Generate one UUID for this intended operation. Reuse it for a transport retry; never reuse it for a different operation.' } }, required: [...tool.inputSchema.required, 'request_id'] },
      description: tool.description + ' Queues this exact operation for approval in the owner\'s Localoud Mac app. Does NOT execute immediately. Return pending_approval and stop; after the user approves, call operation_get with operation_id. Never resubmit the write to check its status.' } : {}) }));
  list.push({ name: 'operation_get', description: 'Read the status and result of this owner\'s previously submitted edit or command. Use after local approval. Never resubmit an operation. pending_approval means the user must approve in the Mac app; completed contains the original tool result.',
    inputSchema: { type: 'object', properties: { operation_id: { type: 'string', format: 'uuid' } }, required: ['operation_id'], additionalProperties: false },
    annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: false } });
  return list;
}
function toolResult(value, isError = false) { return { content: [{ type: 'text', text: JSON.stringify(value) }], ...(isError ? { isError: true } : {}) }; }
async function mcp(request, env, ctx) {
  const owner = ctx.props?.ownerId;
  const scopes = ctx.auth?.scope || [];
  if (!owner || !UUID.test(owner)) return json({ error: 'missing_identity' }, 403);
  const state = await stub(env, owner).fetch(internal('/state', {}));
  if (!state.ok) return json({ error: 'device_revoked' }, 403);
  if (request.method !== 'POST') return new Response(null, { status: 405, headers: { allow: 'POST' } });
  const rpc = await boundedJson(request);
  if (rpc.jsonrpc !== '2.0' || typeof rpc.method !== 'string') return json({ error: { code: -32600, message: 'Invalid request' }, id: rpc.id ?? null, jsonrpc: '2.0' });
  if (rpc.id === undefined) return new Response(null, { status: 202 });
  let result;
  if (rpc.method === 'initialize') result = { protocolVersion: '2025-06-18', capabilities: { tools: {} }, serverInfo: { name: 'localoud-web-gateway', version: '0.1.0' },
    instructions: 'Operate only on projects returned by project_list. All edits and commands require separate approval in the owner\'s local Mac app. A pending_approval receipt means no execution has occurred: tell the user to approve and stop. After approval use operation_get and file_read. Never retry a write or command just to get its status. Commands run with the Mac user\'s permissions.' };
  else if (rpc.method === 'ping') result = {};
  else if (rpc.method === 'tools/list') result = { tools: tools() };
  else if (rpc.method === 'tools/call') {
    const name = rpc.params?.name;
    if (!TOOL_NAMES.has(name) && name !== 'operation_get') return json({ jsonrpc: '2.0', id: rpc.id, error: { code: -32602, message: 'Unknown tool' } });
    const required = scopeFor(name);
    if (!scopes.includes(required)) return json({ error: 'insufficient_scope' }, 403);
    let args = rpc.params?.arguments ?? {};
    if (!args || typeof args !== 'object' || Array.isArray(args)) return json({ jsonrpc: '2.0', id: rpc.id, error: { code: -32602, message: 'Invalid arguments' } });
    let request_id;
    if (MUTATIONS.has(name)) { ({ request_id, ...args } = args); if (!UUID.test(request_id || '')) return json({ jsonrpc: '2.0', id: rpc.id, error: { code: -32602, message: 'request_id UUID is required' } }); }
    const r = name === 'operation_get'
      ? await stub(env, owner).fetch(internal('/job', { operation_id: args.operation_id, scopes }))
      : await stub(env, owner).fetch(internal('/invoke', { tool: name, args, ...(request_id ? { request_id } : {}) }));
    const value = await r.json();
    result = !r.ok ? toolResult(value, true) : value.isError === true ? value : name === 'operation_get' || MUTATIONS.has(name) ? toolResult(value) : value;
  } else return json({ jsonrpc: '2.0', id: rpc.id, error: { code: -32601, message: 'Method not found' } });
  return json({ jsonrpc: '2.0', id: rpc.id, result });
}
function cookie(request, id) { return request.headers.get('cookie')?.split(';').map(s => s.trim()).find(s => s.startsWith('__Host-localoud-' + id + '='))?.split('=')[1] || ''; }
function loginStub(env, code) { if (!UUID.test(code || '')) throw new Error('invalid_pair_code'); return env.LOGINS.getByName(code); }
function page(content, script = '') {
  const nonce = randomSecret();
  const style = 'body{margin:0;background:#f5f4f0;color:#192822;font:16px system-ui;line-height:1.7}main{max-width:700px;margin:56px auto;padding:32px;background:white;border:1px solid #d4ded6;border-radius:16px}h1{font-size:25px}code{background:#e9f0eb;padding:8px;overflow-wrap:anywhere}button{background:#174d38;color:white;border:0;border-radius:8px;padding:12px 20px;font:inherit;cursor:pointer}button:disabled{opacity:.4}label{display:block;margin:8px 0}small{color:#596961}';
  return new Response(`<!doctype html><html lang="ja"><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>Localoud 接続</title><style nonce="${nonce}">${style}</style><main>${content}</main>${script ? `<script nonce="${nonce}">${script}</script>` : ''}</html>`, { headers: {
    'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store', 'referrer-policy': 'same-origin',
    'x-frame-options': 'DENY', 'content-security-policy': `default-src 'none'; style-src 'nonce-${nonce}'; script-src 'nonce-${nonce}'; connect-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'` } });
}
async function other(request, env) {
  const url = new URL(request.url);
  if (url.pathname === '/healthz') return json({ service: 'localoud-mcp-gateway', version: '0.1.0' });
  if (url.pathname === '/') return page('<h1>Localoud Web</h1><p>ChatGPTから、ご自身のMacで選んだプロジェクトを操作します。</p><p>Mac上でLocaloudエージェントを起動し、このサーバーの <code>/mcp</code> をChatGPTのカスタムMCPとして接続してください。編集とコマンドはMac上での承認後に実行します。</p>');
  if (url.pathname === '/agent/enroll' && request.method === 'POST') {
    await boundedJson(request, 65536);
    const owner_id = crypto.randomUUID(), device_id = crypto.randomUUID();
    const device_token = owner_id + '.' + randomSecret();
    await stub(env, owner_id).fetch(internal('/init', { owner_id, device_id, token_hash: await hash(device_token), revoked: false }));
    return json({ owner_id, device_id, device_token }, 201);
  }
  if (url.pathname.startsWith('/agent/')) {
    const identity = await ownerIdentity(request, env);
    if (!identity) return json({ error: 'unauthorized' }, 401);
    if (url.pathname === '/agent/connect' && request.method === 'GET') {
      return stub(env, identity.owner_id).fetch(new Request('https://internal/connect', { headers: request.headers }));
    }
    if (request.method !== 'POST') return json({ error: 'method_not_allowed' }, 405);
    const body = await boundedJson(request, 4096);
    if (url.pathname === '/agent/revoke') return stub(env, identity.owner_id).fetch(internal('/revoke', {}, { authorization: request.headers.get('authorization') }));
    if (url.pathname === '/agent/pair-preview') return loginStub(env, body.code).fetch(internal('/preview', {}));
    if (url.pathname === '/agent/pair-approve') return loginStub(env, body.code).fetch(internal('/approve', identity));
    return json({ error: 'not_found' }, 404);
  }
  if (url.pathname === '/oauth/authorize' && request.method === 'GET') {
    const oauth = env.OAUTH_PROVIDER;
    const authRequest = await oauth.parseAuthRequest(request);
    if (authRequest.scope.some(s => !SCOPES.includes(s) && s !== 'offline_access')) return json({ error: 'unsupported_scope' }, 400);
    const details = await oauth.describeConsent(authRequest);
    const consent = await oauth.beginConsent(authRequest);
    const code = crypto.randomUUID(), secret = randomSecret();
    await loginStub(env, code).fetch(internal('/create', { details: { ...details, scope: [...SCOPES, 'offline_access'] },
      consent_hash: await hash(consent.handle), browser_hash: await hash(secret), expires_at: Date.now() + JOB_TTL }));
    const response = page(`<h1>ChatGPTとMacを接続</h1><p>接続元：<strong>${escape(details.clientName)}</strong></p><p>接続元ドメイン：${escape(details.clientDomain || '自己申告のクライアント名です')}</p><p>認可の戻り先：<strong>${escape(details.redirectHost)}</strong></p>${details.redirectIsLoopback ? '<p>注意：PC上のアプリへ認可を返します。</p>' : ''}<p>Mac上のLocaloud承認画面に、この接続コードを入力してください。</p><p><code data-pair-code="${code}">${code}</code></p><p id="status">Macでの確認を待っています。</p><form method="post" action="/oauth/finish"><input type="hidden" name="login" value="${code}"><input type="hidden" name="handle" value="${escape(consent.handle)}">${[...SCOPES, 'offline_access'].map(s => `<label><input type="checkbox" name="scope" value="${s}" checked> ${escape(s)}</label>`).join('')}<p>許可したファイル内容・コマンド出力は、Localoud中継サーバーとChatGPTに送信されます。コマンドはMac利用者権限で動きます。</p><button id="allow" name="decision" value="approve" disabled>接続を許可</button> <button name="decision" value="deny">キャンセル</button></form>`,
      `setInterval(async()=>{try{const r=await fetch('/oauth/pair-status?id=${code}');if(!r.ok)return;const s=await r.json();if(s.paired){document.getElementById('allow').disabled=false;document.getElementById('status').textContent='Macの本人確認が完了しました。権限を確認し、接続を許可してください。'}}catch{}},1500);`);
    for (const [key, value] of consent.headers) if (key.toLowerCase() !== 'set-cookie') response.headers.set(key, value);
    for (const value of consent.headers.getSetCookie()) response.headers.append('set-cookie', value);
    response.headers.append('set-cookie', `__Host-localoud-${code}=${secret}; Max-Age=600; Path=/; Secure; HttpOnly; SameSite=Lax`);
    return response;
  }
  if (url.pathname === '/oauth/pair-status' && request.method === 'GET') {
    const id = url.searchParams.get('id'); return loginStub(env, id).fetch(internal('/status', { browser_secret: cookie(request, id) }));
  }
  if (url.pathname === '/oauth/finish' && request.method === 'POST') {
    if (request.headers.get('origin') !== env.PUBLIC_ORIGIN) return json({ error: 'invalid_origin' }, 403);
    const raw = await request.text(); if (raw.length > 8192) return json({ error: 'body_too_large' }, 413);
    const form = new URLSearchParams(raw), handle = form.get('handle'), id = form.get('login');
    if (form.get('decision') !== 'approve') {
      const denied = await env.OAUTH_PROVIDER.denyConsent(request, handle);
      return new Response(null, { status: 302, headers: denied.headers });
    }
    const selected = form.getAll('scope');
    if (!selected.includes('files:read') || selected.some(s => ![...SCOPES, 'offline_access'].includes(s))) return json({ error: 'invalid_scope' }, 400);
    // Check Mac ownership AND browser binding before consuming provider consent.
    const paired = await loginStub(env, id).fetch(internal('/consume', { browser_secret: cookie(request, id), handle }));
    if (!paired.ok) return paired;
    const identity = await paired.json();
    const approved = await env.OAUTH_PROVIDER.approveConsent(request, handle, { scope: selected });
    const { redirectTo } = await env.OAUTH_PROVIDER.completeAuthorization({ request: approved.request,
      userId: identity.owner_id, metadata: { device_id: identity.device_id }, scope: approved.request.scope,
      props: { ownerId: identity.owner_id, deviceId: identity.device_id } });
    approved.headers.set('location', redirectTo);
    return new Response(null, { status: 302, headers: approved.headers });
  }
  return json({ error: 'not_found' }, 404);
}
function provider(env) {
  return new OAuthProvider({ apiRoute: '/mcp', apiHandler: { fetch: mcp }, defaultHandler: { fetch: other },
    authorizeEndpoint: '/oauth/authorize', tokenEndpoint: '/oauth/token', clientRegistrationEndpoint: '/oauth/register',
    scopesSupported: [...SCOPES, 'offline_access'], requiredScopes: SCOPES, allowPlainPKCE: false,
    accessTokenTTL: 600, refreshTokenTTL: 30 * 86400, clientIdMetadataDocumentEnabled: true,
    resourceMetadata: { resource: env.PUBLIC_ORIGIN + '/mcp', authorization_servers: [env.PUBLIC_ORIGIN], resource_name: 'Localoud Web' } });
}
export default {
  async fetch(request, env, ctx) {
    try {
      const origin = new URL(request.url).origin;
      if (!env.PUBLIC_ORIGIN || origin !== env.PUBLIC_ORIGIN) return json({ error: 'wrong_origin' }, 400);
      const url = new URL(request.url);
      if (request.method === 'POST' && ['/agent/enroll', '/oauth/register'].includes(url.pathname)) {
        const identity = await hash(request.headers.get('cf-connecting-ip') || 'local');
        const limited = await env.LIMITS.getByName(identity).fetch(new Request('https://internal/rate'));
        if (!limited.ok) return json({ error: 'rate_limited' }, 429);
      }
      if (Number(request.headers.get('content-length')) > MAX_BODY) return json({ error: 'body_too_large' }, 413);
      if (request.body) request = new Request(request, { body: await boundedBytes(request) });
      return await provider(env).fetch(request, env, ctx);
    } catch (error) {
      const message = String(error?.message || '');
      if (message === 'body_too_large') return json({ error: message }, 413);
      if (/invalid_|JSON|Unexpected|scope|consent|authorization|client/i.test(message)) return json({ error: 'invalid_request' }, 400);
      // Never put credentials, tool payloads, or raw exception text in logs/responses.
      return json({ error: 'request_failed' }, 500);
    }
  },
};
