import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { startApprovalApp } from '../app.mjs';
import { ApprovalGate } from '../approval.mjs';
import { randomUUID } from 'node:crypto';
import { get } from 'node:http';

test('approval UI requires a private session, same-origin CSRF and exact operation digest', async t => {
  const dir = await mkdtemp(join(tmpdir(), 'localoud-approval-ui-'));
  let executed = 0;
  const gate = await ApprovalGate.open({ file: join(dir, 'approvals.json'), execute: async () => { executed++; return { ok: true }; } });
  const app = await startApprovalApp({ stateDir: dir, gate, port: 0, pairing: async () => ({}) });
  t.after(async () => { await app.close(); await rm(dir, { recursive: true }); });
  assert.equal((await fetch(app.url + '/api/state')).status, 401);
  const rebound = await new Promise((resolve, reject) => {
    const req = get(app.url, { headers: { host: 'evil.test' } }, res => { res.resume(); resolve(res.statusCode); }); req.on('error', reject);
  });
  assert.equal(rebound, 403);
  const session = await readFile(join(dir, 'ui-token'), 'utf8');
  const login = await fetch(app.url + '/session', { method: 'POST', headers: { origin: app.url,
    'content-type': 'application/x-www-form-urlencoded' }, body: new URLSearchParams({ token: session }).toString(), redirect: 'manual' });
  assert.equal(login.status, 303);
  const cookie = login.headers.get('set-cookie').split(';')[0];
  const html = await (await fetch(app.url, { headers: { cookie } })).text();
  const csrf = html.match(/data-csrf="([a-f0-9]+)"/)[1];
  const request = { operation_id: randomUUID(), tool: 'command_run', arguments: { program: '/bin/echo', args: ['APPROVED'] }, expires_at: Date.now() + 30000 };
  const job = await gate.submit(request);
  async function approve(origin, digest = job.digest, csrfValue = csrf) {
    return fetch(app.url + '/api/approve', { method: 'POST', headers: { cookie, origin, 'content-type': 'application/json', 'x-localoud-csrf': csrfValue },
      body: JSON.stringify({ operation_id: job.operation_id, digest }) });
  }
  assert.equal((await approve('https://evil.test')).status, 403);
  assert.equal((await approve(app.url, job.digest, 'wrong')).status, 403);
  assert.equal((await approve(app.url, '0'.repeat(64))).status, 400);
  assert.equal(executed, 0);
  assert.equal((await approve(app.url)).status, 200);
  assert.equal(executed, 1);
  assert.equal((await approve(app.url)).status, 200);
  assert.equal(executed, 1);
});

test('expired requests remain visible as non-actionable history after app restart', async t => {
  const dir = await mkdtemp(join(tmpdir(), 'localoud-approval-history-'));
  let now = Date.now(), executed = 0;
  const gate = await ApprovalGate.open({file:join(dir,'approvals.json'),now:()=>now,execute:async()=>{executed++;return {ok:true}}});
  const job = await gate.submit({operation_id:randomUUID(),tool:'file_edit',arguments:{project_id:'personal',path:'probe.txt',old_text:'PRIVATE_OLD',new_text:'PRIVATE_NEW'},expires_at:now+1000});
  now += 1001;
  const reopened = await ApprovalGate.open({file:join(dir,'approvals.json'),now:()=>now,execute:async()=>{executed++;return {ok:true}}});
  const app = await startApprovalApp({stateDir:dir,gate:reopened,port:0,pairing:async()=>({}),targetFor:()=>'/current/project'});
  t.after(async()=>{await app.close();await rm(dir,{recursive:true,force:true})});
  const token = await readFile(join(dir,'ui-token'),'utf8');
  const state = await (await fetch(app.url+'/api/state',{headers:{cookie:'localoud_session='+token}})).json();
  assert.deepEqual(state.jobs,[]);
  assert.deepEqual(state.history.map(x=>({operation_id:x.operation_id,status:x.status})),[{operation_id:job.operation_id,status:'expired'}]);
  assert.equal(JSON.stringify(state.history).includes('PRIVATE_'),false);
  assert.equal(JSON.stringify(state.history).includes('/current/project'),false);
  assert.equal(executed,0);
});
