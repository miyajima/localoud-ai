import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { McpClient } from '../mcp.mjs';

test('stdio child receives no gateway credentials and responses match out-of-order IDs', async t => {
  const dir = await mkdtemp(join(tmpdir(), 'localoud-mcp-child-'));
  const fake = join(dir, 'fake.mjs');
  await writeFile(fake, `import{createInterface}from'node:readline';for await(const l of createInterface({input:process.stdin})){const q=JSON.parse(l);if(q.id===undefined)continue;setTimeout(()=>console.log(JSON.stringify({jsonrpc:'2.0',id:q.id,result:{content:[{type:'text',text:JSON.stringify({name:q.params?.name,keyPresent:!!process.env.CONTROL_PLANE_API_KEY})}]}})),q.params?.name==='slow'?30:1)}`);
  process.env.CONTROL_PLANE_API_KEY = 'FIXTURE_NOT_A_REAL_KEY';
  const client = new McpClient({ binary: process.execPath, args: [fake] });
  t.after(async () => { delete process.env.CONTROL_PLANE_API_KEY; await client.close(); await rm(dir, { recursive: true }); });
  await client.initialize();
  const [a, b] = await Promise.all([client.tool('slow', {}), client.tool('fast', {})]);
  assert.deepEqual(JSON.parse(a.content[0].text), { name: 'slow', keyPresent: false });
  assert.deepEqual(JSON.parse(b.content[0].text), { name: 'fast', keyPresent: false });
});
