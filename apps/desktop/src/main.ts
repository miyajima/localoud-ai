import {bindTaskGraphs} from './task-graph.ts';
import './task-graph.css';
import {planningRequest,reviewRequest,recordMarkup} from './task-focus';
import {tabLabels,flowMarkup,welcomeMarkup,executionOverview,executionPlan,executionContext,executionUsage} from './workflow-ui';
import type {ExecutionRecord} from './workflow-ui';
import type {TaskFocus} from './task-focus';
import {syncWorkers,applyWorkerEvent,mergeActivity,progressMarkup,changesMarkup} from './worker-progress';
import type {Progress,Activity,WorkerStep} from './worker-progress';
import './worker-progress.css';
import {setupSettingsNavigation} from './settings-navigation';
import {setupMcpSettings} from './mcp-settings';
import {setupManifestReview} from './manifest-review';
import {setupBrowserWorkspace} from './browser-workspace';
import {setupChatGptUsage} from './chatgpt-usage';
import type {CompletedTurn} from './chatgpt-usage';
import { markdown, diffMarkup, statusLabel, planPreview } from './presentation';
import {setupMemory,memoryPanel,bindMemory} from './memory';
import type {Capture} from './memory';
import { invoke as nativeInvoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './style.css';
import './auto-routing.css';
import './composer.css';
import './workflow-ui.css';
import './native-ui.css';
import {setupComposer} from './composer';
import type {Mention} from './composer-model';
let composer:ReturnType<typeof setupComposer>|undefined;
let preparingComposer=false;
import {appendModelOptions,fillReasoningSelect,profileForChoice,readTarget,resolvedModel} from './model-controls';
import type {ModelChoice,ModelTarget} from './model-controls';
import {setupAutoRouting,showAutoPreview,openAutoSettings} from './auto-routing';
import type {AutoPreview,AutoSettings} from './auto-routing';
async function invoke<T = unknown>(command: string, args?: Record<string, unknown>): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([nativeInvoke<T>(command, args), new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error(`操作の応答がありません (${command})。再開して状態を確認してください。`)), ['preview_auto_route','create_astra_plan','final_review','diagnose_task','composer_catalog','send_composed_turn','complete_prompt'].includes(command) ? 270000 : command === 'run_local' ? 240000 : 40000);
    })]);
  } finally { clearTimeout(timer); }
}
type Project = { id: string; name: string; root: string };
type Thread = { id: string; project_id: string; provider_thread_id: string; provider: string; title: string; status: string };
type AgentEvent = { thread_id: string | null; turn_id: string | null; item_id: string | null; kind: string; text: string; details?:({type:string;command?:string;exit_code?:number|null;[key:string]:unknown}|null) };
type JournalEvent = { sequence: number; event: AgentEvent };
type ChatMessage = { role: string; text: string; key: string; label?:string };
type Snapshot = { active_turn: { id: string } | null; messages: { role: string; text: string }[] };
type View = { messages: ChatMessage[]; events: JournalEvent[]; diff: string; running: boolean; needsResume: boolean; sequence: number; route?: string; summary?: string; review?: string; finalReview?: {verdict:string;findings:string[];rework_instruction:string|null}; recovery?:{action:string;reason:string;instruction:string;replacement:unknown|null}; activeTurn?:string; completedTurns?:Set<string>; capture?:Capture; context?: ContextInspection|null; progress?:Progress; focus?:TaskFocus;execution?:ExecutionRecord };
let projects: Project[] = [], threads: Thread[] = [], activeProject: string | null = null, activeThread: string | null = null;
let loadingWorkspace=false;
const views = new Map<string, View>();
const tabs = ['Chat', 'Plan', 'Diff', 'Agents', 'Terminal', 'Context', 'Usage'];
let taskFilter = '';
function archivedThread(){return !!activeThread&&archivedThreads.includes(activeThread);}
function autonomousMode(){return composerMode()==='autonomous';}
function autonomousThread(){return threads.find(t=>t.id===activeThread)?.provider==='autonomous';}
function workflowThread(){return threads.find(t=>t.id===activeThread)?.provider==='workflow';}
function choosePhase(phase:string){
  if(phase==='autonomous'){chooseAutonomous();return;}
  if(phase==='implement'){setComposerMode('implement');return;}
  browserWorkspace.showBrowser();notice('開いたChatGPTで計画・レビューを依頼してください。');
}
function legacyThread(){const t=threads.find(t=>t.id===activeThread);return !!t&&(t.status==='legacy_read_only'||t.provider==='chatgpt'||t.provider==='workflow');}
function chooseAutonomous(){
  if(busy||activeThread)return;
  (document.querySelector('#task-mode') as HTMLSelectElement).value='autonomous';ui.defaultMode='autonomous';
  browserWorkspace.showWorkspace();
  saveDraft();render();focusComposer();
}
const autonomousRefreshes=new Map<string,Promise<void>>();
async function refreshAutonomous(id:string){
  const pending=autonomousRefreshes.get(id);if(pending)return pending;
  const work=loadAutonomous(id).finally(()=>autonomousRefreshes.delete(id));autonomousRefreshes.set(id,work);return work;
}
async function loadAutonomous(id:string){
  const snapshot=await invoke<{thread:Thread;messages:ChatMessage[];steps:WorkerStep[];children?:{id:string;provider_thread?:{id:string}|null;turn?:{id:string}|null}[];artifact_version:string|null;source_head:string;iteration:number;final_diff:string|null;artifact:{path:string}|null;manifest?:{manifest_id:string;request:string;acceptance?:string[]};review?:{manifest_id:string;verdict:string;summary:string;findings:string[]}}>('autonomous_snapshot',{threadId:id});
  const v=view(id);v.messages=(snapshot.messages||[]).filter(m=>!m.role.endsWith('_evidence')).map((m,i)=>({...m,key:m.key||`autonomous-${i}`}));
  v.running=['queued','running','integrating','dispatching','stopping'].includes(snapshot.thread.status);v.needsResume=['interrupted','failed','reconciliation_required','awaiting_review','needs_attention'].includes(snapshot.thread.status);
  v.route=snapshot.steps?.map(s=>`${s.step.title} · ${statusLabel(s.status)} · ${s.agent_name?`${s.agent_name} · `:''}難易度${s.step.level} · ${s.target.model}${s.target.reasoning?' / '+s.target.reasoning:''}`).join('\n')||'Manifestの実行記録';if(snapshot.final_diff!==null)v.diff=snapshot.final_diff;if(snapshot.artifact&&!v.running)v.messages.push({role:'assistant',text:`成果物の作業フォルダ: ${snapshot.artifact.path}`,key:'autonomous-artifact'});
  if(snapshot.artifact_version)v.messages.push({role:'assistant',key:'review-target',text:`レビュー対象ID: ${id}\n版: ${snapshot.artifact_version}\n基準revision: ${snapshot.source_head}\n試行: ${snapshot.iteration}\n状態: ${statusLabel(snapshot.thread.status)}`});
  const evidence=snapshot.steps.flatMap(s=>(s.verification||[]).map(e=>`${s.step.title}: ${e.status||'unknown'} · ${e.command||'ファイル照合'} · 終了コード ${e.exit_code??'不明／対象外'}\n対象版: ${e.target_revision||'不明'}\nログ: ${e.log_ref||'記録内'}\n${(e.output||'').slice(0,500)}`));
  if(evidence.length)v.messages.push({role:'assistant',key:'verification-summary',text:'保存済み検証結果（コマンド成功だけでは受け入れ条件の合格を意味しません）\n\n'+evidence.join('\n\n').split('\n').map(line=>'    '+line).join('\n')});
  const t=threads.find(t=>t.id===id);if(t)t.status=snapshot.thread.status;
  v.focus={id,projectId:snapshot.thread.project_id,title:snapshot.thread.title,goal:snapshot.manifest?.request||v.messages.find(m=>m.role==='user')?.text||snapshot.thread.title,status:snapshot.thread.status,artifactVersion:snapshot.artifact_version,manifestId:snapshot.manifest?.manifest_id,reviewId:snapshot.review?.manifest_id,verdict:snapshot.review?.verdict,acceptance:snapshot.manifest?.acceptance};
  v.execution={focus:v.focus,steps:snapshot.steps,artifactPath:snapshot.artifact?.path,review:snapshot.review?{summary:snapshot.review.summary||'',findings:snapshot.review.findings||[],verdict:snapshot.review.verdict}:undefined};
  v.progress=syncWorkers(v.progress,snapshot);
  try{const includeChanges=activeThread===id&&activeTab==='Diff';const activity=await invoke<Activity>('autonomous_activity',{threadId:id,includeChanges});mergeActivity(v.progress,activity,includeChanges);}catch(e){v.progress.error=String(e);}
  if(activeThread===id)render();
}
async function sendAutonomous(text:string,projectId:string){
  if(legacyThread()){error('旧ブラウザ方式の記録は閲覧専用です。新しいタスクを作成してください。');return;}
  if(busy||!activeProject||selectedView()?.running||archivedThread())throw Error('現在は取り込めません。作業の状態を確認してください。');
  if(projectId!==activeProject)throw Error('確認した計画と選択中のプロジェクトが一致しません。');
  busy=true;importingManifest=true;error('');notice('確認した内容を取り込み中…');render();
  try{
    const browserSession=draftKey();
    const result=await invoke<{thread:Thread;imported:boolean}>('manifest_import_text',{projectId,text});
    const thread=result.thread;
    (document.querySelector('#task-mode') as HTMLSelectElement).value='autonomous';
    delete ui.drafts![browserSession];delete ui.planningRequests![activeProject!];browserWorkspace.moveSession(browserSession,thread.id);activeThread=thread.id;activeTab='Chat';
    threads=await invoke<Thread[]>('threads');threadModels[thread.id]='自動実行';
    document.querySelector<HTMLTextAreaElement>('#task-input')!.value='';composer?.clear();saveDraft();rememberSelection();browserWorkspace.showWorkspace();await refreshAutonomous(thread.id);
    if(result.imported)notice('Manifestを取り込みました。'+(threads.find(t=>t.id===thread.id)?.status==='completed'?'レビュー合格・タスク完了です。':'現在の状態: '+statusLabel(threads.find(t=>t.id===thread.id)?.status||thread.status)+'。'));
    else notice('このManifestは取り込み済みのため再実行せず、保存済みタスクを開きました。変更された内容は適用していません。中断した作業は「再開・状態を確認」で再開できます。');
  }catch(e){error(String(e));await refreshThreads();throw e;}
  finally{busy=false;importingManifest=false;render();}
}
async function refreshWorkflow(id:string,_resume=false){
  const snapshot=await invoke<{messages:ChatMessage[];status:string}>('workflow_snapshot',{threadId:id});
  const v=view(id);v.messages=snapshot.messages;v.running=false;v.needsResume=false;
  const t=threads.find(t=>t.id===id);if(t)t.status='legacy_read_only';
  if(activeThread===id)render();
}

let archivedThreads:string[]=[],showArchived=false;
let picking = false;
type Draft = {text:string;files:string;preference:string;mode?:string;reasoning?:string|null;mentions?:Mention[]};
let composerReasoning:string|null=null;
let threadReasoning:Record<string,string>={};
function selectedReasoning(){return (document.querySelector('#reasoning') as HTMLSelectElement).value||null;}
function refreshComposerReasoning(){
  const select=document.querySelector<HTMLSelectElement>('#reasoning')!;
  if(!activeThread&&(document.querySelector('#preference') as HTMLSelectElement).value==='auto'){select.replaceChildren(new Option('Auto 設定を使用',''));select.disabled=true;return;}
  const model=activeThread&&!autonomousMode()?modelForThread(activeThread):selectedModel();
  fillReasoningSelect(select,model,activeThread&&!autonomousMode()?threadReasoning[activeThread]||null:composerReasoning);
  select.disabled ||= busy||!!selectedView()?.running||threads.find(t=>t.id===activeThread)?.provider==='spark';
}
type LocalConfig = {endpoint:string;model_id:string;display_name:string;quantization_bits:number};
let modelChoices:ModelChoice[]=[], modelWarnings:string[]=[], threadModels:Record<string,string>={}, threadTargets:Record<string,ModelTarget>={}, localConfig:LocalConfig|null=null;
let loadingModels=false;let modelRefresh:Promise<void>|null=null;let pendingPreference:string|undefined;
function modelForThread(id:string|null){const target=id?threadTargets[id]:null;return target?modelChoices.find(m=>profileForChoice(m)===(target.profile_id||(target.provider==='codex'?'codex':'spark'))&&m.model===target.model):undefined;}
function threadModelLabel(t:Thread|undefined){if(t?.provider==='autonomous')return 'Manifest worker';const model=t?modelForThread(t.id):undefined;return t ? model?.label || threadModels[t.id] || (t.provider==='spark'?'ローカルモデル（過去のタスク）':'モデル未取得') : '';}
function chatgptSelected(){return autonomousMode()||legacyThread();}
function selectedModel(){return resolvedModel(modelChoices.find(m=>m.key===(document.querySelector('#preference') as HTMLSelectElement).value),(document.querySelector('#reasoning') as HTMLSelectElement).value||composerReasoning);}
function planningMode(){return (document.querySelector('#task-mode') as HTMLSelectElement).value==='plan';}
function composerMode(){return (document.querySelector('#task-mode') as HTMLSelectElement).value;}
function cloudMode(){return ['plan','codex-plan','goal'].includes(composerMode());}
function setComposerMode(mode:string){
  const value=mode==='workflow'?'plan':mode==='plan'?'codex-plan':mode==='goal'?'goal':'implement';
  if(activeThread&&(autonomousThread()||legacyThread()||value==='plan'||threads.find(t=>t.id===activeThread)?.provider==='spark'||selectedView()?.running)){error('このタスクでは今モードを変更できません。新規タスクを作成するか、実行完了を待ってください。');return false;}
  (document.querySelector('#task-mode') as HTMLSelectElement).value=value;ui.defaultMode=value;updateModelOptions();saveDraft();render();focusComposer();return true;
}
function updateModelOptions(preferred?:string){
  const select=document.querySelector<HTMLSelectElement>('#preference')!, previous=preferred??pendingPreference??select.value;pendingPreference=previous;
  const choices=modelChoices.filter(m=>!cloudMode()||!m.local);
  select.replaceChildren();
  if(!cloudMode())select.add(new Option('おまかせ','auto'));
  appendModelOptions(select,choices);
  if(Array.from(select.options).some(o=>o.value===previous))select.value=previous;
  else if(previous && previous!=='auto'){const missing=new Option('選択したモデルは利用できません',previous);missing.disabled=true;select.add(missing);select.value=previous;}
  else select.value=autonomousMode()?(choices[0]?.key||''):cloudMode()?(choices[0]?.key||''):'auto';
  refreshComposerReasoning();
  renderModelHint();
}
function renderModelHint(){
  const m=selectedModel(), select=document.querySelector<HTMLSelectElement>('#preference')!;
  const hint=document.querySelector<HTMLElement>('#model-hint')!;
  hint.hidden=!!activeThread&&!loadingModels;
  hint.textContent=autonomousMode()?'ChatGPT Web · 計画の取り込み後に、専用worktreeで実装・別セッションレビューを行います。':loadingModels?'モデル一覧を確認中…':activeThread?'実行先: '+threadModelLabel(threads.find(t=>t.id===activeThread)):composerMode()==='codex-plan'?'選択したクラウドモデルで読み取り専用の計画相談を行います。実装は「計画して実装」から進めます。':composerMode()==='goal'?'Codex · 読み取り専用でゴールを相談します。実装は「計画して実装」から進めます。':planningMode()?'実行計画を作ります。実装は計画の確認後に開始します。':m?.local?'この端末でレビュー付き実装 · 対象ファイルを1〜2件、入力欄の下で指定してください。':select.value==='auto'?'おまかせ · 対象ファイル指定時はレビュー付き実装、未指定時は読み取り専用で相談します。':m?'対象ファイル指定時はレビュー付き実装、未指定時は読み取り専用で相談 · '+m.label:'モデルを取得できません。接続設定で確認できます。';
  const localFiles=document.querySelector<HTMLElement>('#local-scope');
  if(localFiles)localFiles.hidden=!(m?.local&&!autonomousMode()||threads.find(t=>t.id===activeThread)?.provider==='spark'||document.querySelector<HTMLInputElement>('#known-files')?.value.trim());
}
function refreshModels():Promise<void>{
  if(modelRefresh)return modelRefresh;
  modelRefresh=loadModels().finally(()=>{modelRefresh=null;});
  return modelRefresh;
}
async function loadModels(){
  loadingModels=true;renderModelHint();
  try{const result=await invoke<{models:ModelChoice[];warnings:string[]}>('available_models');modelChoices=result.models;modelWarnings=result.warnings;[threadModels,threadReasoning,threadTargets]=await Promise.all([invoke<Record<string,string>>('thread_models'),invoke<Record<string,string>>('thread_reasoning'),invoke<Record<string,ModelTarget>>('thread_model_targets')]);updateModelOptions();}
  catch(e){error(String(e),{label:'再試行',run:()=>refreshModels()});}finally{loadingModels=false;render();}
}
type UiState = {project?:string|null;thread?:string|null;tab?:string;sidebarHidden?:boolean;defaultMode?:string;pins?:string[];drafts?:Record<string,Draft>;plans?:Record<string,string>;planningRequests?:Record<string,string>;reviewRequests?:Record<string,string>;expandedProjects?:string[];pinnedProjects?:string[]};
let ui: UiState = {};
try { ui = JSON.parse(localStorage.getItem('astra-ui-v1') || '{}') || {}; } catch { ui = {}; }
if(!ui || typeof ui!=='object' || Array.isArray(ui))ui={};
if(!ui.drafts || typeof ui.drafts!=='object' || Array.isArray(ui.drafts))ui.drafts={};
if(!ui.plans || typeof ui.plans!=='object' || Array.isArray(ui.plans))ui.plans={};
if(!Array.isArray(ui.pins))ui.pins=[];
ui.planningRequests ||= {};ui.reviewRequests ||= {};ui.expandedProjects ||= [];ui.pinnedProjects ||= [];
let lastTreeProject:string|null=null,lastTreeMarkup='';
let hubFramePending=false;
const hubEvents:JournalEvent[]=[];
function queueHubEvent(record:JournalEvent){
 hubEvents.push(record);if(hubFramePending)return;hubFramePending=true;
 window.requestAnimationFrame(()=>{hubFramePending=false;const events=hubEvents.splice(0);for(const event of events)applyEvent(event,false,true);render();});
}

