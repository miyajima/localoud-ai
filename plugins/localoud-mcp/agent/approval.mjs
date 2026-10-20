import { createHash, randomUUID } from "node:crypto";
import { constants } from "node:fs";
import { lstat, mkdir, open, readFile, rename, rmdir, unlink } from "node:fs/promises";
import { dirname, resolve } from "node:path";

const MAX_REQUEST_BYTES = 64 * 1024;
const MAX_RESULT_BYTES = 128 * 1024;
const MAX_JOBS = 256;
const MAX_DEADLINE_MS = 10 * 60 * 1000;
const TERMINAL_STATUSES = new Set(["completed", "failed", "denied", "expired", "indeterminate"]);
const JOB_STATUSES = new Set(["pending", "running", ...TERMINAL_STATUSES]);
const ALLOWED_TOOLS = new Set(["command_run", "file_edit", "file_create"]);
const REQUEST_KEYS = ["arguments", "expires_at", "operation_id", "tool"];
const fileMutexes = new Map();

function deferred() {
  let resolvePromise;
  let rejectPromise;
  const promise = new Promise((resolveValue, rejectValue) => {
    resolvePromise = resolveValue;
    rejectPromise = rejectValue;
  });
  return { promise, resolve: resolvePromise, reject: rejectPromise };
}

function own(value, key) {
  return Object.prototype.hasOwnProperty.call(value, key);
}

function compareKeys(left, right) {
  return left < right ? -1 : left > right ? 1 : 0;
}

function copyJson(value, seen = new WeakSet()) {
  if (value === null || typeof value === "string" || typeof value === "boolean") return value;
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new TypeError("JSON values must use finite numbers");
    return value;
  }
  if (typeof value !== "object") throw new TypeError("Value must contain only JSON data");
  if (seen.has(value)) throw new TypeError("Cyclic values are not supported");
  seen.add(value);

  if (Array.isArray(value)) {
    if (Object.getPrototypeOf(value) !== Array.prototype) throw new TypeError("Arrays must be plain arrays");
    const keys = Reflect.ownKeys(value);
    if (keys.some((key) => typeof key !== "string" || (key !== "length" && !/^(0|[1-9]\d*)$/u.test(key)))) {
      throw new TypeError("Arrays may not contain extra properties");
    }
    const result = [];
    for (let index = 0; index < value.length; index += 1) {
      if (!own(value, index)) throw new TypeError("Sparse arrays are not supported");
      const descriptor = Object.getOwnPropertyDescriptor(value, String(index));
      if (!descriptor || !descriptor.enumerable || !own(descriptor, "value")) {
        throw new TypeError("JSON values may not contain accessors");
      }
      result.push(copyJson(descriptor.value, seen));
    }
    seen.delete(value);
    return result;
  }

  const prototype = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) throw new TypeError("Objects must be plain objects");
  const keys = Reflect.ownKeys(value);
  if (keys.some((key) => typeof key !== "string")) throw new TypeError("Symbol keys are not supported");
  const result = {};
  for (const key of keys.sort(compareKeys)) {
    const descriptor = Object.getOwnPropertyDescriptor(value, key);
    if (!descriptor || !descriptor.enumerable || !own(descriptor, "value")) {
      throw new TypeError("JSON values may not contain accessors or hidden properties");
    }
    Object.defineProperty(result, key, {
      value: copyJson(descriptor.value, seen),
      enumerable: true,
      configurable: true,
      writable: true,
    });
  }
  seen.delete(value);
  return result;
}

