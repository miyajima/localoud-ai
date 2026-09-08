import { markdown, diffMarkup, statusLabel, planPreview } from './presentation';
import {setupMemory,memoryPanel,bindMemory} from './memory';
import type {Capture} from './memory';
import { invoke as nativeInvoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './style.css';
import './auto-routing.css';
import {fillReasoningSelect,readTarget} from './model-controls';
import type {ModelChoice,ModelTarget} from './model-controls';
import {setupAutoRouting,showAutoPreview,openAutoSettings} from './auto-routing';
import type {AutoPreview} from './auto-routing';
async function invoke<T = unknown>(command: string, args?: Record<string, unknown>): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([nativeInvoke<T>(command, args), new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error(`操作の応答がありません (${command})。再開して状態を確認してください。`)), ['preview_auto_route','create_astra_plan','final_review','diagnose_task'].includes(command) ? 270000 : command === 'run_local' ? 240000 : 40000);
    })]);
  } finally { clearTimeout(timer); }
}
type Project = { id: string; name: string; root: string };
type Thread = { id: string; project_id: string; provider_thread_id: string; provider: string; title: string; status: string };
type AgentEvent = { thread_id: string | null; turn_id: string | null; item_id: string | null; kind: string; text: string };
type JournalEvent = { sequence: number; event: AgentEvent };
type ChatMessage = { role: string; text: string; key: string };
type Snapshot = { active_turn: { id: string } | null; messages: { role: string; text: string }[] };
type View = { messages: ChatMessage[]; events: JournalEvent[]; diff: string; running: boolean; needsResume: boolean; sequence: number; route?: string; summary?: string; review?: string; finalReview?: {verdict:string;findings:string[];rework_instruction:string|null}; recovery?:{action:string;reason:string;instruction:string;replacement:unknown|null}; activeTurn?:string; completedTurns?:Set<string>; capture?:Capture; context?: ContextInspection|null };
let projects: Project[] = [], threads: Thread[] = [], activeProject: string | null = null, activeThread: string | null = null;
const views = new Map<string, View>();
const tabs = ['Chat', 'Plan', 'Diff', 'Agents', 'Terminal', 'Context', 'Usage'];
let taskFilter = '';
let picking = false;
type Draft = {text:string;files:string;preference:string;mode?:string;reasoning?:string|null};
let composerReasoning:string|null=null;
let threadReasoning:Record<string,string>={};
function selectedReasoning(){return (document.querySelector('#reasoning') as HTMLSelectElement).value||null;}
function refreshComposerReasoning(){
  const select=document.querySelector<HTMLSelectElement>('#reasoning')!;
  if(!activeThread&&(document.querySelector('#preference') as HTMLSelectElement).value==='auto'){select.replaceChildren(new Option('Auto 設定を使用',''));select.disabled=true;return;}
  const current=threads.find(t=>t.id===activeThread);
  const model=activeThread?modelChoices.find(m=>m.model===threadModels[activeThread!]&&m.local===(current?.provider==='spark')):selectedModel();
  fillReasoningSelect(select,model,activeThread?threadReasoning[activeThread]||null:composerReasoning);
  select.disabled ||= !!activeThread||busy;
}
type LocalConfig = {endpoint:string;model_id:string;display_name:string;quantization_bits:number};
let modelChoices:ModelChoice[]=[], modelWarnings:string[]=[], threadModels:Record<string,string>={}, localConfig:LocalConfig|null=null;
let loadingModels=false;let modelRefresh:Promise<void>|null=null;let pendingPreference:string|undefined;
function threadModelLabel(t:Thread|undefined){return t ? threadModels[t.id] || (t.provider==='spark'?'ローカルモデル（過去のタスク）':'モデル未取得') : '';}
function selectedModel(){return modelChoices.find(m=>m.key===(document.querySelector('#preference') as HTMLSelectElement).value);}
function planningMode(){return (document.querySelector('#task-mode') as HTMLSelectElement).value==='plan';}
function updateModelOptions(preferred?:string){
  const select=document.querySelector<HTMLSelectElement>('#preference')!, previous=preferred??pendingPreference??select.value;pendingPreference=previous;
  const choices=modelChoices.filter(m=>!planningMode()||!m.local);
  select.replaceChildren();
  if(!planningMode())select.add(new Option('自動','auto'));
  for(const m of choices)select.add(new Option(m.label+(m.local?' · ローカル':''),m.key));
  if(Array.from(select.options).some(o=>o.value===previous))select.value=previous;
  else if(previous && previous!=='auto'){const missing=new Option('選択したモデルは利用できません',previous);missing.disabled=true;select.add(missing);select.value=previous;}
  else select.value=planningMode()?(choices[0]?.key||''):'auto';
  refreshComposerReasoning();
  renderModelHint();
}
function renderModelHint(){
  const m=selectedModel(), select=document.querySelector<HTMLSelectElement>('#preference')!;
  document.querySelector('#model-hint')!.textContent=loadingModels?'モデル一覧を確認中…':activeThread?'このタスクのモデル: '+threadModelLabel(threads.find(t=>t.id===activeThread)):planningMode()?'選択したモデルで計画を作成します。実装は計画の確認後に開始します。':m?.local?'対象ファイルを1〜2件指定してください。':select.value==='auto'?'依頼に応じてモデルを選びます。計画が必要な場合は実装を開始せず案内します。':m?'選択したモデルで実装します。':'接続設定を確認し、モデル一覧を更新してください。';
}
function refreshModels():Promise<void>{
  if(modelRefresh)return modelRefresh;
  modelRefresh=loadModels().finally(()=>{modelRefresh=null;});
  return modelRefresh;
}
async function loadModels(){
  loadingModels=true;renderModelHint();
  try{const result=await invoke<{models:ModelChoice[];warnings:string[]}>('available_models');modelChoices=result.models;modelWarnings=result.warnings;[threadModels,threadReasoning]=await Promise.all([invoke<Record<string,string>>('thread_models'),invoke<Record<string,string>>('thread_reasoning')]);updateModelOptions();if(modelWarnings.length)notice(modelWarnings.join('\n'));}
  catch(e){error(String(e));}finally{loadingModels=false;render();}
}
type UiState = {project?:string|null;thread?:string|null;tab?:string;sidebarHidden?:boolean;pins?:string[];drafts?:Record<string,Draft>;plans?:Record<string,string>};
let ui: UiState = {};
try { ui = JSON.parse(localStorage.getItem('astra-ui-v1') || '{}') || {}; } catch { ui = {}; }
if(!ui || typeof ui!=='object' || Array.isArray(ui))ui={};
if(!ui.drafts || typeof ui.drafts!=='object' || Array.isArray(ui.drafts))ui.drafts={};
if(!ui.plans || typeof ui.plans!=='object' || Array.isArray(ui.plans))ui.plans={};
if(!Array.isArray(ui.pins))ui.pins=[];