type Usage = { provider:string; model:string; prompt_tokens:number|null; completion_tokens:number|null; cached_tokens:number|null; latency_ms:number|null; project_id?:string|null; project_name?:string|null };
const usageByThread=new Map<string,Usage[]>();
type PaneRead={loading:boolean;loaded:boolean;error?:string;retry:()=>Promise<void>};
const paneReads=new Map<string,PaneRead>(),pendingReads=new Map<string,Promise<void>>();
function readPane(id:string,tab:string,load:()=>Promise<void>):Promise<void>{
 const key=`${id}:${tab}`,pending=pendingReads.get(key);if(pending)return pending;
 const current=view(id),hasData=threads.find(t=>t.id===id)?.provider==='autonomous'?!!current.execution:tab==='Diff'?!!current.diff:tab==='Context'?current.context!==undefined:tab==='Usage'?usageByThread.has(id):false;
 const state=paneReads.get(key)||{loading:false,loaded:hasData,retry:()=>readPane(id,tab,load)};
 state.loading=true;state.error=undefined;state.retry=()=>readPane(id,tab,load);paneReads.set(key,state);
 if(activeThread===id&&activeTab===tab)renderContent();
 const work=load().then(()=>{state.loaded=true;}).catch(e=>{state.error=String(e);}).finally(()=>{
  state.loading=false;pendingReads.delete(key);if(activeThread===id&&activeTab===tab)renderContent();
 });pendingReads.set(key,work);return work;
}
type GraphTask = {id:string;title:string;description:string;status:string;dependencies:string[];worktree_id:string|null};
type ContextInspection = {initial_tokens:number;retrieved_tokens:number;current_estimate:number;parent_conversation_inherited:boolean;capsule:{goal:string;budget:{max_total_tokens:number};items:{source:string;reference:string;text:string}[]};retrievals:{source:string;query:string;token_estimate:number;result_ref:string}[]};
let graph:GraphTask[]=[];
let planText=JSON.stringify({title:'実装計画',steps:[{key:'implementation',title:'実装と検証',goal:'ここに具体的な実装内容を記入',dependencies:[],brief:{acceptance_criteria:['ここに合格条件を記入'],constraints:['公開 API を変更しない'],context_items:[],budget:{initial_tokens:6000,max_total_tokens:24000,max_single_retrieval_tokens:2000}}}]},null,2);
const defaultPlanText=planText;
let activeTab = 'Chat', busy = false, stopping=false, importingManifest=false;
let renderedContentKey='';
const paneStates=new Map<string,{scroll:number;disclosures:(readonly [string,boolean])[];innerScroll:(readonly [string|undefined,number,number])[]}>();
const app = document.querySelector<HTMLDivElement>('#app')!;
app.innerHTML = `<aside><div class="sidebar-heading"><div class="brand"><span class="mark" aria-hidden="true">✳</span><span class="brand-wordmark" aria-label="Localoud AI"><span class="brand-local">Lo</span>c<span class="brand-local">a</span>loud<span class="brand-ai"> AI</span></span></div></div><button id="new" class="new" title="新しいタスク ⌘N">＋ 新しいタスク <kbd>⌘N</kbd></button><button id="command-menu" class="command-trigger">タスク・操作を検索 <kbd>⌘K</kbd></button><div class="section-title">プロジェクト <button id="add" aria-label="フォルダを開く" title="フォルダを開く ⌘O">＋</button></div><div class="tree-filter"><button id="show-archived" title="アーカイブを表示" aria-label="アーカイブを表示">アーカイブ</button><input id="task-filter" type="search" spellcheck="false" autocorrect="off" autocapitalize="off" aria-label="タスクを絞り込む" placeholder="タスクを検索"></div><div id="projects" aria-label="プロジェクト別のタスク"></div><button id="settings" class="settings">設定</button><div class="sidebar-bottom"><span class="dot"></span> Local-first <small>Context stays intentional.</small></div></aside><main><a id="skip-to-content" class="skip-link" href="#content">メインコンテンツへ移動</a><header><div><span class="eyebrow">WORKSPACE</span><h1 id="project-title">プロジェクトを開く</h1><button id="project-path" class="project-path" title="フォルダのパスをコピー"></button></div><div class="header-actions"><button id="sidebar-toggle" title="サイドバーを切り替え ⌘B" aria-label="サイドバーを切り替え"><svg aria-hidden="true" viewBox="0 0 16 16"><path d="M2 4h12M2 8h12M2 12h12"/></svg></button><button id="session-actions" title="セッションの操作" aria-label="セッションの操作" hidden><svg aria-hidden="true" viewBox="0 0 16 16"><path d="M3 8h.01M8 8h.01M13 8h.01"/></svg></button><button id="rename-task" hidden>名前を変更</button><span id="status" class="badge"></span><button id="resume" hidden>再開・状態を確認</button></div></header><nav role="tablist" aria-label="ワークスペースの表示">${tabs.map(t => `<button role="tab" aria-controls="content" aria-selected="false" data-tab="${t}">${tabLabels[t]}</button>`).join('')}</nav><div id="error" role="alert" hidden></div><div id="notice" role="status" hidden></div><section id="content" role="tabpanel" tabindex="-1" aria-live="polite"></section><footer><div class="compose"><textarea id="task-input" aria-label="タスクの指示" placeholder="実装したいことを入力…"></textarea><div class="compose-controls"><div class="executor-controls"><div id="operation" role="group" aria-label="依頼先"><button type="button" data-operation="autonomous" aria-pressed="true">ChatGPTに相談</button><button type="button" data-operation="implement" aria-pressed="false">通常の依頼</button></div><span id="execution-source" class="execution-source"></span><select id="task-mode" hidden aria-hidden="true"><option value="autonomous">Manifestを実行</option><option value="implement" selected>実装する</option><option value="codex-plan">プランモード（Codex）</option><option value="goal">ゴールを設定して実行</option><option value="plan">実行計画を作る</option></select><label for="preference">モデル</label><select id="preference"><option value="auto">自動</option></select><label for="reasoning">Reasoning</label><select id="reasoning"><option value="">既定</option></select><button id="auto-settings-button" type="button">エージェント</button><button id="refresh-models" type="button" title="利用できるモデルを再取得" aria-label="モデル一覧を更新"><svg aria-hidden="true" viewBox="0 0 16 16"><path d="M13 6A5 5 0 1 0 14 9"/><path d="M13 3v3h-3"/></svg></button><input spellcheck="false" autocorrect="off" autocapitalize="off" id="known-files" aria-label="対象ファイル" placeholder="対象ファイル（例: src/main.rs）"></div><div><button id="stop" class="round-send" title="停止" aria-label="停止" hidden>■</button><button id="send" class="primary round-send" title="送信" aria-label="送信">↑</button></div></div></div><p id="model-hint" class="model-hint"></p><div class="footnote"><span id="draft-status"></span><span>⌘Enter で送信 · Enter で改行</span></div></footer></main><dialog id="register"><form><h2>プロジェクトを登録</h2><p>作業する Git リポジトリのフォルダを選択してください。</p><button id="browse-project" type="button" class="primary">フォルダを選択…</button><label for="path">フォルダの絶対パス</label><input spellcheck="false" autocorrect="off" autocapitalize="off" id="path" aria-describedby="form-error" required placeholder="/Users/you/projects/my-app"><p id="form-error" role="alert"></p><div class="dialog-actions"><button type="button" id="cancel">キャンセル</button><button class="primary" type="submit">登録する</button></div></form></dialog><dialog id="connection-settings"><form id="settings-form"><h2>接続設定</h2><button id="mcp-settings-button" type="button">MCP公開設定</button><p>デスクトップアプリから使う Codex 実行ファイルを指定します。</p><label for="codex-path">Codex 実行ファイルの絶対パス</label><button id="browse-codex" type="button">ファイルを選択…</button><input spellcheck="false" autocorrect="off" autocapitalize="off" id="codex-path" aria-describedby="settings-error" required placeholder="/opt/homebrew/bin/codex"><label for="astra-mode">レビュー・診断の利用経路</label><select id="astra-mode"><option value="disabled">無効（手動計画）</option><option value="codex_integrated">Codex 認証を利用</option></select><label for="astra-model">既定のレビュー・診断モデル ID</label><input id="astra-model" aria-describedby="settings-error" value="gpt-6-astra"><label for="astra-reasoning">レビュー・診断の Reasoning</label><select id="astra-reasoning"><option value="">既定</option></select><p>直接 API の課金設定とは別です。最終レビュー・診断で使用します。ChatGPTの計画・レビューのモデルは、内蔵ブラウザ側で選択します。</p><fieldset><legend>ローカルモデル</legend><p>対応サービスを起動し、その接続情報を指定します。保存時にモデルを照合し、再起動後に反映します。</p><label for="local-name">表示名</label><input id="local-name" aria-describedby="settings-error" required><label for="local-id">モデル ID（サービスの返す値）</label><input id="local-id" aria-describedby="settings-error" required spellcheck="false" autocorrect="off" autocapitalize="off"><label for="local-endpoint">接続先（ポートを含むURL）</label><input id="local-endpoint" aria-describedby="settings-error" required spellcheck="false" autocorrect="off" autocapitalize="off"><p>専用API（GET /health・POST /v1/generate）または OpenAI互換API（GET /v1/models・POST /v1/chat/completions）を提供するローカル実行環境に対応しています。OpenAI Responses APIだけのサービスは対象外です。</p><p>量子化ビット数は接続先から自動取得します。この画面でモデルの量子化は変更しません。</p></fieldset><p id="settings-error" role="alert"></p><div class="dialog-actions"><button type="button" id="settings-cancel">閉じる</button><button type="submit" class="primary">保存する</button></div></form></dialog>`;
const sessionActions=document.querySelector<HTMLButtonElement>('#session-actions')!;
sessionActions.title='選択中のセッションを整理';
sessionActions.setAttribute('aria-label','選択中のセッションを整理');
sessionActions.insertAdjacentHTML('beforeend','<span>セッション</span>');
document.body.insertAdjacentHTML('beforeend', `<dialog id="commands" aria-label="タスク・操作を検索"><label for="command-query">タスク・操作を検索</label><input id="command-query" type="search" spellcheck="false" autocorrect="off" autocapitalize="off" placeholder="タスク名、プロジェクト名、操作…" autocomplete="off"><div id="command-results"></div><small>↑↓ で選択 · Enter で開く · Esc で閉じる</small></dialog><dialog id="rename-dialog"><form id="rename-form"><h2>タスク名を変更</h2><label for="task-name">タスク名</label><input id="task-name" aria-describedby="rename-error" required maxlength="120"><p id="rename-error" role="alert"></p><div class="dialog-actions"><button type="button" id="rename-cancel">キャンセル</button><button type="submit" class="primary">保存</button></div></form></dialog>`);
document.querySelector<HTMLButtonElement>('#stop')!.innerHTML='<svg aria-hidden="true" viewBox="0 0 16 16"><rect x="5" y="5" width="6" height="6" rx="1"/></svg>';
document.querySelector<HTMLButtonElement>('#send')!.innerHTML='<svg aria-hidden="true" viewBox="0 0 16 16"><path d="M8 13V3M4.5 7.5 8 3l3.5 4.5"/></svg>';
document.querySelector<HTMLElement>('.mark')!.innerHTML='<svg aria-hidden="true" viewBox="0 0 16 16"><path d="M8 1.5 9.7 6.3 14.5 8l-4.8 1.7L8 14.5 6.3 9.7 1.5 8l4.8-1.7L8 1.5Z"/></svg>';
for(const [dialog,title] of [['register','register-title'],['connection-settings','settings-title'],['rename-dialog','rename-title']] as const){const d=document.getElementById(dialog),h=d?.querySelector('h2');if(d&&h){h.id=title;d.setAttribute('aria-labelledby',title);}}
for(const [formId,errorId] of [['#register form','form-error'],['#settings-form','settings-error'],['#rename-form','rename-error']]){
 const form=document.querySelector<HTMLFormElement>(formId)!;form.noValidate=true;
 const summary=document.getElementById(errorId)!;summary.tabIndex=-1;summary.classList.add('error-summary');
 for(const field of Array.from(form.querySelectorAll<HTMLInputElement>('input'))){
  field.insertAdjacentHTML('afterend',`<p id="${field.id}-error" class="field-error"></p>`);
  field.setAttribute('aria-describedby',`${field.id}-error`);
  field.addEventListener('input',()=>{field.removeAttribute('aria-invalid');document.getElementById(`${field.id}-error`)!.textContent='';});
 }
 form.addEventListener('submit',event=>{
  const invalid:Array<{field:HTMLInputElement;message:string}>=[];
  for(const field of Array.from(form.querySelectorAll<HTMLInputElement>('input:not(:disabled)'))){
   let message='';const value=field.value.trim();
   if(field.required&&!value)message='入力してください。';
   else if(value&&['path','codex-path'].includes(field.id)&&!value.startsWith('/'))message='/ から始まる絶対パスを入力してください。';
   else if(value&&field.id==='local-endpoint'){try{const address=new URL(value);if(!['http:','https:'].includes(address.protocol))throw Error();}catch{message='http:// または https:// から始まる接続先URLを入力してください。';}}
   field.toggleAttribute('aria-invalid',!!message);if(message)field.setAttribute('aria-invalid','true');
   document.getElementById(`${field.id}-error`)!.textContent=message;
   if(message)invalid.push({field,message});
  }
  summary.replaceChildren();
  if(!invalid.length)return;
  event.preventDefault();event.stopImmediatePropagation();
  summary.append(`${invalid.length}項目を確認してください。`);
  for(const {field,message} of invalid){const link=document.createElement('a');link.href=`#${field.id}`;const label=form.querySelector(`label[for="${field.id}"]`)?.textContent||'入力';link.textContent=`${label}: ${message}`;link.onclick=e=>{e.preventDefault();field.focus();};summary.append(link);}
  if(invalid.length>1)summary.focus();else invalid[0].field.focus();
 },true);
 form.closest('dialog')?.addEventListener('cancel',event=>{if(form.getAttribute('aria-busy')==='true'&&form.dataset.loading!=='true')event.preventDefault();});
}
function formBusy(form:HTMLFormElement,value:boolean){
 form.setAttribute('aria-busy',String(value));
 form.querySelectorAll<HTMLInputElement|HTMLButtonElement|HTMLSelectElement>('input,button,select').forEach(el=>{
  if(value){el.dataset.wasDisabled=String(el.disabled);el.disabled=true;}
  else{el.disabled=el.dataset.wasDisabled==='true';delete el.dataset.wasDisabled;}
 });
 const submit=form.querySelector<HTMLButtonElement>('[type="submit"]');if(submit){if(value){submit.dataset.label=submit.textContent||'';submit.textContent='保存中…';}else submit.textContent=submit.dataset.label||'保存する';}
}
const more=document.createElement('details');more.id='compose-more';more.innerHTML='<summary aria-label="入力オプション" title="入力オプション"><svg aria-hidden="true" viewBox="0 0 16 16"><path d="M8 3v10M3 8h10"/></svg></summary><div class="compose-more-panel"></div>';
document.querySelector('.executor-controls')!.prepend(more);
document.querySelector('.compose')!.prepend(document.querySelector('#operation')!);
document.querySelector('[data-operation="implement"]')!.textContent='相談・対象実装';
document.querySelector('[data-operation="autonomous"]')!.textContent='計画して実装';
more.querySelector('div')!.insertAdjacentHTML('beforeend','<label for="advanced-operation">追加の操作</label><select id="advanced-operation"><option value="">選択…</option><option value="implement">相談／対象ファイルを実装</option><option value="codex-plan">選択モデルで計画を相談</option><option value="goal">Codexでゴールを相談</option></select>');
document.querySelectorAll<HTMLButtonElement>('[data-operation]').forEach(button=>button.addEventListener('click',()=>{if(button.getAttribute('aria-pressed')!=='true')choosePhase(button.dataset.operation!);}));
document.querySelector('#advanced-operation')!.addEventListener('change',e=>{const mode=(e.target as HTMLSelectElement).value;if(!mode||busy||selectedView()?.running)return;(document.querySelector('#task-mode') as HTMLSelectElement).value=mode;pendingPreference=undefined;composerReasoning=null;updateModelOptions('');saveDraft();render();more.open=false;});
for(const id of ['auto-settings-button','refresh-models'])more.querySelector('div')!.append(document.getElementById(id)!);
const localScope=document.createElement('div');localScope.id='local-scope';localScope.hidden=true;
localScope.innerHTML='<label for="known-files">対象ファイル</label>';
localScope.append(document.getElementById('known-files')!);document.querySelector('.compose-controls')!.before(localScope);
const implementPlan=document.createElement('button');implementPlan.id='implement-plan';implementPlan.type='button';implementPlan.className='primary';implementPlan.textContent='この計画で実装を開始';implementPlan.hidden=true;
document.querySelector('#execution-source')!.after(implementPlan);
implementPlan.onclick=()=>void startImplementationFromPlan();
  const browserWorkspace=setupBrowserWorkspace(invoke);
