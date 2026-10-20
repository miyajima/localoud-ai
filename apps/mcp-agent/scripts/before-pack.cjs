const { readFile } = require("node:fs/promises");
const { resolve } = require("node:path");
module.exports = async (context) => {
  if (context.electronPlatformName !== "darwin")
    throw Error("Only macOS installer builds are currently verified.");
  const { macBinaryArch } = await import("../src/security.mjs");
  const expected = { 0: "x64", 3: "arm64" }[context.arch];
  if (!expected) throw Error("Unsupported Mac build architecture.");
  const actual = macBinaryArch(
    await readFile(
      resolve(context.packager.projectDir, "../../target/release/localoud-mcp"),
    ),
  );
  if (actual !== expected)
    throw Error(
      "Bundled Rust MCP architecture mismatch: build the Rust binary for " +
        expected +
        " first.",
    );
};
