import { lstat, mkdir, open, readFile, unlink, realpath, rename } from 'node:fs/promises';
import { join, resolve, dirname } from 'node:path';
import { randomUUID } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';
import WebSocket from 'ws';
import { ApprovalGate } from './approval.mjs';
import { McpClient } from './mcp.mjs';
import { startApprovalApp } from './app.mjs';

const MUTATIONS = new Set(['command_run', 'file_edit', 'file_create']);
const READS = new Set(['project_list', 'project_get', 'repo_tree', 'file_read', 'file_search', 'git_status', 'git_diff']);
const UUID = /^[a-f0-9]{8}-(?:[a-f0-9]{4}-){3}[a-f0-9]{12}$/;
const json = value => JSON.stringify(value);
function assertPrivate(stat, dir = false) {
  if (stat.isSymbolicLink() || (dir ? !stat.isDirectory() : !stat.isFile() || stat.nlink !== 1)
    || (stat.mode & 0o777) !== (dir ? 0o700 : 0o600) || stat.uid !== process.getuid()) throw new Error('unsafe_local_state');
}
async function privateRead(path) { const stat = await lstat(path); assertPrivate(stat); return readFile(path, 'utf8'); }
async function exclusiveWrite(path, data) {
  const file = await open(path, 'wx', 0o600); try { await file.writeFile(data); await file.sync(); } finally { await file.close(); }
}
async function replacePrivate(path, data) {
  const existing = await lstat(path).catch(e => { if (e.code === 'ENOENT') return null; throw e; });
  if (existing) assertPrivate(existing);
  const temporary = path + '.' + randomUUID() + '.tmp';
  await exclusiveWrite(temporary, data);
  try { await rename(temporary, path); } catch (error) { await unlink(temporary).catch(() => {}); throw error; }
}
async function lockAgent(path) {
  const token = randomUUID();
  try { await exclusiveWrite(path, json({ pid: process.pid, token })); }
  catch (error) {
    if (error.code !== 'EEXIST') throw error;
    const previous = JSON.parse(await privateRead(path));
    if (!Number.isInteger(previous.pid) || previous.pid < 1) throw new Error('invalid_agent_lock');
    try { process.kill(previous.pid, 0); throw new Error('agent_already_running'); }
    catch (e) { if (e.code !== 'ESRCH') throw new Error('agent_already_running'); }
    await unlink(path); await exclusiveWrite(path, json({ pid: process.pid, token }));
  }
  return async () => {
    const current = JSON.parse(await privateRead(path));
    if (current.pid === process.pid && current.token === token) await unlink(path);
  };
}
function publicJob(job) {
  return { operation_id: job.operation_id, tool: job.tool, expires_at: job.expires_at,
    status: job.status === 'pending' ? 'pending_approval' : job.status,
    ...(job.result !== undefined ? { result: job.result } : {}), ...(job.error ? { error: job.error } : {}) };
}
function listen(socket, type, listener) {
  if (typeof socket.on === 'function') socket.on(type, listener);
  else socket.addEventListener(type, event => listener(type === 'message' ? event.data : event));
}
async function defaultConnect(device) {
  const url = new URL('/agent/connect', device.gateway); url.protocol = 'wss:';
  const socket = new WebSocket(url, { headers: { authorization: 'Bearer ' + device.device_token },
    maxPayload: 256 * 1024, handshakeTimeout: 15000, followRedirects: false });
  return new Promise((resolve, reject) => {
    socket.once('open', () => resolve(socket));
    socket.once('error', () => { socket.terminate(); reject(new Error('relay_unavailable')); });
  });
}

