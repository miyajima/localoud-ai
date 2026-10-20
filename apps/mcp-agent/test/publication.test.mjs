import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, writeFile, readdir, rm } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { publishValidatedArtifacts } from "../scripts/artifacts.mjs";
test("failed validation publishes neither a partial installer nor a mixed pair", async (t) => {
  const out = await mkdtemp(join(tmpdir(), "localoud-release-"));
  t.after(() => rm(out, { recursive: true, force: true }));
  await assert.rejects(
    publishValidatedArtifacts({
      outputDir: out,
      label: "0.1.0-arm64",
      produce: async (stage) => {
        await writeFile(join(stage, "app-signed.dmg"), "UNVALIDATED");
        throw Error("notary_failed");
      },
    }),
    /notary_failed/,
  );
  assert.deepEqual(await readdir(out), []);
});
test("validated artifacts appear together with byte hashes in one new release directory", async (t) => {
  const out = await mkdtemp(join(tmpdir(), "localoud-release-"));
  t.after(() => rm(out, { recursive: true, force: true }));
  const result = await publishValidatedArtifacts({
    outputDir: out,
    label: "0.1.0-arm64",
    produce: async (stage) => {
      await writeFile(join(stage, "app-signed.dmg"), "DMG");
      await writeFile(join(stage, "app-signed.zip"), "ZIP");
      return ["app-signed.dmg", "app-signed.zip"];
    },
  });
  assert.equal(await readFile(join(result, "app-signed.dmg"), "utf8"), "DMG");
  const manifest = JSON.parse(
    await readFile(join(result, "manifest.json"), "utf8"),
  );
  assert.equal(manifest.artifacts.length, 2);
  assert.equal(manifest.artifacts[0].sha256.length, 64);
  assert.deepEqual((await readdir(out)).sort(), ["releases"]);
});
