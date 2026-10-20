import { test } from "node:test";
import assert from "node:assert/strict";
import { ApprovalWindows } from "../src/approval-windows.mjs";
const deferred = () => {
  let resolve;
  const promise = new Promise((r) => (resolve = r));
  return { promise, resolve };
};
function fixture(readSession, sessionWait = Promise.resolve()) {
  let agent = { app: { url: "http://127.0.0.1:51000" } };
  const created = [];
  const manager = new ApprovalWindows({
    getAgent: () => agent,
    readSession,
    makeWindow: () => {
      const win = {
        destroyed: false,
        shown: false,
        loaded: false,
        isDestroyed() {
          return this.destroyed;
        },
        destroy() {
          this.destroyed = true;
        },
        show() {
          this.shown = true;
        },
        focus() {},
        async setSession() {
          await sessionWait;
        },
        async load() {
          this.loaded = true;
        },
      };
      created.push(win);
      return win;
    },
  });
  return {
    manager,
    created,
    stop: () => {
      agent = null;
      manager.close();
    },
  };
}
test("stop during private token read cannot create or show a stale approval window", async () => {
  const wait = deferred();
  const f = fixture(() => wait.promise);
  const pending = f.manager.open();
  f.stop();
  wait.resolve("private-session");
  await pending;
  assert.equal(f.created.length, 0);
});
test("stop during cookie initialization destroys the window and cannot load or show it", async () => {
  const wait = deferred();
  const f = fixture(async () => "private-session", wait.promise);
  const pending = f.manager.open();
  await new Promise((r) => setImmediate(r));
  assert.equal(f.created.length, 1);
  f.stop();
  wait.resolve();
  await pending;
  assert.equal(f.created[0].destroyed, true);
  assert.equal(f.created[0].loaded, false);
  assert.equal(f.created[0].shown, false);
});
test("concurrent opens create one authenticated approval window", async () => {
  const f = fixture(async () => "private-session");
  await Promise.all([f.manager.open(), f.manager.open()]);
  assert.equal(f.created.length, 1);
  assert.equal(f.created[0].shown, true);
  f.stop();
});
test("failed authentication closes the incomplete window before retry", async () => {
  const f = fixture(async () => "private-session");
  const make = f.manager.makeWindow;
  let fail = true;
  f.manager.makeWindow = (origin) => {
    const win = make(origin);
    win.setSession = async () => {
      if (fail) throw Error("cookie_failed");
    };
    return win;
  };
  await assert.rejects(f.manager.open(), /cookie_failed/);
  assert.equal(f.created[0].destroyed, true);
  fail = false;
  await f.manager.open();
  assert.equal(f.created.length, 2);
  assert.equal(f.created[1].shown, true);
  f.stop();
});
