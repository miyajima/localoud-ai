const { spawnSync } = require("node:child_process");
const { join } = require("node:path");
const { pathToFileURL } = require("node:url");

module.exports = async ({ appOutDir }) => {
  const runtime = join(
    appOutDir,
    "Localoud Agent.app/Contents/Resources/agent/runtime.mjs",
  );
  const result = spawnSync(
    process.execPath,
    [
      "--input-type=module",
      "-e",
      "await import(process.argv[1])",
      pathToFileURL(runtime).href,
    ],
    { encoding: "utf8", env: { PATH: "/usr/bin:/bin" } },
  );
  if (result.status !== 0)
    throw Error("Packaged agent dependency import failed: " + result.stderr);
};