function canonicalStringify(value) {
  if (value === null || typeof value !== "object") return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonicalStringify).join(",")}]`;
  const keys = Object.keys(value).sort(compareKeys);
  return `{${keys.map((key) => `${JSON.stringify(key)}:${canonicalStringify(value[key])}`).join(",")}}`;
}

function validateRequest(request) {
  if (request === null || typeof request !== "object" || Array.isArray(request)) {
    throw new TypeError("Request must be a plain object");
  }
  const copy = copyJson(request);
  if (Object.getPrototypeOf(request) !== Object.prototype && Object.getPrototypeOf(request) !== null) {
    throw new TypeError("Request must be a plain object");
  }
  const keys = Object.keys(copy).sort(compareKeys);
  if (keys.length !== REQUEST_KEYS.length || keys.some((key, index) => key !== REQUEST_KEYS[index])) {
    throw new TypeError("Request must contain only operation_id, tool, arguments, and expires_at");
  }
  if (typeof copy.operation_id !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/iu.test(copy.operation_id)) {
    throw new TypeError("operation_id must be a UUID");
  }
  if (!ALLOWED_TOOLS.has(copy.tool)) throw new TypeError("tool must be command_run, file_edit, or file_create");
  if (copy.arguments === null || typeof copy.arguments !== "object" || Array.isArray(copy.arguments)) {
    throw new TypeError("arguments must be a plain object");
  }
  if (typeof copy.expires_at !== "number" || !Number.isFinite(copy.expires_at)) {
    throw new TypeError("expires_at must be a finite millisecond timestamp");
  }
  const canonical = canonicalStringify(copy);
  if (Buffer.byteLength(canonical, "utf8") > MAX_REQUEST_BYTES) {
    throw new RangeError("Canonical request exceeds 64 KiB");
  }
  const digest = createHash("sha256").update(canonical, "utf8").digest("hex");
  return { request: copy, canonical, digest };
}

export function digestRequest(request) {
  return validateRequest(request).digest;
}

function deepFreeze(value) {
  if (value && typeof value === "object" && !Object.isFrozen(value)) {
    for (const child of Object.values(value)) deepFreeze(child);
    Object.freeze(value);
  }
  return value;
}

function freezeCopy(value) {
  return deepFreeze(copyJson(value));
}

function makeRequest(job) {
  return {
    operation_id: job.operation_id,
    tool: job.tool,
    arguments: copyJson(job.arguments),
    expires_at: job.expires_at,
  };
}

function snapshot(job) {
  return job === undefined ? null : freezeCopy(job);
}

function currentTime(now) {
  const value = now();
  if (typeof value !== "number" || !Number.isFinite(value)) throw new TypeError("now must return a finite timestamp");
  return value;
}

function isMissing(error) {
  return error?.code === "ENOENT";
}

function assertPrivateDirectory(stat, description) {
  if (!stat.isDirectory()) throw new Error(`${description} must be a directory`);
  if ((stat.mode & 0o777) !== 0o700) throw new Error(`${description} must have private mode 0700`);
  if (typeof process.getuid === "function" && stat.uid !== process.getuid()) {
    throw new Error(`${description} must be owned by the current user`);
  }
}

function assertPrivateFile(stat, description) {
  if (!stat.isFile()) throw new Error(`${description} must be a regular file`);
  if (stat.nlink !== 1) throw new Error(`${description} must not be hardlinked`);
  if ((stat.mode & 0o777) !== 0o600) throw new Error(`${description} must have private mode 0600`);
  if (typeof process.getuid === "function" && stat.uid !== process.getuid()) {
    throw new Error(`${description} must be owned by the current user`);
  }
}

async function ensurePrivateDirectory(directory) {
  await mkdir(directory, { recursive: true, mode: 0o700 });
  const stat = await lstat(directory);
  if (stat.isSymbolicLink()) throw new Error("Approval storage directory may not be a symlink");
  assertPrivateDirectory(stat, "Approval storage directory");
}

function pidIsAlive(pid) {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error?.code === "EPERM";
  }
}

async function readLockOwner(lockPath) {
  const ownerPath = `${lockPath}/owner.json`;
  let stat;
  try {
    stat = await lstat(ownerPath);
  } catch (error) {
    if (isMissing(error)) return null;
    throw error;
  }
  if (stat.isSymbolicLink()) throw new Error("Approval lock owner may not be a symlink");
  assertPrivateFile(stat, "Approval lock owner");
  try {
    return JSON.parse(await readFile(ownerPath, "utf8"));
  } catch (error) {
    if (error instanceof SyntaxError) return null;
    throw error;
  }
}

async function removeStaleLock(lockPath, expectedStat) {
  const current = await lstat(lockPath).catch((error) => (isMissing(error) ? null : Promise.reject(error)));
  if (!current || current.dev !== expectedStat.dev || current.ino !== expectedStat.ino) return;
  await unlink(`${lockPath}/owner.json`).catch((error) => {
    if (!isMissing(error)) throw error;
  });
  await rmdir(lockPath).catch((error) => {
    if (!isMissing(error) && error?.code !== "ENOTEMPTY") throw error;
  });
}

async function acquireFileLock(file) {
  const lockPath = `${file}.lock`;
  const timeoutAt = Date.now() + 15_000;
  const token = randomUUID();
  while (Date.now() < timeoutAt) {
    try {
      await mkdir(lockPath, { mode: 0o700 });
      const dirStat = await lstat(lockPath);
      assertPrivateDirectory(dirStat, "Approval lock directory");
      const ownerPath = `${lockPath}/owner.json`;
      const ownerHandle = await open(ownerPath, "wx", 0o600);
      try {
        await ownerHandle.writeFile(JSON.stringify({ pid: process.pid, token, created_at: Date.now() }), "utf8");
        await ownerHandle.sync();
      } finally {
        await ownerHandle.close();
      }
      return async () => {
        const currentDir = await lstat(lockPath).catch((error) => (isMissing(error) ? null : Promise.reject(error)));
        if (!currentDir || currentDir.dev !== dirStat.dev || currentDir.ino !== dirStat.ino) return;
        const owner = await readLockOwner(lockPath);
        if (owner?.token !== token || owner?.pid !== process.pid) return;
        await unlink(ownerPath);
        await rmdir(lockPath);
      };
    } catch (error) {
      if (error?.code !== "EEXIST") throw error;
    }

    const stat = await lstat(lockPath).catch((error) => (isMissing(error) ? null : Promise.reject(error)));
    if (!stat) continue;
    if (stat.isSymbolicLink()) throw new Error("Approval lock may not be a symlink");
    assertPrivateDirectory(stat, "Approval lock directory");
    const owner = await readLockOwner(lockPath);
    if (owner && !pidIsAlive(owner.pid)) {
      await removeStaleLock(lockPath, stat);
      continue;
    }
    if (!owner && Date.now() - stat.mtimeMs > 30_000) {
      await removeStaleLock(lockPath, stat);
      continue;
    }
    await new Promise((resolveDelay) => setTimeout(resolveDelay, 10));
  }
  throw new Error("Timed out waiting for approval storage lock");
}

async function withLocalMutex(file, action) {
  const previous = fileMutexes.get(file) ?? Promise.resolve();
  let release;
  const current = new Promise((resolvePromise) => { release = resolvePromise; });
  fileMutexes.set(file, current);
  await previous.catch(() => {});
  try {
    const unlockFile = await acquireFileLock(file);
    try {
      return await action();
    } finally {
      await unlockFile();
    }
  } finally {
    release();
    if (fileMutexes.get(file) === current) fileMutexes.delete(file);
  }
}

async function readStore(file) {
  let handle;
  try {
    handle = await open(file, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  } catch (error) {
    if (isMissing(error)) return new Map();
    if (error?.code === "ELOOP") throw new Error("Approval storage file may not be a symlink");
    throw error;
  }
  try {
    const stat = await handle.stat();
    assertPrivateFile(stat, "Approval storage file");
    const text = await handle.readFile("utf8");
    let parsed;
    try {
      parsed = JSON.parse(text);
    } catch {
      throw new Error("Approval storage file is invalid JSON");
    }
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed) || parsed.version !== 1 || !Array.isArray(parsed.jobs)) {
      throw new Error("Approval storage file has an unsupported format");
    }
    if (parsed.jobs.length > MAX_JOBS) throw new Error("Approval storage exceeds the retained job limit");
    const jobs = new Map();
    for (const raw of parsed.jobs) {
      if (!raw || typeof raw !== "object" || Array.isArray(raw)) throw new Error("Approval storage contains an invalid job");
      const { request, digest } = validateRequest({
        operation_id: raw.operation_id,
        tool: raw.tool,
        arguments: raw.arguments,
        expires_at: raw.expires_at,
      });
      if (digest !== raw.digest || !JOB_STATUSES.has(raw.status)) throw new Error("Approval storage contains an invalid job");
      if (typeof raw.created_at !== "number" || !Number.isFinite(raw.created_at)) throw new Error("Approval storage contains an invalid job");
      const job = { ...request, digest, status: raw.status, created_at: raw.created_at };
      for (const field of ["started_at", "finished_at"]) {
        if (own(raw, field)) {
          if (typeof raw[field] !== "number" || !Number.isFinite(raw[field])) throw new Error("Approval storage contains an invalid job");
          job[field] = raw[field];
        }
      }
      if (raw.status === "completed") {
        if (!own(raw, "result")) throw new Error("Approval storage contains an invalid completed job");
        const result = copyJson(raw.result);
        if (Buffer.byteLength(canonicalStringify(result), "utf8") > MAX_RESULT_BYTES) throw new Error("Stored result exceeds 128 KiB");
        job.result = result;
      } else if (raw.status === "failed") {
        if (raw.error !== "execution_failed") throw new Error("Approval storage contains an invalid failed job");
        job.error = "execution_failed";
      } else if (own(raw, "result") || own(raw, "error")) {
        throw new Error("Approval storage contains an invalid terminal payload");
      }
      if (jobs.has(job.operation_id)) throw new Error("Approval storage contains duplicate operation IDs");
      jobs.set(job.operation_id, job);
    }
    return jobs;
  } finally {
    await handle.close();
  }
}

async function verifyExistingFile(file) {
  let stat;
  try {
    stat = await lstat(file);
  } catch (error) {
    if (isMissing(error)) return false;
    throw error;
  }
  if (stat.isSymbolicLink()) throw new Error("Approval storage file may not be a symlink");
  assertPrivateFile(stat, "Approval storage file");
  return true;
}

async function writeStore(file, jobs) {
  await verifyExistingFile(file);
  const parent = dirname(file);
  const temporary = `${file}.${process.pid}.${randomUUID()}.tmp`;
  const data = JSON.stringify({ version: 1, jobs: [...jobs.values()] });
  let handle;
  try {
    handle = await open(temporary, "wx", 0o600);
    await handle.writeFile(data, "utf8");
    await handle.sync();
  } catch (error) {
    await handle?.close().catch(() => {});
    await unlink(temporary).catch(() => {});
    throw error;
  }
  await handle.close();
  try {
    await rename(temporary, file);
    const directoryHandle = await open(parent, constants.O_RDONLY);
    try {
      await directoryHandle.sync();
    } finally {
      await directoryHandle.close();
    }
  } catch (error) {
    await unlink(temporary).catch(() => {});
    throw error;
  }
}

function notify(gate, job) {
  if (typeof gate.onUpdate !== "function") return;
  try {
    const pending = gate.onUpdate(snapshot(job));
    if (pending && typeof pending.then === "function") pending.catch(() => {});
  } catch {
    // Notifications are observational; they must not change the durable state.
  }
}

function expirePending(jobs, nowValue) {
  const changed = [];
  for (const job of jobs.values()) {
    if (job.status === "pending" && job.expires_at <= nowValue) {
      job.status = "expired";
      job.finished_at = nowValue;
      changed.push(job);
    }
  }
  return changed;
}

export class ApprovalGate {
  constructor({ file, execute, onUpdate, now, jobs }) {
    this.file = file;
    this.execute = execute;
    this.onUpdate = onUpdate;
    this.now = now;
    this.jobs = jobs;
    this.active = new Map();
  }

  static async open({ file, execute, onUpdate, now = Date.now } = {}) {
    if (typeof file !== "string" || file.length === 0) throw new TypeError("file must be a non-empty path");
    if (typeof execute !== "function") throw new TypeError("execute must be a function");
    if (onUpdate !== undefined && typeof onUpdate !== "function") throw new TypeError("onUpdate must be a function");
    if (typeof now !== "function") throw new TypeError("now must be a function");
    const resolvedFile = resolve(file);
    await ensurePrivateDirectory(dirname(resolvedFile));
    return withLocalMutex(resolvedFile, async () => {
      await verifyExistingFile(resolvedFile);
      const jobs = await readStore(resolvedFile);
      const changed = [];
      const nowValue = currentTime(now);
      for (const job of jobs.values()) {
        if (job.status === "running") {
          job.status = "indeterminate";
          job.finished_at = nowValue;
          changed.push(job);
        }
      }
      if (changed.length > 0 || !(await verifyExistingFile(resolvedFile))) await writeStore(resolvedFile, jobs);
      const gate = new ApprovalGate({ file: resolvedFile, execute, onUpdate, now, jobs });
      for (const job of changed) notify(gate, job);
      return gate;
    });
  }

  async submit(input) {
    const { request, digest } = validateRequest(input);
    const result = await withLocalMutex(this.file, async () => {
      const jobs = await readStore(this.file);
      const nowValue = currentTime(this.now);
      const expired = expirePending(jobs, nowValue);
      const previous = jobs.get(request.operation_id);
      if (previous) {
        if (previous.digest !== digest) throw new Error("operation_id is already bound to a different request digest");
        if (expired.length > 0) await writeStore(this.file, jobs);
        this.jobs = jobs;
        return { job: previous, changed: expired };
      }
      if (request.expires_at <= nowValue) throw new RangeError("expires_at must be in the future");
      if (request.expires_at > nowValue + MAX_DEADLINE_MS) throw new RangeError("expires_at may be at most 10 minutes in the future");
      if (jobs.size >= MAX_JOBS) throw new Error("Approval storage is full; existing operation IDs are retained");
      const job = { ...request, digest, status: "pending", created_at: nowValue };
      jobs.set(job.operation_id, job);
      await writeStore(this.file, jobs);
      this.jobs = jobs;
      return { job, changed: [...expired, job] };
    });
    for (const job of result.changed) notify(this, job);
    return snapshot(result.job);
  }

  async get(operationId) {
    if (typeof operationId !== "string") throw new TypeError("operation_id must be a UUID");
    const result = await withLocalMutex(this.file, async () => {
      const jobs = await readStore(this.file);
      const nowValue = currentTime(this.now);
      const changed = expirePending(jobs, nowValue);
      if (changed.length > 0) await writeStore(this.file, jobs);
      this.jobs = jobs;
      return { job: jobs.get(operationId), changed };
    });
    for (const job of result.changed) notify(this, job);
    return snapshot(result.job);
  }

  async list({ includeTerminal = false } = {}) {
    const result = await withLocalMutex(this.file, async () => {
      const jobs = await readStore(this.file);
      const changed = expirePending(jobs, currentTime(this.now));
      if (changed.length > 0) await writeStore(this.file, jobs);
      this.jobs = jobs;
      return { jobs: [...jobs.values()].filter((job) => includeTerminal || !TERMINAL_STATUSES.has(job.status)), changed };
    });
    for (const job of result.changed) notify(this, job);
    return result.jobs.map(snapshot);
  }

  async deny(operationId, digest) {
    return this.#finishWithoutExecution(operationId, digest, "denied");
  }

  async approve(operationId, digest) {
    this.#validateApproval(operationId, digest);
    let activeExecution;
    const result = await withLocalMutex(this.file, async () => {
      const jobs = await readStore(this.file);
      const job = jobs.get(operationId);
      if (!job) throw new Error("Unknown operation_id");
      this.#assertDigest(job, digest);
      if (job.status !== "pending") {
        this.jobs = jobs;
        return { job, dispatch: false };
      }
      const nowValue = currentTime(this.now);
      if (job.expires_at <= nowValue) {
        job.status = "expired";
        job.finished_at = nowValue;
        await writeStore(this.file, jobs);
        this.jobs = jobs;
        return { job, dispatch: false, changed: true };
      }
      job.status = "running";
      job.started_at = nowValue;
      await writeStore(this.file, jobs);
      this.jobs = jobs;
      activeExecution = deferred();
      activeExecution.promise.catch(() => {});
      this.active.set(operationId, activeExecution);
      return { job, dispatch: true, changed: true };
    });

    if (result.changed) notify(this, result.job);
    if (!result.dispatch) {
      const active = this.active.get(operationId);
      return active ? snapshot(await active.promise) : snapshot(result.job);
    }

    const execution = this.#executeAndPersist(operationId, digest, result.job);
    execution.then(activeExecution.resolve, activeExecution.reject);
    try {
      return snapshot(await execution);
    } finally {
      if (this.active.get(operationId) === activeExecution) this.active.delete(operationId);
    }
  }

  #validateApproval(operationId, digest) {
    if (typeof operationId !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/iu.test(operationId)) {
      throw new TypeError("operation_id must be a UUID");
    }
    if (typeof digest !== "string" || !/^[0-9a-f]{64}$/u.test(digest)) throw new TypeError("digest must be a SHA-256 hex digest");
  }

  #assertDigest(job, digest) {
    if (job.digest !== digest) throw new Error("Approval digest does not match the pending request");
  }

  async #finishWithoutExecution(operationId, digest, status) {
    this.#validateApproval(operationId, digest);
    const result = await withLocalMutex(this.file, async () => {
      const jobs = await readStore(this.file);
      const job = jobs.get(operationId);
      if (!job) throw new Error("Unknown operation_id");
      this.#assertDigest(job, digest);
      if (job.status !== "pending") {
        this.jobs = jobs;
        return { job };
      }
      const nowValue = currentTime(this.now);
      job.status = job.expires_at <= nowValue ? "expired" : status;
      job.finished_at = nowValue;
      await writeStore(this.file, jobs);
      this.jobs = jobs;
      return { job, changed: true };
    });
    if (result.changed) notify(this, result.job);
    return snapshot(result.job);
  }

  async #executeAndPersist(operationId, digest, job) {
    let status = "completed";
    let result;
    try {
      const request = deepFreeze(makeRequest(job));
      const rawResult = await this.execute(request);
      result = copyJson(rawResult);
      if (Buffer.byteLength(canonicalStringify(result), "utf8") > MAX_RESULT_BYTES) {
        throw new RangeError("Execution result exceeds 128 KiB");
      }
    } catch {
      status = "failed";
      result = undefined;
    }

    const persisted = await withLocalMutex(this.file, async () => {
      const jobs = await readStore(this.file);
      const current = jobs.get(operationId);
      if (!current || current.digest !== digest) throw new Error("Approval state changed during execution");
      if (current.status !== "running") {
        this.jobs = jobs;
        return current;
      }
      current.status = status;
      current.finished_at = currentTime(this.now);
      if (status === "completed") current.result = result;
      else current.error = "execution_failed";
      await writeStore(this.file, jobs);
      this.jobs = jobs;
      return current;
    });
    notify(this, persisted);
    return persisted;
  }
}
