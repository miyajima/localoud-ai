import { isTauri } from '@tauri-apps/api/core';
import './browser-workspace.css';

type Invoke = <T = unknown>(command: string, args?: Record<string, unknown>) => Promise<T>;

export function setupBrowserWorkspace(invoke: Invoke) {
  const app = document.querySelector<HTMLElement>('#app')!;
  const main = app.querySelector<HTMLElement>('main')!;
  const header = main.querySelector('header')!;
  header.querySelector('#project-title')?.parentElement?.classList.add('project-heading');
  const shell = document.createElement('section'); shell.id = 'workflow-shell';
  const workflow = document.createElement('section'); workflow.id = 'workflow-bar';
  workflow.setAttribute('aria-label', '選択中のタスクの工程'); shell.append(workflow);
  for (const id of ['error', 'notice']) { const feedback = document.getElementById(id); if (feedback) shell.append(feedback); }
  header.after(shell);

  const toggle = document.createElement('button'); toggle.id = 'browser-toggle'; toggle.type = 'button';
  toggle.textContent = 'ChatGPTに相談'; toggle.title = 'ChatGPTを開く ⌘⇧B';
  toggle.setAttribute('aria-controls', 'browser-panel'); toggle.setAttribute('aria-haspopup', 'dialog'); header.append(toggle);
  const overlay = document.createElement('div'); overlay.id = 'browser-overlay'; overlay.hidden = true;
  const panel = document.createElement('section'); panel.id = 'browser-panel'; panel.setAttribute('role', 'dialog'); panel.setAttribute('aria-modal', 'true'); panel.setAttribute('aria-labelledby', 'browser-title');
  panel.innerHTML = '<div class="browser-toolbar"><div><strong id="browser-title">ChatGPT Web</strong><span>計画・相談・レビュー</span></div><button id="browser-back" type="button" title="前のページに戻る">← 戻る</button><button id="browser-home" type="button">ChatGPTホーム</button><button id="browser-reload" type="button" aria-label="ChatGPTを再読み込み" title="再読み込み"><svg aria-hidden="true" viewBox="0 0 16 16"><path d="M13 6A5 5 0 1 0 14 9"/><path d="M13 3v3h-3"/></svg></button><button id="browser-close" type="button">作業に戻る <span aria-hidden="true">×</span></button></div><div id="chatgpt-usage"></div><div id="browser-surface"><p id="browser-error" role="alert" aria-live="assertive"></p></div><section id="browser-handoff" aria-label="Localoudとの受け渡し" hidden></section>';
  overlay.append(panel); document.body.append(overlay);
  const divider = document.createElement('div'); divider.id = 'sidebar-divider'; divider.tabIndex = 0;
  divider.setAttribute('role', 'separator'); divider.setAttribute('aria-label', 'プロジェクト一覧の幅'); divider.setAttribute('aria-orientation', 'vertical'); main.before(divider);
  document.body.classList.add('browser-open');
  const stored = Number(localStorage.getItem('localoud-sidebar-width') ?? 234);
  let sidebarWidth = Math.max(160, Math.min(360, Number.isFinite(stored) ? stored : 234));
  let pending = false, again = false, lastBounds = '', focusRevision = 0, appliedFocus = 0, previousFocus: HTMLElement | null = null;
  function reflect() {
    app.style.setProperty('--sidebar-width', `${sidebarWidth}px`);
    divider.setAttribute('aria-valuemin', '160'); divider.setAttribute('aria-valuemax', '360'); divider.setAttribute('aria-valuenow', String(sidebarWidth));
    toggle.setAttribute('aria-expanded', String(!overlay.hidden)); void layout();
  }
  async function layout() {
    const message = panel.querySelector('#browser-error')!;
    if (!isTauri()) { message.textContent = 'ChatGPTはデスクトップアプリで表示できます。'; return; }
    if (pending) { again = true; return; } pending = true;
    const rect = panel.querySelector('#browser-surface')!.getBoundingClientRect();
    const visible = !overlay.hidden && !document.querySelector('dialog[open]');
    const focusRequest = focusRevision;
    const bounds = { x: Math.max(0, rect.x), y: Math.max(0, rect.y), width: Math.max(1, rect.width), height: Math.max(1, rect.height), viewport_height: window.innerHeight, visible, focus: focusRequest !== appliedFocus };
    const key = JSON.stringify(bounds);
    try { if (key !== lastBounds) { await invoke('browser_layout', { bounds }); lastBounds = JSON.stringify({...bounds,focus:false}); appliedFocus = focusRequest; } message.textContent = ''; }
    catch (e) { lastBounds = ''; message.textContent = `ChatGPTを開けませんでした。再読み込みしてください。 ${String(e)}`; }
    finally { pending = false; if (again) { again = false; void layout(); } }
  }
  function closeBrowser() {
    const wasOpen = !overlay.hidden; overlay.hidden = true; app.inert = false;
    if (wasOpen) {
      focusRevision++;
      const composer = main.querySelector<HTMLTextAreaElement>('#task-input');
      (composer?.getClientRects().length && !composer.disabled ? composer : previousFocus || toggle).focus();
    }
    reflect();
  }
  function showBrowser() {
    if (overlay.hidden) {
      previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      overlay.hidden = false; app.inert = true; focusRevision++;
      // Keep local controls and the native ChatGPT webview accessible together.
      panel.querySelector<HTMLButtonElement>('#browser-close')!.focus();
    }
    reflect();
  }
  document.addEventListener('contextmenu', event => event.preventDefault());
  const guide = document.createElement('dialog');
  guide.id = 'chatgpt-paste-guide';
  guide.innerHTML = '<h2>ChatGPTへの依頼をコピーしました</h2><p>クリップボードに依頼を入れました。ChatGPT Webのチャット欄にペーストして送信してください。</p><p id="chatgpt-return-guide"></p><div class="dialog-actions"><button type="button" id="guide-close">閉じる</button><button type="button" id="guide-open" class="primary">ChatGPTを開く</button></div>';
  document.body.append(guide);
  guide.querySelector('#guide-close')!.addEventListener('click', () => guide.close());
  guide.querySelector('#guide-open')!.addEventListener('click', () => { guide.close(); showBrowser(); });
  function showCopiedRequestGuide(kind: 'plan' | 'review') {
    closeBrowser();
    guide.querySelector('#chatgpt-return-guide')!.textContent = `回答が来たら、回答内の${kind === 'plan' ? '計画' : 'レビュー結果'}のJSONをコピーし、画面下部の「${kind === 'plan' ? 'コピーした計画を確認' : 'コピーした結果を確認'}」を押してください。内容を確認してLocaloudに取り込めます。`;
    guide.showModal();
  }
  for (const action of ['back', 'home']) {
    const button = panel.querySelector<HTMLButtonElement>(`#browser-${action}`)!;
    button.addEventListener('click', async () => {
      button.disabled = true;
      try { await invoke(`browser_${action}`); }
      catch (e) { panel.querySelector('#browser-error')!.textContent = `移動できませんでした。 ${String(e)}`; }
      finally { button.disabled = false; }
    });
  }
  toggle.addEventListener('click', showBrowser);
  panel.querySelector('#browser-close')!.addEventListener('click', closeBrowser);
  overlay.addEventListener('click', e => { if (e.target === overlay) closeBrowser(); });
  panel.addEventListener('keydown', e => {
    if (e.key === 'Escape') { e.preventDefault(); closeBrowser(); }
    if (e.key === 'Tab') {
      const controls = Array.from(panel.querySelectorAll<HTMLElement>('button:not(:disabled), summary, a[href]')).filter(el => el.getClientRects().length);
      const first = controls[0], last = controls.at(-1);
      if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last?.focus(); }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first?.focus(); }
    }
  });
  const reload=panel.querySelector<HTMLButtonElement>('#browser-reload')!;
  panel.querySelector('#browser-error')!.removeAttribute('aria-live');
  reload.addEventListener('click', async () => {
    if(reload.disabled)return;reload.disabled=true;reload.setAttribute('aria-busy','true');
    try{await invoke('browser_reload');panel.querySelector('#browser-error')!.textContent='';}
    catch(e){panel.querySelector('#browser-error')!.textContent=`再読み込みに失敗しました。接続を確認して、もう一度お試しください。 ${String(e)}`;}
    finally{reload.disabled=false;reload.setAttribute('aria-busy','false');}
  });
  window.addEventListener('keydown', e => { if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.code === 'KeyB') { e.preventDefault(); overlay.hidden ? showBrowser() : closeBrowser(); } });
  divider.addEventListener('pointerdown', e => {
    divider.setPointerCapture(e.pointerId);
    const move = (event: PointerEvent) => { sidebarWidth = Math.max(160, Math.min(360, event.clientX - app.getBoundingClientRect().x)); reflect(); };
    const stop = () => { localStorage.setItem('localoud-sidebar-width', String(sidebarWidth)); divider.removeEventListener('pointermove', move); divider.removeEventListener('pointerup', stop); divider.removeEventListener('pointercancel', stop); };
    divider.addEventListener('pointermove', move); divider.addEventListener('pointerup', stop); divider.addEventListener('pointercancel', stop);
  });
  divider.addEventListener('keydown', e => { if (!['ArrowLeft', 'ArrowRight'].includes(e.key)) return; e.preventDefault(); sidebarWidth = Math.max(160, Math.min(360, sidebarWidth + (e.key === 'ArrowLeft' ? -16 : 16))); localStorage.setItem('localoud-sidebar-width', String(sidebarWidth)); reflect(); });
  new MutationObserver(() => void layout()).observe(document.body, { subtree: true, attributes: true, attributeFilter: ['open'] });
  new ResizeObserver(() => void layout()).observe(panel.querySelector('#browser-surface')!);
  window.addEventListener('resize', reflect); reflect();
  return {
    showWorkspace: closeBrowser, showBrowser, showCopiedRequestGuide, hideBrowser: closeBrowser, isBrowserOpen: () => !overlay.hidden,
    setSidebarHidden: (value: boolean) => { app.classList.toggle('sidebar-hidden', value); const button = app.querySelector('#sidebar-toggle'); button?.setAttribute('aria-expanded', String(!value)); button?.setAttribute('aria-label', value ? 'プロジェクト一覧を表示' : 'プロジェクト一覧を閉じる'); reflect(); },
    setWorkflow: (markup: string, action: (name: string) => void) => {
      if (workflow.innerHTML === markup) return; workflow.innerHTML = markup;
      const handoff = panel.querySelector<HTMLElement>('#browser-handoff')!;
      handoff.hidden = !workflow.querySelector('[data-flow-kind="manifest"]');
      handoff.innerHTML = handoff.hidden ? '' : (workflow.querySelector('.flow-title')?.outerHTML || '') + (workflow.querySelector('.flow-next')?.outerHTML || '');
      for (const surface of [workflow, handoff]) surface.querySelectorAll<HTMLButtonElement>('[data-flow-action]').forEach(button => button.addEventListener('click', () => action(button.dataset.flowAction!)));
      void layout();
    }
  };
}
