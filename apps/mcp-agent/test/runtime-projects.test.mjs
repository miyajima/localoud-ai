import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, realpath, rm } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { EventEmitter } from "node:events";
import { randomUUID, createHash } from "node:crypto";
import { AgentController } from "../src/controller.mjs";
import { startAgent } from "../../../plugins/localoud-mcp/agent/runtime.mjs";
const binary = process.env.LOCALOUD_MCP_TEST_BINARY;
function data(result) {
  assert.notEqual(result.isError, true, JSON.stringify(result));
  return JSON.parse(result.content[0].text);
}
class Socket extends EventEmitter {
  readyState = 1;
  updates = new Map();
  send(text) {
    const message = JSON.parse(text);
    if (message.type === "operation_update")
      this.updates.set(message.job.operation_id, message.job);
    this.emit("sent", message);
  }
  close() {
    this.readyState = 3;
    this.emit("close");
  }
}
async function fixture(t) {
  const dir = await realpath(
      await mkdtemp(join(tmpdir(), "localoud-runtime-multi-")),
    ),
    roots = [join(dir, "edit-only"), join(dir, "exec-only")];
  for (const root of roots) await mkdir(root);
  let socket,
    enrolls = 0;
  const owner = randomUUID();
  const c = new AgentController({
    dataDir: join(dir, "private"),
    binary,
    platform: "darwin",
    startAgent: (options) =>
      startAgent({
        ...options,
        fetcher: async () => {
          enrolls++;
          return new Response(
            JSON.stringify({
              owner_id: owner,
              device_id: randomUUID(),
              device_token: owner + "." + "a".repeat(64),
            }),
            { status: 201 },
          );
        },
        connect: async () => (socket = new Socket()),
      }),
  });
  await c.initialize();
  await c.addFolders(roots);
  const [a, b] = c.state().projects;
  await c.updateProject({
    id: a.id,
    writeEnabled: true,
    executionEnabled: false,
  });
  await c.updateProject({
    id: b.id,
    writeEnabled: false,
    executionEnabled: true,
  });
  await c.saveProjects();
  await c.start();
  t.after(async () => {
    await c.stop();
    await rm(dir, { recursive: true, force: true });
  });
  function request(tool, args) {
    const id = randomUUID(),
      operation = {
        operation_id: id,
        tool,
        arguments: args,
        expires_at: Date.now() + 60000,
      };
    return new Promise((resolve, reject) => {
      const current = socket;
      const timer = setTimeout(() => {
        current.off("sent", onMessage);
        reject(Error("request_timeout"));
      }, 5000);
      function onMessage(message) {
        if (message.type === "response" && message.id === id) {
          clearTimeout(timer);
          current.off("sent", onMessage);
          resolve({ id, result: message.result });
        }
      }
      current.on("sent", onMessage);
      current.emit(
        "message",
        JSON.stringify({
          type: "request",
          id,
          tool,
          arguments: args,
          operation,
        }),
      );
    });
  }
  return {
    c,
    roots,
    a,
    b,
    request,
    updates: (id) => socket.updates.get(id),
    enrolls: () => enrolls,
  };
}
test(
  "real runtime denies per-project disabled operations before approval and executes enabled ones only after approval",
  { skip: !binary },
  async (t) => {
    const f = await fixture(t);
    const list = data((await f.request("project_list", {})).result).projects;
    assert.deepEqual(
      list.map((p) => [p.file_edit_enabled, p.command_execution_enabled]),
      [
        [true, false],
        [false, true],
      ],
    );
    const deniedEdit = await f.request("file_create", {
      project_id: f.b.id,
      path: "blocked.txt",
      content: "NO",
    });
    assert.equal(deniedEdit.result.isError, true);
    assert.equal(f.updates(deniedEdit.id)?.status, "failed");
    assert.equal(f.c.agent.gate.jobs.has(deniedEdit.id), false);
    await assert.rejects(readFile(join(f.roots[1], "blocked.txt")), {
      code: "ENOENT",
    });
    const deniedCommand = await f.request("command_run", {
      project_id: f.a.id,
      program: "/usr/bin/touch",
      args: ["blocked-command"],
    });
    assert.equal(deniedCommand.result.isError, true);
    assert.equal(f.c.agent.gate.jobs.has(deniedCommand.id), false);
    await assert.rejects(readFile(join(f.roots[0], "blocked-command")), {
      code: "ENOENT",
    });
    const created = await f.request("file_create", {
      project_id: f.a.id,
      path: "probe.txt",
      content: "BEFORE",
    });
    assert.equal(created.result.status, "pending_approval");
    await assert.rejects(readFile(join(f.roots[0], "probe.txt")), {
      code: "ENOENT",
    });
    let job = await f.c.agent.gate.get(created.id);
    data((await f.c.agent.gate.approve(job.operation_id, job.digest)).result);
    assert.equal(
      await readFile(join(f.roots[0], "probe.txt"), "utf8"),
      "BEFORE",
    );
    const edited = await f.request("file_edit", {
      project_id: f.a.id,
      path: "probe.txt",
      expected_sha256: createHash("sha256").update("BEFORE").digest("hex"),
      old_text: "BEFORE",
      new_text: "AFTER",
    });
    assert.equal(
      await readFile(join(f.roots[0], "probe.txt"), "utf8"),
      "BEFORE",
    );
    job = await f.c.agent.gate.get(edited.id);
    data((await f.c.agent.gate.approve(job.operation_id, job.digest)).result);
    assert.equal(
      await readFile(join(f.roots[0], "probe.txt"), "utf8"),
      "AFTER",
    );
    const command = await f.request("command_run", {
      project_id: f.b.id,
      program: "/usr/bin/printf",
      args: ["MULTI_PROJECT_COMMAND_OK"],
    });
    assert.equal(command.result.status, "pending_approval");
    job = await f.c.agent.gate.get(command.id);
    const result = data(
      (await f.c.agent.gate.approve(job.operation_id, job.digest)).result,
    );
    assert.equal(result.exit_code, 0);
    assert.equal(result.stdout, "MULTI_PROJECT_COMMAND_OK");
  },
);
test(
  "permission change denies previous pending approvals and keeps device pairing",
  { skip: !binary },
  async (t) => {
    const f = await fixture(t);
    const pending = await f.request("file_create", {
      project_id: f.a.id,
      path: "stale.txt",
      content: "NEVER",
    });
    assert.equal(pending.result.status, "pending_approval");
    await f.c.updateProject({
      id: f.a.id,
      writeEnabled: false,
      executionEnabled: false,
    });
    await f.c.saveProjects();
    await f.c.start();
    assert.equal(f.enrolls(), 1);
    assert.equal((await f.c.agent.gate.get(pending.id)).status, "denied");
    await assert.rejects(readFile(join(f.roots[0], "stale.txt")), {
      code: "ENOENT",
    });
  },
);