const openManifestReview=setupManifestReview(invoke,sendAutonomous,()=>modelChoices);
const quota=document.querySelector<HTMLElement>('#chatgpt-usage')!;document.querySelector('#settings-form')!.append(quota);
const recordChatGptTurn=setupChatGptUsage(quota);
if(isTauri())void listen<CompletedTurn>('chatgpt-turn-completed',event=>recordChatGptTurn(event.payload));
document.querySelector('#settings-form')!.insertAdjacentHTML('beforeend','<section id="total-usage"></section>');
const localSettingsFieldset=document.querySelector<HTMLFieldSetElement>('#settings-form fieldset')!;
localSettingsFieldset.insertAdjacentHTML('afterend','<fieldset id="provider-settings"><legend>API providers</legend><p>APIキー値は保存しません。アプリ起動時に参照する環境変数名だけを登録します。profile保存時はモデル一覧の取得だけを行います。</p><div id="provider-profile-list"></div><button id="provider-profile-add" type="button">API providerを追加</button></fieldset>');
document.body.insertAdjacentHTML('beforeend','<dialog id="provider-profile-dialog"><form id="provider-profile-form"><h2>API provider</h2><input id="provider-revision" type="hidden"><label for="provider-id">Profile ID</label><input id="provider-id" required pattern="[A-Za-z0-9_-]{1,80}" spellcheck="false"><label for="provider-name">表示名</label><input id="provider-name" required maxlength="120"><label for="provider-protocol">API形式</label><select id="provider-protocol"><option value="open_ai_chat">OpenAI互換 Chat Completions</option><option value="open_ai_responses">OpenAI Responses</option><option value="anthropic_messages">Anthropic Messages</option></select><label for="provider-locality">接続区分</label><select id="provider-locality"><option value="cloud">Cloud HTTPS</option><option value="local">Local loopback HTTP</option></select><label for="provider-base-url">API root URL</label><input id="provider-base-url" required spellcheck="false" placeholder="https://api.anthropic.com/v1"><label for="provider-credential-env">APIキーの環境変数名</label><input id="provider-credential-env" spellcheck="false" placeholder="ANTHROPIC_API_KEY"><label for="provider-concurrency">同時実行上限</label><input id="provider-concurrency" type="number" min="1" max="8" value="1" required><label><input id="provider-enabled" type="checkbox" checked>有効</label><p id="provider-profile-error" role="alert"></p><div class="dialog-actions"><button id="provider-profile-cancel" type="button">キャンセル</button><button type="submit" class="primary">接続確認して保存</button></div></form></dialog>');
// The navigation toggle must remain outside the region it hides.
let mobileSidebarOpen=false;
const sidebarButton=document.querySelector<HTMLButtonElement>('#sidebar-toggle')!;
document.querySelector('main>header')!.prepend(sidebarButton);
sidebarButton.setAttribute('aria-controls','project-sidebar');
const sidebar=document.querySelector<HTMLElement>('#app>aside')!;sidebar.id='project-sidebar';sidebar.setAttribute('aria-label','プロジェクトとタスク');
sidebar.querySelector('.sidebar-heading')!.insertAdjacentHTML('beforeend','<button id="sidebar-close" type="button" aria-label="プロジェクト一覧を閉じる">閉じる</button>');
app.insertAdjacentHTML('beforeend','<div id="sidebar-backdrop" hidden></div>');
app.prepend(document.querySelector('#skip-to-content')!);
document.querySelector('#content')!.removeAttribute('aria-live');
document.querySelector('#content')!.setAttribute('tabindex','0');
document.querySelector('#status')!.setAttribute('role','status');
document.querySelector('#task-input')!.setAttribute('aria-describedby','model-hint');
const modelPicker=document.createElement('details');modelPicker.id='model-picker';modelPicker.innerHTML='<summary aria-label="モデルと思考量"><span id="model-picker-name"></span> <span id="model-picker-effort"></span><svg class="model-chevron" aria-hidden="true" viewBox="0 0 16 16"><path d="m4 6 4 4 4-4"/></svg></summary><div class="model-picker-panel"></div>';document.querySelector('#preference')!.before(modelPicker);
for(const id of ['preference','reasoning']){modelPicker.querySelector('div')!.append(document.querySelector(`label[for=${id}]`)!,document.getElementById(id)!);}
const modelHealth=document.createElement('section');modelHealth.id='model-health';modelHealth.hidden=true;document.querySelector('#model-hint')!.after(modelHealth);
modelPicker.querySelector('div')!.insertAdjacentHTML('beforeend','<p class="model-help">おまかせは依頼に応じて振り分けます。ローカルLLMは1〜2ファイルの小さな編集、Codexは調査・実装・検証に使えます。</p>');

