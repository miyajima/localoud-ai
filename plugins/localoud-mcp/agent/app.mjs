import { createServer } from 'node:http';
import { randomBytes, timingSafeEqual, createHash } from 'node:crypto';
import { writeFile, lstat } from 'node:fs/promises';
import { join } from 'node:path';
import { dashboard } from './dashboard.mjs';

const escape = value => String(value).replace(/[&<>"']/g, c => `&#${c.charCodeAt(0)};`);
const same = (a, b) => typeof a === 'string' && a.length === b.length && timingSafeEqual(Buffer.from(a), Buffer.from(b));
const sha = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
async function body(req) {
  let size = 0; const chunks = [];
  for await (const chunk of req) { size += chunk.length; if (size > 8192) throw new Error('body_too_large'); chunks.push(chunk); }
  return Buffer.concat(chunks).toString('utf8');
}
const stylesheet = 'body{font:15px system-ui;line-height:1.65;background:#f4f5f1;color:#1c3026;margin:0}main{max-width:900px;margin:20px auto;padding:16px}h1{font-size:25px}section,article{background:white;border:1px solid #cdd8cf;border-radius:12px;padding:20px;margin:16px 0}pre{font:13px/1.45 ui-monospace,SFMono-Regular,monospace;white-space:pre-wrap;overflow-wrap:anywhere;background:#f0f3ee;padding:14px;border-radius:8px}.operation-detail{max-height:22vh;overflow:auto;overscroll-behavior:contain}input{font:inherit;padding:10px;max-width:95%}button{font:inherit;padding:10px 18px;border:0;border-radius:8px;background:#17503b;color:white;cursor:pointer;margin:4px}button.deny{background:#704534}small{color:#586d60}.status{font-weight:600}summary{cursor:pointer;font-weight:600;padding:12px}details{border:1px solid #cdd8cf;border-radius:12px;background:white;margin:12px 0;padding:8px}#message:empty{display:none}';
function html(content, script = '') {
  const nonce = randomBytes(24).toString('hex');
  return { body: `<!doctype html><html lang="ja"><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>Localoud Mac 承認</title><style nonce="${nonce}">${stylesheet}</style><main>${content}</main>${script ? `<script nonce="${nonce}">${script}</script>` : ''}</html>`, headers: {
    'content-type': 'text/html; charset=utf-8', 'content-security-policy': `default-src 'none'; style-src 'nonce-${nonce}'; script-src 'nonce-${nonce}'; connect-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'`,
    'x-frame-options': 'DENY', 'referrer-policy': 'same-origin', 'cache-control': 'no-store' } };
}

export async function startApprovalApp({ stateDir, gate, pairing, online = () => false, targetFor = () => null, port = 8795 }) {
  const token = randomBytes(32).toString('hex'), csrf = randomBytes(32).toString('hex');
  const tokenPath = join(stateDir, 'ui-token');
  const stat = await lstat(tokenPath).catch(e => { if (e.code === 'ENOENT') return null; throw e; });
  if (stat && (!stat.isFile() || stat.nlink !== 1 || (stat.mode & 0o777) !== 0o600 || stat.uid !== process.getuid())) throw new Error('unsafe_ui_token');
  await writeFile(tokenPath, token, { mode: 0o600 });
  const previews = new Map();
  let url;
  const server = createServer(async (req, res) => {
    const send = (value, status = 200, headers = {}) => { res.writeHead(status, { 'content-type': 'application/json', 'cache-control': 'no-store', ...headers }); res.end(JSON.stringify(value)); };
    try {
      if (req.headers.host !== new URL(url).host || req.headers['sec-fetch-site'] === 'cross-site') return send({ error: 'invalid_origin' }, 403);
      const path = new URL(req.url, url).pathname;
      const cookies = String(req.headers.cookie || '').split(';').map(s => s.trim());
      const session = cookies.find(s => s.startsWith('localoud_session='))?.slice('localoud_session='.length);
      const authenticated = same(session, token);
      if (path === '/session' && req.method === 'POST') {
        if (req.headers.origin !== url || !String(req.headers['content-type']).startsWith('application/x-www-form-urlencoded')) return send({ error: 'invalid_origin' }, 403);
        const data = new URLSearchParams(await body(req));
        if (!same(data.get('token'), token)) return send({ error: 'unauthorized' }, 401);
        res.writeHead(303, { location: '/', 'cache-control': 'no-store', 'set-cookie': `localoud_session=${token}; HttpOnly; SameSite=Strict; Path=/` }); return res.end();
      }
      if (req.method === 'GET' && path === '/') {
        const page = authenticated ? dashboard(csrf, html) : html(`<h1>Localoud Mac 承認</h1><p>このMacの承認画面にログインします。起動時に保存した <code>ui-token</code> の値を入力してください。外部サービスに送信されません。</p><form action="/session" method="post"><input type="password" name="token" autocomplete="off" aria-label="ローカル認証コード" required><button>承認画面を開く</button></form>`);
        res.writeHead(200, page.headers); return res.end(page.body);
      }
      if (!authenticated) return send({ error: 'unauthorized' }, 401);
      if (req.method === 'GET' && path === '/api/state') {
        const all = await gate.list({ includeTerminal: true });
        const active = job => job.status === 'pending' || job.status === 'running';
        return send({ online: online(),
          jobs: all.filter(active).map(job => ({ ...job, target_root: targetFor(job.arguments.project_id) })),
          // Historical arguments and roots may belong to a previous scope. Keep
          // the history projection minimal rather than relabeling old requests.
          history: all.filter(job => !active(job)).map(job => ({ operation_id: job.operation_id, tool: job.tool,
            status: job.status, created_at: job.created_at, finished_at: job.finished_at, expires_at: job.expires_at })),
        });
      }
      if (req.method !== 'POST') return send({ error: 'not_found' }, 404);
      if (req.headers.origin !== url || !same(req.headers['x-localoud-csrf'], csrf) || !String(req.headers['content-type']).startsWith('application/json')) return send({ error: 'invalid_origin' }, 403);
      const data = JSON.parse(await body(req));
      if (path === '/api/approve') return send(await gate.approve(data.operation_id, data.digest));
      if (path === '/api/deny') return send(await gate.deny(data.operation_id, data.digest));
      if (path === '/api/pair-preview') {
        const details = await pairing('preview', data.code);
        const preview = { code: data.code, details, preview_digest: sha({ code: data.code, details }) };
        previews.clear(); previews.set(data.code, { digest: preview.preview_digest, expires: Date.now() + 120000 });
        return send(preview);
      }
      if (path === '/api/pair-approve') {
        const preview = previews.get(data.code);
        if (!preview || preview.expires < Date.now() || preview.digest !== data.preview_digest) return send({ error: 'preview_required' }, 400);
        previews.delete(data.code); return send(await pairing('approve', data.code));
      }
      return send({ error: 'not_found' }, 404);
    } catch { if (!res.headersSent) send({ error: 'operation_rejected' }, 400); else res.end(); }
  });
  server.requestTimeout = 150000; server.headersTimeout = 10000; server.maxHeadersCount = 32;
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(port, '127.0.0.1', resolve); });
  url = `http://127.0.0.1:${server.address().port}`;
  return { url, close: () => new Promise(resolve => { server.closeAllConnections(); server.close(resolve); }) };
}