type Usage = { provider:string; model:string; prompt_tokens:number|null; completion_tokens:number|null; cached_tokens:number|null; latency_ms:number|null };
let usage:Usage[]=[];
type GraphTask = {id:string;title:string;description:string;status:string;dependencies:string[];worktree_id:string|null};
type ContextInspection = {initial_tokens:number;retrieved_tokens:number;current_estimate:number;parent_conversation_inherited:boolean;capsule:{goal:string;budget:{max_total_tokens:number};items:{source:string;reference:string;text:string}[]};retrievals:{source:string;query:string;token_estimate:number;result_ref:string}[]};
let graph:GraphTask[]=[];
let planText=JSON.stringify({title:'実装計画',steps:[{key:'implementation',title:'実装と検証',goal:'ここに具体的な実装内容を記入',dependencies:[],brief:{acceptance_criteria:['ここに合格条件を記入'],constraints:['公開 API を変更しない'],context_items:[],budget:{initial_tokens:6000,max_total_tokens:24000,max_single_retrieval_tokens:2000}}}]},null,2);
const defaultPlanText=planText;
let activeTab = 'Chat', busy = false;
let renderedContentKey='';
const app = document.querySelector<HTMLDivElement>('#app')!;
app.innerHTML = `<aside><div class="brand"><span class="mark">✳</span> Astra Hub <small>LOCAL WORKSPACE</small></div><button id="new" class="new" title="新しいタスク ⌘N">＋ 新しいタスク <kbd>⌘N</kbd></button><button id="command-menu" class="command-trigger">タスク・操作を検索 <kbd>⌘K</kbd></button><div class="section-title">PROJECTS <button id="add" aria-label="フォルダを開く" title="フォルダを開く ⌘O">＋</button></div><div id="projects"></div><div class="section-title">TASKS</div><input id="task-filter" type="search" spellcheck="false" autocorrect="off" autocapitalize="off" aria-label="タスクを絞り込む" placeholder="タスクを絞り込む"><div id="threads"></div><button id="settings" class="settings">接続設定</button><div class="sidebar-bottom"><span class="dot"></span> Local-first <small>Context stays intentional.</small></div></aside><main><header><div><span class="eyebrow">WORKSPACE</span><h1 id="project-title">プロジェクトを開く</h1><button id="project-path" class="project-path" title="フォルダのパスをコピー"></button></div><div class="header-actions"><button id="sidebar-toggle" title="サイドバーを切り替え ⌘B" aria-label="サイドバーを切り替え">☰</button><button id="rename-task" title="タスク名を変更" hidden>名前を変更</button><span id="status" class="badge">Codex app-server</span><button id="resume" hidden>再開・状態を確認</button></div></header><nav aria-label="ワークスペースの表示">${tabs.map(t => `<button data-tab="${t}">${t}</button>`).join('')}</nav><div id="error" role="alert" hidden></div><div id="notice" role="status" hidden></div><section id="content" aria-live="polite"></section><footer><div class="compose"><textarea id="task-input" aria-label="タスクの指示" placeholder="実装したいことを入力…"></textarea><div class="compose-controls"><div class="executor-controls"><label for="task-mode">操作</label><select id="task-mode"><option value="implement">実装する</option><option value="plan">計画する</option></select><label for="preference">モデル</label><select id="preference"><option value="auto">自動</option></select><label for="reasoning">Reasoning</label><select id="reasoning"><option value="">既定</option></select><button id="auto-settings-button" type="button">Auto 設定</button><button id="refresh-models" type="button" title="利用できるモデルを再取得" aria-label="モデル一覧を更新">↻</button><input spellcheck="false" autocorrect="off" autocapitalize="off" id="known-files" aria-label="対象ファイル" placeholder="対象ファイル（例: src/main.rs）"></div><div><button id="stop" hidden>停止</button><button id="send" class="primary">実行 ↑</button></div></div></div><p id="model-hint" class="model-hint"></p><div class="footnote"><span id="draft-status"></span><span>⌘Enter で送信 · Enter で改行</span></div></footer></main><dialog id="register"><form><h2>プロジェクトを登録</h2><p>作業する Git リポジトリのフォルダを選択してください。</p><button id="browse-project" type="button" class="primary">フォルダを選択…</button><label for="path">フォルダの絶対パス</label><input spellcheck="false" autocorrect="off" autocapitalize="off" id="path" required placeholder="/Users/you/projects/my-app"><p id="form-error" role="alert"></p><div class="dialog-actions"><button type="button" id="cancel">キャンセル</button><button class="primary" type="submit">登録する</button></div></form></dialog><dialog id="connection-settings"><form id="settings-form"><h2>接続設定</h2><p>デスクトップアプリから使う Codex 実行ファイルを指定します。</p><label for="codex-path">Codex 実行ファイルの絶対パス</label><button id="browse-codex" type="button">ファイルを選択…</button><input spellcheck="false" autocorrect="off" autocapitalize="off" id="codex-path" required placeholder="/opt/homebrew/bin/codex"><label for="astra-mode">レビュー・診断の利用経路</label><select id="astra-mode"><option value="disabled">無効（手動計画）</option><option value="codex_integrated">Codex 認証を利用</option></select><label for="astra-model">既定のレビュー・診断モデル ID</label><input id="astra-model" value="gpt-6-astra"><label for="astra-reasoning">レビュー・診断の Reasoning</label><select id="astra-reasoning"><option value="">既定</option></select><p>直接 API の課金設定とは別です。最終レビュー・診断で使用します。計画のモデルは入力欄で選択します。</p><fieldset><legend>ローカルモデル</legend><p>対応サービスを起動し、その接続情報を指定します。保存時にモデルを照合し、再起動後に反映します。</p><label for="local-name">表示名</label><input id="local-name" required><label for="local-id">モデル ID（サービスの返す値）</label><input id="local-id" required spellcheck="false" autocorrect="off" autocapitalize="off"><label for="local-endpoint">接続先</label><input id="local-endpoint" required spellcheck="false" autocorrect="off" autocapitalize="off"><label for="local-bits">量子化ビット数</label><input id="local-bits" type="number" min="1" max="32" required></fieldset><p id="settings-error" role="alert"></p><div class="dialog-actions"><button type="button" id="settings-cancel">閉じる</button><button type="submit" class="primary">保存する</button></div></form></dialog>`;
document.body.insertAdjacentHTML('beforeend', `<dialog id="commands" aria-label="タスク・操作を検索"><label for="command-query">タスク・操作を検索</label><input id="command-query" type="search" spellcheck="false" autocorrect="off" autocapitalize="off" placeholder="タスク名、プロジェクト名、操作…" autocomplete="off"><div id="command-results"></div><small>↑↓ で選択 · Enter で開く · Esc で閉じる</small></dialog><dialog id="rename-dialog"><form id="rename-form"><h2>タスク名を変更</h2><label for="task-name">タスク名</label><input id="task-name" required maxlength="120"><p id="rename-error" role="alert"></p><div class="dialog-actions"><button type="button" id="rename-cancel">キャンセル</button><button type="submit" class="primary">保存</button></div></form></dialog>`);
const escape = (s: string) => s.replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]!));
function view(id: string): View { if (!views.has(id)) views.set(id, { messages: [], events: [], diff: '', running: false, needsResume: true, sequence: 0 }); return views.get(id)!; }
function selectedView() { return activeThread ? view(activeThread) : null; }
function error(message: string) { if(message)notice(''); const el = document.querySelector<HTMLDivElement>('#error')!; el.hidden = !message; el.innerHTML = message ? `<span>${escape(message)}</span><button aria-label="エラーを閉じる" id="dismiss-error">×</button>` : ''; document.querySelector('#dismiss-error')?.addEventListener('click',()=>error('')); }
function render() {
  document.querySelector('#app')!.classList.toggle('sidebar-hidden',!!ui.sidebarHidden);
  document.querySelector('#projects')!.innerHTML = projects.map(p => `<button class="project ${p.id === activeProject ? 'selected' : ''}" data-project="${p.id}" title="${escape(p.root)}">◈ <span>${escape(p.name)}</span></button>`).join('') || '<p class="muted side-empty">リポジトリを登録して<br>作業を始めましょう。</p>';
  renderThreads();
  const project = projects.find(p=>p.id===activeProject);
  const path = document.querySelector<HTMLButtonElement>('#project-path')!; path.textContent=project?.root || ''; path.hidden=!project; path.title=`${project?.root || ''} — クリックでコピー`;
  document.querySelector<HTMLButtonElement>('#rename-task')!.hidden=!activeThread;
  document.querySelector('#project-title')!.textContent = projects.find(p => p.id === activeProject)?.name || 'プロジェクトを開く';
  document.querySelectorAll<HTMLButtonElement>('[data-tab]').forEach(b => { b.classList.toggle('active', b.dataset.tab === activeTab); b.onclick = () => { activeTab = b.dataset.tab!; rememberSelection(); render(); if(activeTab === 'Diff' && activeThread) void refreshDiff(activeThread); if(activeTab === 'Usage') void refreshUsage(); if(activeTab === 'Context' && activeThread) void refreshContext(activeThread); if(activeTab === 'Plan') void refreshGraph(); }; });
  document.querySelectorAll<HTMLButtonElement>('[data-project]').forEach(b => b.onclick = () => { switchProject(b.dataset.project!); });
  document.querySelectorAll<HTMLButtonElement>('[data-thread]').forEach(b => b.onclick = () => void selectThread(b.dataset.thread!));
  const v = selectedView();
  const current = threads.find(t=>t.id===activeThread);
  const providerLabel = threadModelLabel(current);
  const modelSelect=document.querySelector<HTMLSelectElement>('#preference')!;
  modelSelect.disabled = !!activeThread || busy;
  if(current){const key=`thread:${current.id}`;let option=Array.from(modelSelect.options).find(o=>o.value===key);if(!option){option=new Option(threadModelLabel(current),key);modelSelect.add(option);}option.text=threadModelLabel(current);modelSelect.value=key;}
  const status = document.querySelector('#status')!;
  status.textContent = busy ? '処理中…' : v?.needsResume ? '状態確認が必要' : v?.running ? `${providerLabel} · 実行中` : current ? providerLabel : 'Local workspace';
  const resume = document.querySelector<HTMLButtonElement>('#resume')!; resume.hidden = !activeThread; resume.disabled = busy;
  const send = document.querySelector<HTMLButtonElement>('#send')!; send.disabled = busy || !activeProject || !!v?.needsResume || !isTauri(); send.textContent = busy ? '処理中…' : v?.running ? '追記を送る ↑' : '実行 ↑';
  const stop = document.querySelector<HTMLButtonElement>('#stop')!; stop.hidden = !v?.running || current?.provider === 'spark'; stop.disabled = busy || !!v?.needsResume;
  const input = document.querySelector<HTMLTextAreaElement>('#task-input')!; input.disabled = busy || !activeProject || !isTauri(); input.placeholder = v?.running ? '実行中のタスクに追加の指示…' : '実装したいことを入力…';
  (document.querySelector('#known-files') as HTMLInputElement).hidden=planningMode()&&!activeThread;
  (document.querySelector('#task-mode') as HTMLSelectElement).disabled=!!activeThread||busy;
  (document.querySelector('#refresh-models') as HTMLButtonElement).disabled=busy||loadingModels;
  if(!busy&&!v?.running)send.textContent=planningMode()&&!activeThread?'計画を作成 ↑':'実行 ↑';
  refreshComposerReasoning();
  (document.querySelector('#auto-settings-button') as HTMLButtonElement).disabled=busy;
  if(!activeThread && (planningMode() || (document.querySelector('#preference') as HTMLSelectElement).value!=='auto'))send.disabled ||= !readTarget(document.querySelector<HTMLSelectElement>('#preference')!,document.querySelector<HTMLSelectElement>('#reasoning')!,modelChoices);
  renderModelHint();
  renderContent();
}
function renderContent() {
  const content = document.querySelector('#content')!, v = selectedView();
  const key=`${activeProject}:${activeThread}:${activeTab}`, sameView=key===renderedContentKey;renderedContentKey=key;
  const oldScroll = sameView?content.scrollTop:0, follow = !sameView || content.scrollHeight-content.clientHeight-oldScroll < 64;
  const focused = document.activeElement as HTMLInputElement | HTMLTextAreaElement | null;
  const focusId = sameView && content.contains(focused) ? focused?.id : undefined;
  const selection = focusId && (focused instanceof HTMLTextAreaElement || focused instanceof HTMLInputElement) ? [focused.selectionStart, focused.selectionEnd] : null;
  const expanded = sameView ? Array.from(content.querySelectorAll<HTMLDetailsElement>('details[id][open]')).map(d=>d.id) : [];
  const current=threads.find(t=>t.id===activeThread), label=threadModelLabel(current);
  if (activeTab === 'Chat') {
    if (!v?.messages.length) content.innerHTML = `<div class="welcome"><span class="big-mark">✳</span><h2>次の一歩を、ここから。</h2><p>${activeProject ? '下の入力欄でモデルと操作を選び、作業を依頼できます。<br>変更と実行ログは、隣のタブで確認できます。' : 'プロジェクト、エージェント、必要なコンテキスト。<br>ひとつのワークスペースで見通せます。'}</p>${!activeProject ? '<button class="primary" id="welcome-add">プロジェクトを登録</button>' : ''}</div>`;
    else content.innerHTML = `<div class="conversation">${v.route?`<details class="route-note"><summary>${escape(v.route.split('\n')[0])}</summary><p>${escape(v.route)}</p></details>`:''}${v.messages.map((m,index) => `<article class="message ${m.role === 'user' ? 'user' : ''}"><div class="message-role">${m.role === 'user' ? 'あなた' : label}<button class="copy-message" data-copy-message="${index}" title="本文をコピー">コピー</button></div><div class="message-text ${m.role === 'user' ? '' : 'markdown'}">${m.role === 'user' ? escape(m.text) : markdown(m.text)}</div></article>`).join('')}${v.running ? `<div class="working">● ${escape(label)} が作業中</div>` : ''}</div>`;
  } else if (activeTab === 'Diff') content.innerHTML = v?.diff ? `<div class="diff-toolbar"><button id="refresh-diff">更新</button><button id="copy-diff">差分をコピー</button></div>${diffMarkup(v.diff)}` : '<div class="empty"><h2>変更を確認する</h2><p>変更がないか、まだ取得されていません。</p><button id="refresh-diff">差分を更新</button><p></p></div>';
  else if (activeTab === 'Terminal') content.innerHTML = v?.events.length ? `<div class="event-list">${v.events.filter(e => !['message_delta', 'message_completed'].includes(e.event.kind)).map(e => `<div class="event-row"><span>${escape(e.event.kind)}</span><pre>${escape(e.event.text)}</pre></div>`).join('')}</div>` : '<div class="empty"><h2>実行ログ</h2><p>ツールの実行と結果をここに表示します。</p></div>';
  else if (activeTab === 'Agents') content.innerHTML = `<div class="inspector"><span class="eyebrow">EXECUTION</span><h2>実行エージェント</h2>${activeThread ? `<div class="agent-card"><strong>${escape(label)}</strong><span>${v?.running ? '実行中' : v?.needsResume ? '状態確認待ち' : '待機中'}</span><p>${escape(threads.find(t => t.id === activeThread)?.title || '')}</p><small>${escape(v?.route || current?.provider || '')}</small><div class="agent-actions"><button id="summarize">進捗を要約</button><button id="first-review">一次レビュー</button><button id="final-review">Astra 最終レビュー</button><button id="diagnose">Astra 診断・設計相談</button></div>${v?.summary ? `<pre class="summary">${escape(v.summary)}</pre>` : ''}${v?.review ? `<pre class="summary">${escape(v.review)}</pre>` : ''}${v?.finalReview?`<h3>最終レビュー: ${escape(v.finalReview.verdict)}</h3><pre class="summary">${escape(v.finalReview.findings.join('\n'))}</pre>${v.finalReview.verdict==='rework'?'<button id="apply-rework">既存 worker に修正を戻す</button>':''}`:''}${v?.recovery?`<h3>診断: ${escape(v.recovery.action)}</h3><pre class="summary">${escape(v.recovery.reason+'\n'+v.recovery.instruction)}</pre>${v.recovery.action==='retry_worker'?'<button id="apply-recovery">診断の修正を既存 worker へ</button>':''}${v.recovery.replacement?'<button id="use-replan">再計画を Plan で確認</button>':''}`:''}</div>` : '<p>まだエージェントは起動していません。</p>'}</div>`;
  else if (activeTab === 'Context') {
    const c=v?.context;
    content.innerHTML = `<div class="inspector"><span class="eyebrow">CONTEXT INSPECTOR</span><h2>渡した情報が、見える。</h2>${c?`<p>${escape(c.capsule.goal)}</p><div class="context-row"><span>初期 Capsule（推定 tokens）</span><strong>${c.initial_tokens}</strong></div><div class="context-row"><span>追加取得（推定 tokens）</span><strong>${c.retrieved_tokens}</strong></div><div class="context-row"><span>現在 / 上限</span><strong>${c.current_estimate} / ${c.capsule.budget.max_total_tokens}</strong></div><div class="context-row"><span>親の会話の自動コピー</span><strong class="safe">${c.parent_conversation_inherited?'有効':'無効'}</strong></div><h3>参照元</h3>${c.capsule.items.map(i=>`<details><summary>${escape(i.source)} · ${escape(i.reference)}</summary><pre>${escape(i.text)}</pre></details>`).join('')||'<p>タスク目標・合格条件・制約のみ</p>'}<h3>追加取得の履歴</h3>${c.retrievals.map(r=>`<div class="context-row"><span>${escape(r.source)}: ${escape(r.query)}<small>${escape(r.result_ref)}</small></span><strong>${r.token_estimate}</strong></div>`).join('')||'<p>追加取得はありません。</p>'}<p class="muted">bytes / 3 の推定値です。Codex の共通指示・ツール定義・推論中の会話は含みません。プロバイダーの実測使用量は Usage に表示します。</p>`:'<p>Plan から起動した worker を選ぶと、Capsule と取得履歴を確認できます。この会話に記録済みの Capsule はありません。</p>'}</div>`;
    content.insertAdjacentHTML('beforeend',memoryPanel(v?.capture));if(activeThread)content.insertAdjacentHTML('beforeend','<div class="inspector"><button id="capture-memory">完了結果からメモリ候補を抽出</button></div>');

  }
  else if(activeTab === 'Plan') content.innerHTML = `<div class="inspector"><span class="eyebrow">EXECUTION PLAN</span><h2>依存関係を確認して実行</h2><p>各ステップを独立した worktree で実行します。依存する成果の差分を受け取り、親ブランチへの統合は行いません。</p><button id="astra-plan">計画するモデルを選ぶ</button><div id="plan-preview-area">${planText===defaultPlanText?'<p>指示欄に実装したい内容を入力して計画を作成してください。手動で作成する場合は、下の詳細を開いて編集できます。</p>':planPreview(planText)}</div><details id="plan-source"><summary>詳細を編集（JSON）</summary><label for="plan-json">計画 JSON</label><textarea id="plan-json" spellcheck="false" autocorrect="off" autocapitalize="off" aria-label="計画 JSON" rows="16">${escape(planText)}</textarea></details><button id="run-plan" class="primary" ${!activeProject||planText===defaultPlanText?'disabled':''}>計画を実行（最大2並列）</button><button id="resume-plan" ${!activeProject?'disabled':''}>保存済みの計画を再開</button><h3>タスクの状態</h3>${graph.map(t=>`<div class="agent-card"><strong>${escape(t.title)}</strong><span>${escape(statusLabel(t.status))}</span><p>依存: ${t.dependencies.map(id=>escape(graph.find(d=>d.id===id)?.title||id)).join(', ')||'なし'}</p><small>${escape(t.id)}</small></div>`).join('')||'<p>実行前です。</p>'}</div>`;
  else if(activeTab === 'Usage') content.innerHTML = `<div class="inspector usage"><span class="eyebrow">MODEL USAGE</span><h2>保存済みの推論記録</h2><p>未取得の数値は — で表示します。失敗した推論の全使用量を含む集計ではありません。</p><table><thead><tr><th>モデル</th><th>入力</th><th>出力</th><th>キャッシュ</th><th>時間</th></tr></thead><tbody>${usage.map(u=>`<tr><td>${escape(u.model)}</td><td>${u.prompt_tokens??'—'}</td><td>${u.completion_tokens??'—'}</td><td>${u.cached_tokens??'—'}</td><td>${u.latency_ms===null?'—':(u.latency_ms/1000).toFixed(1)+' s'}</td></tr>`).join('')}</tbody></table></div>`;
  else content.innerHTML = `<div class="empty"><h2>${escape(activeTab)}</h2><p>${activeTab === 'Plan' ? '計画機能は Milestone F で接続します。' : 'プロバイダーの使用量はまだ集計していません。'}</p><span class="muted">未取得の数値はゼロとして扱いません。</span></div>`;
  content.querySelectorAll<HTMLButtonElement>('button').forEach(b=>b.disabled ||= busy);
  expanded.forEach(id=>{const d=document.getElementById(id);if(d instanceof HTMLDetailsElement)d.open=true;});
  if(focusId){const next=document.getElementById(focusId);if(next instanceof HTMLInputElement || next instanceof HTMLTextAreaElement){next.focus({preventScroll:true});if(selection&&selection[0]!==null&&selection[1]!==null)next.setSelectionRange(selection[0],selection[1]);}}
  content.scrollTop = activeTab==='Chat' && follow ? content.scrollHeight : oldScroll;
  content.querySelectorAll<HTMLAnchorElement>('a').forEach(a=>a.addEventListener('click',e=>{e.preventDefault();if(a.dataset.externalUrl)void invoke('open_web_link',{url:a.dataset.externalUrl}).catch(e=>error(String(e)));}));
  content.querySelectorAll<HTMLButtonElement>('[data-copy-message]').forEach(b=>b.onclick=()=>void copyText(v?.messages[Number(b.dataset.copyMessage)]?.text || '',b));
  document.querySelector('#refresh-diff')?.addEventListener('click',()=>{if(activeThread)void refreshDiff(activeThread);});
  document.querySelector<HTMLButtonElement>('#copy-diff')?.addEventListener('click',e=>void copyText(v?.diff || '',e.currentTarget as HTMLButtonElement));
  bindMemory();
  document.querySelector('#capture-memory')?.addEventListener('click',async()=>{if(!activeThread||busy)return;const id=activeThread;busy=true;render();try{await invoke('extract_memory',{threadId:id});await refreshContext(id);}catch(e){error(String(e));}finally{busy=false;}});
  document.querySelector('#astra-plan')?.addEventListener('click',()=>{newTask();(document.querySelector('#task-mode') as HTMLSelectElement).value='plan';updateModelOptions('');saveDraft();render();focusComposer();});
  document.querySelector('#final-review')?.addEventListener('click',()=>void astraInsight('final_review'));
  document.querySelector('#diagnose')?.addEventListener('click',()=>void astraInsight('diagnose_task'));
  document.querySelector('#apply-recovery')?.addEventListener('click',async()=>{if(!activeThread||busy)return;busy=true;render();try{await invoke('apply_recovery',{threadId:activeThread});}catch(e){error(String(e));}finally{busy=false;render();}});
  document.querySelector('#apply-rework')?.addEventListener('click',()=>void applyRework());
  document.querySelector('#use-replan')?.addEventListener('click',()=>{const p=selectedView()?.recovery?.replacement;if(p){planText=JSON.stringify(p,null,2);if(activeProject)ui.plans![activeProject]=planText;saveUi();activeTab='Plan';rememberSelection();render();}});
  document.querySelector('#plan-json')?.addEventListener('input',e=>{planText=(e.target as HTMLTextAreaElement).value;if(activeProject){ui.plans![activeProject]=planText;saveUi();}const preview=document.getElementById('plan-preview-area');if(preview)preview.innerHTML=planPreview(planText);const run=document.querySelector<HTMLButtonElement>('#run-plan');if(run)run.disabled=busy||!activeProject||planText===defaultPlanText;});
  document.querySelector('#resume-plan')?.addEventListener('click',()=>void resumePlan());
  document.querySelector('#run-plan')?.addEventListener('click',()=>void runPlan());
  document.querySelector('#welcome-add')?.addEventListener('click', openDialog);
  document.querySelector('#summarize')?.addEventListener('click',()=>void localInsight('summarize_task'));
  document.querySelector('#first-review')?.addEventListener('click',()=>void localInsight('review_task'));
}
function applyEvent(record: JournalEvent, replay = false) {
  const e = record.event;
  if (!e.thread_id) {
    if(e.kind==='memory_unavailable')error(e.text);
    if(['task_state','plan_finished','worker_assigned'].includes(e.kind)) {void refreshGraph();void refreshThreads();if(e.kind==='plan_finished')notice(e.text);}
    if (['disconnected', 'protocol_error', 'persistence_error'].includes(e.kind)) { for (const v of views.values()) v.needsResume = true; error(e.text); render(); }
    return;
  }
  const thread = threads.find(t => t.provider_thread_id === e.thread_id); if (!thread) return;
  const v = view(thread.id); if (record.sequence <= v.sequence) return; v.sequence = record.sequence;
  v.events.push(record);
  if (e.kind === 'diff') v.diff = e.text;
  if(e.kind === 'routing') v.route = e.text;
  if(e.kind === 'user_message') v.messages.push({key:`user-${e.turn_id}`,role:'user',text:e.text});
  if (!replay && e.kind === 'message_delta') {
    const key = e.item_id || e.turn_id || 'stream'; let message = v.messages.find(m => m.key === key);
    if (!message) { message = { key, role: 'assistant', text: '' }; v.messages.push(message); }
    message.text += e.text;
  }
  if ((!replay || thread.provider === 'spark') && e.kind === 'message_completed') {
    const key = e.item_id || e.turn_id || 'stream'; const message = v.messages.find(m => m.key === key);
    if (message) message.text = e.text; else v.messages.push({ key, role: 'assistant', text: e.text });
  }
  if (e.kind === 'turn_started' && (!e.turn_id || !v.completedTurns?.has(e.turn_id))) { v.running = true; v.activeTurn=e.turn_id||undefined; thread.status = 'running'; }
  if (e.kind === 'turn_completed') { v.completedTurns??=new Set(); if(e.turn_id)v.completedTurns.add(e.turn_id); if(!v.activeTurn||v.activeTurn===e.turn_id){v.running=false;thread.status=e.text;} }
  if (['error', 'approval_required'].includes(e.kind)) error(e.text);
  if (thread.id === activeThread) render();
}
function knownFiles():string[] { return (document.querySelector('#known-files') as HTMLInputElement).value.split(',').map(s=>s.trim()).filter(Boolean); }
async function astraPlan(){if(!activeProject||busy)return;const model=selectedModel();if(!model||model.local){error('計画するモデルを選択してください。');return;}const goal=(document.querySelector('#task-input') as HTMLTextAreaElement).value.trim();if(!goal){error('下の指示欄に、計画したい内容を入力してください。');return;}busy=true;error('');render();try{const p=await invoke('create_astra_plan',{projectId:activeProject,goal,model:model.model,reasoning:selectedReasoning()});planText=JSON.stringify(p,null,2);if(activeProject)ui.plans![activeProject]=planText;saveUi();activeTab='Plan';rememberSelection();}catch(e){error(String(e));}finally{busy=false;render();}}
async function astraInsight(command:'final_review'|'diagnose_task'){if(!activeThread||busy)return;const id=activeThread;busy=true;error('');render();try{if(command==='final_review')view(id).finalReview=await invoke(command,{threadId:id});else view(id).recovery=await invoke(command,{threadId:id,question:(document.querySelector('#task-input') as HTMLTextAreaElement).value.trim()||null});}catch(e){error(String(e));}finally{busy=false;render();}}
async function applyRework(){if(!activeThread||busy)return;busy=true;render();try{await invoke('apply_rework',{threadId:activeThread});}catch(e){error(String(e));}finally{busy=false;render();}}
async function refreshThreads() {try {threads=await invoke<Thread[]>('threads');render();}catch(e){error(String(e));}}
async function refreshGraph() {if(!activeProject)return;try {const id=activeProject;const tasks=await invoke<GraphTask[]>('task_graph',{projectId:id});if(activeProject!==id)return;graph=tasks;if(activeTab==='Plan')renderContent();}catch(e){error(String(e));}}
async function refreshContext(id:string) {try {view(id).context=await invoke<ContextInspection|null>('inspect_context',{threadId:id});view(id).capture=await invoke<Capture>('memory_candidates',{threadId:id});if(activeThread===id&&activeTab==='Context')renderContent();}catch(e){error(String(e));}}
async function runPlan() {if(!activeProject||busy)return;busy=true;error('');render();try {graph=await invoke<GraphTask[]>('run_plan',{projectId:activeProject,plan:JSON.parse(planText),concurrency:2});await refreshThreads();}catch(e){error(String(e));}finally{busy=false;render();}}
async function refreshUsage() { try {usage=await invoke<Usage[]>('model_usage');if(activeTab==='Usage')renderContent();} catch(e){error(String(e));} }
async function localInsight(command:'summarize_task'|'review_task') {
  if(!activeThread || busy)return;busy=true;render();
  try {const result=await invoke<Record<string,unknown>>(command,{threadId:activeThread});const v=view(activeThread); if(command==='summarize_task')v.summary=JSON.stringify(result,null,2);else v.review=JSON.stringify(result,null,2);}
  catch(e){error(String(e));}finally{busy=false;render();}
}
async function refreshDiff(id: string) {
  try { view(id).diff = await invoke<string>('repo_diff', {threadId:id}); if(activeThread === id && activeTab === 'Diff') renderContent(); }
  catch(e) { error(String(e)); }
}
async function selectThread(id: string) {
  if (busy) return; notice(''); saveDraft(); const t=threads.find(t=>t.id===id); if(t)activeProject=t.project_id; activeThread = id; restoreDraft(); rememberSelection(); busy = true; error(''); render();
  try {
    const snapshot = threads.find(t=>t.id===id)?.provider === 'spark' ? {active_turn:null,messages:[]} : await invoke<Snapshot>('resume_task', { threadId: id });
    const v = view(id); if(threads.find(t=>t.id===id)?.provider === 'spark') {v.sequence=0;v.events=[];} v.messages = snapshot.messages.map((m, i) => ({ ...m, key: `restored-${i}` }));
    let after = v.sequence;
    for (;;) { const history = await invoke<JournalEvent[]>('event_history', { threadId: id, after }); for (const e of history) applyEvent(e, true); if (history.length < 2000) break; after = history[history.length - 1].sequence; }
    const insights=await invoke<{review:View['finalReview'];recovery:View['recovery']}>('worker_insights',{threadId:id});v.finalReview=insights.review||undefined;v.recovery=insights.recovery||undefined;
    v.running = !!snapshot.active_turn && !v.completedTurns?.has(snapshot.active_turn.id); v.activeTurn=snapshot.active_turn?.id; v.needsResume = false;
  } catch (e) { view(id).needsResume = true; error(String(e)); } finally { busy = false; render(); }
  if(activeTab==='Diff')void refreshDiff(id);if(activeTab==='Context')void refreshContext(id);
}
async function send(approvedRoute?:AutoPreview) {
  const input = document.querySelector<HTMLTextAreaElement>('#task-input')!, text = input.value.trim();
  if (!text || busy || !activeProject || selectedView()?.needsResume) return;
  if(!activeThread&&planningMode()){await astraPlan();return;}
  busy = true; error(''); render();
  try {
    if (!activeThread) {
      const choice=selectedModel(), key=(document.querySelector('#preference') as HTMLSelectElement).value;
      if(key!=='auto'&&!choice)throw new Error('選択したモデルは利用できません。モデル一覧を更新してください。');
      if(approvedRoute&&key!=='auto')throw new Error('モデル選択が変更されました。もう一度実行してください。');
      if(key==='auto'&&!approvedRoute){
        const preview=await invoke<AutoPreview>('preview_auto_route',{projectId:activeProject,text,knownFiles:knownFiles()});
        if(preview.confirm_before_run||preview.blocked||!preview.target){showAutoPreview(preview);return;}
        approvedRoute=preview;
      }
      const result = await invoke<{thread:Thread;target:ModelTarget;route:{decision:{reason:string;executor:string}}}>('create_routed_task', { request:{projectId: activeProject, text, knownFiles:knownFiles(), preference:key==='auto'?'auto':choice!.local?'spark':'codex',model:choice?.model??null,reasoning:key==='auto'?null:selectedReasoning(),autoRouteId:approvedRoute?.id??null,autoRouteRevision:approvedRoute?.revision??null} });
      const thread=result.thread; delete ui.drafts![draftKey()]; threads.unshift(thread); activeThread = thread.id; saveDraft(); rememberSelection(); view(thread.id).needsResume = false; view(thread.id).route=`${result.route.decision.executor}: ${result.route.decision.reason}`;
      threadModels[thread.id]=result.target.model;if(result.target.reasoning)threadReasoning[thread.id]=result.target.reasoning;
      void invoke<Record<string,string>>('thread_models').then(models=>{threadModels=models;render();}).catch(e=>notice(String(e)));
    }
    const v = view(activeThread);
    if(threads.find(t=>t.id===activeThread)?.provider === 'spark') { await invoke('run_local',{threadId:activeThread,text,knownFiles:knownFiles()}); }
    else { v.messages.push({ role: 'user', text, key: `user-${Date.now()}` });
    if (v.running) await invoke('steer_turn', { threadId: activeThread, text });
    else { await invoke('send_turn', { threadId: activeThread, text }); }
    }
    input.value = ''; saveDraft();
  } catch (e) { if (activeThread) view(activeThread).needsResume = true; error(String(e)); }
  finally { busy = false; render(); }
}
function openDialog() { void pickProject(); }
async function pickProject() {
  if(picking || busy)return;
  if(!isTauri()){error('フォルダ選択はデスクトップアプリで利用できます。');return;}
  picking=true; error('');
  try {
    const path=await nativeInvoke<string|null>('choose_path',{kind:'project'});
    if(path)await registerProject(path);
  } catch(e){error(String(e));} finally {picking=false;}
}
async function registerProject(path:string) {
  try {
    const p=await invoke<Project>('register_project',{path:path.trim()});
    projects=await invoke<Project[]>('projects'); switchProject(p.id,true); activeTab='Chat'; rememberSelection(); render();
    (document.querySelector('#register') as HTMLDialogElement).close(); notice(`${p.name} を開きました。`); focusComposer();
  } catch(e){
    (document.querySelector('#path') as HTMLInputElement).value=path;
    document.querySelector('#form-error')!.textContent=String(e).includes('Git repository')?'Git リポジトリのフォルダを選んでください。既存のファイルは変更していません。':String(e);
    const dialog=document.querySelector<HTMLDialogElement>('#register')!;if(!dialog.open)dialog.showModal();
  }
}
document.querySelector('#settings')!.addEventListener('click', async () => {
  document.querySelector('#settings-error')!.textContent='';
  (document.querySelector('#connection-settings') as HTMLDialogElement).showModal();
  try { localConfig=await invoke<LocalConfig>('local_model_settings');for(const [id,value] of Object.entries({'local-name':localConfig.display_name,'local-id':localConfig.model_id,'local-endpoint':localConfig.endpoint,'local-bits':String(localConfig.quantization_bits)}))(document.getElementById(id) as HTMLInputElement).value=value; (document.querySelector('#codex-path') as HTMLInputElement).value = await invoke<string>('codex_binary'); const config=await invoke<{mode:string;model:string;reasoning:string|null}>('astra_settings');(document.querySelector('#astra-mode') as HTMLSelectElement).value=config.mode;(document.querySelector('#astra-model') as HTMLInputElement).value=config.model;fillReasoningSelect(document.querySelector<HTMLSelectElement>('#astra-reasoning')!,modelChoices.find(m=>!m.local&&m.model===config.model),config.reasoning); }
  catch (e) { document.querySelector('#settings-error')!.textContent = String(e); }
});
document.querySelector('#settings-cancel')!.addEventListener('click', () => (document.querySelector('#connection-settings') as HTMLDialogElement).close());
document.querySelector('#settings-form')!.addEventListener('submit', async e => {
  e.preventDefault(); try {
    const value=(id:string)=>(document.getElementById(id) as HTMLInputElement).value.trim();
    const next:LocalConfig={display_name:value('local-name'),model_id:value('local-id'),endpoint:value('local-endpoint'),quantization_bits:Number(value('local-bits'))};
    const localChanged=JSON.stringify(next)!==JSON.stringify(localConfig&&{display_name:localConfig.display_name,model_id:localConfig.model_id,endpoint:localConfig.endpoint,quantization_bits:localConfig.quantization_bits});
    if(localChanged){await invoke('set_local_model_settings',{config:next});localConfig=next;notice('ローカルモデル設定を保存しました。アプリを再起動すると反映されます。');}
    const path=(document.querySelector('#codex-path') as HTMLInputElement).value;if(path!==await invoke<string>('codex_binary')) {await invoke('set_codex_binary',{path});for(const v of views.values())v.needsResume=true;}
    await invoke('set_astra_settings',{config:{mode:(document.querySelector('#astra-mode') as HTMLSelectElement).value,model:(document.querySelector('#astra-model') as HTMLInputElement).value,reasoning:(document.querySelector('#astra-reasoning') as HTMLSelectElement).value||null}});
    document.querySelector('#settings-error')!.textContent = '';
    (document.querySelector('#connection-settings') as HTMLDialogElement).close(); error('');void refreshModels();
  } catch(e) { document.querySelector('#settings-error')!.textContent = String(e); }
});
document.querySelector('#add')!.addEventListener('click', openDialog);
document.querySelector('#new')!.addEventListener('click', newTask);
document.querySelector('#resume')!.addEventListener('click', () => { if (activeThread) void selectThread(activeThread); });
document.querySelector('#send')!.addEventListener('click', () => void send());
document.querySelector<HTMLTextAreaElement>('#task-input')!.addEventListener('keydown', e => { if (!e.isComposing && e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); void send(); } });
document.querySelector('#stop')!.addEventListener('click', async () => { if (!activeThread || busy) return; busy = true; render(); try { await invoke('interrupt_turn', { threadId: activeThread }); } catch (e) { view(activeThread).needsResume = true; error(String(e)); } finally { busy = false; render(); } });
document.querySelector('#cancel')!.addEventListener('click', () => (document.querySelector('#register') as HTMLDialogElement).close());
document.querySelector('#register form')!.addEventListener('submit', async e => {
  e.preventDefault(); if(busy)return; busy=true;render();try{await registerProject((document.querySelector('#path') as HTMLInputElement).value);}finally{busy=false;render();focusComposer();}
});
document.querySelector('#browse-project')!.addEventListener('click',()=>{(document.querySelector('#register') as HTMLDialogElement).close();void pickProject();});
document.querySelector('#browse-codex')!.addEventListener('click',async()=>{if(picking)return;picking=true;try{const path=await nativeInvoke<string|null>('choose_path',{kind:'codex_binary'});if(path)(document.querySelector('#codex-path') as HTMLInputElement).value=path;}catch(e){document.querySelector('#settings-error')!.textContent=String(e);}finally{picking=false;}});
setupMemory(()=>activeProject,error);
setupAutoRouting({call:invoke,models:()=>modelChoices,refresh:refreshModels,busy:()=>busy,run:preview=>void send(preview),plan:()=>{(document.querySelector('#task-mode') as HTMLSelectElement).value='plan';composerReasoning=null;updateModelOptions('');saveDraft();render();focusComposer();},notice});
document.querySelector('#auto-settings-button')!.addEventListener('click',()=>void openAutoSettings());
document.querySelector('#astra-model')!.addEventListener('input',()=>fillReasoningSelect(document.querySelector<HTMLSelectElement>('#astra-reasoning')!,modelChoices.find(m=>!m.local&&m.model===(document.querySelector('#astra-model') as HTMLInputElement).value)));
document.querySelector('#reasoning')!.addEventListener('change',()=>{composerReasoning=selectedReasoning();saveDraft();render();});
window.addEventListener('unhandledrejection', e => error(String(e.reason)));
window.addEventListener('error', e => error(e.message));
render();
if (isTauri()) {
  void refreshModels();
  await listen<JournalEvent>('hub-event', e => applyEvent(e.payload));
  await listen<number>('hub-stream-gap', () => { if (activeThread) { view(activeThread).needsResume = true; error('イベント配信が遅れました。「再開・状態を確認」で保存済み履歴を読み直してください。'); render(); } });
  try { [projects, threads] = await Promise.all([invoke<Project[]>('projects'), invoke<Thread[]>('threads')]); activeProject = projects.find(p=>p.id===ui.project)?.id || projects[0]?.id || null; activeTab=tabs.includes(ui.tab||'')?ui.tab!:'Chat';planText=activeProject?ui.plans![activeProject]||planText:planText;restoreDraft();render();if(ui.thread && threads.some(t=>t.id===ui.thread&&t.project_id===activeProject))await selectThread(ui.thread);if(activeTab==='Plan')void refreshGraph();if(activeTab==='Diff'&&activeThread)void refreshDiff(activeThread);if(activeTab==='Context'&&activeThread)void refreshContext(activeThread);if(activeTab==='Usage')void refreshUsage(); } catch (e) { error(String(e)); }
}

