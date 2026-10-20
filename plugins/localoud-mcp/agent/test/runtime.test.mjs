import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { startAgent } from '../runtime.mjs';

test('private agent state must be disjoint from every published root, even read-only roots', async t => {
  const root = await mkdtemp(join(tmpdir(), 'localoud-private-scope-'));
  t.after(() => rm(root, { recursive: true }));
  let enrollments = 0;
  const forbiddenFetch = async () => { enrollments++; throw new Error('must_not_enroll'); };
  const publicRoot = join(root, 'public'); await mkdir(publicRoot, { mode: 0o700 });
  const config = join(root, 'projects.json');
  await writeFile(config, JSON.stringify({ projects: [{ id: 'read-only', name: 'read-only', root: publicRoot }] }), { mode: 0o600 });
  const stateDir = join(publicRoot, 'state');
  await assert.rejects(startAgent({ stateDir, config, binary: '/does/not/exist', gateway: 'https://gateway.test', fetcher: forbiddenFetch, port: 0 }), /private_state_overlaps_project/);
  assert.equal(enrollments, 0);
  await assert.rejects(readFile(join(stateDir, 'ui-token')), { code: 'ENOENT' });
  await assert.rejects(readFile(join(stateDir, 'device.json')), { code: 'ENOENT' });
  const privateParent = join(root, 'private-parent'); await mkdir(join(privateParent, 'project'), { recursive: true, mode: 0o700 });
  await writeFile(config, JSON.stringify({ projects: [{ id: 'read-only', root: join(privateParent, 'project') }] }), { mode: 0o600 });
  await assert.rejects(startAgent({ stateDir: privateParent, config, binary: '/does/not/exist', gateway: 'https://gateway.test', fetcher: forbiddenFetch, port: 0 }), /private_state_overlaps_project/);
  assert.equal(enrollments, 0);
});
