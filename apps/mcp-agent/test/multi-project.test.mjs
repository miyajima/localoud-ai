import { test } from "node:test";
import assert from "node:assert/strict";
import {
  mkdtemp,
  mkdir,
  readFile,
  writeFile,
  realpath,
  rm,
  symlink,
} from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { AgentController } from "../src/controller.mjs";
async function fixture(t) {
  const dir = await realpath(await mkdtemp(join(tmpdir(), "localoud-multi-")));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const roots = [join(dir, "A"), join(dir, "B")];
  for (const p of roots) await mkdir(p);
  const events = [];
  const options = {
    dataDir: join(dir, "private"),
    binary: "/bundled/localoud-mcp",
    platform: "darwin",
    startAgent: async () => {
      events.push("start");
      return { online: () => true, close: async () => events.push("stop") };
    },
  };
  const c = new AgentController(options);
  await c.initialize();
  return {
    c,
    dir,
    roots,
    events,
    options,
    config: join(dir, "private/projects.json"),
  };
}
test("two native selections default off, persist distinct flags and stable IDs across restart", async (t) => {
  const f = await fixture(t);
  await f.c.addFolders(f.roots);
  let projects = f.c.state().projects;
  assert.equal(projects.length, 2);
  assert.notEqual(projects[0].id, projects[1].id);
  for (const p of projects) {
    assert.equal(p.writeEnabled, false);
    assert.equal(p.executionEnabled, false);
  }
  await f.c.updateProject({
    id: projects[0].id,
    writeEnabled: true,
    executionEnabled: false,
  });
  await f.c.updateProject({
    id: projects[1].id,
    writeEnabled: false,
    executionEnabled: true,
  });
  await assert.rejects(f.c.start(), /setup_required/);
  await f.c.saveProjects();
  await f.c.start();
  await f.c.stop();
  const next = new AgentController(f.options);
  await next.initialize();
  assert.deepEqual(next.state().projects, f.c.state().projects);
  const saved = JSON.parse(await readFile(f.config, "utf8")).projects;
  assert.deepEqual(
    saved.map((p) => [p.write_enabled, p.execution_enabled]),
    [
      [true, false],
      [false, true],
    ],
  );
});
test("legacy personal registration and permissions survive adding a second project", async (t) => {
  const f = await fixture(t);
  await writeFile(
    f.config,
    JSON.stringify({
      projects: [
        {
          id: "personal",
          name: "A",
          root: f.roots[0],
          write_enabled: true,
          execution_enabled: false,
        },
      ],
    }),
    { mode: 0o600 },
  );
  const c = new AgentController(f.options);
  await c.initialize();
  await c.addFolders([f.roots[1]]);
  await c.saveProjects();
  assert.equal(c.state().projects[0].id, "personal");
  assert.equal(c.state().projects[0].writeEnabled, true);
  assert.equal(c.state().projects[1].writeEnabled, false);
});
test("duplicates, symlink aliases, overlapping roots and private-state roots do not broaden scope", async (t) => {
  const f = await fixture(t);
  await f.c.addFolders([f.roots[0]]);
  await f.c.saveProjects();
  await f.c.start();
  const alias = join(f.dir, "alias");
  await symlink(f.roots[0], alias);
  for (const root of [f.roots[0], alias, f.dir, join(f.dir, "private")])
    await assert.rejects(
      f.c.addFolders([root]),
      /project_overlap|private_state_overlaps_project/,
    );
  assert.equal(f.c.state().projects.length, 1);
  assert.equal(f.c.state().running, true);
  await f.c.stop();
});
test("permission edits stop the old runtime and cannot inject paths or unknown IDs", async (t) => {
  const f = await fixture(t);
  await f.c.addFolders(f.roots);
  await f.c.saveProjects();
  await f.c.start();
  const id = f.c.state().projects[0].id;
  await assert.rejects(
    f.c.updateProject({
      id,
      root: "/other",
      writeEnabled: true,
      executionEnabled: true,
    }),
    /invalid_options/,
  );
  await assert.rejects(
    f.c.updateProject({
      id: "unknown",
      writeEnabled: true,
      executionEnabled: true,
    }),
    /unknown_project/,
  );
  assert.equal(f.c.state().running, true);
  await f.c.updateProject({ id, writeEnabled: false, executionEnabled: true });
  assert.deepEqual(f.events, ["start", "stop"]);
  assert.equal(f.c.state().dirty, true);
  await assert.rejects(f.c.start(), /setup_required/);
  await f.c.saveProjects();
  assert.equal(f.c.state().projects[1].executionEnabled, false);
});
test("removal preserves other IDs and removing all stays empty after restart", async (t) => {
  const f = await fixture(t);
  await f.c.addFolders(f.roots);
  await f.c.saveProjects();
  const [a, b] = f.c.state().projects;
  await f.c.removeProject(a.id);
  await f.c.saveProjects();
  assert.equal(f.c.state().projects[0].id, b.id);
  await f.c.removeProject(b.id);
  await f.c.saveProjects();
  const next = new AgentController(f.options);
  await next.initialize();
  assert.deepEqual(next.state().projects, []);
  assert.equal(next.state().configured, false);
  await assert.rejects(next.start(), /setup_required/);
});
test("unavailable project is retained for recovery without discarding other registrations", async (t) => {
  const f = await fixture(t);
  await f.c.addFolders(f.roots);
  await f.c.saveProjects();
  const id = f.c.state().projects[0].id;
  await rm(f.roots[0], { recursive: true });
  const next = new AgentController(f.options);
  await next.initialize();
  assert.equal(next.state().projects.length, 2);
  assert.equal(next.state().projects[0].available, false);
  assert.equal(next.state().configured, false);
  await next.removeProject(id);
  await next.saveProjects();
  assert.equal(next.state().configured, true);
});
test("loading invalid duplicate IDs fails closed without changing the saved file", async (t) => {
  const f = await fixture(t);
  const p = {
    id: "same",
    name: "A",
    root: f.roots[0],
    write_enabled: false,
    execution_enabled: false,
  };
  const text = JSON.stringify({ projects: [p, { ...p, root: f.roots[1] }] });
  await writeFile(f.config, text, { mode: 0o600 });
  const next = new AgentController(f.options);
  await assert.rejects(next.initialize(), /invalid_configuration/);
  assert.equal(await readFile(f.config, "utf8"), text);
});
test("over-limit selection is rejected atomically before any project is added", async (t) => {
  const f = await fixture(t);
  await assert.rejects(
    f.c.addFolders(Array(101).fill(f.roots[0])),
    /project_limit/,
  );
  assert.deepEqual(f.c.state().projects, []);
});

test("restored folders can be rechecked without changing IDs, permissions or starting", async (t) => {
  const f = await fixture(t);
  await f.c.addFolders(f.roots);
  const original = f.c.state().projects;
  await f.c.updateProject({
    id: original[0].id,
    writeEnabled: true,
    executionEnabled: false,
  });
  await f.c.saveProjects();
  await rm(f.roots[0], { recursive: true });
  const next = new AgentController(f.options);
  await next.initialize();
  await mkdir(f.roots[0]);
  await next.recheckProjects();
  assert.equal(next.state().configured, true);
  assert.equal(next.state().projects[0].id, original[0].id);
  assert.equal(next.state().projects[0].writeEnabled, true);
  assert.equal(next.state().running, false);
});
