// Signed release pipeline. Credentials stay in the existing macOS Keychain.
import { spawnSync } from "node:child_process";
import {
  readFile,
  mkdtemp,
  mkdir,
  cp,
  symlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { publishValidatedArtifacts } from "./artifacts.mjs";
const root = fileURLToPath(new URL("..", import.meta.url));
const identity = process.env.MAC_SIGN_IDENTITY,
  profile = process.env.MAC_NOTARY_PROFILE;
if (
  process.platform !== "darwin" ||
  !identity?.startsWith("Developer ID Application:") ||
  !profile
) {
  console.error(
    "Requires macOS, MAC_SIGN_IDENTITY (Developer ID Application) and MAC_NOTARY_PROFILE (existing notarytool Keychain profile name). No credential values belong in this repository.",
  );
  process.exit(2);
}
const arch = process.arch;
if (!["arm64", "x64"].includes(arch)) {
  console.error("Unsupported Mac architecture");
  process.exit(2);
}
const out = resolve(root, "../../target/localoud-agent-app");
function run(command, args, options = {}) {
  const r = spawnSync(command, args, {
    cwd: root,
    encoding: "utf8",
    env: { ...process.env, CSC_NAME: identity },
    ...options,
  });
  if (r.status !== 0) {
    if (r.stderr) process.stderr.write(r.stderr);
    throw Error("release_step_failed: " + command);
  }
  return r.stdout;
}
const manifest = JSON.parse(await readFile(join(root, "package.json"), "utf8"));
try {
  run(
    join(root, "node_modules/.bin/electron-builder"),
    ["--mac", "--dir", "--" + arch, "--publish", "never"],
    { stdio: "inherit" },
  );
  const app = join(
    out,
    arch === "arm64" ? "mac-arm64" : "mac",
    "Localoud Agent.app",
  );
  run("/usr/bin/codesign", ["--verify", "--deep", "--strict", app]);
  const signature = spawnSync(
    "/usr/bin/codesign",
    ["-dv", "--verbose=2", app],
    { encoding: "utf8" },
  );
  if (
    signature.status !== 0 ||
    !signature.stderr.includes("Authority=" + identity)
  )
    throw Error("developer_id_signature_required");
  const staging = await mkdtemp(join(tmpdir(), "localoud-release-"));
  await writeFile(
    join(staging, "README.txt"),
    "Localoud signed release staging.\n",
  );
  const zip = join(staging, "notarize.zip");
  run("/usr/bin/ditto", ["-c", "-k", "--keepParent", app, zip]);
  const notarize = (path) => {
    const result = JSON.parse(
      run("/usr/bin/xcrun", [
        "notarytool",
        "submit",
        path,
        "--keychain-profile",
        profile,
        "--wait",
        "--output-format",
        "json",
      ]),
    );
    if (result.status !== "Accepted") throw Error("notarization_not_accepted");
  };
  notarize(zip);
  run("/usr/bin/xcrun", ["stapler", "staple", app]);
  run("/usr/sbin/spctl", ["--assess", "--type", "execute", "--verbose=2", app]);
  const contents = join(staging, "dmg");
  await mkdir(contents);
  await cp(app, join(contents, "Localoud Agent.app"), {
    recursive: true,
    verbatimSymlinks: true,
  });
  await symlink("/Applications", join(contents, "Applications"));
  const release = await publishValidatedArtifacts({
    outputDir: out,
    label: `${manifest.version}-${arch}`,
    produce: async (releaseStage) => {
      const dmgName = `Localoud-Agent-${manifest.version}-${arch}-signed.dmg`;
      const zipName = `Localoud-Agent-${manifest.version}-${arch}-signed.zip`;
      const dmg = join(releaseStage, dmgName);
      run("/usr/bin/hdiutil", [
        "create",
        "-volname",
        "Localoud Agent",
        "-srcfolder",
        contents,
        "-ov",
        "-format",
        "UDZO",
        dmg,
      ]);
      run("/usr/bin/codesign", ["--sign", identity, "--timestamp", dmg]);
      notarize(dmg);
      run("/usr/bin/xcrun", ["stapler", "staple", dmg]);
      run("/usr/bin/xcrun", ["stapler", "validate", dmg]);
      run("/usr/sbin/spctl", [
        "--assess",
        "--type",
        "open",
        "--context",
        "context:primary-signature",
        "--verbose=2",
        dmg,
      ]);
      run("/usr/bin/ditto", [
        "-c",
        "-k",
        "--keepParent",
        app,
        join(releaseStage, zipName),
      ]);
      return [dmgName, zipName];
    },
  });
  console.log("Signed and notarized artifacts ready:", release);
} catch (error) {
  console.error(error.message);
  process.exit(1);
}
