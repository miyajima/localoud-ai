import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, stat, chmod, writeFile, symlink, link, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import test from "node:test";
import { ApprovalGate, digestRequest } from "../approval.mjs";

async function privateDir(t) {
  const root = await mkdtemp(join(tmpdir(), "localoud-approval-test-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const dir = join(root, "state");
  await mkdir(dir, { mode: 0o700 });
  await chmod(dir, 0o700);
  return dir;
}

function request(overrides = {}) {
  return {
    operation_id: randomUUID(),
    tool: "command_run",
    arguments: { program: "echo", args: ["hello"], nested: { z: 1, a: true } },
    expires_at: Date.now() + 60_000,
    ...overrides,
  };
}

test("submit persists pending without executing, then matching approval completes once", async (t) => {
  const dir = await privateDir(t);
  const file = join(dir, "approval.json");
  let executions = 0;
  const gate = await ApprovalGate.open({
    file,
    execute: async (value) => {
      executions += 1;
      assert.ok(Object.isFrozen(value));
      assert.ok(Object.isFrozen(value.arguments));
      return { exit_code: 0 };
    },
  });
  const original = request();
  const digest = digestRequest(original);
  const submitted = await gate.submit(original);

  assert.equal(submitted.status, "pending");
  assert.equal(executions, 0);
  assert.equal((await gate.get(original.operation_id)).digest, digest);
  assert.match(await readFile(file, "utf8"), /"pending"/u);
  assert.equal((await stat(dir)).mode & 0o777, 0o700);
  assert.equal((await stat(file)).mode & 0o777, 0o600);

  await assert.rejects(gate.approve(original.operation_id, "0".repeat(64)), /digest/u);
  assert.equal((await gate.get(original.operation_id)).status, "pending");
  const completed = await gate.approve(original.operation_id, digest);
  assert.equal(completed.status, "completed");
  assert.deepEqual(completed.result, { exit_code: 0 });
  assert.equal(executions, 1);
  assert.equal((await gate.approve(original.operation_id, digest)).status, "completed");
  assert.equal(executions, 1);
});

test("digest sorts nested object keys and operation IDs cannot be rebound", async (t) => {
  const dir = await privateDir(t);
  const gate = await ApprovalGate.open({ file: join(dir, "approval.json"), execute: async () => null });
  const first = request({ arguments: { z: 1, nested: { y: 2, x: 3 } } });
  const reordered = {
    expires_at: first.expires_at,
    arguments: { nested: { x: 3, y: 2 }, z: 1 },
    tool: first.tool,
    operation_id: first.operation_id,
  };

  assert.equal(digestRequest(first), digestRequest(reordered));
  await gate.submit(first);
  assert.equal((await gate.submit(reordered)).status, "pending");
  await assert.rejects(
    gate.submit({ ...reordered, arguments: { ...reordered.arguments, z: 2 } }),
    /operation_id|different request|digest/u,
  );
});

test("oversized canonical requests are rejected without creating an operation", async (t) => {
  const file = join(await privateDir(t), "approval.json");
  const gate = await ApprovalGate.open({ file, execute: async () => null });
  const oversized = request({ arguments: { payload: "x".repeat(64 * 1024) } });

  await assert.rejects(gate.submit(oversized), /64 KiB/u);
  assert.equal(await gate.get(oversized.operation_id), null);
  assert.deepEqual(await gate.list(), []);
});

test("the retention limit rejects new IDs without evicting existing approvals", async (t) => {
  const file = join(await privateDir(t), "approval.json");
  const gate = await ApprovalGate.open({ file, execute: async () => null });
  let firstId;

  for (let index = 0; index < 256; index += 1) {
    const submitted = await gate.submit(request({ arguments: { index } }));
    if (index === 0) firstId = submitted.operation_id;
  }

  await assert.rejects(gate.submit(request()), /full|limit/u);
  assert.equal((await gate.get(firstId)).status, "pending");
  assert.equal((await gate.list()).length, 256);
});

test("denial and expired requests never execute", async (t) => {
  const file = join(await privateDir(t), "approval.json");
  let now = 10_000;
  let executions = 0;
  const gate = await ApprovalGate.open({ file, now: () => now, execute: async () => { executions += 1; } });
  const deniedRequest = request({ expires_at: now + 1_000 });
  const denied = await gate.submit(deniedRequest);
  assert.equal((await gate.deny(deniedRequest.operation_id, denied.digest)).status, "denied");
  assert.equal((await gate.approve(deniedRequest.operation_id, denied.digest)).status, "denied");

  const expiredRequest = request({ expires_at: now + 1_000 });
  const pending = await gate.submit(expiredRequest);
  now += 1_001;
  assert.equal((await gate.approve(expiredRequest.operation_id, pending.digest)).status, "expired");
  assert.equal(executions, 0);
  await assert.rejects(gate.submit(request({ expires_at: now + 10 * 60_000 + 1 })), /10 minutes|deadline|expires/u);
});

test("concurrent approvals dispatch only once and failures expose no raw error", async (t) => {
  const file = join(await privateDir(t), "approval.json");
  let executions = 0;
  const gate = await ApprovalGate.open({
    file,
    execute: async () => {
      executions += 1;
      throw new Error("fixture secret and stack must not escape");
    },
  });
  const req = request();
  const pending = await gate.submit(req);
  const results = await Promise.all([
    gate.approve(req.operation_id, pending.digest),
    gate.approve(req.operation_id, pending.digest),
  ]);

  assert.equal(executions, 1);
  assert.ok(results.every((job) => job.status === "failed"));
  assert.equal(results[0].error, "execution_failed");
  assert.equal("result" in results[0], false);
  assert.equal(JSON.stringify(results).includes("fixture secret"), false);
  assert.equal((await gate.list()).length, 0);
});

test("a persisted running job recovers as indeterminate and is never replayed", async (t) => {
  const file = join(await privateDir(t), "approval.json");
  let executeCalls = 0;
  let releaseExecution;
  let started;
  const executionStarted = new Promise((resolve) => { started = resolve; });
  const originalGate = await ApprovalGate.open({
    file,
    execute: async () => {
      executeCalls += 1;
      started();
      return new Promise((resolve) => { releaseExecution = resolve; });
    },
  });
  const req = request();
  const submitted = await originalGate.submit(req);
  const approving = originalGate.approve(req.operation_id, submitted.digest);
  await executionStarted;

  const recoveredGate = await ApprovalGate.open({ file, execute: async () => { executeCalls += 1; } });
  assert.equal((await recoveredGate.get(req.operation_id)).status, "indeterminate");
  assert.equal(executeCalls, 1);
  releaseExecution({ exit_code: 0 });
  await approving;
});

test("unsafe directories, symlinked files, and hardlinked files are rejected", async (t) => {
  const root = await privateDir(t);
  const unsafe = join(root, "unsafe");
  await mkdir(unsafe, { mode: 0o755 });
  await chmod(unsafe, 0o755);
  await assert.rejects(
    ApprovalGate.open({ file: join(unsafe, "approval.json"), execute: async () => null }),
    /permission|private|mode/u,
  );
  assert.equal((await stat(unsafe)).mode & 0o777, 0o755);

  const target = join(root, "target.json");
  await writeFile(target, JSON.stringify({ version: 1, jobs: [] }), { mode: 0o600 });
  const alias = join(root, "alias.json");
  await symlink(target, alias);
  await assert.rejects(ApprovalGate.open({ file: alias, execute: async () => null }), /symlink|symbolic/u);

  const hardlinked = join(root, "hardlinked.json");
  await link(target, hardlinked);
  await assert.rejects(ApprovalGate.open({ file: target, execute: async () => null }), /link|hard/u);
});

test("persistence failure before dispatch leaves execution untouched", async (t) => {
  const root = await privateDir(t);
  const dir = join(root, "replaceable");
  const file = join(dir, "approval.json");
  let executions = 0;
  const gate = await ApprovalGate.open({ file, execute: async () => { executions += 1; } });
  const req = request();
  const pending = await gate.submit(req);
  await rm(dir, { recursive: true });
  await writeFile(dir, "not a directory");

  await assert.rejects(gate.approve(req.operation_id, pending.digest));
  assert.equal(executions, 0);
});

test("MCP command results between 64 and 128 KiB complete and survive reopening", async (t) => {
  const file = join(await privateDir(t), "approval.json");
  const stdout = '"'.repeat(16 * 1024);
  const stderr = '"'.repeat(16 * 1024);
  const result = {
    content: [{ type: "text", text: `exit_code: 0\nstdout: ${stdout}\nstderr: ${stderr}` }],
    structuredContent: { exit_code: 0 },
  };
  const outerResultBytes = Buffer.byteLength(JSON.stringify(result), "utf8");
  assert.ok(outerResultBytes > 64 * 1024);
  assert.ok(outerResultBytes < 128 * 1024);

  const gate = await ApprovalGate.open({ file, execute: async () => result });
  const req = request();
  const pending = await gate.submit(req);
  const completed = await gate.approve(req.operation_id, pending.digest);

  assert.equal(completed.status, "completed");
  assert.deepEqual(completed.result, result);

  const reopened = await ApprovalGate.open({ file, execute: async () => assert.fail("completed jobs must not replay") });
  const restored = await reopened.get(req.operation_id);
  assert.equal(restored.status, "completed");
  assert.deepEqual(restored.result, result);
});

test("results larger than 128 KiB fail safely without retaining their payload", async (t) => {
  const file = join(await privateDir(t), "approval.json");
  const secretPayload = "x".repeat(130 * 1024);
  const gate = await ApprovalGate.open({
    file,
    execute: async () => ({ content: [{ type: "text", text: secretPayload }] }),
  });
  const req = request();
  const pending = await gate.submit(req);
  const failed = await gate.approve(req.operation_id, pending.digest);

  assert.equal(failed.status, "failed");
  assert.equal(failed.error, "execution_failed");
  assert.equal("result" in failed, false);
  assert.equal(JSON.stringify(failed).includes(secretPayload), false);
  const reopened = await ApprovalGate.open({ file, execute: async () => assert.fail("failed jobs must not replay") });
  assert.equal((await reopened.get(req.operation_id)).error, "execution_failed");
});
