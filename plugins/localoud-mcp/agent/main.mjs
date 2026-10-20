#!/usr/bin/env node
import { parseArgs } from 'node:util';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { startAgent } from './runtime.mjs';

const { values } = parseArgs({ options: { gateway: { type: 'string' }, config: { type: 'string' },
  'state-dir': { type: 'string' }, binary: { type: 'string' }, port: { type: 'string', default: '8795' } } });
if (!values.gateway || !values.config || !values['state-dir']) {
  console.error('Usage: node main.mjs --gateway https://HOST --config /absolute/projects.json --state-dir /absolute/private-state [--binary /absolute/localoud-mcp] [--port 8795]'); process.exit(2);
}
const port = Number(values.port);
if (!Number.isInteger(port) || port < 1 || port > 65535) { console.error('Invalid port'); process.exit(2); }
try {
  const agent = await startAgent({ gateway: values.gateway, config: resolve(values.config), stateDir: resolve(values['state-dir']),
    binary: values.binary || fileURLToPath(new URL('../../../target/release/localoud-mcp', import.meta.url)), port });
  console.log(JSON.stringify({ service: 'localoud-mac-agent', approval_url: agent.app.url, online: agent.online(),
    authentication: 'Enter the ui-token stored privately in the selected state directory. Never share device.json or ui-token.' }));
  let closing = false;
  const close = async () => { if (closing) return; closing = true; await agent.close(); process.exit(0); };
  process.on('SIGINT', close); process.on('SIGTERM', close);
} catch { console.error('Localoud agent failed to start; verify private state permissions, selected project config, binary and gateway connectivity.'); process.exit(1); }
