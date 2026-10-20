import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import beforePack from "../scripts/before-pack.cjs";
test("packager stops instead of placing an ARM executable into an Intel installer", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "localoud-pack-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const projectDir = join(root, "apps", "mcp-agent");
  await mkdir(projectDir, { recursive: true });
  await mkdir(join(root, "target", "release"), { recursive: true });
  const header = Buffer.alloc(32);
  header.writeUInt32LE(0xfeedfacf, 0);
  header.writeUInt32LE(0x0100000c, 4);
  await writeFile(join(root, "target", "release", "localoud-mcp"), header);
  await beforePack({
    electronPlatformName: "darwin",
    arch: 3,
    packager: { projectDir },
  });
  await assert.rejects(
    beforePack({
      electronPlatformName: "darwin",
      arch: 0,
      packager: { projectDir },
    }),
    /architecture mismatch/,
  );
  await assert.rejects(
    beforePack({
      electronPlatformName: "win32",
      arch: 0,
      packager: { projectDir },
    }),
    /Only macOS/,
  );
});
test("public release fails before packaging if signing or notarization identity is missing", () => {
  const r = spawnSync(process.execPath, ["scripts/release-mac.mjs"], {
    cwd: new URL("..", import.meta.url),
    env: { PATH: process.env.PATH, HOME: process.env.HOME },
    encoding: "utf8",
  });
  assert.equal(r.status, 2);
  assert.match(r.stderr, /MAC_SIGN_IDENTITY/);
});
test("packaged runtime must import with all dependencies from its own resources", async (t) => {
  const { cp } = await import("node:fs/promises");
  const { default: afterPack } = await import("../scripts/after-pack.cjs");
  const out = await mkdtemp(join(tmpdir(), "localoud-packed-"));
  t.after(() => rm(out, { recursive: true, force: true }));
  const agent = join(out, "Localoud Agent.app/Contents/Resources/agent");
  await mkdir(agent, { recursive: true });
  for (const name of [
    "runtime.mjs",
    "dashboard.mjs",
    "presentation.mjs",
    "approval.mjs",
    "app.mjs",
    "mcp.mjs",
    "package.json",
  ])
    await cp(
      new URL("../../../plugins/localoud-mcp/agent/" + name, import.meta.url),
      join(agent, name),
    );
  await assert.rejects(
    afterPack({ appOutDir: out }),
    /Packaged agent dependency import failed/,
  );
  await mkdir(join(agent, "node_modules"), { recursive: true });
  await cp(
    new URL(
      "../../../plugins/localoud-mcp/agent/node_modules/ws",
      import.meta.url,
    ),
    join(agent, "node_modules/ws"),
    { recursive: true },
  );
  await afterPack({ appOutDir: out });
});
