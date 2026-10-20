import { test } from 'node:test';
import assert from 'node:assert/strict';
import { presentApprovals } from '../presentation.mjs';

test('active requests are ordered for approval while old outcomes stay in history', () => {
  const expired = { operation_id: 'old', status: 'pending', created_at: 1, expires_at: 9 };
  const newer = { operation_id: 'newer', status: 'pending', created_at: 4, expires_at: 50 };
  const older = { operation_id: 'older', status: 'pending', created_at: 3, expires_at: 50 };
  const completed = { operation_id: 'done', status: 'completed', created_at: 2, expires_at: 50 };
  const view = presentApprovals([expired, newer, completed, older], 10);
  assert.deepEqual(view.pending.map(job => job.operation_id), ['older', 'newer']);
  assert.deepEqual(view.history.map(job => [job.operation_id, job.status]), [['done', 'completed'], ['old', 'expired']]);
  assert.equal(expired.status, 'pending', 'display classification must not mutate persisted receipts');
});

test('only a pending request strictly before its finite deadline gets an approval action', () => {
  const jobs = [
    { operation_id: 'boundary', status: 'pending', expires_at: 10 },
    { operation_id: 'invalid', status: 'pending' },
    { operation_id: 'running', status: 'running', expires_at: 100 },
    { operation_id: 'done', status: 'completed', expires_at: 100 },
    { operation_id: 'active', status: 'pending', expires_at: 11 },
  ];
  const view = presentApprovals(jobs, 10);
  assert.deepEqual(view.pending.map(job => job.operation_id), ['active']);
  assert.equal(view.history.length, 4);
});

test('a terminal receipt cannot regain an approval action on subsequent refresh', () => {
  const job = { operation_id: 'one', status: 'pending', expires_at: 100 };
  assert.equal(presentApprovals([job], 10).pending.length, 1);
  assert.equal(presentApprovals([{ ...job, status: 'completed' }], 11).pending.length, 0);
  assert.equal(presentApprovals([{ ...job, status: 'expired' }], 11).pending.length, 0);
});

test('an expired approval response is reported as not executed, never as a recorded result', async () => {
  const { approvalNotice } = await import('../presentation.mjs');
  assert.equal(approvalNotice('expired').outcome, 'not_executed');
  assert.equal(approvalNotice('denied').outcome, 'not_executed');
  assert.equal(approvalNotice('completed').outcome, 'recorded');
  assert.equal(approvalNotice('running').outcome, 'running');
  assert.equal(approvalNotice('indeterminate').outcome, 'unknown');
  assert.equal(approvalNotice('unexpected').outcome, 'unknown');
});