app.insertAdjacentHTML('beforeend','<dialog id="lifecycle-dialog" aria-label="プロジェクト・セッションの操作"><h2 id="lifecycle-title"></h2><p id="lifecycle-description"></p><div id="lifecycle-actions" class="dialog-actions"></div><p id="lifecycle-error" role="alert"></p><button id="lifecycle-close">閉じる</button></dialog>');
const escape = (s: string) => s.replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]!));
function view(id: string): View { if (!views.has(id)) views.set(id, { messages: [], events: [], diff: '', running: false, needsResume: true, sequence: 0 }); return views.get(id)!; }
function selectedView() { return activeThread ? view(activeThread) : null; }
type ErrorRecovery = {label: string; run: () => void | Promise<void>};
let errorRecovery: ErrorRecovery | undefined;
function error(message: string, recovery?: ErrorRecovery) {
  if(message)notice('');
  errorRecovery=message?recovery:undefined;
  const el=document.querySelector<HTMLDivElement>('#error')!;
  el.hidden=!message;
  el.innerHTML=message?`<span>${escape(message)}</span><span class="error-actions">${recovery?`<button type="button" id="retry-error">${escape(recovery.label)}</button>`:''}<button type="button" aria-label="エラーを閉じる" id="dismiss-error"><svg aria-hidden="true" viewBox="0 0 16 16"><path d="m4 4 8 8M12 4l-8 8"/></svg></button></span>`:'';
  document.querySelector('#dismiss-error')?.addEventListener('click',()=>error(''));
  document.querySelector('#retry-error')?.addEventListener('click',()=>{const run=errorRecovery?.run;error('');if(run)void run();});
}
function focusContent() {
  const content=document.querySelector<HTMLElement>('#content');
  const active=document.querySelector<HTMLButtonElement>(`[data-tab="${activeTab}"]`);
  if(content&&active){active.id=`tab-${activeTab}`;content.setAttribute('aria-labelledby',active.id);content.focus({preventScroll:true});}
}
function updateSendAvailability(){
 const send=document.querySelector<HTMLButtonElement>('#send')!,input=document.querySelector<HTMLTextAreaElement>('#task-input')!;
 send.disabled=busy||!activeProject||!!selectedView()?.needsResume||!isTauri()||legacyThread()||!input.value.trim();
 if(!autonomousMode()&&!activeThread&&document.querySelector<HTMLSelectElement>('#preference')!.value!=='auto')send.disabled ||= !readTarget(document.querySelector<HTMLSelectElement>('#preference')!,document.querySelector<HTMLSelectElement>('#reasoning')!,modelChoices);
}
function controlKey(el:HTMLElement):string {
  if(el.id)return `id:${el.id}`;
  for(const name of ['data-copy-message','data-flow-action','data-thread','data-project','data-project-toggle','data-project-menu','data-project-new','data-pin'])if(el.hasAttribute(name))return `${name}:${el.getAttribute(name)}`;
  if(el.tagName==='SUMMARY'&&el.parentElement?.id)return `summary:${el.parentElement.id}`;
  return `${el.tagName}:${el.getAttribute('aria-label')||el.textContent}`;
}
function render() {
  composer?.syncContext();
  browserWorkspace.setSidebarHidden(!!ui.sidebarHidden);
  renderThreads();
  document.querySelectorAll<HTMLButtonElement>('[data-project-menu]').forEach(b=>b.onclick=()=>openProjectMenu(b.dataset.projectMenu!,b));
  const project = projects.find(p=>p.id===activeProject);
  const path = document.querySelector<HTMLButtonElement>('#project-path')!; path.textContent=project?.root || ''; path.hidden=!project; path.title=`${project?.root || ''} — クリックでコピー`;
  document.querySelector<HTMLButtonElement>('#session-actions')!.hidden=!activeThread;
  document.querySelector('#project-title')!.textContent = projects.find(p => p.id === activeProject)?.name || 'プロジェクトを開く';
  document.querySelectorAll<HTMLButtonElement>('[data-tab]').forEach(b => {
    const selected=b.dataset.tab===activeTab;
    b.classList.toggle('active', selected); b.setAttribute('aria-selected',String(selected)); b.tabIndex=selected?0:-1; b.id=`tab-${b.dataset.tab}`;
    b.onclick = () => { activeTab = b.dataset.tab!; rememberSelection(); render(); b.focus(); if(activeTab === 'Diff' && activeThread) void refreshDiff(activeThread); if(activeTab === 'Usage') void refreshUsage(); if(activeTab === 'Context' && activeThread) void refreshContext(activeThread); if(activeTab === 'Plan') void refreshGraph(); };
    b.onkeydown = event => {
      if(!['ArrowLeft','ArrowRight','Home','End'].includes(event.key))return;
      event.preventDefault(); const tabButtons=Array.from(document.querySelectorAll<HTMLButtonElement>('[data-tab]')); const index=tabButtons.indexOf(b);
      const next=event.key==='Home'?0:event.key==='End'?tabButtons.length-1:(index+(event.key==='ArrowRight'?1:-1)+tabButtons.length)%tabButtons.length;
      tabButtons.forEach((tab,i)=>tab.tabIndex=i===next?0:-1);
      tabButtons[next]?.focus();
    };
  });
  document.querySelector('#content')?.setAttribute('aria-labelledby',`tab-${activeTab}`);
  document.querySelectorAll<HTMLButtonElement>('[data-thread]').forEach(b => b.onclick = () => void selectThread(b.dataset.thread!));
  const v = selectedView();
  const current = threads.find(t=>t.id===activeThread);
  browserWorkspace.setSession(draftKey(),current?.title||`${project?.name||'プロジェクト'}の新しいタスク`);
  document.querySelector('main')!.classList.toggle('composing-task',!current&&activeTab==='Chat');
  const providerLabel = threadModelLabel(current);
  const modelSelect=document.querySelector<HTMLSelectElement>('#preference')!;
  modelSelect.disabled = busy||!!v?.running||!!v?.needsResume||current?.provider==='spark'||autonomousMode();
  if(current&&!autonomousMode()){
    const selected=modelForThread(current.id);
    if(selected)modelSelect.value=selected.key;
    else {const key=`thread:${current.id}`;let option=Array.from(modelSelect.options).find(o=>o.value===key);if(!option){option=new Option(threadModelLabel(current),key);modelSelect.add(option);}option.text=threadModelLabel(current);modelSelect.value=key;}
  }
  const status = document.querySelector<HTMLElement>('#status')!;
  status.textContent = importingManifest ? 'Manifest取り込み中…' : busy ? '処理中…' : v?.needsResume ? '状態確認が必要' : v?.running ? `${providerLabel} · 実行中` : current ? autonomousThread()?statusLabel(current.status):providerLabel : '';
  status.hidden=!status.textContent;
  if(!busy&&!v?.running&&!v?.needsResume&&v?.execution?.artifactPath){status.hidden=false;status.textContent='ワークツリー';status.setAttribute('title',v.execution.artifactPath);}
  const resume = document.querySelector<HTMLButtonElement>('#resume')!; resume.hidden = !activeThread || (autonomousThread()&&!v?.needsResume); resume.disabled = busy;
  const send = document.querySelector<HTMLButtonElement>('#send')!;
  const stop = document.querySelector<HTMLButtonElement>('#stop')!; stop.disabled=stopping||!!v?.needsResume;
  const input = document.querySelector<HTMLTextAreaElement>('#task-input')!; input.hidden=autonomousMode()&&!!activeThread||legacyThread(); input.disabled = busy || !activeProject || !isTauri() || legacyThread(); input.placeholder = v?.running ? '実行中のタスクに追加の指示…' : 'やりたいこと、困っていることを入力…';
  (document.querySelector('#known-files') as HTMLInputElement).hidden=planningMode()&&!activeThread;
  (document.querySelector('#task-mode') as HTMLSelectElement).disabled=busy||!!v?.running||current?.provider==='spark';
  (document.querySelector('#task-mode option[value=plan]') as HTMLOptionElement).disabled=!!activeThread;
  (document.querySelector('#refresh-models') as HTMLButtonElement).disabled=busy||loadingModels;
  refreshComposerReasoning();
  (document.querySelector('#auto-settings-button') as HTMLButtonElement).disabled=busy;
  app.classList.toggle('manifest-mode',autonomousMode());
  const mode=composerMode();
  const operation=document.querySelector<HTMLElement>('#operation')!;
  operation.hidden=!!current;
  operation.querySelectorAll<HTMLButtonElement>('[data-operation]').forEach(button=>{const selected=button.dataset.operation===(autonomousMode()?'autonomous':'implement');button.setAttribute('aria-pressed',String(selected));button.classList.toggle('active',selected);button.disabled=busy||!!v?.running||!!current;});
  const advanced=document.querySelector<HTMLSelectElement>('#advanced-operation')!;advanced.value=mode;advanced.disabled=busy||!!v?.running||workflowThread()||current?.provider==='spark';
  document.querySelector('#execution-source')!.textContent=mode==='codex-plan'?'計画を相談中':mode==='goal'?'ゴールを実行':'';
  implementPlan.hidden=mode!=='codex-plan'||!current||busy||!!v?.running||!!v?.needsResume||!v?.messages.some(m=>m.role==='assistant');
  for(const id of ['preference','reasoning']){document.getElementById(id)!.hidden=autonomousMode();document.querySelector<HTMLLabelElement>(`label[for=${id}]`)!.hidden=autonomousMode();}
  renderModelHint();
  modelHealth.hidden=autonomousMode()||!modelWarnings.length;
  const healthMarkup=modelWarnings.length?`<div class="model-health-summary"><span>${modelWarnings.some(w=>w.startsWith('ローカルモデル:'))?'ローカルモデルは未接続':'モデルの接続を確認してください'}</span><button type="button" data-model-health="refresh">${loadingModels?'確認中…':'再確認'}</button><button type="button" data-model-health="settings">接続設定</button></div><details><summary>詳細</summary><pre>${escape(modelWarnings.join('\n'))}</pre></details>`:'';
  if(modelHealth.innerHTML!==healthMarkup){modelHealth.innerHTML=healthMarkup;modelHealth.querySelector('[data-model-health="refresh"]')?.addEventListener('click',()=>void refreshModels());modelHealth.querySelector('[data-model-health="settings"]')?.addEventListener('click',()=>document.querySelector<HTMLButtonElement>('#settings')!.click());}
  modelHealth.querySelectorAll<HTMLButtonElement>('button').forEach(b=>b.disabled=busy||loadingModels);
  modelPicker.hidden=autonomousMode();
  const pickerModel=activeThread?modelForThread(activeThread):selectedModel();
  document.querySelector('#model-picker-name')!.textContent=(modelSelect.selectedOptions[0]?.textContent||'モデルを選択').replace(/^(GPT-\d+(?:\.\d+)?)-([A-Za-z])/, '$1 $2');
  const effort=document.querySelector<HTMLSelectElement>('#reasoning')!.value||pickerModel?.default_reasoning||'';
  document.querySelector('#model-picker-effort')!.textContent=pickerModel?.local||modelSelect.value==='auto'?'':({none:'なし',minimal:'最小',low:'軽',medium:'中',high:'高',xhigh:'最高',max:'最大',ultra:'最大'} as Record<string,string>)[effort]||effort;
  const focus=autonomousThread()?v?.focus:undefined;
  const planning=activeProject?ui.planningRequests![activeProject]:undefined;
  browserWorkspace.setWorkflow(flowMarkup({project:project?.name||'',title:current?.title||planning||'新しいタスク',status:v?.needsResume?'reconciliation_required':current?.status||'draft',kind:legacyThread()?'legacy':autonomousMode()?'manifest':'direct',hasTask:!!current,planningRequested:!!planning,reviewRequested:!!focus?.artifactVersion&&ui.reviewRequests![focus.id]===focus.artifactVersion,id:current?.id,version:focus?.artifactVersion,iteration:v?.progress?.iteration,busy,importing:importingManifest,stopping,running:!!v?.running,writing:activeTab==='Chat'}),handleFlowAction);
  resume.hidden=true;
  stop.hidden=autonomousMode()||!v?.running;
  stop.innerHTML='<svg aria-hidden="true" viewBox="0 0 16 16"><rect x="5" y="5" width="6" height="6" rx="1"/></svg>';
  stop.title=stopping?'停止中…':'実行を停止';
  stop.setAttribute('aria-label',stop.title);
  stop.setAttribute('aria-busy',String(stopping));
  const planningDraft=autonomousMode()&&!activeThread;
  const sendLabel=planningDraft?'ChatGPTへの依頼をコピー':planningMode()&&!activeThread?'計画を作成':mode==='codex-plan'?'プランを相談':mode==='goal'?'ゴールを開始':'送信';
  send.innerHTML=`<svg aria-hidden="true" viewBox="0 0 16 16"><path d="M8 13V3M4.5 7.5 8 3l3.5 4.5"/></svg><span>${busy?'処理中…':escape(sendLabel)}</span>`;
  send.title=sendLabel;send.setAttribute('aria-label',sendLabel);send.setAttribute('aria-busy',String(busy));
  send.classList.toggle('manifest-send',!activeThread);
  send.hidden=autonomousMode()&&!!activeThread||!!v?.running;
  updateSendAvailability();
  if(planningDraft)input.placeholder='何を実現したいですか？ 条件や制約もここに書けます。';
  document.querySelector('.footnote>span:last-child')!.textContent=planningDraft?'⌘Enter でコピー · Enter で改行':'⌘Enter で送信 · Enter で改行';
  document.querySelector<HTMLElement>('main>footer')!.hidden=activeTab!=='Chat'||autonomousMode()&&!!activeThread||legacyThread()||archivedThread();
  more.hidden=autonomousMode();
  renderNavigation();
  renderContent();
}
function handleFlowAction(action:string){
 if(action==='project'){openDialog();return;}
 if(action==='chat'){browserWorkspace.showBrowser();return;}
 if(action==='direct'){setComposerMode('implement');browserWorkspace.showWorkspace();return;}
 if(action==='write'){activeTab='Chat';rememberSelection();browserWorkspace.showWorkspace();render();focusComposer();return;}
 if(action==='import'){void reviewManifest();return;}
 if(action==='resume'){document.querySelector<HTMLButtonElement>('#resume')!.click();return;}
 if(action==='stop'){document.querySelector<HTMLButtonElement>('#stop')!.click();return;}
 if(action==='new'){newTask();return;}
 if(action==='review')void copyReviewRequest();
}
async function startImplementationFromPlan(){
 if(busy||selectedView()?.running||selectedView()?.needsResume||!activeThread)return;
 if(!setComposerMode('implement'))return;
 const input=document.querySelector<HTMLTextAreaElement>('#task-input')!;
 input.value='直前の計画に従って、実装・検証まで進めてください。未解決の必須入力がある場合だけ確認し、それ以外は現在の作業状態を確認して続行してください。';
 updateSendAvailability();render();notice('計画モードを終了し、同じタスクで実装を開始します。');
 await send();
}
async function reviewManifest(){
 const project=projects.find(p=>p.id===activeProject);
 if(!project||busy||selectedView()?.running||archivedThread()||legacyThread())return;
 const returnToChat=browserWorkspace.isBrowserOpen();browserWorkspace.showWorkspace();
 await openManifestReview(project,()=>{if(returnToChat)browserWorkspace.showBrowser();});
}
async function copyPlanningRequest(){
 const project=projects.find(p=>p.id===activeProject);
 const goal=document.querySelector<HTMLTextAreaElement>('#task-input')!.value.trim();
 if(!project||!goal||busy||activeThread)return;
 if(composer?.getMentions().some(m=>m.kind!=='file')){error('スキル・プラグイン・担当の指定は「実行する」で利用できます。ChatGPTへの依頼には、目的と対象ファイルを指定してください。');return;}
 busy=true;error('');render();
 try{
  if(!await copyText(planningRequest(project,goal,knownFiles())))return;
  ui.planningRequests![project.id]=goal;saveDraft();browserWorkspace.showCopiedRequestGuide('plan');
  notice('依頼をコピーしました。ChatGPTに貼り付けて送信してください。');
 }finally{busy=false;render();}
}
async function copyReviewRequest(){
 const focus=selectedView()?.focus;if(!focus?.artifactVersion)return;
 if(await copyText(reviewRequest(focus))){ui.reviewRequests![focus.id]=focus.artifactVersion;saveUi();browserWorkspace.showCopiedRequestGuide('review');notice('レビュー依頼をコピーしました。ChatGPTの会話へ貼り付けてください。');render();}
}
function renderContent() {
  const content = document.querySelector('#content')!, v = selectedView();
  if(renderedContentKey)paneStates.set(renderedContentKey,{scroll:content.scrollTop,disclosures:Array.from(content.querySelectorAll<HTMLDetailsElement>('details[id]')).map(d=>[d.id,d.open] as const),innerScroll:Array.from(content.querySelectorAll<HTMLElement>('[data-scroll-key]')).map(e=>[e.dataset.scrollKey,e.scrollTop,e.scrollLeft] as const)});
  const key=`${activeProject}:${activeThread}:${activeTab}`, sameView=key===renderedContentKey;renderedContentKey=key;
  const priorPane=paneStates.get(key);
  const oldScroll=priorPane?.scroll||0,follow=sameView&&content.scrollHeight-content.clientHeight-oldScroll<64;

  const focused = document.activeElement as HTMLInputElement | HTMLTextAreaElement | null;
  const focusKey = sameView && focused && content.contains(focused) ? controlKey(focused) : undefined;
  const selection = focusKey && (focused instanceof HTMLTextAreaElement || focused instanceof HTMLInputElement) ? [focused.selectionStart, focused.selectionEnd] : null;
  const disclosures=priorPane?.disclosures||[],innerScroll=priorPane?.innerScroll||[];
  const current=threads.find(t=>t.id===activeThread), label=threadModelLabel(current);
  if(autonomousThread()&&v?.execution&&activeTab==='Chat')content.innerHTML=executionOverview(v.execution,v.progress,statusLabel);
  else if(autonomousThread()&&v?.execution&&activeTab==='Plan')content.innerHTML=executionPlan(v.execution,statusLabel);
  else if(autonomousThread()&&v?.execution&&activeTab==='Context')content.innerHTML=executionContext(v.execution);
  else if(autonomousThread()&&v?.execution&&activeTab==='Usage')content.innerHTML=executionUsage(v.execution);
  else if(autonomousThread()&&v&&activeTab==='Agents')content.innerHTML=progressMarkup(v.progress,statusLabel);
  else if(autonomousThread()&&v&&activeTab==='Terminal')content.innerHTML=progressMarkup(v.progress,statusLabel,true);
  else if(autonomousThread()&&v&&activeTab==='Diff')content.innerHTML=(v.progress?.error?`<p class="progress-error">差分の更新に失敗しました。前回の表示を維持しています。${escape(v.progress.error)}</p>`:'')+changesMarkup(v.progress?.changes||[],diffMarkup);
  else if (activeTab === 'Chat') {
    if (!v?.messages.length) content.innerHTML=welcomeMarkup(!!activeProject,autonomousMode(),activeProject?ui.planningRequests![activeProject]:undefined);
    else if(autonomousThread())content.innerHTML=`<div class="conversation task-records"><details id="task-goal"><summary>依頼内容・合格条件を確認</summary><div class="message-text">${escape(v.focus?.goal||'')}${v.focus?.acceptance?.length?`<ul>${v.focus.acceptance.map(a=>`<li>${escape(a)}</li>`).join('')}</ul>`:''}</div></details>${v.focus?.manifestId?`<p class="handoff-receipt">実行依頼 · ID <code>${escape(v.focus.manifestId)}</code></p>`:''}${v.focus?.reviewId?`<p class="handoff-receipt">レビュー結果 · ${escape(v.focus.verdict||'')} · ID <code>${escape(v.focus.reviewId)}</code></p>`:''}<details id="task-records"><summary>作業記録・検証の詳細（${v.messages.filter(m=>m.role!=='user').length}件）</summary>${v.messages.map((m,index)=>m.role==='user'?'':recordMarkup(m.text,m.key,index,markdown)).join('')}</details></div>`;
    else content.innerHTML = `<div class="conversation">${v.route?(autonomousThread()?`<div class="route-note"><pre>${escape(v.route)}</pre></div>`:`<details class="route-note"><summary>${escape(v.route.split('\n')[0])}</summary><p>${escape(v.route)}</p></details>`):''}${v.messages.map((m,index) => `<article class="message ${m.role === 'user' ? 'user' : ''}"><div class="message-role">${m.role === 'user' ? 'あなた' : escape(m.label||label)}<button class="copy-message" data-copy-message="${index}" title="本文をコピー">コピー</button></div><div class="message-text ${m.role === 'user' ? '' : 'markdown'}">${m.role === 'user' ? escape(m.text) : markdown(m.text)}</div></article>`).join('')}${v.running ? `<div class="working">● ${escape(label)} が作業中</div>` : ''}</div>`;
    if(autonomousThread()&&v)content.insertAdjacentHTML('beforeend',progressMarkup(v.progress,statusLabel));
  } else if (activeTab === 'Diff') content.innerHTML = v?.diff ? `<div class="diff-toolbar"><span class="muted">保存済みの変更</span></div>${diffMarkup(v.diff)}` : `<div class="empty"><h2>変更</h2><p>${!activeThread?'タスクを選択すると変更を確認できます。':paneReads.get(`${activeThread}:Diff`)?.loaded?'保存済みの変更はありません。':'変更を読み込みます。'}</p></div>`;
  else if (activeTab === 'Terminal') content.innerHTML = v?.events.length ? `<div class="event-list">${v.events.filter(e => !['message_delta', 'message_completed'].includes(e.event.kind)).map(e => `<div class="event-row"><span>${escape(e.event.kind)}</span><pre>${escape(e.event.text)}</pre></div>`).join('')}</div>` : '<div class="empty"><h2>実行ログ</h2><p>ツールの実行と結果をここに表示します。</p></div>';
  else if (activeTab === 'Agents') content.innerHTML = `<section class="read-panel"><h2>実行担当</h2>${current?`<article class="read-card"><div class="read-row"><strong>${escape(label)}</strong><span>${escape(statusLabel(current.status))}</span></div><p>${escape(current.title)}</p>${v?.summary?`<p>${escape(v.summary)}</p>`:''}</article>`:'<p>計画を取り込むと担当が表示されます。</p>'}</section>`;
  else if (activeTab === 'Context') {
    const c=v?.context;
    content.innerHTML = `<div class="inspector"><span class="eyebrow">CONTEXT INSPECTOR</span><h2>渡した情報が、見える。</h2>${c?`<p>${escape(c.capsule.goal)}</p><div class="context-row"><span>初期 Capsule（推定 tokens）</span><strong>${c.initial_tokens}</strong></div><div class="context-row"><span>追加取得（推定 tokens）</span><strong>${c.retrieved_tokens}</strong></div><div class="context-row"><span>現在 / 上限</span><strong>${c.current_estimate} / ${c.capsule.budget.max_total_tokens}</strong></div><div class="context-row"><span>親の会話の自動コピー</span><strong class="safe">${c.parent_conversation_inherited?'有効':'無効'}</strong></div><h3>参照元</h3>${c.capsule.items.map(i=>`<details><summary>${escape(i.source)} · ${escape(i.reference)}</summary><pre>${escape(i.text)}</pre></details>`).join('')||'<p>タスク目標・合格条件・制約のみ</p>'}<h3>追加取得の履歴</h3>${c.retrievals.map(r=>`<div class="context-row"><span>${escape(r.source)}: ${escape(r.query)}<small>${escape(r.result_ref)}</small></span><strong>${r.token_estimate}</strong></div>`).join('')||'<p>追加取得はありません。</p>'}<p class="muted">bytes / 3 の推定値です。Codex の共通指示・ツール定義・推論中の会話は含みません。プロバイダーの実測使用量は Usage に表示します。</p>`:'<p>Plan から起動した worker を選ぶと、Capsule と取得履歴を確認できます。この会話に記録済みの Capsule はありません。</p>'}</div>`;


  }
  else if(activeTab === 'Plan') content.innerHTML = `<section class="read-panel"><h2>計画</h2><p>${activeThread?'このタスクには取り込まれた実行計画がありません。直接実行の依頼と結果は「概要」で確認できます。':'チャットで依頼を相談し、確定した計画を取り込むと、担当・順序・完了条件がここに表示されます。'}</p></section>`;
  else if(activeTab === 'Usage') content.innerHTML = usageMarkup(activeThread?usageByThread.get(activeThread)||[]:[],activeThread?'このタスクの使用量':'タスクを選択してください');
  else content.innerHTML = `<div class="empty"><h2>${escape(activeTab)}</h2><p>${activeTab === 'Plan' ? '計画機能は Milestone F で接続します。' : 'プロバイダーの使用量はまだ集計していません。'}</p><span class="muted">未取得の数値はゼロとして扱いません。</span></div>`;
  const read=activeThread?paneReads.get(`${activeThread}:${activeTab}`):undefined;
  content.setAttribute('aria-busy',String(!!read?.loading));
  if(read?.loading||read?.error){
    const feedback=`<div class="pane-feedback" ${read.error?'role="alert"':'role="status"'}><p>${read.error?`${escape(tabLabels[activeTab])}を読み込めませんでした。${read.loaded?'前回の表示を残しています。':''}接続を確認して、再試行してください。`:`${escape(tabLabels[activeTab])}を読み込み中…`}</p>${read.error?`<details><summary>エラーの詳細</summary><pre>${escape(read.error)}</pre></details><button id="retry-pane" type="button">再試行</button>`:''}</div>`;
    if(!read.loaded)content.innerHTML=`<section class="read-panel"><h2>${escape(tabLabels[activeTab])}</h2>${feedback}</section>`;
    else content.insertAdjacentHTML('afterbegin',feedback);
  }
  document.querySelector('#retry-pane')?.addEventListener('click',()=>{void read?.retry();focusContent();});
  if(autonomousThread()&&v?.progress?.error){const alert=content.querySelector('.progress-error');if(alert){const retry=document.createElement('button');retry.id='retry-activity';retry.type='button';retry.textContent='進捗を再読み込み';const id=activeThread!,tab=activeTab;retry.onclick=()=>{void readPane(id,tab,()=>refreshAutonomous(id));focusContent();};alert.append(retry);}}
  content.querySelectorAll<HTMLButtonElement>('button').forEach(b=>b.disabled ||= busy);
  content.querySelectorAll<HTMLElement>('.flow-actions[aria-label]').forEach(el=>el.setAttribute('role','group'));
  content.querySelectorAll<HTMLButtonElement>('[data-flow-action]').forEach(b=>b.onclick=()=>handleFlowAction(b.dataset.flowAction!));
  disclosures.forEach(([id,open])=>{const d=document.getElementById(id);if(d instanceof HTMLDetailsElement)d.open=open;});
  bindTaskGraphs(content);
  for(const el of Array.from(content.querySelectorAll<HTMLElement>('[data-scroll-key]'))){const prior=innerScroll.find(([key])=>key===el.dataset.scrollKey);if(prior){el.scrollTop=prior[1];el.scrollLeft=prior[2];}}
  if(focusKey){const next=Array.from(content.querySelectorAll<HTMLElement>('button,a,summary,input,textarea,select')).find(el=>controlKey(el)===focusKey);if(next){next.focus({preventScroll:true});if(selection&&selection[0]!==null&&selection[1]!==null&&(next instanceof HTMLInputElement||next instanceof HTMLTextAreaElement))next.setSelectionRange(selection[0],selection[1]);}}
  content.scrollTop = activeTab==='Chat'&&!autonomousThread()&&!!v?.messages.length&&(follow||!priorPane) ? content.scrollHeight : oldScroll;
  content.querySelectorAll<HTMLAnchorElement>('a').forEach(a=>a.addEventListener('click',e=>{e.preventDefault();if(a.dataset.externalUrl)void invoke('open_web_link',{url:a.dataset.externalUrl}).catch(e=>error(String(e)));}));
  content.querySelectorAll<HTMLButtonElement>('[data-copy-message]').forEach(b=>b.onclick=()=>void copyText(v?.messages[Number(b.dataset.copyMessage)]?.text || '',b));
  document.querySelector('#show-worker-changes')?.addEventListener('click',()=>{if(activeThread){activeTab='Diff';rememberSelection();render();void refreshDiff(activeThread);}});
  document.querySelector('#refresh-diff')?.addEventListener('click',()=>{if(activeThread)void refreshDiff(activeThread);});
  document.querySelector<HTMLButtonElement>('#copy-diff')?.addEventListener('click',e=>void copyText(v?.diff || '',e.currentTarget as HTMLButtonElement));
  bindMemory();
  document.querySelector('#capture-memory')?.addEventListener('click',async()=>{if(!activeThread||busy)return;const id=activeThread;busy=true;render();try{await invoke('extract_memory',{threadId:id});await refreshContext(id);}catch(e){error(String(e));}finally{busy=false;}});
  document.querySelector('#astra-plan')?.addEventListener('click',()=>{newTask();(document.querySelector('#task-mode') as HTMLSelectElement).value='codex-plan';updateModelOptions('');saveDraft();render();focusComposer();});
  document.querySelector('#final-review')?.addEventListener('click',()=>choosePhase('review'));
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
function applyEvent(record: JournalEvent, replay = false, deferred = false) {
  const e = record.event;
  for(const [id,v] of views){const worker=v.progress?.workers.find(w=>w.providerThread===e.thread_id);if(worker){applyWorkerEvent(worker,record,!replay);if(activeThread===id&&!replay&&!deferred)renderContent();return;}}
  const auto=threads.find(t=>t.provider==='autonomous'&&t.provider_thread_id===e.thread_id);if(auto&&!replay){void refreshAutonomous(auto.id);return;}
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
  if (e.kind === 'turn_completed') { v.completedTurns??=new Set(); if(e.turn_id)v.completedTurns.add(e.turn_id); if(!e.turn_id||!v.activeTurn||v.activeTurn===e.turn_id){v.running=false;thread.status=e.text;} }
  if (e.kind === 'error') error(e.text);
  if (!replay && e.kind === 'approval_required' && e.item_id) {
    try {
      const request=JSON.parse(e.text) as {executable:string;argv:string[];cwd:string;additional_permissions:string[]};
      const prompt=`API workerが次のコマンドを一度だけ実行する承認を求めています。\n\n実行ファイル: ${request.executable}\nargv: ${JSON.stringify(request.argv)}\ncwd: ${request.cwd}\n追加権限: ${request.additional_permissions.join(', ')}\n\n承認しますか？`;
      const approve=window.confirm(prompt);
      void invoke('decide_api_tool_approval',{approvalId:e.item_id,approve}).catch(problem=>error(String(problem)));
    } catch { error('承認要求の内容を検証できません。実行は保留されています。'); }
  }
  if(!replay&&thread.id===activeThread&&activeTab==='Usage'&&['turn_completed','usage'].includes(e.kind))void refreshUsage();
  if(thread.id===activeThread&&['input_required','goal_updated','turn_completed'].includes(e.kind))void composer?.refreshThread();
  if (thread.id === activeThread&&!replay&&!deferred) render();
}
function knownFiles():string[] { return [...new Set([...(document.querySelector('#known-files') as HTMLInputElement).value.split(',').map(s=>s.trim()).filter(Boolean),...(composer?.getMentions()||[]).filter(m=>m.kind==='file').map(m=>m.kind==='file'?m.path:'')])]; }
async function astraPlan(){if(!activeProject||busy)return;const model=selectedModel();if(!model||model.local){error('計画するモデルを選択してください。');return;}const goal=(document.querySelector('#task-input') as HTMLTextAreaElement).value.trim();if(!goal){error('下の指示欄に、計画したい内容を入力してください。');return;}busy=true;error('');render();try{const p=await invoke('create_astra_plan',{projectId:activeProject,goal,model:model.model,reasoning:selectedReasoning()});planText=JSON.stringify(p,null,2);if(activeProject)ui.plans![activeProject]=planText;saveUi();activeTab='Plan';rememberSelection();}catch(e){error(String(e));}finally{busy=false;render();}}
async function astraInsight(command:'final_review'|'diagnose_task'){if(!activeThread||busy)return;if(chatgptSelected()){error('ChatGPTの会話欄でレビュー・診断を依頼してください。Codexには送りません。');return;}const id=activeThread;busy=true;error('');render();try{if(command==='final_review')view(id).finalReview=await invoke(command,{threadId:id});else view(id).recovery=await invoke(command,{threadId:id,question:(document.querySelector('#task-input') as HTMLTextAreaElement).value.trim()||null});}catch(e){error(String(e));}finally{busy=false;render();}}
async function applyRework(){if(!activeThread||busy)return;busy=true;render();try{await invoke('apply_rework',{threadId:activeThread});}catch(e){error(String(e));}finally{busy=false;render();}}
async function refreshThreads() {try {[threads,archivedThreads]=await Promise.all([invoke<Thread[]>('threads'),invoke<string[]>('archived_threads')]);render();}catch(e){error(String(e),{label:'再試行',run:()=>refreshThreads()});}}
async function syncArchivedSessions() {try {const failures=await invoke<string[]>('sync_archived_sessions');if(failures.length)error(failures.join('\n'),{label:'一覧で状態を確認',run:()=>refreshThreads()});}catch(e){error(`アーカイブ同期の結果を確認できませんでした。一覧を更新して現在の状態を確認してください。 ${String(e)}`,{label:'一覧で状態を確認',run:()=>refreshThreads()});}}
async function refreshGraph() {if(autonomousThread()&&activeThread){const id=activeThread;return readPane(id,'Plan',()=>refreshAutonomous(id));}const id=activeProject;if(!id)return;try {const tasks=await invoke<GraphTask[]>('task_graph',{projectId:id});if(activeProject!==id)return;graph=tasks;if(activeTab==='Plan')renderContent();}catch(e){if(activeProject===id)error(String(e),{label:'再試行',run:()=>refreshGraph()});}}
async function refreshContext(id:string) {return readPane(id,'Context',async()=>{if(threads.find(t=>t.id===id)?.provider==='autonomous'){await refreshAutonomous(id);return;}const [context,capture]=await Promise.all([invoke<ContextInspection|null>('inspect_context',{threadId:id}),invoke<Capture>('memory_candidates',{threadId:id})]);view(id).context=context;view(id).capture=capture;});}
async function runPlan() {if(!activeProject||busy)return;busy=true;error('');render();try {graph=await invoke<GraphTask[]>('run_plan',{projectId:activeProject,plan:JSON.parse(planText),concurrency:2});await refreshThreads();}catch(e){error(String(e));}finally{busy=false;render();}}
function usageMarkup(rows:Usage[],title:string,showProject=false){return `<div class="inspector usage"><h2>${escape(title)}</h2><p>${rows.length}件の推論記録（記録済みの実行のみ）。未取得は —。時間はローカルLLMの処理時間、Codexのターン経過時間です。</p><table><thead><tr>${showProject?'<th>プロジェクト</th>':''}<th>モデル</th><th>入力</th><th>出力</th><th>キャッシュ</th><th>時間</th></tr></thead><tbody>${rows.map(u=>`<tr>${showProject?`<td title="${escape(u.project_id||'')}">${escape(u.project_name||'プロジェクト不明')}</td>`:''}<td>${escape(u.model)}</td><td>${u.prompt_tokens??'—'}</td><td>${u.completion_tokens??'—'}</td><td>${u.cached_tokens??'—'}</td><td>${u.latency_ms==null?'—':(u.latency_ms/1000).toFixed(1)+' s'}</td></tr>`).join('')}</tbody></table></div>`;}
async function refreshUsage() {
 const id=activeThread;if(!id)return;
 return readPane(id,'Usage',async()=>{if(threads.find(t=>t.id===id)?.provider==='autonomous'){await refreshAutonomous(id);return;}const rows=await invoke<Usage[]>('model_usage',{threadId:id});usageByThread.set(id,rows);});
}
async function localInsight(command:'summarize_task'|'review_task') {
  if(!activeThread || busy)return;busy=true;render();
  try {const result=await invoke<Record<string,unknown>>(command,{threadId:activeThread});const v=view(activeThread); if(command==='summarize_task')v.summary=JSON.stringify(result,null,2);else v.review=JSON.stringify(result,null,2);}
  catch(e){error(String(e));}finally{busy=false;render();}
}
async function refreshDiff(id: string) {
  return readPane(id,'Diff',async()=>{if(threads.find(t=>t.id===id)?.provider==='autonomous'){await refreshAutonomous(id);return;}view(id).diff=await invoke<string>('repo_diff',{threadId:id});});
}
async function selectThread(id: string) {
  if (busy) return; mobileSidebarOpen=false;notice(''); saveDraft(); const t=threads.find(t=>t.id===id); if(t)activeProject=t.project_id; activeThread = id; restoreDraft(); rememberSelection(); busy = true; error(''); render();
  try {
    if(t?.provider==='autonomous'){(document.querySelector('#task-mode') as HTMLSelectElement).value='autonomous';await refreshAutonomous(id);return;}
    if(t?.provider==='workflow'){await refreshWorkflow(id);return;}
    if(!legacyThread()&&composerMode()==='autonomous')(document.querySelector('#task-mode') as HTMLSelectElement).value='implement';
    const snapshot = threads.find(t=>t.id===id)?.provider === 'spark' ? {active_turn:null,messages:[]} : await invoke<Snapshot>('resume_task', { threadId: id });
    const v = view(id); if(threads.find(t=>t.id===id)?.provider === 'spark') {v.sequence=0;v.events=[];} v.messages = snapshot.messages.map((m, i) => ({ ...m, key: threads.find(t=>t.id===id)?.provider==='chatgpt'?`chatgpt-${i}`:`restored-${i}` }));
    let after = v.sequence;
    for (;;) { const history = await invoke<JournalEvent[]>('event_history', { threadId: id, after }); for (const e of history) applyEvent(e, true); if (history.length < 2000) break; after = history[history.length - 1].sequence; }
    const insights=await invoke<{review:View['finalReview'];recovery:View['recovery']}>('worker_insights',{threadId:id});v.finalReview=insights.review||undefined;v.recovery=insights.recovery||undefined;
    v.running = !!snapshot.active_turn && !v.completedTurns?.has(snapshot.active_turn.id); v.activeTurn=snapshot.active_turn?.id; v.needsResume = false;
  } catch (e) { view(id).needsResume = true; error(String(e),{label:'再試行',run:()=>selectThread(id)}); } finally { busy = false; render(); focusContent(); }
  if(activeTab==='Usage')void refreshUsage();if(activeTab==='Diff')void refreshDiff(id);if(activeTab==='Context')void refreshContext(id);
  if(threads.find(t=>t.id===id)?.provider==='codex')void composer?.refreshThread(true);
  if(activeProject)void composer?.seedHistory(view(id).messages.filter(m=>m.role==='user').map(m=>m.text).slice(-10),activeProject);
}
async function send(approvedRoute?:AutoPreview) {
  if(archivedThread()){error('アーカイブを解除してから送信してください。');return;}
  if(legacyThread()){error('旧ブラウザ方式の記録は閲覧専用です。新しいタスクで開始してください。');return;}
  if(autonomousMode()){await copyPlanningRequest();return;}
  if(busy||preparingComposer)return;preparingComposer=true;
  try{if(!approvedRoute&&!chatgptSelected()&&composer&&!await composer.beforeSend())return;}catch(e){error(String(e));return;}finally{preparingComposer=false;}
  const input = document.querySelector<HTMLTextAreaElement>('#task-input')!, text = input.value.trim();
  if (!text || busy || !activeProject || selectedView()?.needsResume) return;
  const mentions=composer?.getMentions()||[],files=knownFiles();
  if(chatgptSelected()&&(cloudMode()||mentions.length||files.length)){error('ChatGPTでは通常の実装モードを使い、ファイル参照はCodexで行ってください。');return;}
  if(planningMode()&&mentions.length){error('参照・スキル等を使う計画は「プランモード（Codex）」で作成してください。');return;}
  if(!activeThread&&planningMode()){await astraPlan();void composer?.remember(text);return;}
  const requiresCodex=composerMode()==='goal'||mentions.some(m=>m.kind!=='file');
  if(requiresCodex&&(activeThread||(document.querySelector('#preference') as HTMLSelectElement).value!=='auto')&&!(activeThread?threads.find(t=>t.id===activeThread)?.provider==='codex':selectedModel()&&!selectedModel()!.local)){error('このモード・スキル・プラグイン・サブエージェントの利用にはCodexのモデルを選択してください。');return;}
  if(selectedView()?.running&&mentions.length){error('参照を追加するときは、実行完了を待ってから送信してください。');return;}
  if(composerMode()==='goal'&&text.length>4000){error('ゴールは4000文字以内で指定してください。');return;}
  busy = true; error(''); render();
  try {
    if (!activeThread) {
      const browserSession=draftKey();
      let choice=selectedModel();const key=(document.querySelector('#preference') as HTMLSelectElement).value;
      if(key!=='auto'&&!choice)throw new Error('選択したモデルは利用できません。モデル一覧を更新してください。');
      if(approvedRoute&&key!=='auto')throw new Error('モデル選択が変更されました。もう一度実行してください。');
      if(key==='auto'&&!approvedRoute){
        notice('依頼の内容を確認しています…');
        const preview=await invoke<AutoPreview>('preview_auto_route',{projectId:activeProject,text,knownFiles:files});
        if(preview.confirm_before_run||(!preview.needs_plan&&(preview.blocked||!preview.target))){notice('実行方法を確認してください。');showAutoPreview(preview);return;}
        approvedRoute=preview;
      }
      const autoPlan=key==='auto'&&!!approvedRoute?.needs_plan;
      const autoExplore=key==='auto'&&!autoPlan&&approvedRoute?.level===null&&['codex','api'].includes(approvedRoute?.target?.provider||'');
      let planTarget:ModelTarget|undefined;
      let planAgentName:string|undefined;
      if(autoPlan){
        const config=await invoke<AutoSettings>('auto_settings');
        planTarget=config.planner_default||config.levels.find(row=>row.level===5)?.target;
        const planAssignment=config.route_agents?.find(assignment=>assignment.route==='planner');
        planAgentName=config.agents?.find(agent=>agent.id===planAssignment?.agent_id)?.name;
        if(!planTarget||planTarget.provider==='local')throw new Error('計画用エージェントのクラウドモデルが未設定です。エージェント設定を確認してください。');
        choice=modelChoices.find(m=>profileForChoice(m)===(planTarget!.profile_id||(planTarget!.provider==='codex'?'codex':''))&&m.model===planTarget!.model);
        if(!choice)throw new Error('計画用モデルを利用できません。接続設定とモデル一覧を確認してください。');
        notice('まず計画を作成します。内容を確認してから実装へ進めます。');
      }
      if(!autoPlan)notice(autoExplore?'まず関連箇所を調査し、進め方を整理します。':'依頼を実行しています…');
      if(requiresCodex&&!autoPlan&&approvedRoute?.target?.provider==='local')throw new Error('この参照はCodexが必要です。「おまかせ」からクラウドモデルを指定してください。');
      const explicitProfile=choice?profileForChoice(choice):'';
      const explicitPreference=explicitProfile==='spark'?'spark':explicitProfile==='codex'?'codex':'api';
      const result = await invoke<{thread:Thread;target:ModelTarget;route:{decision:{reason:string;executor:string}};started:boolean}>('create_routed_task', { request:{projectId: activeProject, text, knownFiles:files, preference:autoPlan?(planTarget!.provider==='api'?'api':'codex'):key==='auto'?'auto':explicitPreference,profileId:(autoPlan?planTarget!.profile_id:(choice?profileForChoice(choice):null))??null,model:choice?.model??null,reasoning:autoPlan?planTarget!.reasoning:key==='auto'?null:selectedReasoning(),autoRouteId:autoPlan?null:approvedRoute?.id??null,autoRouteRevision:autoPlan?null:approvedRoute?.revision??null,reviewedWrite:!autoPlan&&!autoExplore&&files.length>0} });
      if(autoPlan||autoExplore)(document.querySelector('#task-mode') as HTMLSelectElement).value='codex-plan';
      const thread=result.thread; delete ui.drafts![browserSession]; threads.unshift(thread); browserWorkspace.moveSession(browserSession,thread.id);activeThread = thread.id; saveDraft(); rememberSelection(); view(thread.id).needsResume = false; view(thread.id).route=autoExplore?`調査から開始 · ${result.target.model}\n${approvedRoute!.reason}`:autoPlan?`${planAgentName?`${planAgentName} · `:''}${result.target.model}\n${approvedRoute!.blocked||approvedRoute!.reason}`:`${key==='auto'&&approvedRoute?.agent_name?`${approvedRoute.agent_name} · `:key==='auto'?'おまかせ · ':''}${result.target.model}\n${result.route.decision.reason}`;
      if(result.started&&thread.provider==='autonomous'){
        (document.querySelector('#task-mode') as HTMLSelectElement).value='autonomous';
        input.value='';composer?.clear();saveDraft();await refreshAutonomous(thread.id);return;
      }
      threadModels[thread.id]=result.target.model;threadTargets[thread.id]=result.target;if(result.target.reasoning)threadReasoning[thread.id]=result.target.reasoning;
      void invoke<Record<string,string>>('thread_models').then(models=>{threadModels=models;render();}).catch(e=>notice(String(e)));
    }
    const v = view(activeThread);
    if(threads.find(t=>t.id===activeThread)?.provider === 'spark') { await invoke('run_local',{threadId:activeThread,text,knownFiles:files}); }
    else { v.messages.push({ role: 'user', text, key: `user-${Date.now()}` });
    if (v.running) await invoke('steer_turn', { threadId: activeThread, text });
    else { await invoke(threadModels[activeThread]||mentions.length||cloudMode()?'send_composed_turn':'send_turn', { threadId: activeThread, text,options:{mode:composerMode()==='codex-plan'?'plan':'default',mentions,goal:composerMode()==='goal'?text:null} }); }
    }
    void composer?.remember(text);input.value = '';composer?.clear();
    if(composerMode()==='goal')(document.querySelector('#task-mode') as HTMLSelectElement).value='implement';
    saveDraft();void composer?.refreshThread();
  } catch (e) { if(String(e).includes('ローカル処理を停止しました'))notice('ローカル処理を停止しました。');else{if (activeThread) view(activeThread).needsResume = true; error(String(e));} }
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
let settingsReady=false,settingsSaving=false,settingsLoading=false;
type ProviderProfileStatus={id:string;name:string;protocol:'open_ai_chat'|'open_ai_responses'|'anthropic_messages'|'local_dedicated'|'codex_app_server';base_url:string|null;locality:'local'|'cloud';credential_env:string|null;max_concurrency:number;enabled:boolean;revision:number;credential_present:boolean};
let providerProfiles:ProviderProfileStatus[]=[];
function renderProviderProfiles(){
 const list=document.querySelector<HTMLElement>('#provider-profile-list')!;
 const editable=providerProfiles.filter(profile=>!['codex','spark'].includes(profile.id));
 list.innerHTML=editable.length?editable.map(profile=>{const choices=modelChoices.filter(model=>profileForChoice(model)===profile.id);const needsCanary=['open_ai_chat','open_ai_responses'].includes(profile.protocol)&&(profile.base_url||'').replace(/\/$/,'')!=='https://api.openai.com/v1';return `<article class="provider-profile"><div><strong>${escape(profile.name)}</strong><small>${escape(profile.protocol)} · ${escape(profile.base_url||'')}</small><small>${profile.credential_env?`${escape(profile.credential_env)}: ${profile.credential_present?'設定済み':'未設定'}`:'認証なし'} · 同時実行 ${profile.max_concurrency} · ${profile.enabled?'有効':'無効'}</small>${choices.length?`<small>${choices.map(model=>`${escape(model.model)}: ${model.tools?'tool利用可':'read-only'}`).join(' · ')}</small>`:''}${needsCanary?`<label>tool canary対象モデル<select data-provider-canary-model="${escape(profile.id)}">${choices.map(model=>`<option value="${escape(model.model)}">${escape(model.model)}</option>`).join('')}</select></label><button type="button" data-provider-canary="${escape(profile.id)}" ${choices.length?'':'disabled'}>推論1回でtool機能を確認</button>`:''}</div><button type="button" data-provider-edit="${escape(profile.id)}">編集</button></article>`;}).join(''):'<p class="muted">API providerはまだありません。</p>';
 list.querySelectorAll<HTMLButtonElement>('[data-provider-edit]').forEach(button=>button.onclick=()=>openProviderProfile(providerProfiles.find(profile=>profile.id===button.dataset.providerEdit)));
 list.querySelectorAll<HTMLButtonElement>('[data-provider-canary]').forEach(button=>button.onclick=async()=>{const profileId=button.dataset.providerCanary!;const modelId=Array.from(list.querySelectorAll<HTMLSelectElement>('[data-provider-canary-model]')).find(select=>select.dataset.providerCanaryModel===profileId)?.value;if(!modelId)return;if(!window.confirm(`${profileId} / ${modelId} にtool canaryを1回送信します。API課金が発生する可能性があります。実行しますか？`))return;button.disabled=true;try{await invoke('run_provider_tool_canary',{profileId,modelId});notice(`${profileId} / ${modelId} のtool機能を確認しました。`);await refreshModels();renderProviderProfiles();}catch(e){error(String(e));}finally{button.disabled=false;}});
}
function openProviderProfile(profile?:ProviderProfileStatus){
 const value=(id:string)=>(document.getElementById(id) as HTMLInputElement|HTMLSelectElement);
 value('provider-revision').value=profile?String(profile.revision):'0';
 value('provider-id').value=profile?.id||'';(value('provider-id') as HTMLInputElement).readOnly=!!profile;
 value('provider-name').value=profile?.name||'';
 value('provider-protocol').value=profile?.protocol||'anthropic_messages';
 value('provider-locality').value=profile?.locality||'cloud';
 value('provider-base-url').value=profile?.base_url||(profile?.protocol==='open_ai_responses'?'https://api.openai.com/v1':'https://api.anthropic.com/v1');
 value('provider-credential-env').value=profile?.credential_env||(profile?.protocol==='open_ai_responses'?'OPENAI_API_KEY':'ANTHROPIC_API_KEY');
 value('provider-concurrency').value=String(profile?.max_concurrency||1);
 (document.getElementById('provider-enabled') as HTMLInputElement).checked=profile?.enabled??true;
 document.getElementById('provider-profile-error')!.textContent='';
 (document.getElementById('provider-profile-dialog') as HTMLDialogElement).showModal();
}
document.getElementById('provider-profile-add')!.addEventListener('click',()=>openProviderProfile());
document.getElementById('provider-profile-cancel')!.addEventListener('click',()=>document.querySelector<HTMLDialogElement>('#provider-profile-dialog')!.close());
document.getElementById('provider-protocol')!.addEventListener('change',()=>{
 const protocol=(document.getElementById('provider-protocol') as HTMLSelectElement).value;
 if(protocol==='anthropic_messages'){
  (document.getElementById('provider-base-url') as HTMLInputElement).value='https://api.anthropic.com/v1';
  (document.getElementById('provider-credential-env') as HTMLInputElement).value='ANTHROPIC_API_KEY';
  (document.getElementById('provider-locality') as HTMLSelectElement).value='cloud';
 }else if(protocol==='open_ai_responses'){
  (document.getElementById('provider-base-url') as HTMLInputElement).value='https://api.openai.com/v1';
  (document.getElementById('provider-credential-env') as HTMLInputElement).value='OPENAI_API_KEY';
 }
});
document.getElementById('provider-profile-form')!.addEventListener('submit',async event=>{
 event.preventDefault();const form=event.currentTarget as HTMLFormElement;if(!form.reportValidity())return;
 const input=(id:string)=>(document.getElementById(id) as HTMLInputElement).value.trim();
 const revision=Number(input('provider-revision'));
 const profile={id:input('provider-id'),name:input('provider-name'),protocol:(document.getElementById('provider-protocol') as HTMLSelectElement).value,base_url:input('provider-base-url'),locality:(document.getElementById('provider-locality') as HTMLSelectElement).value,credential_env:input('provider-credential-env')||null,max_concurrency:Number(input('provider-concurrency')),enabled:(document.getElementById('provider-enabled') as HTMLInputElement).checked,revision:revision?revision+1:1};
 formBusy(form,true);document.getElementById('provider-profile-error')!.textContent='';
 try{
  const models=await invoke<{id:string}[]>('set_provider_profile',{profile});
  providerProfiles=await invoke<ProviderProfileStatus[]>('provider_profiles');renderProviderProfiles();
  document.querySelector<HTMLDialogElement>('#provider-profile-dialog')!.close();
  notice(`${profile.name}を保存しました。${models.length?` ${models.length}モデルを確認しました。`:''}`);void refreshModels();
 }catch(e){document.getElementById('provider-profile-error')!.textContent=String(e);}
 finally{formBusy(form,false);}
});
async function loadConnectionSettings(){
 if(settingsLoading)return;settingsLoading=true;settingsReady=false;
 const form=document.querySelector<HTMLFormElement>('#settings-form')!,summary=document.querySelector<HTMLElement>('#settings-error')!;
 formBusy(form,true);form.dataset.loading='true';document.querySelector<HTMLButtonElement>('#settings-cancel')!.disabled=false;form.querySelector<HTMLButtonElement>('[type="submit"]')!.textContent='読み込み中…';summary.textContent='';
 try{
  const [local,path,config,profiles]=await Promise.all([invoke<LocalConfig>('local_model_settings'),invoke<string>('codex_binary'),invoke<{mode:string;model:string;reasoning:string|null}>('astra_settings'),invoke<ProviderProfileStatus[]>('provider_profiles')]);
  providerProfiles=profiles;renderProviderProfiles();
  localConfig=local;for(const [id,value] of Object.entries({'local-name':local.display_name,'local-id':local.model_id,'local-endpoint':local.endpoint,'codex-path':path,'astra-mode':config.mode,'astra-model':config.model}))(document.getElementById(id) as HTMLInputElement|HTMLSelectElement).value=value;
  fillReasoningSelect(document.querySelector<HTMLSelectElement>('#astra-reasoning')!,modelChoices.find(m=>!m.local&&m.model===config.model),config.reasoning);settingsReady=true;
  form.querySelectorAll<HTMLInputElement>('input').forEach(field=>{field.removeAttribute('aria-invalid');document.getElementById(`${field.id}-error`)!.textContent='';});
 }catch(e){summary.textContent=`設定を読み込めませんでした。保存はまだできません。 ${String(e)}`;const retry=document.createElement('button');retry.type='button';retry.textContent='設定を再読み込み';retry.onclick=()=>void loadConnectionSettings();summary.append(retry);}
 finally{settingsLoading=false;delete form.dataset.loading;formBusy(form,false);form.querySelector<HTMLButtonElement>('[type="submit"]')!.disabled=!settingsReady;}
}
document.querySelector('#settings')!.addEventListener('click', async () => {
  document.querySelector('#settings-error')!.textContent='';
  settingsNavigation.select('connection');
  (document.querySelector('#connection-settings') as HTMLDialogElement).showModal();
  const total=document.querySelector('#total-usage')!;total.textContent='使用量を読み込み中…';void invoke<Usage[]>('model_usage').then(rows=>total.innerHTML=usageMarkup(rows,'全体の使用量',true)).catch(e=>total.textContent=String(e));
  await loadConnectionSettings();
});
document.querySelector('#settings-cancel')!.addEventListener('click', () => (document.querySelector('#connection-settings') as HTMLDialogElement).close());
document.querySelector('#settings-form')!.addEventListener('submit', async e => {
  e.preventDefault();if(settingsSaving||!settingsReady)return;settingsSaving=true;
  const form=document.querySelector<HTMLFormElement>('#settings-form')!;formBusy(form,true);let savedPart=false;
  try {
    const value=(id:string)=>(document.getElementById(id) as HTMLInputElement).value.trim();
    const next={display_name:value('local-name'),model_id:value('local-id'),endpoint:value('local-endpoint')};
    const localChanged=JSON.stringify(next)!==JSON.stringify(localConfig&&{display_name:localConfig.display_name,model_id:localConfig.model_id,endpoint:localConfig.endpoint});
    if(localChanged){localConfig=await invoke<LocalConfig>('set_local_model_settings',{config:next});savedPart=true;notice('ローカルモデル設定を保存しました。アプリを再起動すると反映されます。');}
    const path=(document.querySelector('#codex-path') as HTMLInputElement).value;if(path!==await invoke<string>('codex_binary')) {await invoke('set_codex_binary',{path});savedPart=true;for(const v of views.values())v.needsResume=true;}
    await invoke('set_astra_settings',{config:{mode:(document.querySelector('#astra-mode') as HTMLSelectElement).value,model:(document.querySelector('#astra-model') as HTMLInputElement).value,reasoning:(document.querySelector('#astra-reasoning') as HTMLSelectElement).value||null}});
    document.querySelector('#settings-error')!.textContent = '';
    (document.querySelector('#connection-settings') as HTMLDialogElement).close(); error('');if(!localChanged)notice('接続設定を保存しました。');void refreshModels();
  } catch(e) { const summary=document.querySelector<HTMLElement>('#settings-error')!;summary.textContent=(savedPart?'一部の設定は保存済みです。未完了の設定を確認してください。 ':'保存できませんでした。入力内容を残しています。 ')+String(e);summary.focus(); }
  finally{settingsSaving=false;formBusy(form,false);}
});
document.querySelector('#add')!.addEventListener('click', openDialog);
document.querySelector('#new')!.addEventListener('click', newTask);
document.querySelector<HTMLTextAreaElement>('#task-input')!.addEventListener('input',updateSendAvailability);
document.querySelector('#resume')!.addEventListener('click', async () => { if (!activeThread||busy)return; if(!autonomousThread()){void selectThread(activeThread);return;}const id=activeThread;busy=true;render();try{await invoke('autonomous_resume',{threadId:id});await refreshAutonomous(id);}catch(e){error(String(e));}finally{busy=false;render();} });
document.querySelector('#send')!.addEventListener('click', () => void send());
document.querySelector<HTMLTextAreaElement>('#task-input')!.addEventListener('keydown', e => { if (!e.isComposing && e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); void send(); } });
document.querySelector('#stop')!.addEventListener('click', async () => { if (!activeThread || stopping) return; const id=activeThread;stopping=true;render();try{await invoke(autonomousThread()?'autonomous_stop':'interrupt_turn',{threadId:id});if(autonomousThread())await refreshAutonomous(id);else if(workflowThread())await refreshWorkflow(id);}catch(e){view(id).needsResume=true;error(String(e));}finally{stopping=false;render();}});
document.querySelector('#cancel')!.addEventListener('click', () => (document.querySelector('#register') as HTMLDialogElement).close());
document.querySelector('#register form')!.addEventListener('submit', async e => {
  e.preventDefault(); if(busy)return; busy=true;const form=e.currentTarget as HTMLFormElement;formBusy(form,true);render();try{await registerProject((document.querySelector('#path') as HTMLInputElement).value);}finally{busy=false;formBusy(form,false);render();if(document.querySelector<HTMLDialogElement>('#register')!.open)document.querySelector<HTMLElement>('#form-error')!.focus();else focusComposer();}
});
document.querySelector('#browse-project')!.addEventListener('click',()=>{(document.querySelector('#register') as HTMLDialogElement).close();void pickProject();});
document.querySelector('#browse-codex')!.addEventListener('click',async()=>{if(picking)return;picking=true;try{const path=await nativeInvoke<string|null>('choose_path',{kind:'codex_binary'});if(path)(document.querySelector('#codex-path') as HTMLInputElement).value=path;}catch(e){document.querySelector('#settings-error')!.textContent=String(e);}finally{picking=false;}});
const settingsDialog=document.querySelector<HTMLDialogElement>('#connection-settings')!;
const settingsNavigation=setupSettingsNavigation(settingsDialog);
const openMcpSettings=setupMcpSettings(invoke,()=>projects,()=>notice('MCP公開設定を保存しました。'),{container:settingsNavigation.mcp,open:()=>settingsNavigation.select('mcp'),close:()=>settingsDialog.close()});
settingsNavigation.setMcpLoader(openMcpSettings);
setupMemory(()=>activeProject,error);
composer=setupComposer({call:invoke,localOnly:chatgptSelected,project:()=>activeProject,thread:()=>activeThread,busy:()=>busy,running:()=>!!selectedView()?.running,codex:()=>threads.find(t=>t.id===activeThread)?.provider==='codex',mode:composerMode,setMode:setComposerMode,models:()=>modelChoices,changed:()=>{saveDraft();render();},notice,error});
setupAutoRouting({call:invoke,models:()=>modelChoices,refresh:refreshModels,busy:()=>busy,run:preview=>void send(preview),plan:preview=>void send(preview),notice});
document.querySelector('#auto-settings-button')!.textContent='エージェント';
document.querySelector('#auto-settings-button')!.addEventListener('click',()=>void openAutoSettings());
document.querySelector('#astra-model')!.addEventListener('input',()=>fillReasoningSelect(document.querySelector<HTMLSelectElement>('#astra-reasoning')!,modelChoices.find(m=>!m.local&&m.model===(document.querySelector('#astra-model') as HTMLInputElement).value)));
window.addEventListener('unhandledrejection', e => error(String(e.reason)));
window.addEventListener('error', e => error(e.message));



render();
if (isTauri()) {
  void refreshModels();
  void syncArchivedSessions();
  await listen<JournalEvent>('hub-event', e => queueHubEvent(e.payload));
  await listen<number>('hub-stream-gap', () => { if (activeThread) { view(activeThread).needsResume = true; error('イベント配信が遅れました。「再開・状態を確認」で保存済み履歴を読み直してください。'); render(); } });
  await loadWorkspace();
}
async function loadWorkspace(){
 if(loadingWorkspace)return;loadingWorkspace=true;render();
 try { [projects, threads, archivedThreads] = await Promise.all([invoke<Project[]>('projects'), invoke<Thread[]>('threads'),invoke<string[]>('archived_threads')]); activeProject = projects.find(p=>p.id===ui.project)?.id || projects[0]?.id || null; activeTab=tabs.includes(ui.tab||'')?ui.tab!:'Chat';planText=activeProject?ui.plans![activeProject]||planText:planText;restoreDraft();render();if(ui.thread && threads.some(t=>t.id===ui.thread&&t.project_id===activeProject))await selectThread(ui.thread);if(activeTab==='Plan')void refreshGraph();if(activeTab==='Diff'&&activeThread)void refreshDiff(activeThread);if(activeTab==='Context'&&activeThread)void refreshContext(activeThread);if(activeTab==='Usage')void refreshUsage(); }
 catch(e){error(`作業一覧を読み込めませんでした。 ${String(e)}`,{label:'一覧を再読み込み',run:()=>loadWorkspace()});}
 finally{loadingWorkspace=false;render();}
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
  ui.drafts![draftKey()]={text:input.value,files:(document.querySelector('#known-files') as HTMLInputElement).value,preference:(document.querySelector('#preference') as HTMLSelectElement).value,mode:(document.querySelector('#task-mode') as HTMLSelectElement).value,reasoning:selectedReasoning(),mentions:composer?.getMentions()||[]};
  saveUi(); document.querySelector('#draft-status')!.textContent=input.value?'下書きはこの端末に保存':'';
}
function restoreDraft() {
  const draft=ui.drafts![draftKey()];
  (document.querySelector('#task-input') as HTMLTextAreaElement).value=typeof draft?.text==='string'?draft.text:'';
  (document.querySelector('#known-files') as HTMLInputElement).value=typeof draft?.files==='string'?draft.files:'';
  composerReasoning=draft?.reasoning||null;
  composer?.setMentions(Array.isArray(draft?.mentions)?draft.mentions:[]);
  const mode=['autonomous','implement','codex-plan','goal'].includes(draft?.mode||'')?draft!.mode!:('implement');
  (document.querySelector('#task-mode') as HTMLSelectElement).value=mode;updateModelOptions(draft?.preference||'auto');
  document.querySelector('#draft-status')!.textContent=draft?.text?'保存した下書き':'';
}
function focusComposer() { document.querySelector<HTMLTextAreaElement>('#task-input')!.focus(); }
function switchProject(id:string, force=false) {
  if(busy&&!force)return;mobileSidebarOpen=false;error('');notice(''); saveDraft();activeProject=id;activeThread=null;activeTab='Chat';graph=[];
  planText=ui.plans![id] || defaultPlanText;restoreDraft();rememberSelection();render();
  if(activeTab==='Plan')void refreshGraph();focusComposer();
}
function newTask() {
  if(busy)return;mobileSidebarOpen=false;notice('');saveDraft();activeThread=null;activeTab='Chat';restoreDraft();rememberSelection();error('');render();focusComposer();
  if(!activeProject)void pickProject();
}
function notice(message:string) { const el=document.querySelector<HTMLDivElement>('#notice')!;el.hidden=!message;el.textContent=message; }
async function copyText(text:string, button?:HTMLButtonElement) {
  try { await navigator.clipboard.writeText(text); if(button){const old=button.textContent;button.textContent='コピー済み';setTimeout(()=>{if(button.isConnected)button.textContent=old;},1400);} else notice('コピーしました。'); return true; }
  catch { error('コピーできませんでした。テキストを選択して ⌘C を押してください。'); return false; }
}
function renderThreads() {
  const root=document.querySelector<HTMLElement>('#projects')!,focused=document.activeElement;
  const focusKey=focused instanceof HTMLElement&&root.contains(focused)?controlKey(focused):undefined;
  const archive=document.querySelector<HTMLButtonElement>('#show-archived')!;archive.classList.toggle('active',showArchived);archive.setAttribute('aria-pressed',String(showArchived));
  if(lastTreeProject!==activeProject){lastTreeProject=activeProject;if(activeProject&&!ui.expandedProjects!.includes(activeProject))ui.expandedProjects!.push(activeProject);}
  const query=taskFilter.toLowerCase();
  const markup=[...projects].sort((a,b)=>Number(ui.pinnedProjects!.includes(b.id))-Number(ui.pinnedProjects!.includes(a.id))).map(p=>{
    const all=threads.filter(t=>t.project_id===p.id&&archivedThreads.includes(t.id)===showArchived);
    const matches=all.filter(t=>!query||p.name.toLowerCase().includes(query)||`${t.title} ${statusLabel(t.status)}`.toLowerCase().includes(query));
    if(query&&!matches.length&&!p.name.toLowerCase().includes(query))return '';
    matches.sort((a,b)=>Number(ui.pins!.includes(b.id))-Number(ui.pins!.includes(a.id)));
    const expanded=!!query||ui.expandedProjects!.includes(p.id);
    return `<section class="project-group ${p.id===activeProject?'active-project':''}" data-project-group="${p.id}"><div class="project-row"><button class="project-toggle" data-project-toggle="${p.id}" aria-label="${escape(p.name)}のタスクを${expanded?'閉じる':'表示'}" aria-expanded="${expanded}">${expanded?'⌄':'›'}</button><button class="project" data-project="${p.id}" title="${escape(p.root)}"><span>${escape(p.name)}</span><small>${all.length}</small></button><button class="project-more" data-project-menu="${p.id}" aria-label="${escape(p.name)}の操作">⋯</button><button class="project-more" data-project-new="${p.id}" aria-label="${escape(p.name)}で新規セッションを開始" title="新規セッション">＋</button></div>${expanded?`<div class="project-tasks" aria-label="${escape(p.name)}のタスク">${matches.map(t=>`<div class="thread-row"><button class="thread ${t.id===activeThread?'selected':''}" data-thread="${t.id}" title="${escape(t.title)}"><span>${escape(t.title)}</span><small class="state-${escape(t.status)}">${escape(statusLabel(t.status))}</small></button><button class="pin-thread ${ui.pins!.includes(t.id)?'pinned':''}" data-pin="${t.id}" aria-label="${escape(t.title)}を${ui.pins!.includes(t.id)?'固定解除':'固定'}" aria-pressed="${ui.pins!.includes(t.id)}">${ui.pins!.includes(t.id)?'★':'☆'}</button></div>`).join('')||`<p class="side-empty muted">${showArchived?'アーカイブはありません。':'タスクはまだありません。'}</p>`}</div>`:''}</section>`;
  }).join('')||`<p class="side-empty muted">${loadingWorkspace?'プロジェクトを読み込み中…':!projects.length?'フォルダを開くと、プロジェクトとタスクをここに表示します。':'該当するプロジェクト・タスクはありません。検索語を短くしてお試しください。'}</p>`;
  if(lastTreeMarkup===markup)return;lastTreeMarkup=markup;root.innerHTML=markup;
  document.querySelectorAll<HTMLButtonElement>('[data-project-toggle]').forEach(b=>{const expanded=b.getAttribute('aria-expanded')==='true';b.innerHTML=`<svg aria-hidden="true" viewBox="0 0 16 16"><path d="${expanded?'m4 6 4 4 4-4':'m6 4 4 4-4 4'}"/></svg>`;});
  document.querySelectorAll<HTMLButtonElement>('[data-project-menu]').forEach(b=>{b.innerHTML='<svg aria-hidden="true" viewBox="0 0 16 16"><path d="M3.5 8h.01M8 8h.01M12.5 8h.01"/></svg>';});
  document.querySelectorAll<HTMLButtonElement>('[data-project-new]').forEach(b=>{b.innerHTML='<svg aria-hidden="true" viewBox="0 0 16 16"><path d="M8 3v10M3 8h10"/></svg>';});
  document.querySelectorAll<HTMLButtonElement>('[data-pin]').forEach(b=>{b.innerHTML=`<svg aria-hidden="true" viewBox="0 0 16 16"><path d="m8 2 1.8 3.7 4.2.6-3 2.9.7 4.2L8 11.4l-3.7 2 .7-4.2-3-2.9 4.2-.6L8 2Z" ${b.getAttribute('aria-pressed')==='true'?'fill="currentColor" stroke="none"':''}/></svg>`;});
  document.querySelectorAll<HTMLElement>('.thread-row').forEach(row=>{
    const thread=row.querySelector<HTMLButtonElement>('[data-thread]');if(!thread)return;
    row.classList.toggle('selected',thread.classList.contains('selected'));
    const button=document.createElement('button');button.className='session-more';button.dataset.threadMenu=thread.dataset.thread!;
    button.title='アーカイブ・削除';button.setAttribute('aria-label',`${thread.title}のセッション操作`);
    button.innerHTML='<svg aria-hidden="true" viewBox="0 0 16 16"><path d="M3.5 8h.01M8 8h.01M12.5 8h.01"/></svg>';row.append(button);
  });
  document.querySelectorAll<HTMLButtonElement>('[data-project-new]').forEach(b=>b.onclick=()=>{if(busy)return;switchProject(b.dataset.projectNew!);newTask();});
  document.querySelectorAll<HTMLButtonElement>('[data-project-toggle]').forEach(b=>b.onclick=()=>{const id=b.dataset.projectToggle!;ui.expandedProjects=ui.expandedProjects!.includes(id)?ui.expandedProjects!.filter(p=>p!==id):[...ui.expandedProjects!,id];saveUi();renderThreads();});
  document.querySelectorAll<HTMLButtonElement>('[data-project]').forEach(b=>b.onclick=()=>switchProject(b.dataset.project!));
  document.querySelectorAll<HTMLButtonElement>('[data-project-menu]').forEach(b=>b.onclick=()=>openProjectMenu(b.dataset.projectMenu!,b));
  document.querySelectorAll<HTMLButtonElement>('[data-thread]').forEach(b=>b.onclick=()=>void selectThread(b.dataset.thread!));
  document.querySelectorAll<HTMLButtonElement>('[data-pin]').forEach(b=>b.onclick=()=>{const id=b.dataset.pin!;ui.pins=ui.pins!.includes(id)?ui.pins!.filter(p=>p!==id):[...ui.pins!,id];saveUi();renderThreads();});
  document.querySelectorAll<HTMLButtonElement>('[data-thread-menu]').forEach(b=>b.onclick=()=>openLifecycle('session',b.dataset.threadMenu!));
  if(focusKey)Array.from(root.querySelectorAll<HTMLElement>('button')).find(el=>controlKey(el)===focusKey)?.focus({preventScroll:true});
}
type MenuAction={title:string;detail:string;run:()=>void};
function menuActions():MenuAction[] {
  return [
    {title:'フォルダを開く',detail:'⌘O',run:()=>void pickProject()},
    {title:'パスを指定してフォルダを開く',detail:'フォルダの絶対パスを入力',run:()=>{document.querySelector<HTMLDialogElement>('#register')!.showModal();}},
    {title:'新しいタスク',detail:'⌘N',run:newTask},
    {title:'接続設定',detail:'⌘,',run:()=>document.querySelector<HTMLButtonElement>('#settings')!.click()},
    {title:'エージェントの振り分け設定',detail:'追加・削除、モデル、Reasoning',run:()=>void openAutoSettings()},
    ...tabs.map(tab=>({title:`${tabLabels[tab]} を表示`,detail:'表示を切り替え',run:()=>{activeTab=tab;rememberSelection();render();focusContent();if(tab==='Diff'&&activeThread)void refreshDiff(activeThread);if(tab==='Plan')void refreshGraph();if(tab==='Context'&&activeThread)void refreshContext(activeThread);if(tab==='Usage')void refreshUsage();}})),
    ...projects.map(p=>({title:p.name,detail:p.root,run:()=>switchProject(p.id)})),
    ...threads.filter(t=>!archivedThreads.includes(t.id)&&projects.some(p=>p.id===t.project_id)).map(t=>({title:t.title,detail:`${projects.find(p=>p.id===t.project_id)?.name||''} · ${statusLabel(t.status)}`,run:()=>{activeTab='Chat';void selectThread(t.id);}})),
  ];
}
function renderCommands() {
  const query=(document.querySelector('#command-query') as HTMLInputElement).value.toLowerCase();
  const matches=menuActions().filter(a=>`${a.title} ${a.detail}`.toLowerCase().includes(query));
  document.querySelector('#command-results')!.innerHTML=matches.map((a,i)=>`<button class="command-result" data-command="${i}"><strong>${escape(a.title)}</strong><small>${escape(a.detail)}</small></button>`).join('')||'<p>一致するタスク・操作がありません。</p>';
  document.querySelectorAll<HTMLButtonElement>('[data-command]').forEach(b=>b.onclick=()=>{(document.querySelector('#commands') as HTMLDialogElement).close();matches[Number(b.dataset.command)].run();});
}
function openCommands() { if(picking||document.querySelector('dialog[open]'))return;const d=document.querySelector<HTMLDialogElement>('#commands')!; (document.querySelector('#command-query') as HTMLInputElement).value='';renderCommands();d.showModal();document.querySelector<HTMLInputElement>('#command-query')!.focus(); }
function narrowWindow(){return window.matchMedia?.('(max-width: 650px)').matches||false;}
function renderNavigation(){
 const narrow=narrowWindow(),open=narrow&&mobileSidebarOpen;
 app.classList.toggle('mobile-sidebar-open',open);
 document.querySelector<HTMLElement>('main')!.inert=open;
 document.querySelector<HTMLElement>('#sidebar-backdrop')!.hidden=!open;
 document.querySelector<HTMLButtonElement>('#sidebar-close')!.hidden=!narrow;
 sidebarButton.setAttribute('aria-expanded',String(narrow?open:!ui.sidebarHidden));
 sidebarButton.setAttribute('aria-label',(narrow?!open:ui.sidebarHidden)?'プロジェクト一覧を表示':'プロジェクト一覧を閉じる');
 if(open){sidebar.setAttribute('role','dialog');sidebar.setAttribute('aria-modal','true');}
 else{sidebar.removeAttribute('role');sidebar.removeAttribute('aria-modal');}
}
function closeNavigation(){mobileSidebarOpen=false;renderNavigation();sidebarButton.focus();}
function toggleSidebar() {
 if(narrowWindow()){mobileSidebarOpen=!mobileSidebarOpen;renderNavigation();(mobileSidebarOpen?document.querySelector<HTMLElement>('#sidebar-close'):sidebarButton)?.focus();}
 else{ui.sidebarHidden=!ui.sidebarHidden;saveUi();render();sidebarButton.focus();}
}
document.querySelector('#sidebar-close')!.addEventListener('click',closeNavigation);
document.querySelector('#sidebar-backdrop')!.addEventListener('click',closeNavigation);
sidebar.addEventListener('keydown',e=>{
 if(!mobileSidebarOpen||!narrowWindow())return;
 if(e.key==='Escape'){e.preventDefault();closeNavigation();}
 if(e.key==='Tab'){
  const controls=Array.from(sidebar.querySelectorAll<HTMLElement>('button:not(:disabled),input')).filter(el=>el.getClientRects().length);
  const first=controls[0],last=controls.at(-1);
  if(e.shiftKey&&document.activeElement===first){e.preventDefault();last?.focus();}
  else if(!e.shiftKey&&document.activeElement===last){e.preventDefault();first?.focus();}
 }
});
window.addEventListener('resize',()=>{if(!narrowWindow())mobileSidebarOpen=false;renderNavigation();});
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
  if(e.shiftKey&&e.key.toLowerCase()==='v'&&autonomousMode()&&!archivedThread()){e.preventDefault();void reviewManifest();return;}
  const action=({o:()=>void pickProject(),n:newTask,k:openCommands,',':()=>document.querySelector<HTMLButtonElement>('#settings')!.click(),b:toggleSidebar} as Record<string,()=>void>)[e.key.toLowerCase()];
  if(action&&!e.shiftKey){e.preventDefault();action();}
});

