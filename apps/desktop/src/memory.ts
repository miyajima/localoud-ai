import { invoke } from '@tauri-apps/api/core';
export type Capture = { candidates: { candidate: { kind: string; statement: string; reason: string; source_refs: string[] } }[]; saved: boolean };
type Config = { endpoint: string; tenant: string; remote_project_id: string; auto_store: boolean };
let project: () => string | null;
let report: (message: string) => void;
let results: { id: string; text: string; source_refs: string[] }[] = [];
const esc = (s: string) => s.replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]!));
const field = (id: string) => document.getElementById(id) as HTMLInputElement;
export function setupMemory(getProject: () => string | null, error: (message: string) => void) {
  project = getProject; report = error;
  document.body.insertAdjacentHTML('beforeend', `<dialog id="memory-settings"><form id="memory-form"><h2>OrgBrain 接続</h2><p>現在のプロジェクトに接続します。認証情報は macOS Keychain に保存します。</p><label for="memory-endpoint">MCP エンドポイント</label><input id="memory-endpoint" type="url" required placeholder="https://your-gateway/mcp"><label for="memory-tenant">テナント</label><input id="memory-tenant" value="default" required><label for="memory-project">OrgBrain のプロジェクト ID</label><input id="memory-project" required><label for="memory-client">CF Access Client ID</label><input id="memory-client" autocomplete="off" placeholder="保存済みの場合は空欄"><label for="memory-secret">CF Access Client Secret</label><input id="memory-secret" type="password" autocomplete="off" placeholder="保存済みの場合は空欄"><label><input id="memory-autosave" type="checkbox"> 完了時の永続候補を自動保存する</label><p>一時的な進捗・生のツールログは保存対象にしません。</p><p id="memory-error" role="alert"></p><div class="dialog-actions"><button id="memory-close" type="button">閉じる</button><button class="primary" type="submit">接続を保存</button></div></form></dialog>`);
  document.querySelector('#memory-close')!.addEventListener('click', close);
  document.querySelector('#memory-form')!.addEventListener('submit', async e => {
    e.preventDefault();
    try {
      await invoke('set_orgbrain_settings', { projectId: project(), config: { endpoint: field('memory-endpoint').value, tenant: field('memory-tenant').value, remote_project_id: field('memory-project').value, auto_store: field('memory-autosave').checked }, clientId: field('memory-client').value, clientSecret: field('memory-secret').value });
      close();
    } catch (e) { document.querySelector('#memory-error')!.textContent = String(e); }
  });
}
function close() {
  (document.querySelector('#memory-settings') as HTMLDialogElement).close();
  field('memory-client').value = ''; field('memory-secret').value = '';
}
async function open() {
  if (!project()) return;
  (document.querySelector('#memory-settings') as HTMLDialogElement).showModal();
  try {
    const c = await invoke<Config | null>('orgbrain_settings', { projectId: project() });
    field('memory-endpoint').value = c?.endpoint || ''; field('memory-tenant').value = c?.tenant || 'default'; field('memory-project').value = c?.remote_project_id || ''; field('memory-autosave').checked = c?.auto_store || false;
    field('memory-client').value = ''; field('memory-secret').value = '';
  } catch (e) { document.querySelector('#memory-error')!.textContent = String(e); }
}
export function memoryPanel(c?: Capture) {
  return `<div class="inspector"><h3>OrgBrain</h3><button id="memory-config">接続設定</button><label for="memory-query">過去の判断・失敗を検索</label><input id="memory-query" placeholder="例: キャッシュ設計で避けること"><button id="memory-search">検索</button><div id="memory-results"></div><h3>この worker の永続メモリ候補</h3><p>${c?.saved ? 'OrgBrain への保存済み' : 'OrgBrain への保存は未確認'}</p>${c?.candidates.map(i => `<div class="agent-card"><strong>${esc(i.candidate.kind)}</strong><p>${esc(i.candidate.statement)}</p><p>理由: ${esc(i.candidate.reason)}</p><small>${esc(i.candidate.source_refs.join(', '))}</small></div>`).join('') || '<p>記録済みの候補はありません。抽出失敗は Terminal で確認できます。</p>'}</div>`;
}
export function bindMemory() {
  document.querySelector('#memory-config')?.addEventListener('click', () => void open());
  document.querySelector('#memory-search')?.addEventListener('click', async () => {
    if (!project()) return;
    const button = document.querySelector('#memory-search') as HTMLButtonElement;
    button.disabled = true;
    try {
      results = await invoke('search_memory', { projectId: project(), query: field('memory-query').value });
      document.querySelector('#memory-results')!.innerHTML = results.map(r => `<details open><summary>${esc(r.id)}</summary><pre>${esc(r.text)}</pre><small>${esc(r.source_refs.join(', '))}</small></details>`).join('') || '<p>該当する記録はありません。</p>';
    } catch (e) { report(String(e)); } finally { button.disabled = false; }
  });
}
