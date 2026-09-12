import './settings-navigation.css';

export function setupSettingsNavigation(dialog: HTMLDialogElement) {
  const form = dialog.querySelector<HTMLFormElement>('#settings-form')!;
  const heading = form.querySelector('h2')!;
  heading.textContent = '設定';
  const layout = document.createElement('div'); layout.className = 'settings-layout';
  const menu = document.createElement('nav'); menu.className = 'settings-menu'; menu.setAttribute('aria-label', '設定メニュー');
  const content = document.createElement('div'); content.className = 'settings-content';
  const panels = new Map<string, HTMLElement>();
  const buttons = new Map<string, HTMLButtonElement>();
  function select(id: string) {
    for (const [key, panel] of panels) { panel.hidden = key !== id; buttons.get(key)!.setAttribute('aria-current', key === id ? 'page' : 'false'); }
    content.scrollTop = 0;
  }
  for (const [id, title] of [['connection', '接続・モデル'], ['providers', 'APIプロバイダー'], ['mcp', 'MCP公開'], ['usage', '使用量']]) {
    const panel = document.createElement('section'); panel.id = `settings-${id}-panel`; panels.set(id, panel);
    const button = document.createElement('button'); button.type = 'button'; button.textContent = title; button.dataset.settingsPage = id;
    button.setAttribute('aria-controls', panel.id); button.addEventListener('click', () => { select(id); if (id === 'mcp' && !mcpLoaded) { mcpLoaded = true; void loadMcp?.(); } });
    buttons.set(id, button); menu.append(button); content.append(panel);
  }
  let mcpLoaded = false;
  dialog.addEventListener('close', () => { mcpLoaded = false; });
  let loadMcp: (() => Promise<void>) | undefined;
  const providers = dialog.querySelector('#provider-settings')!;
  panels.get('providers')!.append(providers);
  panels.get('usage')!.append(dialog.querySelector('#total-usage')!);
  const chatHeading = document.createElement('h2'); chatHeading.textContent = 'ChatGPT Webの使用量';
  const note = document.createElement('p'); note.textContent = '内蔵ChatGPTの記録にはプロジェクト情報がないため、プロジェクト別の内訳には含めていません。';
  panels.get('usage')!.append(chatHeading, note, dialog.querySelector('#chatgpt-usage')!);
  form.querySelector('#mcp-settings-button')!.remove();
  const connectionTitle = document.createElement('h2'); connectionTitle.textContent = '接続・モデル'; form.prepend(connectionTitle);
  panels.get('connection')!.append(form);
  const top = document.createElement('div'); top.className = 'settings-heading';
  const close = document.createElement('button'); close.type = 'button'; close.textContent = '閉じる'; close.addEventListener('click', () => dialog.close());
  top.append(heading, close); layout.append(menu, content); dialog.append(top, layout);
  dialog.querySelectorAll('label:has(input[type="checkbox"])').forEach(label => label.classList.add('settings-check'));
  select('connection');
  return { mcp: panels.get('mcp')!, select, setMcpLoader: (load: () => Promise<void>) => { loadMcp = load; } };
}
