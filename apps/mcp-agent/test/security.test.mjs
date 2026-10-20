import { test } from "node:test";
import assert from "node:assert/strict";
import {
  trustedIpc,
  approvalOrigin,
  allowApprovalNavigation,
  macBinaryArch,
} from "../src/security.mjs";
test("IPC accepts only the exact top level local app frame", () => {
  const url =
    "file:///Applications/Localoud%20Agent.app/Contents/Resources/app.asar/src/renderer.html";
  assert.equal(trustedIpc({ url, mainFrame: true }, url), true);
  for (const frame of [
    { url: "https://chatgpt.com", mainFrame: true },
    { url, mainFrame: false },
    { url: url + "?injected=1", mainFrame: true },
  ])
    assert.equal(trustedIpc(frame, url), false);
});
test("approval navigation is restricted to the exact loopback server", () => {
  const origin = approvalOrigin("http://127.0.0.1:54321");
  assert.equal(origin, "http://127.0.0.1:54321");
  assert.equal(allowApprovalNavigation(origin + "/", origin), true);
  for (const u of [
    "https://evil.test",
    "http://127.0.0.1.evil.test:54321",
    "http://localhost:54321",
    "file:///tmp/x",
    "javascript:alert(1)",
    origin + "/else",
  ])
    assert.equal(allowApprovalNavigation(u, origin), false);
  assert.throws(() => approvalOrigin("https://evil.test"));
});
test("packaging rejects an executable built for a different Mac architecture", () => {
  const b = Buffer.alloc(32);
  b.writeUInt32LE(0xfeedfacf, 0);
  b.writeUInt32LE(0x0100000c, 4);
  assert.equal(macBinaryArch(b), "arm64");
  b.writeUInt32LE(0x01000007, 4);
  assert.equal(macBinaryArch(b), "x64");
  assert.throws(() => macBinaryArch(Buffer.from("not a binary")));
});
