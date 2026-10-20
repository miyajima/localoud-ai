import { approvalOrigin } from "./security.mjs";

// Invalidate every asynchronous step when the agent or its scope is stopped.
export class ApprovalWindows {
  constructor({ getAgent, readSession, makeWindow }) {
    Object.assign(this, { getAgent, readSession, makeWindow });
    this.generation = 0;
    this.window = null;
    this.opening = null;
    this.owner = null;
  }
  close() {
    this.generation++;
    if (this.window && !this.window.isDestroyed()) this.window.destroy();
    this.window = null;
    this.owner = null;
    this.opening = null;
  }
  open() {
    const agent = this.getAgent();
    if (!agent) return Promise.reject(Error("agent_not_running"));
    if (this.owner && this.owner !== agent) this.close();
    if (this.opening) return this.opening;
    if (this.window && !this.window.isDestroyed()) {
      this.window.show();
      this.window.focus();
      return Promise.resolve();
    }
    const origin = approvalOrigin(agent.app.url);
    const generation = this.generation;
    this.owner = agent;
    const current = () =>
      this.generation === generation && this.getAgent() === agent;
    const opening = (async () => {
      const token = await this.readSession(agent);
      if (!current()) return;
      const win = this.makeWindow(origin);
      this.window = win;
      const valid = () =>
        current() && this.window === win && !win.isDestroyed();
      try {
        await win.setSession(token);
        if (!valid()) return;
        await win.load(origin + "/");
        if (!valid()) return;
        win.show();
      } catch (error) {
        if (!win.isDestroyed()) win.destroy();
        if (this.window === win) this.window = null;
        throw error;
      } finally {
        if (!valid() && !win.isDestroyed()) win.destroy();
      }
    })();
    this.opening = opening;
    return opening.finally(() => {
      if (this.opening === opening) this.opening = null;
    });
  }
}
