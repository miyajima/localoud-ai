import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
if (process.platform !== "darwin" || !["arm64", "x64"].includes(process.arch)) {
  console.error(
    "Mac installer packaging requires a matching macOS host and Rust binary.",
  );
  process.exit(2);
}
const result = spawnSync(
  fileURLToPath(
    new URL("../node_modules/.bin/electron-builder", import.meta.url),
  ),
  [
    "--mac",
    "dmg",
    "zip",
    "--" + process.arch,
    "--config.mac.identity=-",
    "--publish",
    "never",
  ],
  { cwd: fileURLToPath(new URL("..", import.meta.url)), stdio: "inherit" },
);
process.exit(result.status ?? 1);
