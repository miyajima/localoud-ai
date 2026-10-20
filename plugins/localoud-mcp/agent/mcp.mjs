import { spawn } from 'node:child_process';

export class McpClient {
  constructor({ binary, args, config }) {
    const env = {};
    for (const key of ['PATH', 'HOME', 'TMPDIR', 'LANG', 'LC_ALL']) if (process.env[key]) env[key] = process.env[key];
    this.pending = new Map(); this.nextId = 1; this.buffer = ''; this.closed = false;
    this.child = spawn(binary, args ?? ['--config', config], { env, stdio: ['pipe', 'pipe', 'ignore'] });
    this.child.stdout.setEncoding('utf8');
    this.child.stdout.on('data', chunk => {
      this.buffer += chunk;
      if (Buffer.byteLength(this.buffer) > 512 * 1024) { this.fail(); this.child.kill(); return; }
      let split;
      while ((split = this.buffer.indexOf('\n')) >= 0) {
        const line = this.buffer.slice(0, split); this.buffer = this.buffer.slice(split + 1);
        let response; try { response = JSON.parse(line); } catch { this.fail(); this.child.kill(); return; }
        const entry = this.pending.get(response.id); if (!entry) continue;
        clearTimeout(entry.timer); this.pending.delete(response.id);
        if (response.error) entry.reject(new Error('mcp_tool_error')); else entry.resolve(response.result);
      }
    });
    this.child.on('error', () => { this.closed = true; this.fail(); });
    this.child.on('exit', () => { this.closed = true; this.fail(); });
    this.child.stdin.on('error', () => this.fail());
  }
  fail() { for (const e of this.pending.values()) { clearTimeout(e.timer); e.reject(new Error('mcp_unavailable')); } this.pending.clear(); }
  request(method, params, timeout = 125000) {
    if (this.closed) return Promise.reject(new Error('mcp_unavailable'));
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        if (!this.closed) this.child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/cancelled', params: { requestId: id } }) + '\n');
        reject(new Error('mcp_timeout'));
      }, timeout);
      this.pending.set(id, { resolve, reject, timer });
      this.child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
    });
  }
  async initialize() {
    await this.request('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'localoud-mac-agent', version: '0.1.0' } }, 10000);
    this.child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n');
  }
  tool(name, args) { return this.request('tools/call', { name, arguments: args }); }
  async close() {
    if (this.closed) return;
    for (const id of this.pending.keys()) this.child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/cancelled', params: { requestId: id } }) + '\n');
    this.child.stdin.end();
    await new Promise(resolve => {
      const timer = setTimeout(() => { this.child.kill('SIGKILL'); resolve(); }, 3000);
      this.child.once('exit', () => { clearTimeout(timer); resolve(); });
    });
    this.closed = true; this.fail();
  }
}
