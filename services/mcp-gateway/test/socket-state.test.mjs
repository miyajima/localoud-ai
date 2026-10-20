import { test } from 'node:test';
import assert from 'node:assert/strict';
import { rejectSocketPending } from '../src/socket-state.js';

test('a delayed close fails only requests belonging to that socket', () => {
  const oldSocket = {}, currentSocket = {};
  const rejected = [];
  const pending = new Map([
    ['old', { socket: oldSocket, reject: () => rejected.push('old'), timer: setTimeout(() => {}, 1000) }],
    ['current', { socket: currentSocket, reject: () => rejected.push('current'), timer: setTimeout(() => {}, 1000) }],
  ]);
  rejectSocketPending(pending, oldSocket);
  assert.deepEqual(rejected, ['old']); assert.equal(pending.has('current'), true); assert.equal(pending.has('old'), false);
  rejectSocketPending(pending, oldSocket);
  assert.deepEqual(rejected, ['old']);
  rejectSocketPending(pending, currentSocket);
  assert.deepEqual(rejected, ['old', 'current']); assert.equal(pending.size, 0);
});