document.querySelector('#task-mode')!.addEventListener('change',()=>{pendingPreference=undefined;composerReasoning=null;updateModelOptions('');saveDraft();render();});
async function saveActiveModelTarget(){
  if(!activeThread||autonomousMode()||busy)return;
  const target=readTarget(document.querySelector<HTMLSelectElement>('#preference')!,document.querySelector<HTMLSelectElement>('#reasoning')!,modelChoices);
  if(!target){error('利用できるモデルとreasoningを選択してください。');return;}
  if(target.provider==='local'){error('既存セッションはbounded localモデルへ切り替えられません。新しいタスクを作成してください。');return;}
  const id=activeThread;busy=true;error('');render();
  try{
    await invoke('set_thread_model_target',{threadId:id,target});
    threadTargets[id]=target;threadModels[id]=target.model;
    if(target.reasoning)threadReasoning[id]=target.reasoning;else delete threadReasoning[id];
    threads=await invoke<Thread[]>('threads');
    notice(`次のターンは ${modelChoices.find(m=>profileForChoice(m)===(target.profile_id||(target.provider==='codex'?'codex':'spark'))&&m.model===target.model)?.label||target.model} を使います。`);
  }catch(e){error(String(e));await refreshModels();}
  finally{busy=false;render();}
}
document.querySelector('#preference')!.addEventListener('change',()=>{
  composerReasoning=null;refreshComposerReasoning();pendingPreference=(document.querySelector('#preference') as HTMLSelectElement).value;
  if(activeThread)void saveActiveModelTarget();else {saveDraft();render();}
});
document.querySelector('#reasoning')!.addEventListener('change',()=>{if(activeThread)void saveActiveModelTarget();else {composerReasoning=selectedReasoning();saveDraft();render();}});
document.querySelector('#refresh-models')!.addEventListener('click',()=>void refreshModels());

