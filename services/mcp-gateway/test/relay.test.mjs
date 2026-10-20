import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Miniflare } from 'miniflare';
import { createHash, randomBytes } from 'node:crypto';
import { mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { startAgent } from '../../../plugins/localoud-mcp/agent/runtime.mjs';

test('real Rust sidecars stay owner-isolated; edits and commands require exact local approval', async t => {
  const origin = 'https://localoud-relay.test';
  const mf = new Miniflare({ modules: true, scriptPath: new URL('../dist/worker.js', import.meta.url).pathname,
    compatibilityDate: '2026-07-01', compatibilityFlags: ['nodejs_compat', 'global_fetch_strictly_public'],
    kvNamespaces: ['OAUTH_KV'], durableObjects: { OWNERS: { className: 'Owner', useSQLite: true }, LOGINS: { className: 'Login', useSQLite: true }, LIMITS: { className: 'RateLimit', useSQLite: true } }, bindings: { PUBLIC_ORIGIN: origin } });
  const fetcher = (url, opts) => mf.dispatchFetch(url, opts);
  const root = await mkdtemp(join(tmpdir(), 'localoud-relay-e2e-'));
  const agents = [];
  t.after(async () => { for (const agent of agents) await agent.close(); await mf.dispose(); await rm(root, { recursive: true }); });
  async function makeAgent(name) {
    const folder = join(root, name); await (await import('node:fs/promises')).mkdir(folder, { mode: 0o700 });
    await writeFile(join(folder, 'probe.txt'), name + '\n');
    const config = join(root, name + '.json');
    await writeFile(config, JSON.stringify({ projects: [{ id: 'fixture', name, root: folder, write_enabled: true, execution_enabled: true }] }), { mode: 0o600 });
    const agent = await startAgent({ stateDir: join(root, name + '-state'), gateway: origin,
      binary: resolve('../../target/release/localoud-mcp'), config, port: 0, fetcher,
      connect: async device => { const r = await fetcher(origin + '/agent/connect', { headers: { Upgrade: 'websocket', authorization: 'Bearer ' + device.device_token } });
        assert.equal(r.status, 101); r.webSocket.accept(); return r.webSocket; } });
    agents.push(agent); return { agent, folder, config, stateDir: join(root, name + '-state') };
  }
  async function authorize(agent) {
    const post = (path, data) => fetcher(origin + path, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(data) });
    const client = await (await post('/oauth/register', { client_name: 'Relay fixture', redirect_uris: ['https://client.test/cb'], token_endpoint_auth_method: 'none', grant_types: ['authorization_code'], response_types: ['code'] })).json();
    const verifier = randomBytes(32).toString('base64url');
    const q = new URLSearchParams({ client_id: client.client_id, redirect_uri: 'https://client.test/cb', response_type: 'code', code_challenge: createHash('sha256').update(verifier).digest('base64url'), code_challenge_method: 'S256', resource: origin + '/mcp', scope: 'files:read files:write commands:execute' });
    const response = await fetcher(origin + '/oauth/authorize?' + q); const html = await response.text();
    const code = html.match(/data-pair-code="([a-f0-9-]+)"/)[1], handle = html.match(/name="handle" value="([^"]+)"/)[1];
    await agent.pairing('preview', code); await agent.pairing('approve', code);
    const cookie = response.headers.getSetCookie().map(s => s.split(';')[0]).join('; ');
    const form = new URLSearchParams({ login: code, handle, decision: 'approve' });
    for (const scope of ['files:read', 'files:write', 'commands:execute']) form.append('scope', scope);
    const finish = await fetcher(origin + '/oauth/finish', { method: 'POST', headers: { cookie, origin, 'content-type': 'application/x-www-form-urlencoded' }, body: form.toString(), redirect: 'manual' });
    assert.equal(finish.status, 302);
    const tokenResponse = await fetcher(origin + '/oauth/token', { method: 'POST', headers: { 'content-type': 'application/x-www-form-urlencoded' }, body: new URLSearchParams({ grant_type: 'authorization_code', code: new URL(finish.headers.get('location')).searchParams.get('code'), client_id: client.client_id, redirect_uri: 'https://client.test/cb', code_verifier: verifier, resource: origin + '/mcp' }).toString() });
    assert.equal(tokenResponse.status, 200); return (await tokenResponse.json()).access_token;
  }
  let rpcId = 0;
  async function tool(token, name, args) {
    if (['file_edit', 'file_create', 'command_run'].includes(name)) args = { request_id: crypto.randomUUID(), ...args };
    const r = await fetcher(origin + '/mcp', { method: 'POST', headers: { 'content-type': 'application/json', authorization: 'Bearer ' + token }, body: JSON.stringify({ jsonrpc: '2.0', id: ++rpcId, method: 'tools/call', params: { name, arguments: args } }) });
    assert.equal(r.status, 200); const response = await r.json(); assert.ok(response.result, JSON.stringify(response));
    return JSON.parse(response.result.content[0].text);
  }
  const a = await makeAgent('owner-a'), b = await makeAgent('owner-b');
  const ta = await authorize(a.agent), tb = await authorize(b.agent);
  const first = await tool(ta, 'file_read', { project_id: 'fixture', path: 'probe.txt' });
  assert.equal(first.lines[0].text, 'owner-a');
  assert.equal((await tool(tb, 'file_read', { project_id: 'fixture', path: 'probe.txt' })).lines[0].text, 'owner-b');
  const editArgs = { request_id: crypto.randomUUID(), project_id: 'fixture', path: 'probe.txt', expected_sha256: first.sha256, old_text: 'owner-a', new_text: 'approved-a' };
  const edit = await tool(ta, 'file_edit', editArgs);
  assert.equal(edit.status, 'pending_approval');
  const repeated = await tool(ta, 'file_edit', editArgs);
  assert.equal(repeated.operation_id, edit.operation_id, 'transport retry must return the same operation');
  assert.equal((await a.agent.gate.list()).length, 1);
  assert.equal((await tool(ta, 'file_edit', { ...editArgs, new_text: 'substituted' })).error, 'request_id_conflict');
  assert.equal(await readFile(join(a.folder, 'probe.txt'), 'utf8'), 'owner-a\n');
  const hidden = await tool(tb, 'operation_get', { operation_id: edit.operation_id });
  assert.equal(hidden.error, 'operation_not_found');
  const job = await a.agent.gate.get(edit.operation_id);
  await assert.rejects(a.agent.gate.approve(edit.operation_id, '0'.repeat(64)));
  await a.agent.gate.approve(edit.operation_id, job.digest);
  assert.equal(await readFile(join(a.folder, 'probe.txt'), 'utf8'), 'approved-a\n');
  assert.equal(await readFile(join(b.folder, 'probe.txt'), 'utf8'), 'owner-b\n');
  const command = await tool(ta, 'command_run', { project_id: 'fixture', program: '/bin/sh', args: ['-c', "printf 'APPROVED_COMMAND\\n' > command.txt; cat command.txt"], timeout_ms: 5000 });
  assert.equal(command.status, 'pending_approval');
  await assert.rejects(readFile(join(a.folder, 'command.txt')), { code: 'ENOENT' });
  const commandJob = await a.agent.gate.get(command.operation_id);
  await a.agent.gate.approve(command.operation_id, commandJob.digest);
  assert.equal(await readFile(join(a.folder, 'command.txt'), 'utf8'), 'APPROVED_COMMAND\n');
  const receipt = JSON.parse((await a.agent.gate.get(command.operation_id)).result.content[0].text);
  assert.equal(receipt.exit_code, 0);
  let cloudReceipt;
  for (let attempt = 0; attempt < 20; attempt++) {
    cloudReceipt = await tool(ta, 'operation_get', { operation_id: command.operation_id });
    if (cloudReceipt.status === 'completed') break;
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  assert.equal(cloudReceipt.status, 'completed');
  assert.equal(JSON.parse(cloudReceipt.result.content[0].text).exit_code, 0);
  const concurrentArgs = { request_id: crypto.randomUUID(), project_id: 'fixture', program: '/bin/echo', args: ['ONCE'] };
  const concurrent = await Promise.all([tool(ta, 'command_run', concurrentArgs), tool(ta, 'command_run', concurrentArgs)]);
  assert.equal(concurrent[0].operation_id, concurrent[1].operation_id);
  const concurrentJob = await a.agent.gate.get(concurrent[0].operation_id);
  await a.agent.gate.deny(concurrentJob.operation_id, concurrentJob.digest);
  const denied = await tool(ta, 'command_run', { project_id: 'fixture', program: '/bin/touch', args: ['DENIED.txt'] });
  const deniedJob = await a.agent.gate.get(denied.operation_id); await a.agent.gate.deny(denied.operation_id, deniedJob.digest);
  await assert.rejects(readFile(join(a.folder, 'DENIED.txt')), { code: 'ENOENT' });
  const stale = await tool(ta, 'file_create', { project_id: 'fixture', path: 'REBOUND.txt', content: 'MUST_NOT_WRITE' });
  const staleJob = await a.agent.gate.get(stale.operation_id);
  await a.agent.close();
  await writeFile(a.config, JSON.stringify({ projects: [{ id: 'fixture', name: 'changed', root: b.folder, write_enabled: true, execution_enabled: true }] }), { mode: 0o600 });
  const replacement = await startAgent({ stateDir: a.stateDir, gateway: origin, binary: resolve('../../target/release/localoud-mcp'), config: a.config, port: 0, fetcher,
    connect: async device => { const r = await fetcher(origin + '/agent/connect', { headers: { Upgrade: 'websocket', authorization: 'Bearer ' + device.device_token } }); assert.equal(r.status, 101); r.webSocket.accept(); return r.webSocket; } });
  agents.push(replacement);
  const staleResult = await replacement.gate.approve(stale.operation_id, staleJob.digest);
  assert.equal(staleResult.status, 'denied', 'config changes must invalidate earlier approvals');
  await assert.rejects(readFile(join(b.folder, 'REBOUND.txt')), { code: 'ENOENT' });
});
