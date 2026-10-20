import { test } from "node:test";
import assert from "node:assert/strict";
import {
  mkdtemp,
  mkdir,
  readFile,
  writeFile,
  lstat,
  symlink,
  rm,
  realpath,
} from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { AgentController } from "../src/controller.mjs";

async function configure(controller, options) {
  const id = controller.state().projects[0]?.id;
  await controller.updateProject({ id, ...options });
  return controller.saveProjects();
}
async function fixture(t) {
  const dir = await realpath(await mkdtemp(join(tmpdir(), "localoud-app-")));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const project = join(dir, "project");
  await mkdir(project, { mode: 0o700 });
  const starts = [],
    stops = [];
  let online = true;
  const controller = new AgentController({
    dataDir: join(dir, "private"),
    binary: "/bundled/localoud-mcp",
    platform: "darwin",
    arch: "arm64",
    startAgent: async (options) => {
      starts.push(options);
      return {
        online: () => online,
        app: { url: "http://127.0.0.1:50000" },
        close: async () => stops.push(true),
        device_token: "NEVER_RENDER",
      };
    },
  });
  await controller.initialize();
  return {
    dir,
    project,
    controller,
    starts,
    stops,
    setOnline: (v) => {
      online = v;
    },
  };
}
test("first launch exposes no project and starts nothing", async (t) => {
  const f = await fixture(t);
  assert.equal(f.controller.state().projects[0]?.root ?? null, null);
  assert.equal(f.controller.state().configured, false);
  assert.equal(f.starts.length, 0);
  await assert.rejects(f.controller.start(), /setup_required/);
});
test("native selection requires explicit permission choices; config is private and start uses bundled binary", async (t) => {
  const f = await fixture(t);
  await f.controller.addFolders([f.project]);
  assert.equal(f.controller.state().configured, false);
  await configure(f.controller, {
    writeEnabled: true,
    executionEnabled: false,
  });
  const config = JSON.parse(
    await readFile(join(f.dir, "private", "projects.json"), "utf8"),
  );
  assert.equal(config.projects[0].root, f.project);
  assert.equal(config.projects[0].write_enabled, true);
  assert.equal(config.projects[0].execution_enabled, false);
  assert.equal(
    (await lstat(join(f.dir, "private", "projects.json"))).mode & 0o777,
    0o600,
  );
  await f.controller.start();
  assert.equal(f.starts[0].binary, "/bundled/localoud-mcp");
  assert.equal(f.starts[0].port, 0);
  assert.equal(f.controller.state().online, true);
  assert.ok(!JSON.stringify(f.controller.state()).includes("NEVER_RENDER"));
  await f.controller.stop();
  assert.equal(f.stops.length, 1);
});
test("renderer cannot supply a path or implicit capability values", async (t) => {
  const f = await fixture(t);
  await f.controller.addFolders([f.project]);
  await assert.rejects(
    configure(f.controller, {
      root: "/other",
      writeEnabled: true,
      executionEnabled: true,
    }),
    /invalid_options/,
  );
  await assert.rejects(
    configure(f.controller, { writeEnabled: "true", executionEnabled: false }),
    /invalid_options/,
  );
  assert.equal(f.controller.state().configured, false);
});
test("private state cannot be published even by choosing its ancestor", async (t) => {
  const f = await fixture(t);
  await assert.rejects(
    f.controller.addFolders([f.dir]),
    /private_state_overlaps_project/,
  );
  assert.equal(f.starts.length, 0);
  await assert.rejects(readFile(join(f.dir, "private", "projects.json")), {
    code: "ENOENT",
  });
});
test("concurrent start calls launch only once and state reflects disconnect", async (t) => {
  const f = await fixture(t);
  await f.controller.addFolders([f.project]);
  await configure(f.controller, {
    writeEnabled: false,
    executionEnabled: true,
  });
  await Promise.all([f.controller.start(), f.controller.start()]);
  assert.equal(f.starts.length, 1);
  f.setOnline(false);
  assert.equal(f.controller.state().online, false);
  await f.controller.stop();
});
test("changing scope stops the previous runtime before persisting new capabilities", async (t) => {
  const f = await fixture(t);
  await f.controller.addFolders([f.project]);
  await configure(f.controller, {
    writeEnabled: false,
    executionEnabled: false,
  });
  await f.controller.start();
  await configure(f.controller, { writeEnabled: true, executionEnabled: true });
  assert.equal(f.stops.length, 1);
  assert.equal(f.controller.state().running, false);
  await f.controller.start();
  assert.equal(f.starts.length, 2);
  await f.controller.stop();
});
test("reopening private config restores choices without automatically publishing them", async (t) => {
  const f = await fixture(t);
  await f.controller.addFolders([f.project]);
  await configure(f.controller, {
    writeEnabled: true,
    executionEnabled: false,
  });
  const next = new AgentController({
    dataDir: join(f.dir, "private"),
    binary: "/bundled/localoud-mcp",
    platform: "darwin",
    startAgent: () => {
      throw Error("must_not_start");
    },
  });
  await next.initialize();
  assert.equal(next.state().projects[0]?.root ?? null, f.project);
  assert.equal(next.state().configured, true);
  assert.equal(next.state().running, false);
});
test("unsafe config symlinks are rejected without reading their target", async (t) => {
  const f = await fixture(t);
  const secret = join(f.dir, "external");
  await writeFile(secret, "do not parse");
  await symlink(secret, join(f.dir, "private", "projects.json"));
  const next = new AgentController({
    dataDir: join(f.dir, "private"),
    binary: "/bundled/localoud-mcp",
    platform: "darwin",
    startAgent: () => {},
  });
  await assert.rejects(next.initialize(), /unsafe_local_state/);
});
test("Windows is explicitly unsupported before Unix state or execution can start", async () => {
  const c = new AgentController({
    platform: "win32",
    dataDir: "/unused",
    binary: "/unused",
    startAgent: () => {
      throw Error("must_not_start");
    },
  });
  await assert.rejects(c.initialize(), /platform_not_supported/);
});

test("adding another project stops publication until the collection is saved and restarted", async (t) => {
  const f = await fixture(t);
  await f.controller.addFolders([f.project]);
  await configure(f.controller, {
    writeEnabled: true,
    executionEnabled: false,
  });
  await f.controller.start();
  const other = join(f.dir, "other");
  await mkdir(other);
  await f.controller.addFolders([other]);
  assert.equal(f.stops.length, 1);
  assert.equal(f.controller.state().running, false);
  assert.equal(f.controller.state().configured, false);
  await assert.rejects(f.controller.start(), /setup_required/);
});

test("a removed project reopens as recoverable setup without losing private receipts", async (t) => {
  const f = await fixture(t);
  await f.controller.addFolders([f.project]);
  await configure(f.controller, { writeEnabled: true, executionEnabled: true });
  await mkdir(join(f.dir, "private", "agent"), { mode: 0o700 });
  await writeFile(join(f.dir, "private", "agent", "receipt-probe"), "KEEP", {
    mode: 0o600,
  });
  await rm(f.project, { recursive: true });
  const next = new AgentController({
    dataDir: join(f.dir, "private"),
    binary: "/bundled/localoud-mcp",
    platform: "darwin",
    startAgent: () => {
      throw Error("must_not_start");
    },
  });
  await next.initialize();
  assert.equal(next.state().configured, false);
  assert.equal(next.state().projects[0].available, false);
  assert.equal(next.state().error, "project_unavailable");
  assert.equal(
    await readFile(join(f.dir, "private", "agent", "receipt-probe"), "utf8"),
    "KEEP",
  );
  await assert.rejects(next.start(), /setup_required/);
});