document.querySelector('#show-archived')!.addEventListener('click',()=>{showArchived=!showArchived;renderThreads();});
document.querySelector('#session-actions')!.addEventListener('click',()=>{if(activeThread)openLifecycle('session',activeThread);});
document.querySelector('#lifecycle-close')!.addEventListener('click',()=>document.querySelector<HTMLDialogElement>('#lifecycle-dialog')!.close());
function openLifecycle(kind:'project'|'session',id:string){
  if(busy)return;const dialog=document.querySelector<HTMLDialogElement>('#lifecycle-dialog')!;
  document.querySelector('#lifecycle-title')!.textContent=kind==='project'?projects.find(p=>p.id===id)?.name||'プロジェクト':threads.find(t=>t.id===id)?.title||'セッション';
  document.querySelector('#lifecycle-description')!.textContent=kind==='project'?'アプリから登録を解除します。フォルダとファイルは残り、同じフォルダを登録するとセッションも戻ります。':'アーカイブはCodex側にも反映され、後から復元できます。削除すると、このアプリの会話履歴が削除されます。プロバイダー側の会話と作業ファイルは残ります。';
  const actions=document.querySelector('#lifecycle-actions')!;actions.replaceChildren();document.querySelector('#lifecycle-error')!.textContent='';
  const items=kind==='project'?[['unregister','登録を解除']]:[['rename','名前を変更'],[archivedThreads.includes(id)?'restore':'archive',archivedThreads.includes(id)?'アーカイブから復元':'アーカイブ'],['delete','セッションを削除']];
  for(const [action,label] of items){const button=document.createElement('button');button.textContent=label;button.onclick=async()=>{
    if(action==='rename'){dialog.close();document.querySelector<HTMLButtonElement>('#rename-task')!.click();return;}
    if(action==='delete'&&button.dataset.confirmed!=='true'){
      button.dataset.confirmed='true';button.classList.add('danger');button.textContent='このセッションを削除する';
      document.querySelector('#lifecycle-description')!.textContent=`「${threads.find(t=>t.id===id)?.title||id}」をLocaloudから削除します。プロバイダー側の会話と作業ファイルは残ります。この操作は取り消せません。`;
      button.focus();return;
    }
    button.disabled=true;
    try{
      if(action==='unregister'){await invoke('unregister_project',{projectId:id});projects=await invoke<Project[]>('projects');if(activeProject===id){saveDraft();activeProject=projects[0]?.id||null;activeThread=null;restoreDraft();rememberSelection();}}
      else {await invoke('manage_session',{threadId:id,action});if(action==='delete'){views.delete(id);ui.pins=ui.pins!.filter(p=>p!==id);if(activeThread===id){activeThread=null;restoreDraft();rememberSelection();}}else if(action==='archive'&&activeThread===id){activeThread=null;restoreDraft();rememberSelection();}await refreshThreads();if(action==='restore'&&activeThread===id)await selectThread(id);}
      dialog.close();render();
    }catch(e){document.querySelector('#lifecycle-error')!.textContent=String(e);}finally{button.disabled=false;}
  };actions.append(button);}
  dialog.showModal();
}

