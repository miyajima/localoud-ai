import {
  mkdir,
  mkdtemp,
  lstat,
  readFile,
  writeFile,
  rename,
  rm,
} from "node:fs/promises";
import { join, basename } from "node:path";
import { createHash, randomUUID } from "node:crypto";

// Publish one validated pair atomically; failed releases leave no public candidate.
export async function publishValidatedArtifacts({ outputDir, label, produce }) {
  if (!/^[a-zA-Z0-9._-]+$/.test(label)) throw Error("invalid_release_label");
  await mkdir(outputDir, { recursive: true });
  const stage = await mkdtemp(join(outputDir, ".staging-"));
  try {
    const names = await produce(stage);
    if (
      !Array.isArray(names) ||
      names.length !== 2 ||
      new Set(names).size !== 2 ||
      !names.some((n) => n.endsWith(".dmg")) ||
      !names.some((n) => n.endsWith(".zip"))
    )
      throw Error("invalid_artifact_pair");
    const artifacts = [];
    for (const name of names) {
      if (typeof name !== "string" || basename(name) !== name)
        throw Error("invalid_artifact_name");
      const path = join(stage, name),
        stat = await lstat(path);
      if (!stat.isFile() || stat.isSymbolicLink() || stat.size === 0)
        throw Error("invalid_artifact");
      artifacts.push({
        name,
        bytes: stat.size,
        sha256: createHash("sha256")
          .update(await readFile(path))
          .digest("hex"),
      });
    }
    await writeFile(
      join(stage, "manifest.json"),
      JSON.stringify({ artifacts }, null, 2) + "\n",
    );
    const releases = join(outputDir, "releases");
    await mkdir(releases, { recursive: true });
    const destination = join(releases, label + "-" + randomUUID());
    await rename(stage, destination);
    return destination;
  } catch (error) {
    await rm(stage, { recursive: true, force: true });
    throw error;
  }
}