export async function startAgent({ stateDir, gateway, binary, config, port = 8795, fetcher = fetch, connect = defaultConnect }) {
  const address = new URL(gateway);
  if (address.protocol !== 'https:' || address.username || address.password || address.pathname !== '/' || address.search || address.hash) throw new Error('gateway_requires_https_origin');
  gateway = address.origin;
  stateDir = resolve(stateDir); await mkdir(stateDir, { recursive: true, mode: 0o700 }); assertPrivate(await lstat(stateDir), true);
  stateDir = await realpath(stateDir);
  const unlock = await lockAgent(join(stateDir, 'agent.lock'));
  let client, app, socket, reconnect, heartbeat, stopped = false, backoff = 1000;
  try {
    const configText = await privateRead(resolve(config));
    const projects = JSON.parse(configText);
    if (!Array.isArray(projects.projects) || !projects.projects.length) throw new Error('projects_required');
    const scope = [];
    const ids = new Set();
    for (const project of projects.projects) {
      if (typeof project.id !== 'string' || !project.id || ids.has(project.id) || typeof project.root !== 'string' || resolve(project.root) !== project.root) throw new Error('invalid_project');
      ids.add(project.id);
      project.root = await realpath(project.root);
      if (project.root === stateDir || project.root.startsWith(stateDir + '/') || stateDir.startsWith(project.root === '/' ? '/' : project.root + '/')) throw new Error('private_state_overlaps_project');
      const info = await lstat(project.root); if (!info.isDirectory()) throw new Error('project_directory_required');
      scope.push({ id: project.id, root: project.root, dev: info.dev, ino: info.ino,
        write_enabled: project.write_enabled === true, execution_enabled: project.execution_enabled === true });
    }
    scope.sort((a, b) => a.id.localeCompare(b.id));
    const targets = new Map(scope.map(project => [project.id, project]));
    const scopeFile = join(stateDir, 'scope.json');
    const previousScope = await privateRead(scopeFile).catch(e => { if (e.code === 'ENOENT') return null; throw e; });
    const currentScope = json(scope);
    // Invalidate before replacing the config. A pending digest is never allowed
    // to acquire a different project root or capabilities after a restart.
    const recoveryGate = await ApprovalGate.open({ file: join(stateDir, 'approvals.json'), execute: async () => { throw new Error('startup_only'); } });
    if (previousScope !== currentScope) for (const job of await recoveryGate.list()) {
      if (job.status === 'pending') await recoveryGate.deny(job.operation_id, job.digest);
    }
    // Freeze the chosen projects for this run; external edits to the original
    // config do not silently widen an already running agent's published scope.
    const runtimeConfig = join(stateDir, 'projects.json');
    await replacePrivate(runtimeConfig, json(projects));
    await replacePrivate(scopeFile, currentScope);
    const deviceFile = join(stateDir, 'device.json');
    let device;
    try { device = JSON.parse(await privateRead(deviceFile)); }
    catch (error) {
      if (error.code !== 'ENOENT') throw error;
      const response = await fetcher(gateway + '/agent/enroll', { method: 'POST', headers: { 'content-type': 'application/json' }, body: '{}', redirect: 'error', signal: AbortSignal.timeout(15000) });
      if (response.status !== 201) throw new Error('device_enrollment_failed');
      device = { ...await response.json(), gateway };
      if (!UUID.test(device.owner_id || '') || !UUID.test(device.device_id || '') || !String(device.device_token).startsWith(device.owner_id + '.')) throw new Error('invalid_device_response');
      await exclusiveWrite(deviceFile, json(device));
    }
    if (device.gateway !== gateway || !UUID.test(device.owner_id) || !UUID.test(device.device_id)
      || !new RegExp('^' + device.owner_id + '\\.[a-f0-9]{64}$').test(device.device_token)) throw new Error('invalid_device_configuration');
    client = new McpClient({ binary: resolve(binary), config: runtimeConfig }); await client.initialize();
    const online = () => socket?.readyState === 1 && !stopped;
    const send = message => { if (online()) socket.send(json(message)); };
    const gate = await ApprovalGate.open({ file: join(stateDir, 'approvals.json'),
      execute: async request => {
        if (!online()) throw new Error('device_offline');
        const target = targets.get(request.arguments.project_id);
        if (!target || await realpath(target.root) !== target.root) throw new Error('project_scope_changed');
        const current = await lstat(target.root);
        if (current.dev !== target.dev || current.ino !== target.ino) throw new Error('project_scope_changed');
        return client.tool(request.tool, request.arguments);
      },
      onUpdate: job => send({ type: 'operation_update', job: publicJob(job) }) });
    async function pairing(action, code) {
      if (!['preview', 'approve'].includes(action) || !UUID.test(code || '')) throw new Error('invalid_pairing');
      const response = await fetcher(gateway + '/agent/pair-' + action, { method: 'POST', headers: { 'content-type': 'application/json', authorization: 'Bearer ' + device.device_token },
        body: json({ code }), redirect: 'error', signal: AbortSignal.timeout(15000) });
      if (!response.ok) throw new Error('pairing_rejected'); return response.json();
    }
    async function handle(raw) {
      let message;
      try {
        const text = typeof raw === 'string' ? raw : Buffer.from(raw).toString('utf8');
        if (Buffer.byteLength(text) > 128 * 1024) throw new Error('frame_too_large');
        message = JSON.parse(text);
        if (message.type !== 'request' || !UUID.test(message.id || '') || (!READS.has(message.tool) && !MUTATIONS.has(message.tool))) throw new Error('invalid_request');
        if (!message.arguments || typeof message.arguments !== 'object' || Array.isArray(message.arguments)) throw new Error('invalid_arguments');
        let result;
        if (MUTATIONS.has(message.tool)) {
          const operation = message.operation;
          if (!operation || operation.operation_id !== message.id || operation.tool !== message.tool || !isDeepStrictEqual(operation.arguments, message.arguments)) throw new Error('approval_binding');
          const target = targets.get(message.arguments.project_id);
          if (!target || (message.tool === 'command_run' ? !target.execution_enabled : !target.write_enabled)) {
            // Match the gateway's existing job so operation_get becomes terminal too.
            send({ type: 'operation_update', job: { operation_id: message.id, tool: message.tool,
              expires_at: operation.expires_at, status: 'failed', error: 'project_operation_not_enabled' } });
            throw new Error('project_operation_not_enabled');
          }
          const job = await gate.submit(operation);
          result = { ...publicJob(job), next_tool: 'operation_get' };
        } else result = await client.tool(message.tool, message.arguments);
        send({ type: 'response', id: message.id, result });
      } catch {
        if (UUID.test(message?.id || '')) send({ type: 'response', id: message.id, result: { content: [{ type: 'text', text: json({ error: 'local_request_rejected' }) }], isError: true } });
      }
    }
    app = await startApprovalApp({ stateDir, gate, pairing, online, port, targetFor: id => targets.get(id)?.root || null });
    async function attach() {
      if (stopped) return;
      try {
        const connected = await connect(device);
        if (stopped) { connected.close(); return; }
        socket = connected; backoff = 1000;
        listen(socket, 'message', raw => { handle(raw).catch(() => {}); });
        const lost = () => {
          if (socket !== connected) return;
          socket = null; clearInterval(heartbeat);
          if (!stopped) { reconnect = setTimeout(attach, backoff); reconnect.unref?.(); backoff = Math.min(backoff * 2, 30000); }
        };
        listen(socket, 'close', lost); listen(socket, 'error', lost);
        for (const job of gate.jobs.values()) send({ type: 'operation_update', job: publicJob(job) });
        if (typeof socket.ping === 'function') { heartbeat = setInterval(() => { if (online()) socket.ping(); }, 25000); heartbeat.unref(); }
      } catch {
        if (!stopped) { reconnect = setTimeout(attach, backoff); reconnect.unref?.(); backoff = Math.min(backoff * 2, 30000); }
      }
    }
    await attach();
    return { app, gate, pairing, online, ownerId: device.owner_id,
      async close() { if (stopped) return; stopped = true; clearTimeout(reconnect); clearInterval(heartbeat); socket?.close(1000, 'agent_stopped'); await app.close(); await client.close(); await unlock(); } };
  } catch (error) { stopped = true; socket?.close(); await app?.close(); await client?.close(); await unlock(); throw error; }
}
