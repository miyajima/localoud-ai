import { after, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { Miniflare } from 'miniflare';
import { createHash, randomBytes } from 'node:crypto';

let mf;
const origin = 'https://localoud.test';
before(async () => {
  mf = new Miniflare({ modules: true, scriptPath: new URL('../dist/worker.js', import.meta.url).pathname,
    compatibilityDate: '2026-07-01', compatibilityFlags: ['nodejs_compat', 'global_fetch_strictly_public'],
    kvNamespaces: ['OAUTH_KV'], durableObjects: {
      OWNERS: { className: 'Owner', useSQLite: true },
      LOGINS: { className: 'Login', useSQLite: true },
      LIMITS: { className: 'RateLimit', useSQLite: true },
    }, bindings: { PUBLIC_ORIGIN: origin } });
});
after(async () => { await mf?.dispose(); });
const call = (path, options = {}) => mf.dispatchFetch(origin + path, options);
async function post(path, body, token, headers = {}) {
  return call(path, { method: 'POST', headers: { 'content-type': 'application/json',
    ...(token ? { authorization: `Bearer ${token}` } : {}), ...headers }, body: JSON.stringify(body) });
}
async function enroll() { const r = await post('/agent/enroll', {}); assert.equal(r.status, 201); return r.json(); }

test('MCP is protected and advertises canonical OAuth resource metadata', async () => {
  const r = await post('/mcp', { jsonrpc: '2.0', id: 1, method: 'tools/list' });
  assert.equal(r.status, 401);
  assert.match(r.headers.get('www-authenticate'), /oauth-protected-resource/);
  const meta = await (await call('/.well-known/oauth-protected-resource/mcp')).json();
  assert.equal(meta.resource, origin + '/mcp');
  assert.deepEqual(meta.scopes_supported, ['files:read', 'files:write', 'commands:execute']);
  const auth = await (await call('/.well-known/oauth-authorization-server')).json();
  assert.deepEqual(auth.code_challenge_methods_supported, ['S256']);
});

test('device credentials cannot be replaced with another owner ID or random token', async () => {
  const a = await enroll(), b = await enroll();
  assert.notEqual(a.owner_id, b.owner_id);
  const wrong = `${b.owner_id}.${a.device_token.split('.')[1]}`;
  assert.equal((await post('/agent/pair-preview', { code: crypto.randomUUID() }, wrong)).status, 401);
  assert.equal((await post('/agent/pair-preview', { code: crypto.randomUUID() }, 'invalid')).status, 401);
  assert.equal((await call('/agent/connect', { headers: { Upgrade: 'websocket' } })).status, 401);
});

async function beginOAuth() {
  const clientResponse = await post('/oauth/register', { client_name: 'Localoud test',
    redirect_uris: ['https://client.test/callback'], token_endpoint_auth_method: 'none',
    grant_types: ['authorization_code', 'refresh_token'], response_types: ['code'] });
  assert.equal(clientResponse.status, 201);
  const client = await clientResponse.json();
  const verifier = randomBytes(32).toString('base64url');
  const q = new URLSearchParams({ client_id: client.client_id, redirect_uri: 'https://client.test/callback',
    response_type: 'code', code_challenge: createHash('sha256').update(verifier).digest('base64url'),
    code_challenge_method: 'S256', resource: origin + '/mcp', scope: 'files:read files:write commands:execute', state: 'fixture-state' });
  const r = await call('/oauth/authorize?' + q);
  assert.equal(r.status, 200);
  const html = await r.text();
  const code = html.match(/data-pair-code="([a-f0-9-]+)"/)[1];
  const handle = html.match(/name="handle" value="([^"]+)"/)[1];
  const cookie = r.headers.getSetCookie().map(s => s.split(';')[0]).join('; ');
  return { client, verifier, code, handle, cookie };
}
async function finishOAuth(flow, cookie = flow.cookie) {
  const body = new URLSearchParams({ login: flow.code, handle: flow.handle, decision: 'approve' });
  for (const scope of ['files:read', 'files:write', 'commands:execute']) body.append('scope', scope);
  return call('/oauth/finish', { method: 'POST', headers: { 'content-type': 'application/x-www-form-urlencoded',
    origin, cookie }, body: body.toString(), redirect: 'manual' });
}
async function token(flow, code, verifier = flow.verifier) {
  return call('/oauth/token', { method: 'POST', headers: { 'content-type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({ grant_type: 'authorization_code', code, code_verifier: verifier,
      client_id: flow.client.client_id, redirect_uri: 'https://client.test/callback', resource: origin + '/mcp' }).toString() });
}
test('OAuth cannot finish without Mac possession, browser binding and consent; PKCE is enforced', async () => {
  const flow = await beginOAuth();
  assert.equal((await finishOAuth(flow)).status, 403);
  const device = await enroll();
  const preview = await post('/agent/pair-preview', { code: flow.code }, device.device_token);
  assert.equal(preview.status, 200);
  assert.equal((await preview.json()).redirectHost, 'client.test');
  assert.equal((await post('/agent/pair-approve', { code: flow.code }, device.device_token)).status, 200);
  assert.equal((await finishOAuth(flow, '')).status, 403);
  const finished = await finishOAuth(flow);
  assert.equal(finished.status, 302);
  const url = new URL(finished.headers.get('location'));
  assert.equal(url.searchParams.get('state'), 'fixture-state');
  assert.equal((await token(flow, url.searchParams.get('code'), 'wrong')).status, 400);
});

test('two independently paired users cannot cross devices; revoked owner tokens fail closed', async () => {
  const device = await enroll(), flow = await beginOAuth();
  await post('/agent/pair-approve', { code: flow.code }, device.device_token);
  const finished = await finishOAuth(flow);
  const code = new URL(finished.headers.get('location')).searchParams.get('code');
  const issued = await token(flow, code);
  assert.equal(issued.status, 200);
  const creds = await issued.json();
  assert.ok(creds.access_token);
  const r = await post('/mcp', { jsonrpc: '2.0', id: 2, method: 'tools/call',
    params: { name: 'project_list', arguments: {} } }, creds.access_token);
  const result = await r.json();
  assert.equal(result.result.isError, true);
  assert.match(result.result.content[0].text, /device_offline/);
  assert.equal((await post('/agent/revoke', {}, device.device_token)).status, 200);
  const revoked = await post('/mcp', { jsonrpc: '2.0', id: 3, method: 'tools/list' }, creds.access_token);
  assert.equal(revoked.status, 403);
  assert.equal((await token(flow, code)).status, 400, 'authorization code cannot be replayed');
});

test('a browser cannot swap another consent request into a Mac-approved pairing', async () => {
  const device = await enroll(), a = await beginOAuth(), b = await beginOAuth();
  await post('/agent/pair-approve', { code: a.code }, device.device_token);
  const swapped = { ...a, handle: b.handle, cookie: a.cookie + '; ' + b.cookie };
  assert.equal((await finishOAuth(swapped)).status, 403);
});

test('cross-origin OAuth mutation and oversized requests are rejected', async () => {
  const r = await call('/oauth/finish', { method: 'POST', headers: { origin: 'https://evil.test',
    'content-type': 'application/x-www-form-urlencoded' }, body: 'decision=approve' });
  assert.equal(r.status, 403);
  assert.equal((await post('/agent/enroll', { payload: 'x'.repeat(70000) })).status, 413);
});