let workflowPolling=false;
if(isTauri())setInterval(()=>{
  if(workflowPolling||busy||!activeThread||(!workflowThread()&&!autonomousThread())||!selectedView()?.running)return;
  const id=activeThread;workflowPolling=true;
  void (autonomousThread()?refreshAutonomous(id):refreshWorkflow(id)).catch(e=>{const v=view(id);if(v.progress)v.progress.error=String(e);else error(String(e));render();}).finally(()=>workflowPolling=false);
},1500);

function projectIcon(name:string){
 const paths:Record<string,string>={pin:'<path d="m16 3 5 5-4 2-3 5-5-5 5-3zM9 10l-4 4 5 5 4-4M7 17l-4 4"/>',edit:'<path d="m9 3 6 0 1 3 3 1 2 5-2 5-3 1-1 3H9l-1-3-3-1-2-5 2-5 3-1z"/><circle cx="12" cy="12" r="3"/>',sessions:'<path d="M8 5h13M8 12h13M8 19h13M3 5h.01M3 12h.01M3 19h.01"/>',folder:'<path d="M3 8V5h6l3 3h9v3M3 8h6l3 3h10l-3 9H3z"/>',worktree:'<path d="M4 12h7l9-9M14 3h6v6M11 12l9 9M14 21h6v-6"/>',archive:'<rect x="3" y="3" width="18" height="5" rx="1"/><path d="M5 8v13h14V8M9 12h6"/>',close:'<path d="m5 5 14 14M19 5 5 19"/>'};
 return `<svg viewBox="0 0 24 24" width="21" height="21" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths[name]||''}</svg>`;
}
function openProjectMenu(id:string,anchor:HTMLElement){
 if(busy)return;const project=projects.find(p=>p.id===id);if(!project)return;
 document.querySelector('#project-context-menu')?.remove();
 const menu=document.createElement('dialog');menu.id='project-context-menu';menu.setAttribute('aria-label',`${project.name}の操作`);
 const pinned=ui.pinnedProjects!.includes(id);
 menu.innerHTML=`<div role="menu"><button role="menuitem" data-action="pin"><span>${projectIcon('pin')}</span>${pinned?'ピン留めを解除':'ピン留め'}</button><button role="menuitem" data-action="edit"><span>${projectIcon('edit')}</span>編集</button><hr><button role="menuitem" data-action="sessions" aria-haspopup="menu" aria-expanded="false"><span>${projectIcon('sessions')}</span>セッション <span class="menu-arrow">›</span></button><div id="project-session-menu" role="menu" hidden></div><button role="menuitem" data-action="reveal"><span>${projectIcon('folder')}</span>Finder で表示</button><button role="menuitem" data-action="worktree"><span>${projectIcon('worktree')}</span>永続的な Worktree を作成する</button><hr><button role="menuitem" data-action="archive"><span>${projectIcon('archive')}</span>チャットをアーカイブ</button><hr><button role="menuitem" data-action="delete"><span>${projectIcon('close')}</span>プロジェクトを削除</button></div><p class="project-menu-error" role="alert" hidden></p>`;
 app.append(menu);const rect=anchor.getBoundingClientRect();menu.showModal();menu.style.left=`${Math.max(8,Math.min(rect.left,window.innerWidth-menu.offsetWidth-8))}px`;menu.style.top=`${Math.max(8,Math.min(rect.bottom+4,window.innerHeight-menu.offsetHeight-8))}px`;menu.style.maxHeight=`${window.innerHeight-parseFloat(menu.style.top)-8}px`;
 menu.addEventListener('click',e=>{if(e.target===menu){const r=menu.getBoundingClientRect();if(e.clientX<r.left||e.clientX>r.right||e.clientY<r.top||e.clientY>r.bottom)menu.close();}});
 menu.addEventListener('close',()=>{menu.remove();document.querySelector<HTMLButtonElement>(`[data-project-menu="${id}"]`)?.focus();});
 menu.addEventListener('keydown',e=>{if(!['ArrowDown','ArrowUp','Home','End'].includes(e.key))return;e.preventDefault();const buttons=Array.from(menu.querySelectorAll<HTMLButtonElement>('button')).filter(b=>!b.disabled&&!b.closest('[hidden]'));const i=buttons.indexOf(document.activeElement as HTMLButtonElement);buttons[e.key==='Home'?0:e.key==='End'?buttons.length-1:(i+(e.key==='ArrowDown'?1:-1)+buttons.length)%buttons.length]?.focus();});
 menu.querySelectorAll<HTMLButtonElement>('[data-action]').forEach(button=>button.onclick=async()=>{
  const action=button.dataset.action!;
  if(action==='pin'){ui.pinnedProjects=pinned?ui.pinnedProjects!.filter(p=>p!==id):[...ui.pinnedProjects!,id];saveUi();renderThreads();menu.close();return;}
  if(action==='sessions'){
   const sub=menu.querySelector<HTMLElement>('#project-session-menu')!;sub.hidden=!sub.hidden;button.setAttribute('aria-expanded',String(!sub.hidden));
   sub.innerHTML=threads.filter(t=>t.project_id===id&&!archivedThreads.includes(t.id)).map(t=>`<button role="menuitem" data-session="${t.id}">${escape(t.title)}</button>`).join('')||'<p>セッションはありません。</p>';
   sub.querySelectorAll<HTMLButtonElement>('[data-session]').forEach(b=>b.onclick=()=>{menu.close();activeTab='Chat';void selectThread(b.dataset.session!);});return;
  }
  if(action==='edit'){
   menu.close();const d=document.createElement('dialog');d.innerHTML=`<form><h2>プロジェクトを編集</h2><label>プロジェクト名<input name="name" required maxlength="120" value="${escape(project.name)}"></label><p>${escape(project.root)}</p><p role="alert"></p><div class="dialog-actions"><button type="button">キャンセル</button><button type="submit">保存</button></div></form>`;app.append(d);d.showModal();d.querySelector('button')!.onclick=()=>d.close();d.onclose=()=>d.remove();d.querySelector('form')!.onsubmit=async e=>{e.preventDefault();const submit=d.querySelector<HTMLButtonElement>('[type=submit]')!;submit.disabled=true;try{await invoke('manage_project',{projectId:id,action:'rename',name:d.querySelector('input')!.value});projects=await invoke<Project[]>('projects');render();d.close();}catch(e){d.querySelector('[role=alert]')!.textContent=String(e);}finally{submit.disabled=false;}};return;
  }
  if(action==='delete'){menu.close();openLifecycle('project',id);return;}
  const buttons=menu.querySelectorAll<HTMLButtonElement>('button');buttons.forEach(b=>b.disabled=true);
  try{
   const path=await invoke<string|null>('manage_project',{projectId:id,action});
   if(action==='archive'){await refreshThreads();if(threads.some(t=>t.id===activeThread&&t.project_id===id)){activeThread=null;restoreDraft();rememberSelection();}render();}
   if(action==='worktree'){projects=await invoke<Project[]>('projects');renderThreads();notice(`永続的なWorktreeを作成しました: ${path}`);}
   menu.close();
  }catch(e){const el=menu.querySelector<HTMLElement>('[role=alert]')!;el.hidden=false;el.textContent=String(e);}finally{buttons.forEach(b=>b.disabled=false);}
 });
}
