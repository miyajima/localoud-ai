type Project = { id: string; name: string; root: string };
type Config = { enabled: boolean; project_ids: string[]; connection: { kind: 'unconfigured' } | { kind: 'secure_tunnel'; tunnel_id: string } | { kind: 'https_proxy'; url: string } };
type Status = { config: Config; running: boolean; error: string | null; local_endpoint: string };
type Invoke = <T = unknown>(command: string, args?: Record<string, unknown>) => Promise<T>;

export function setupMcpSettings(call: Invoke, projects: () => Project[], saved?: () => void) {
  const dialog = document.createElement('dialog');
  dialog.id = 'mcp-settings'; dialog.setAttribute('aria-label', 'MCP公開設定');
  dialog.innerHTML = `<form><h2>MCP公開設定</h2><p>ChatGPTに読ませるプロジェクトを選びます。MCPから実行・変更はできません。</p>
    <label><input type="checkbox" id="mcp-enabled">読み取り専用MCPを有効にする</label>
    <fieldset><legend>公開するプロジェクト</legend><div id="mcp-projects"></div></fieldset>
    <p id="mcp-status" role="status"></p><button type="button" id="mcp-copy-token">ローカル接続キーをコピー</button>
    <label for="mcp-route">ChatGPTへの接続経路</label><select id="mcp-route"><option value="unconfigured">未設定</option><option value="secure_tunnel">Secure MCP Tunnel</option><option value="https_proxy">認証付きHTTPSプロキシ</option></select>
    <label for="mcp-address" id="mcp-address-label">接続先</label><input id="mcp-address" spellcheck="false" autocomplete="off">
    <p>この画面は接続情報を保存します。トンネルや公開サーバーは作成しません。localhostのURLだけではChatGPTから接続できません。接続キーはトンネル／プロキシのローカル接続に設定し、ChatGPTの会話には貼らないでください。</p>
    <p id="mcp-error" role="alert"></p><div class="dialog-actions"><button type="button" id="mcp-close">閉じる</button><button type="submit" class="primary">保存する</button></div></form>`;
  document.body.append(dialog);
  const enabled = dialog.querySelector<HTMLInputElement>('#mcp-enabled')!;
  const route = dialog.querySelector<HTMLSelectElement>('#mcp-route')!;
  const address = dialog.querySelector<HTMLInputElement>('#mcp-address')!;
  const error = dialog.querySelector('#mcp-error')!;
  let saving = false;
  function showRoute() {
    address.hidden = route.value === 'unconfigured';
    dialog.querySelector<HTMLElement>('#mcp-address-label')!.hidden = address.hidden;
    dialog.querySelector('#mcp-address-label')!.textContent = route.value === 'secure_tunnel' ? 'Tunnel ID' : 'HTTPSのMCP URL';
    address.placeholder = route.value === 'secure_tunnel' ? 'tunnel_…' : 'https://…/mcp';
  }
  function showStatus(s: Status) {
    dialog.querySelector('#mcp-status')!.textContent = `${s.running ? 'ローカルMCP起動中' : 'ローカルMCP停止中'} · ${s.local_endpoint} · ChatGPT接続は別途確認が必要です。`;
    error.textContent = s.error || '';
  }
  route.addEventListener('change', showRoute);
  dialog.querySelector('#mcp-close')!.addEventListener('click', () => dialog.close());
  dialog.querySelector('#mcp-copy-token')!.addEventListener('click', () => { void call('mcp_copy_token').then(() => { error.textContent = '接続キーをコピーしました。'; }).catch(e => { error.textContent = String(e); }); });
  dialog.querySelector('form')!.addEventListener('submit', async e => {
    e.preventDefault(); if (saving) return; saving = true; error.textContent = '';
    const button = dialog.querySelector<HTMLButtonElement>('button[type=submit]')!; button.disabled = true;
    const connection: Config['connection'] = route.value === 'secure_tunnel' ? { kind: 'secure_tunnel', tunnel_id: address.value.trim() } : route.value === 'https_proxy' ? { kind: 'https_proxy', url: address.value.trim() } : { kind: 'unconfigured' };
    const config: Config = { enabled: enabled.checked, project_ids: Array.from(dialog.querySelectorAll<HTMLInputElement>('#mcp-projects input:checked')).map(e => e.value), connection };
    try { const status = await call<Status>('set_mcp_settings', { config }); showStatus(status); if (!status.error) { dialog.close(); saved?.(); } }
    catch (e) { error.textContent = String(e); }
    finally { saving = false; button.disabled = false; }
  });
  return async () => {
    error.textContent = '';
    try {
      const status = await call<Status>('mcp_settings');
      enabled.checked = status.config.enabled; route.value = status.config.connection.kind;
      address.value = 'tunnel_id' in status.config.connection ? status.config.connection.tunnel_id : 'url' in status.config.connection ? status.config.connection.url : '';
      const list = dialog.querySelector('#mcp-projects')!; list.replaceChildren();
      for (const project of projects()) {
        const label = document.createElement('label'), box = document.createElement('input');
        box.type = 'checkbox'; box.value = project.id; box.checked = status.config.project_ids.includes(project.id);
        label.append(box, document.createTextNode(project.name)); label.title = project.root; list.append(label);
      }
      showRoute(); showStatus(status);
    } catch (e) { error.textContent = String(e); }
    if (!dialog.open) dialog.showModal();
  };
}