async function resumePlan() {if(!activeProject||busy)return;busy=true;error('');render();try{graph=await invoke<GraphTask[]>('resume_plan',{projectId:activeProject});await refreshThreads();}catch(e){error(String(e));}finally{busy=false;render();}}

function saveUi() {
  try { localStorage.setItem('astra-ui-v1',JSON.stringify(ui)); }
  catch { notice('下書きを端末に保存できません。閉じる前に本文をコピーしてください。'); }
}
function rememberSelection() { ui.project=activeProject;ui.thread=activeThread;ui.tab=activeTab;saveUi(); }
function draftKey() { return activeThread || `new:${activeProject || 'none'}`; }
function saveDraft() {
  const input=document.querySelector<HTMLTextAreaElement>('#task-input')!;
  ui.drafts![draftKey()]={text:input.value,files:(document.querySelector('#known-files') as HTMLInputElement).value,preference:(document.querySelector('#preference') as HTMLSelectElement).value,mode:(document.querySelector('#task-mode') as HTMLSelectElement).value,reasoning:selectedReasoning()};
  saveUi(); document.querySelector('#draft-status')!.textContent=input.value?'下書きはこの端末に保存':'';
}
function restoreDraft() {
  const draft=ui.drafts![draftKey()];
  (document.querySelector('#task-input') as HTMLTextAreaElement).value=typeof draft?.text==='string'?draft.text:'';
  (document.querySelector('#known-files') as HTMLInputElement).value=typeof draft?.files==='string'?draft.files:'';
  composerReasoning=draft?.reasoning||null;
  (document.querySelector('#task-mode') as HTMLSelectElement).value=draft?.mode==='plan'?'plan':'implement';updateModelOptions(draft?.preference||'auto');
  document.querySelector('#draft-status')!.textContent=draft?.text?'保存した下書き':'';
}
function focusComposer() { document.querySelector<HTMLTextAreaElement>('#task-input')!.focus(); }
function switchProject(id:string, force=false) {
  if(busy&&!force)return; notice(''); saveDraft();activeProject=id;activeThread=null;activeTab='Chat';graph=[];
  planText=ui.plans![id] || defaultPlanText;restoreDraft();rememberSelection();render();
  if(activeTab==='Plan')void refreshGraph();focusComposer();
}
function newTask() {
  if(busy)return;notice('');saveDraft();activeThread=null;activeTab='Chat';restoreDraft();rememberSelection();error('');render();focusComposer();
  if(!activeProject)void pickProject();
}
function notice(message:string) { const el=document.querySelector<HTMLDivElement>('#notice')!;el.hidden=!message;el.textContent=message; }
async function copyText(text:string, button?:HTMLButtonElement) {
  try { await navigator.clipboard.writeText(text); if(button){const old=button.textContent;button.textContent='コピー済み';setTimeout(()=>{if(button.isConnected)button.textContent=old;},1400);} else notice('コピーしました。'); }
  catch { error('コピーできませんでした。テキストを選択して ⌘C を押してください。'); }
}
function renderThreads() {
  const matches=threads.filter(t=>t.project_id===activeProject&&`${t.title} ${statusLabel(t.status)}`.toLowerCase().includes(taskFilter.toLowerCase()));
  matches.sort((a,b)=>Number(ui.pins!.includes(b.id))-Number(ui.pins!.includes(a.id)));
  document.querySelector('#threads')!.innerHTML=matches.map(t=>`<div class="thread-row"><button class="thread ${t.id===activeThread?'selected':''}" data-thread="${t.id}" title="${escape(t.title)}"><span>${escape(t.title)}</span><small class="state-${escape(t.status)}">${escape(statusLabel(t.status))}</small></button><button class="pin-thread ${ui.pins!.includes(t.id)?'pinned':''}" data-pin="${t.id}" aria-label="${escape(t.title)}を${ui.pins!.includes(t.id)?'固定解除':'固定'}" aria-pressed="${ui.pins!.includes(t.id)}">${ui.pins!.includes(t.id)?'★':'☆'}</button></div>`).join('')||`<p class="side-empty muted">${taskFilter?'一致するタスクがありません。':'新しいタスクから始めましょう。'}</p>`;
  document.querySelectorAll<HTMLButtonElement>('[data-thread]').forEach(b=>b.onclick=()=>void selectThread(b.dataset.thread!));
  document.querySelectorAll<HTMLButtonElement>('[data-pin]').forEach(b=>b.onclick=()=>{const id=b.dataset.pin!;ui.pins=ui.pins!.includes(id)?ui.pins!.filter(p=>p!==id):[...ui.pins!,id];saveUi();renderThreads();});
}
type MenuAction={title:string;detail:string;run:()=>void};
function menuActions():MenuAction[] {
  return [
    {title:'フォルダを開く',detail:'⌘O',run:()=>void pickProject()},
    {title:'新しいタスク',detail:'⌘N',run:newTask},
    {title:'接続設定',detail:'⌘,',run:()=>document.querySelector<HTMLButtonElement>('#settings')!.click()},
    {title:'Auto の振り分け設定',detail:'5段階のモデルと reasoning',run:()=>void openAutoSettings()},
    ...tabs.map(tab=>({title:`${tab} を表示`,detail:'表示を切り替え',run:()=>{activeTab=tab;rememberSelection();render();if(tab==='Diff'&&activeThread)void refreshDiff(activeThread);if(tab==='Plan')void refreshGraph();if(tab==='Context'&&activeThread)void refreshContext(activeThread);if(tab==='Usage')void refreshUsage();}})),
    ...projects.map(p=>({title:p.name,detail:p.root,run:()=>switchProject(p.id)})),
    ...threads.map(t=>({title:t.title,detail:`${projects.find(p=>p.id===t.project_id)?.name||''} · ${statusLabel(t.status)}`,run:()=>{activeTab='Chat';void selectThread(t.id);}})),
  ];
}
function renderCommands() {
  const query=(document.querySelector('#command-query') as HTMLInputElement).value.toLowerCase();
  const matches=menuActions().filter(a=>`${a.title} ${a.detail}`.toLowerCase().includes(query));
  document.querySelector('#command-results')!.innerHTML=matches.map((a,i)=>`<button class="command-result" data-command="${i}"><strong>${escape(a.title)}</strong><small>${escape(a.detail)}</small></button>`).join('')||'<p>一致するタスク・操作がありません。</p>';
  document.querySelectorAll<HTMLButtonElement>('[data-command]').forEach(b=>b.onclick=()=>{(document.querySelector('#commands') as HTMLDialogElement).close();matches[Number(b.dataset.command)].run();});
}
function openCommands() { if(picking||document.querySelector('dialog[open]'))return;const d=document.querySelector<HTMLDialogElement>('#commands')!; (document.querySelector('#command-query') as HTMLInputElement).value='';renderCommands();d.showModal();document.querySelector<HTMLInputElement>('#command-query')!.focus(); }
function toggleSidebar() { ui.sidebarHidden=!ui.sidebarHidden;saveUi();render(); }
for(const id of ['task-input','known-files','preference'])document.getElementById(id)!.addEventListener('input',saveDraft);
document.querySelector<HTMLInputElement>('#task-filter')!.addEventListener('input',e=>{taskFilter=(e.target as HTMLInputElement).value;renderThreads();});
document.querySelector('#command-menu')!.addEventListener('click',openCommands);
document.querySelector('#sidebar-toggle')!.addEventListener('click',toggleSidebar);
document.querySelector('#project-path')!.addEventListener('click',()=>{const p=projects.find(p=>p.id===activeProject);if(p)void copyText(p.root);});
document.querySelector('#command-query')!.addEventListener('input',renderCommands);
document.querySelector('#commands')!.addEventListener('keydown',e=>{
  const event=e as KeyboardEvent;if(event.isComposing)return;
  const buttons=Array.from(document.querySelectorAll<HTMLButtonElement>('[data-command]'));const index=buttons.indexOf(document.activeElement as HTMLButtonElement);
  if(event.key==='ArrowDown'||event.key==='ArrowUp'){event.preventDefault();buttons[(index+(event.key==='ArrowDown'?1:-1)+buttons.length)%buttons.length]?.focus();}
  if(event.key==='Enter'&&document.activeElement?.id==='command-query'){event.preventDefault();buttons[0]?.click();}
});
document.querySelector('#rename-task')!.addEventListener('click',()=>{if(!activeThread||busy)return;(document.querySelector('#task-name') as HTMLInputElement).value=threads.find(t=>t.id===activeThread)?.title||'';document.querySelector('#rename-error')!.textContent='';(document.querySelector('#rename-dialog') as HTMLDialogElement).showModal();document.querySelector<HTMLInputElement>('#task-name')!.select();});
document.querySelector('#rename-cancel')!.addEventListener('click',()=> (document.querySelector('#rename-dialog') as HTMLDialogElement).close());
document.querySelector('#rename-form')!.addEventListener('submit',async e=>{e.preventDefault();if(!activeThread||busy)return;const id=activeThread;busy=true;render();try{await invoke('rename_thread',{threadId:id,title:(document.querySelector('#task-name') as HTMLInputElement).value});await refreshThreads();(document.querySelector('#rename-dialog') as HTMLDialogElement).close();}catch(e){document.querySelector('#rename-error')!.textContent=String(e);}finally{busy=false;render();}});
document.addEventListener('keydown',e=>{
  if(e.isComposing||!(e.metaKey||e.ctrlKey)||e.altKey||picking||document.querySelector('dialog[open]'))return;
  const action=({o:()=>void pickProject(),n:newTask,k:openCommands,',':()=>document.querySelector<HTMLButtonElement>('#settings')!.click(),b:toggleSidebar} as Record<string,()=>void>)[e.key.toLowerCase()];
  if(action&&!e.shiftKey){e.preventDefault();action();}
});

document.querySelector('#task-mode')!.addEventListener('change',()=>{pendingPreference=undefined;composerReasoning=null;updateModelOptions('');saveDraft();render();});
document.querySelector('#preference')!.addEventListener('change',()=>{composerReasoning=null;refreshComposerReasoning();pendingPreference=(document.querySelector('#preference') as HTMLSelectElement).value;saveDraft();render();});
document.querySelector('#refresh-models')!.addEventListener('click',()=>void refreshModels());
