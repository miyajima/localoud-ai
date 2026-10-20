import {
  lstat,
  mkdir,
  realpath,
  readFile,
  open,
  rename,
  unlink,
} from "node:fs/promises";
import { join, basename, resolve, sep } from "node:path";
import { randomUUID } from "node:crypto";

export const GATEWAY = "https://localoud-mcp-gateway.miyazima.workers.dev";
function privateStat(s, dir = false) {
  if (
    s.isSymbolicLink() ||
    !(dir ? s.isDirectory() : s.isFile()) ||
    (!dir && s.nlink !== 1) ||
    (s.mode & 0o777) !== (dir ? 0o700 : 0o600) ||
    s.uid !== process.getuid()
  )
    throw Error("unsafe_local_state");
}
async function existing(path, dir = false) {
  try {
    const s = await lstat(path);
    privateStat(s, dir);
    return true;
  } catch (e) {
    if (e.code === "ENOENT") return false;
    throw e;
  }
}
async function writePrivate(path, value) {
  await existing(path);
  const temporary = path + "." + randomUUID() + ".tmp";
  const file = await open(temporary, "wx", 0o600);
  try {
    await file.writeFile(JSON.stringify(value));
    await file.sync();
  } finally {
    await file.close();
  }
  try {
    await rename(temporary, path);
  } catch (e) {
    await unlink(temporary).catch(() => {});
    throw e;
  }
}
function overlap(a, b) {
  return (
    a === b ||
    a.startsWith(b + sep) ||
    b.startsWith(a + sep) ||
    a === sep ||
    b === sep
  );
}
const MAX_PROJECTS = 100;
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
export class AgentController {
  constructor({
    dataDir,
    binary,
    startAgent,
    platform = process.platform,
    arch = process.arch,
  }) {
    Object.assign(this, {
      dataDir: resolve(dataDir),
      binary,
      startAgent,
      platform,
      arch,
    });
    this.projects = [];
    this.savedProjects = [];
    this.available = new Map();
    this.agent = null;
    this.error = null;
    this.startAtLogin = false;
    this.tail = Promise.resolve();
  }
  exclusive(action) {
    const promise = this.tail.then(action);
    this.tail = promise.catch(() => {});
    return promise;
  }
  async initialize() {
    if (!["darwin", "linux"].includes(this.platform))
      throw Error("platform_not_supported");
    await mkdir(this.dataDir, { recursive: true, mode: 0o700 });
    privateStat(await lstat(this.dataDir), true);
    this.dataDir = await realpath(this.dataDir);
    this.config = join(this.dataDir, "projects.json");
    if (await existing(this.config)) {
      const config = JSON.parse(await readFile(this.config, "utf8"));
      if (
        !Array.isArray(config.projects) ||
        config.projects.length > MAX_PROJECTS
      )
        throw Error("invalid_configuration");
      const ids = new Set(),
        roots = [];
      for (const p of config.projects) {
        if (
          !p ||
          Object.keys(p).sort().join(",") !==
            "execution_enabled,id,name,root,write_enabled" ||
          typeof p.id !== "string" ||
          !/^[a-zA-Z0-9_-]{1,80}$/.test(p.id) ||
          ids.has(p.id) ||
          typeof p.name !== "string" ||
          !p.name.trim() ||
          Buffer.byteLength(p.name) > 120 ||
          typeof p.root !== "string" ||
          resolve(p.root) !== p.root ||
          typeof p.write_enabled !== "boolean" ||
          typeof p.execution_enabled !== "boolean"
        )
          throw Error("invalid_configuration");
        ids.add(p.id);
        if (roots.some((root) => overlap(root, p.root)))
          throw Error("invalid_configuration");
        roots.push(p.root);
        try {
          const canonical = await this.validateFolder(p.root);
          if (canonical !== p.root) throw Error("project_root_changed");
          this.available.set(p.id, true);
        } catch (error) {
          if (
            !["ENOENT", "ENOTDIR", "EACCES", "EPERM"].includes(error.code) &&
            error.message !== "project_root_changed"
          )
            throw error;
          this.available.set(p.id, false);
          this.error = "project_unavailable";
        }
      }
      this.projects = config.projects.map((p) => ({ ...p }));
      this.savedProjects = this.projects.map((p) => ({ ...p }));
    }
    return this.state();
  }
  async validateFolder(path) {
    if (typeof path !== "string" || resolve(path) !== path)
      throw Error("invalid_folder");
    const folder = await realpath(path);
    if (!(await lstat(folder)).isDirectory()) throw Error("invalid_folder");
    if (overlap(folder, this.dataDir))
      throw Error("private_state_overlaps_project");
    return folder;
  }
  async addFolders(paths) {
    return this.exclusive(async () => {
      if (!Array.isArray(paths) || !paths.length)
        throw Error("invalid_options");
      if (this.projects.length + paths.length > MAX_PROJECTS)
        throw Error("project_limit");
      const added = [];
      for (const path of paths) {
        const root = await this.validateFolder(path);
        if ([...this.projects, ...added].some((p) => overlap(root, p.root)))
          throw Error("project_overlap");
        const name = basename(root);
        if (Buffer.byteLength(name) > 120) throw Error("invalid_folder");
        added.push({
          id: randomUUID(),
          name,
          root,
          write_enabled: false,
          execution_enabled: false,
        });
      }
      await this.stopNow();
      this.projects.push(...added);
      for (const p of added) this.available.set(p.id, true);
      this.refreshError();
      return this.state();
    });
  }
  async updateProject(options) {
    return this.exclusive(async () => {
      if (
        !options ||
        Object.keys(options).sort().join(",") !==
          "executionEnabled,id,writeEnabled" ||
        typeof options.id !== "string" ||
        typeof options.writeEnabled !== "boolean" ||
        typeof options.executionEnabled !== "boolean"
      )
        throw Error("invalid_options");
      const p = this.projects.find((p) => p.id === options.id);
      if (!p) throw Error("unknown_project");
      if (
        p.write_enabled === options.writeEnabled &&
        p.execution_enabled === options.executionEnabled
      )
        return this.state();
      await this.stopNow();
      p.write_enabled = options.writeEnabled;
      p.execution_enabled = options.executionEnabled;
      return this.state();
    });
  }
  async removeProject(id) {
    return this.exclusive(async () => {
      if (typeof id !== "string" || !this.projects.some((p) => p.id === id))
        throw Error("unknown_project");
      await this.stopNow();
      this.projects = this.projects.filter((p) => p.id !== id);
      this.available.delete(id);
      this.refreshError();
      return this.state();
    });
  }
  refreshError() {
    this.error = this.projects.some((p) => this.available.get(p.id) === false)
      ? "project_unavailable"
      : null;
  }
  async verifyRoots() {
    for (const p of this.projects) {
      try {
        if ((await this.validateFolder(p.root)) !== p.root)
          throw Error("project_root_changed");
        this.available.set(p.id, true);
      } catch {
        this.available.set(p.id, false);
        this.error = "project_unavailable";
        throw Error("project_unavailable");
      }
    }
    this.refreshError();
  }
  async recheckProjects() {
    return this.exclusive(async () => {
      await this.verifyRoots();
      return this.state();
    });
  }
  async saveProjects() {
    return this.exclusive(async () => {
      await this.verifyRoots();
      await this.stopNow();
      await writePrivate(this.config, { projects: this.projects });
      this.savedProjects = this.projects.map((p) => ({ ...p }));
      return this.state();
    });
  }
  async start() {
    return this.exclusive(async () => {
      if (this.agent) return this.state();
      if (!this.state().configured) throw Error("setup_required");
      await this.verifyRoots();
      try {
        this.agent = await this.startAgent({
          stateDir: join(this.dataDir, "agent"),
          config: this.config,
          gateway: GATEWAY,
          binary: this.binary,
          port: 0,
        });
        this.error = null;
      } catch {
        this.error = "agent_start_failed";
        throw Error("agent_start_failed");
      }
      return this.state();
    });
  }
  async stopNow() {
    if (this.agent) {
      const agent = this.agent;
      await agent.close();
      this.agent = null;
    }
    return this.state();
  }
  async stop() {
    return this.exclusive(() => this.stopNow());
  }
  state() {
    const dirty = !same(this.projects, this.savedProjects);
    return {
      platform: this.platform,
      arch: this.arch,
      projects: this.projects.map((p) => ({
        id: p.id,
        name: p.name,
        root: p.root,
        writeEnabled: p.write_enabled,
        executionEnabled: p.execution_enabled,
        available: this.available.get(p.id) !== false,
      })),
      dirty,
      configured:
        this.projects.length > 0 &&
        !dirty &&
        this.projects.every((p) => this.available.get(p.id) !== false),
      running: Boolean(this.agent),
      online: Boolean(this.agent?.online()),
      startAtLogin: this.startAtLogin,
      mcpUrl: GATEWAY + "/mcp",
      error: this.error,
    };
  }
}
