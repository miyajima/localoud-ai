import {tabLabels,flowMarkup,executionOverview,executionPlan,executionContext,executionUsage} from '../src/workflow-ui.ts';
import {focusMarkup,reviewRequest,recordMarkup} from '../src/task-focus.ts';
import {syncWorkers,applyWorkerEvent,mergeActivity,progressMarkup,changesMarkup} from '../src/worker-progress.ts';
import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import {JSDOM} from 'jsdom';
import ts from 'typescript';
import {appendModelOptions,fillReasoningSelect,readTarget,resolvedModel,modelForTarget} from '../src/model-controls.ts';
const source=(await readFile(new URL('../src/main.ts',import.meta.url),'utf8')).replace(/^import .*;\s*$/gm,'');
const js=ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ES2022}}).outputText.replace(/export\s*\{\s*\};?/g,'');
async function fixture({saved,existing=false,connected=true,duplicate=false,importStatus,importGate,importError,multipleProjects=false,browserOpen=true,autoPreview,planTarget,routeError}={}){
 const dom=new JSDOM('<div id="app"></div>',{url:'https://workflow.fixture'});
 if(saved)dom.window.localStorage.setItem('astra-ui-v1',saved);
 dom.window.HTMLDialogElement.prototype.showModal=function(){this.setAttribute('open','');};dom.window.HTMLDialogElement.prototype.close=function(){this.removeAttribute('open');this.dispatchEvent(new dom.window.Event('close'));};
 const calls=[];const clipboard=[];const navigator={clipboard:{writeText:async text=>{clipboard.push(text);}}};
 const project={id:'project',name:'Fixture',root:'/fixture'};
 const root={id:'root-session',project_id:project.id,provider:'workflow',provider_thread_id:'workflow:root',title:'入力を保持する',status:'idle'};
 const state={projects:multipleProjects?[project,{id:'other',name:'別プロジェクト',root:'/other'}]:[project],threads:existing?[root]:[],running:false,failReview:false,messages:existing?[{role:'user',text:'保存済みの依頼',key:'old-user'},{role:'assistant',text:'保存済みのプラン',key:'old-plan',label:'プラン · ChatGPT'}]:[],targets:existing?{plan:{model:'gpt-6-astra',reasoning:'high',chatgpt:true},review:{model:'gpt-6-astra',reasoning:'high',chatgpt:true},implement:{model:'codex-implementation',reasoning:'high',chatgpt:false}}:{},phase:'plan',model:'gpt-6-astra'};
 const invoke=async(command,args)=>{
  calls.push({command,args});
  switch(command){
   case 'available_models':return {models:[{key:'codex:gpt-6-astra',label:'Astra on Codex',model:'gpt-6-astra',local:false,reasoning:['high'],default_reasoning:null},{key:'codex:codex-implementation',label:'Implementation',model:'codex-implementation',local:false,reasoning:['high','low'],default_reasoning:null}],warnings:[]};
   case 'preview_auto_route':return autoPreview||{id:'route-1',revision:0,target:{provider:'codex',model:'codex-implementation',reasoning:'high'},reason:'routine',needs_plan:false,blocked:null,confirm_before_run:false};
   case 'auto_settings':return {levels:[{level:5,target:planTarget||{provider:'codex',model:'gpt-6-astra',reasoning:'high'}}]};
   case 'create_routed_task':{if(routeError)throw Error(routeError);const thread={id:'direct-task',project_id:project.id,provider:'codex',provider_thread_id:'provider-direct',title:args.request.text,status:'idle'};state.threads.push(thread);return {thread,target:{model:'codex-implementation',reasoning:'high'},route:{decision:{executor:'codex',reason:'explicit'}}};}
   case 'interrupt_turn':case 'send_composed_turn':case 'send_turn':case 'steer_turn':return;
   case 'resume_task':return {active_turn:null,messages:[]};
   case 'event_history':return [];
   case 'model_usage':return args?.threadId?[{model:'session-model',prompt_tokens:1,latency_ms:1000}]:[{model:'global-model',prompt_tokens:99,latency_ms:2000}];
   case 'manage_project':return null;
   case 'worker_insights':return {review:null,recovery:null};
   case 'projects':return state.projects||[project];
   case 'threads':return state.threads;
   case 'sync_archived_sessions':return [];
   case 'archived_threads':return state.archived||[];
   case 'thread_models':return state.threads.some(t=>t.provider==='codex')?{'direct-task':'codex-implementation'}:{};
   case 'thread_reasoning':return {};
   case 'workflow_snapshot':return {status:'legacy_read_only',running:state.running,messages:state.messages,phase:state.phase,model:state.model,targets:state.targets,handoff:state.phase==='review'?'要件・プラン・実装結果・作業差分':'これまでの会話'};
   case 'workflow_stop':state.running=false;return;
   case 'workflow_role_settings':return {plan:{model:'planner',reasoning:'medium'},review:{model:'reviewer',reasoning:'pro'}};
   case 'manifest_import_clipboard':if(importGate)await importGate;if(importError)throw Error(importError);state.threads=[{...root,provider:'autonomous',status:importStatus||(duplicate?'interrupted':'queued')}];state.running=!duplicate;return {thread:state.threads[0],imported:!duplicate};
   case 'autonomous_snapshot':return {thread:state.threads.find(t=>t.id===args.threadId)||state.threads[0],messages:[{role:'user',text:'依頼だけ'}],steps:state.steps||[],children:state.children||[],final_diff:null,artifact:null,...state.snapshot};
   case 'autonomous_activity':if(state.activityError)throw Error(state.activityError);return state.activity||{workers:[],changes:[]};
   case 'autonomous_stop':state.running=false;state.threads[0].status='interrupted';return;
   default:throw Error('Unexpected command: '+command);
  }
 };
 const noop=()=>{};
 const context={tabLabels,flowMarkup,executionOverview,executionPlan,executionContext,executionUsage,focusMarkup,reviewRequest,recordMarkup,syncWorkers,applyWorkerEvent,mergeActivity,progressMarkup,changesMarkup,navigator,window:dom.window,document:dom.window.document,localStorage:dom.window.localStorage,console,Option:dom.window.Option,setTimeout,clearTimeout,setInterval:()=>0,clearInterval:noop,nativeInvoke:invoke,isTauri:()=>true,listen:async()=>noop,appendModelOptions,fillReasoningSelect,readTarget,resolvedModel,modelForTarget,markdown:s=>s,diffMarkup:s=>s,statusLabel:s=>s,planPreview:()=>'',setupMcpSettings:()=>noop,setupChatGptUsage:noop,setupBrowserWorkspace:()=>{const bar=dom.window.document.createElement('section');bar.id='workflow-bar';dom.window.document.body.append(bar);return {showBrowser:noop,showWorkspace:noop,isBrowserOpen:()=>browserOpen,setSidebarHidden:noop,setWorkflow:(html,action)=>{bar.innerHTML=html;bar.querySelectorAll('[data-flow-action]').forEach(b=>b.onclick=()=>action(b.dataset.flowAction));}}},setupMemory:noop,memoryPanel:()=>'',bindMemory:noop,setupAutoRouting:noop,showAutoPreview:preview=>calls.push({command:'showAutoPreview',args:preview}),openAutoSettings:noop,setupComposer:()=>({syncContext:noop,getMentions:()=>[],setMentions:noop,clear:noop,refreshThread:async()=>{},seedHistory:async()=>{},remember:async()=>{},beforeSend:async()=>true})};
 for(const name of ['HTMLElement','HTMLButtonElement','HTMLSelectElement','HTMLInputElement','HTMLTextAreaElement','HTMLDialogElement','HTMLDetailsElement','HTMLAnchorElement','HTMLOptionElement'])context[name]=dom.window[name];
 const api=await vm.runInNewContext(`(async()=>{${js}\nawait refreshModels();return {render,newTask,choosePhase,send,sendAutonomous,selectThread,refreshWorkflow,refreshAutonomous,refreshDiff,refreshThreads,applyEvent,selectedView,activeId:()=>activeThread};})()`,context);
 return {dom,calls,clipboard,state,api,select:dom.window.document.querySelector('#preference'),input:dom.window.document.querySelector('#task-input'),document:dom.window.document,close:()=>dom.window.close()};
}
test('startup never reads clipboard or invokes the retired browser transport',async()=>{
 const f=await fixture();try{assert.ok(!f.calls.some(c=>/chatgpt|workflow_run|manifest_import/.test(c.command)));assert.equal(f.document.querySelector('#chatgpt-dialog'),null);}finally{f.close();}
});
test('explicit paste button imports a Manifest once and tracks worker state',async()=>{
 const f=await fixture();try{f.api.choosePhase('autonomous');await f.api.sendAutonomous();const calls=f.calls.filter(c=>c.command==='manifest_import_clipboard');assert.equal(calls.length,1);assert.equal(calls[0].args.projectId,'project');assert.equal(f.api.selectedView().running,true);assert.equal(f.input.hidden,true);}finally{f.close();}
});
test('legacy records are readable but cannot be resent by changing composer mode',async()=>{
 const f=await fixture({existing:true});try{await f.api.selectThread('root-session');f.api.choosePhase('autonomous');await f.api.sendAutonomous();assert.ok(f.calls.some(c=>c.command==='workflow_snapshot'));assert.ok(!f.calls.some(c=>/manifest_import|workflow_run|send_turn|chatgpt_send/.test(c.command)));assert.match(f.document.querySelector('#error').textContent,/閲覧専用/);}finally{f.close();}
});

test('duplicate paste opens the interrupted task with resume visible and no new execution',async()=>{
 const f=await fixture({duplicate:true});try{f.api.choosePhase('autonomous');await f.api.sendAutonomous();assert.equal(f.api.activeId(),'root-session');assert.equal(f.api.selectedView().running,false);assert.equal(f.api.selectedView().needsResume,true);assert.ok(f.document.querySelector('[data-flow-action="resume"]'));assert.match(f.document.querySelector('#notice').textContent,/保存済みタスクを開きました/);assert.ok(!f.calls.some(c=>/autonomous_resume|send_turn/.test(c.command)));}finally{f.close();}
});

test('paste immediately signals progress, prevents duplicate clicks and confirms review completion',async()=>{
 let release;const gate=new Promise(resolve=>{release=resolve;});
 const f=await fixture({importStatus:'completed',importGate:gate});try{
  f.api.choosePhase('autonomous');const pending=f.api.sendAutonomous();
  assert.match(f.document.querySelector('[data-flow-action="import"]').textContent,/取り込み中/);
  assert.equal(f.document.querySelector('[data-flow-action="import"]').disabled,true);
  assert.match(f.document.querySelector('#notice').textContent,/取り込み中/);
  await f.api.sendAutonomous();assert.equal(f.calls.filter(c=>c.command==='manifest_import_clipboard').length,1);
  release();await pending;
  assert.match(f.document.querySelector('#notice').textContent,/レビュー合格・タスク完了/);
  assert.equal(f.document.querySelector('#status').textContent,'completed');
  assert.equal(f.document.querySelector('#send').getAttribute('aria-busy'),'false');
 }finally{release();f.close();}
});
test('failed import replaces progress with error and enables retry',async()=>{
 const f=await fixture({importError:'ManifestのJSON形式が不正です'});try{
  f.api.choosePhase('autonomous');await f.api.sendAutonomous();
  assert.match(f.document.querySelector('#error').textContent,/JSON形式が不正/);
  assert.equal(f.document.querySelector('#notice').hidden,true);
  assert.equal(f.document.querySelector('[data-flow-action="import"]').disabled,false);
  assert.equal(f.document.querySelector('[data-flow-action="import"]').textContent,'計画を取り込んで実行');
 }finally{f.close();}
});

test('worker progress survives snapshots, streams child events and reads actual worktree diffs',async()=>{
 const f=await fixture();try{
  f.state.steps=[{step:{key:'one',title:'実装担当',level:2,owned_paths:['new.txt']},status:'running',target:{model:'worker',reasoning:'high'},child_id:'child',verification:[]}];
  f.state.children=[{id:'child',provider_thread:{id:'provider-child'},turn:{id:'turn'}}];
  f.state.activity={workers:[{child_id:'child',events:[{sequence:1,event:{thread_id:'provider-child',turn_id:'turn',item_id:'cmd',kind:'item_started',text:'ファイルを確認',details:{type:'command',command:'cat new.txt'}}}]}],changes:[{key:'one',title:'実装担当',path:'/fixture/worktree',diff:'diff --git a/new.txt b/new.txt\n+++ b/new.txt\n+hello',error:null}]};
  f.api.choosePhase('autonomous');await f.api.sendAutonomous();
  assert.match(f.document.querySelector('#content').textContent,/ファイルを確認/);
  f.api.applyEvent({sequence:2,event:{thread_id:'provider-child',turn_id:'turn',item_id:'message',kind:'message_delta',text:'これから変更します'}});
  assert.match(f.document.querySelector('#content').textContent,/これから変更します/);
  await f.api.refreshAutonomous('root-session');
  assert.match(f.document.querySelector('#content').textContent,/これから変更します/);
  f.document.querySelector('[data-tab="Diff"]').click();await new Promise(r=>setTimeout(r,20));
  assert.match(f.document.querySelector('#content').textContent,/new.txt/);
  assert.ok(f.calls.some(c=>c.command==='autonomous_activity'&&c.args.includeChanges));
  assert.ok(!f.calls.some(c=>c.command==='repo_diff'));
  f.state.activityError='一時的な読み取り失敗';await f.api.refreshAutonomous('root-session');
  assert.equal(f.api.selectedView().running,true);
  assert.match(f.api.selectedView().progress.error,/一時的/);
  delete f.state.activityError;await f.api.refreshAutonomous('root-session');assert.equal(f.api.selectedView().progress.error,undefined);
 }finally{f.close();}
});

test('task purpose stays outside the scroller and logs keep closed state and output position',async()=>{
 const f=await fixture({importStatus:'awaiting_review'});try{
  f.state.snapshot={artifact_version:'version-123',manifest:{manifest_id:'manifest-123',request:'目的を忘れずに修正する'},review:{manifest_id:'review-123',verdict:'inconclusive'},messages:[{role:'user',text:'目的を忘れずに修正する'},{role:'assistant',text:'{"kind":"review","manifest_id":"review-123","private_payload":"long-data"}'}]};
  f.state.steps=[{step:{key:'one',title:'実装担当',level:2},status:'completed',target:{model:'worker'},child_id:'child',verification:[]}];
  f.state.activity={workers:[{child_id:'child',events:[{sequence:1,event:{kind:'item_completed',text:'long output',details:{type:'command',command:'cat file',exit_code:0}}}]}],changes:[]};
  f.api.choosePhase('autonomous');await f.api.sendAutonomous();
  const focus=f.document.querySelector('#workflow-bar');assert.equal(f.document.querySelector('#content').contains(focus),false);
  assert.ok(focus.querySelector('[data-flow-action="review"]'));assert.equal(f.document.querySelector('#overview-ids').open,false);
  assert.ok(!focus.textContent.includes('private_payload'));
  f.document.querySelector('[data-tab="Terminal"]').click();
  f.document.querySelector('#worker-log-one').open=false;
  await f.api.refreshAutonomous('root-session');assert.equal(f.document.querySelector('#worker-log-one').open,false);
  const event=f.document.querySelector('#one-event-1');event.open=true;
  event.querySelector('pre').scrollTop=45;
  await f.api.refreshAutonomous('root-session');
  assert.equal(f.document.querySelector('#one-event-1').open,true);
  assert.equal(f.document.querySelector('#one-event-1 pre').scrollTop,45);
  f.document.querySelector('[data-tab="Chat"]').click();f.document.querySelector('[data-tab="Terminal"]').click();
  assert.equal(f.document.querySelector('#worker-log-one').open,false);
  assert.equal(f.document.querySelector('#one-event-1 pre').scrollTop,45);
  const prompt=reviewRequest(f.api.selectedView().focus);assert.match(prompt,/Task ID: root-session/);assert.match(prompt,/Artifact version: version-123/);
 }finally{f.close();}
});

test('ChatGPT planning starts in its pane and the Localoud composer stays hidden',async()=>{
 const f=await fixture();try{
  f.api.choosePhase('autonomous');
  assert.equal(f.document.querySelector('main>footer').hidden,true);
  assert.match(f.document.querySelector('#content').textContent,/左のChatGPT/);
  assert.ok(f.document.querySelector('[data-flow-action="import"]'));
  await f.api.send();assert.equal(f.clipboard.length,0);
  assert.ok(!f.calls.some(c=>/manifest_import|chatgpt_send|send_turn/.test(c.command)));
 }finally{f.close();}
});
test('every Manifest tab is read-only and shows task-specific plan, context and usage',async()=>{
 const f=await fixture({importStatus:'awaiting_review'});try{
  f.state.steps=[{step:{key:'one',title:'検索を改善',goal:'入力を保持する',level:2,owned_paths:['search.ts'],dependencies:[],acceptance:['入力が残る']},status:'completed',target:{model:'worker'},capsule:JSON.stringify({task:{goal:'入力を保持する'},instructions:'他の作業には変更しない'}),usage:[{usage:{prompt_tokens:17,completion_tokens:9}}],verification:[]}];
  f.state.snapshot={artifact_version:'rev',manifest:{manifest_id:'m1',request:'検索画面',acceptance:['受け入れ条件']}};
  await f.api.sendAutonomous();
  for(const tab of ['Chat','Plan','Diff','Agents','Terminal','Context','Usage']){
   f.document.querySelector(`[data-tab="${tab}"]`).click();await new Promise(r=>setTimeout(r,10));
   assert.equal(f.document.querySelectorAll('#content button,#content textarea,#content select,#content input').length,0,tab+' has an operation');
   if(tab==='Plan'){assert.match(f.document.querySelector('#content').textContent,/入力を保持する/);assert.match(f.document.querySelector('#content').textContent,/受け入れ条件/);}
   if(tab==='Context')assert.match(f.document.querySelector('#content').textContent,/他の作業には変更しない/);
   if(tab==='Usage')assert.match(f.document.querySelector('#content').textContent,/17/);
  }
  assert.ok(!f.calls.some(c=>['task_graph','inspect_context','memory_candidates','model_usage'].includes(c.command)));
  assert.equal(f.document.querySelector('main>footer').hidden,true);
  assert.equal(f.document.querySelectorAll('[data-flow-action="review"]').length,1);
 }finally{f.close();}
});
test('project tree nests sessions under their owner and search can reveal another project',async()=>{
 const f=await fixture({multipleProjects:true});try{
  f.state.threads.push({id:'other-session',project_id:'other',provider:'autonomous',title:'別プロジェクトの作業',status:'completed'});
  await f.api.refreshThreads();
  const other=f.document.querySelector('[data-project-group="other"]');assert.ok(other);
  other.querySelector('[data-project-toggle]').click();
  assert.equal(f.document.querySelector('[data-thread="other-session"]').closest('[data-project-group]').dataset.projectGroup,'other');
  f.document.querySelector('[data-thread="other-session"]').click();await new Promise(r=>setTimeout(r,20));
  assert.equal(f.api.activeId(),'other-session');
  assert.match(f.document.querySelector('#project-title').textContent,/別プロジェクト/);
  assert.match(f.document.querySelector('#workflow-bar').textContent,/別プロジェクトの作業/);
  const filter=f.document.querySelector('#task-filter');filter.value='別プロジェクトの作業';filter.dispatchEvent(new f.dom.window.Event('input'));
  assert.equal(f.document.querySelectorAll('[data-project-group]').length,1);
  assert.equal(f.document.querySelector('[data-project-group]').dataset.projectGroup,'other');
 }finally{f.close();}
});

test('review handoff copies the exact target and waits for explicit result import',async()=>{
 const f=await fixture({importStatus:'awaiting_review'});try{
  f.state.snapshot={artifact_version:'version-one',manifest:{manifest_id:'m',request:'検索画面を改善'}};
  await f.api.sendAutonomous();const before=f.calls.filter(c=>c.command==='manifest_import_clipboard').length;
  f.document.querySelector('[data-flow-action="review"]').click();await new Promise(r=>setTimeout(r,10));
  assert.match(f.clipboard.at(-1),/Task ID: root-session/);assert.match(f.clipboard.at(-1),/Artifact version: version-one/);
  assert.equal(f.calls.filter(c=>c.command==='manifest_import_clipboard').length,before);
  assert.equal(f.document.querySelector('[aria-current="step"]').textContent,'4レビュー');
  assert.match(f.document.querySelector('[data-flow-action="review"]').textContent,/再コピー/);
  assert.equal(f.document.querySelector('.flow-actions button').dataset.flowAction,'import');
  f.state.snapshot.artifact_version='version-two';await f.api.refreshAutonomous('root-session');
  assert.equal(f.document.querySelector('[data-flow-action="review"]').textContent,'レビュー依頼をコピー');
  assert.equal(f.document.querySelector('.flow-actions button').dataset.flowAction,'review');
  assert.ok(!f.calls.some(c=>/chatgpt_send|send_turn/.test(c.command)));
 }finally{f.close();}
});

test('direct tasks send, accept steering and keep direct mode on the next task',async()=>{
 const f=await fixture();try{
  f.document.querySelector('[data-operation="implement"]').click();f.select.value='codex:codex-implementation';f.select.dispatchEvent(new f.dom.window.Event('change'));
  f.input.value='名前の表示を直して';await f.api.send();
  assert.equal(f.calls.filter(c=>c.command==='create_routed_task').length,1);
  assert.ok(f.calls.some(c=>c.command==='send_composed_turn'));
  assert.equal(f.document.querySelector('main>footer').hidden,false);
  f.api.applyEvent({sequence:1,event:{thread_id:'provider-direct',turn_id:'t1',kind:'turn_started',text:'running'}});
  assert.equal(f.document.querySelector('#send').hidden,true);
  const stop=f.document.querySelector('#stop');assert.equal(stop.hidden,false);assert.equal(stop.disabled,false);
  stop.click();assert.equal(stop.disabled,true);await new Promise(r=>setTimeout(r,10));
  assert.equal(f.calls.filter(c=>c.command==='interrupt_turn').length,1);assert.equal(f.calls.find(c=>c.command==='interrupt_turn').args.threadId,'direct-task');
  f.input.value='テストも確認して';await f.api.send();assert.ok(f.calls.some(c=>c.command==='steer_turn'));
  f.api.applyEvent({sequence:2,event:{thread_id:'provider-direct',turn_id:'t1',kind:'turn_completed',text:'completed'}});
  assert.equal(f.document.querySelector('[aria-current="step"]').textContent,'3完了');
  assert.equal(f.document.querySelector('#send').hidden,false);assert.equal(f.document.querySelector('#stop').hidden,true);
  f.api.newTask();assert.equal(f.document.querySelector('#operation [aria-pressed="true"]').dataset.operation,'implement');
  assert.equal(f.document.querySelector('main>footer').hidden,false);
  assert.ok(!f.calls.some(c=>/manifest_import|chatgpt_send/.test(c.command)));
 }finally{f.close();}
});

test('a hidden ChatGPT pane cannot strand a saved planning draft without an editor',async()=>{
 const f=await fixture({browserOpen:false,saved:JSON.stringify({project:'project',drafts:{'project:new':{text:'Keep my draft',mode:'autonomous'}}})});try{
  assert.equal(f.document.querySelector('#operation [aria-pressed="true"]').dataset.operation,'implement');
  assert.equal(f.document.querySelector('main>footer').hidden,false);
  assert.equal(f.document.querySelector('[aria-current="step"]').textContent,'1依頼');
  assert.equal(f.select.value,'auto');
 }finally{f.close();}
});

test('project menu follows clicked project, pins it, and adjacent plus starts there',async()=>{
 const f=await fixture({multipleProjects:true});try{
  f.document.querySelector('[data-project-menu="other"]').click();
  const menu=f.document.querySelector('#project-context-menu');
  assert.ok(menu.open);assert.match(menu.textContent,/ピン留め.*編集.*セッション.*Finder で表示.*永続的な Worktree.*チャットをアーカイブ.*プロジェクトを削除/s);
  menu.querySelector('[data-action="reveal"]').click();await new Promise(r=>setTimeout(r,5));
  assert.ok(f.calls.some(c=>c.command==='manage_project'&&c.args.projectId==='other'&&c.args.action==='reveal'));
  f.document.querySelector('[data-project-menu="other"]').click();f.document.querySelector('[data-action="pin"]').click();
  assert.equal(f.document.querySelector('[data-project-group]').dataset.projectGroup,'other');
  f.document.querySelector('[data-project-new="other"]').click();assert.equal(f.document.querySelector('#project-title').textContent,'別プロジェクト');assert.equal(f.api.activeId(),null);
 }finally{f.close();}
});
test('usage tab requests only selected session and settings requests global history',async()=>{
 const f=await fixture();try{
  f.state.threads.push({id:'direct',project_id:'project',provider:'codex',provider_thread_id:'p',title:'Direct',status:'completed'});
  await f.api.selectThread('direct');f.document.querySelector('[data-tab="Usage"]').click();await new Promise(r=>setTimeout(r,5));
  assert.match(f.document.querySelector('#content').textContent,/session-model/);assert.doesNotMatch(f.document.querySelector('#content').textContent,/global-model/);
  assert.ok(f.calls.some(c=>c.command==='model_usage'&&c.args.threadId==='direct'));
  f.document.querySelector('#settings').click();await new Promise(r=>setTimeout(r,5));assert.match(f.document.querySelector('#total-usage').textContent,/global-model/);
 }finally{f.close();}
});

test('new request defaults to automatic routing even while the browser is open',async()=>{
 const f=await fixture();try{
  assert.equal(f.document.querySelector('#task-mode').value,'implement');assert.equal(f.select.value,'auto');assert.equal(f.input.hidden,false);
  assert.equal(f.document.querySelector('#operation').closest('details').id,'compose-more');
  f.input.value='修正をお願いします';await f.api.send();
  assert.equal(f.calls.filter(c=>c.command==='preview_auto_route').length,1);
  assert.equal(f.calls.find(c=>c.command==='create_routed_task').args.request.autoRouteId,'route-1');
  assert.equal(f.calls.find(c=>c.command==='send_composed_turn').args.options.mode,'default');
 }finally{f.close();}
});
test('planning decision starts configured planner in plan mode without model selection',async()=>{
 const f=await fixture({autoPreview:{id:'plan-1',revision:0,needs_plan:true,blocked:'設計の検討が必要',target:null,confirm_before_run:false}});try{
  f.input.value='認証の設計を変更したい';await f.api.send();
  assert.ok(!f.calls.some(c=>c.command==='showAutoPreview'));
  const request=f.calls.find(c=>c.command==='create_routed_task').args.request;
  assert.equal(request.model,'gpt-6-astra');assert.equal(request.reasoning,'high');assert.equal(request.autoRouteId,null);
  assert.equal(f.calls.find(c=>c.command==='send_composed_turn').args.options.mode,'plan');
  assert.equal(f.document.querySelector('#task-mode').value,'codex-plan');
  f.api.selectedView().messages.push({role:'assistant',text:'計画です',key:'plan'});f.api.render();
  const start=f.document.querySelector('#implement-plan');assert.equal(start.hidden,false);start.click();await new Promise(r=>setTimeout(r,10));
  assert.equal(f.calls.filter(c=>c.command==='create_routed_task').length,1);assert.equal(f.calls.filter(c=>c.command==='send_composed_turn').at(-1).args.options.mode,'default');
 }finally{f.close();}
});
test('confirmation preference and unavailable planner preserve the unsent request',async()=>{
 for(const confirm of [true,false]){
 const f=await fixture({autoPreview:{id:'plan-1',revision:0,needs_plan:true,blocked:'計画が必要',target:null,confirm_before_run:confirm},planTarget:{provider:'local',model:'local'}});try{
  f.input.value='大きな設計変更';await f.api.send();assert.equal(f.input.value,'大きな設計変更');assert.ok(!f.calls.some(c=>c.command==='create_routed_task'));
  if(confirm)assert.ok(f.calls.some(c=>c.command==='showAutoPreview'));else assert.match(f.document.querySelector('#error').textContent,/未設定/);
 }finally{f.close();}}
});

test('uncertain requests investigate with the selected fallback before implementation',async()=>{
 const f=await fixture({autoPreview:{id:'uncertain',revision:0,level:null,target:{provider:'codex',model:'codex-implementation'},needs_plan:false,blocked:null,used_fallback:true,confirm_before_run:false}});try{
  f.input.value='なぜか動かないので調べて';await f.api.send();
  const request=f.calls.find(c=>c.command==='create_routed_task').args.request;
  assert.equal(request.preference,'auto');assert.equal(request.autoRouteId,'uncertain');
  assert.equal(f.calls.find(c=>c.command==='send_composed_turn').args.options.mode,'plan');
  assert.ok(!f.calls.some(c=>c.command==='auto_settings'));assert.match(f.api.selectedView().route,/調査から開始/);
 }finally{f.close();}
});
test('a saved explicit model remains explicit and routing failure preserves the draft',async()=>{
 const f=await fixture({routeError:'接続できません'});try{
  f.select.value='codex:codex-implementation';f.input.value='指定したモデルで修正';await f.api.send();
  assert.ok(!f.calls.some(c=>c.command==='preview_auto_route'));assert.equal(f.calls.find(c=>c.command==='create_routed_task').args.request.model,'codex-implementation');
  assert.equal(f.input.value,'指定したモデルで修正');assert.equal(f.api.activeId(),null);
 }finally{f.close();}
});

test('archived session history cannot send until explicitly restored',async()=>{
 const f=await fixture();try{
  f.select.value='codex:codex-implementation';f.input.value='履歴を確認';await f.api.send();
  f.state.archived=['direct-task'];await f.api.refreshThreads();await f.api.selectThread('direct-task');
  assert.equal(f.document.querySelector('main>footer').hidden,true);
  const count=f.calls.filter(c=>c.command==='send_composed_turn').length;
  f.input.value='送信しない';await f.api.send();
  assert.equal(f.calls.filter(c=>c.command==='send_composed_turn').length,count);assert.match(f.document.querySelector('#error').textContent,/アーカイブを解除/);
 }finally{f.close();}
});

 test('explicit import shortcut selects overview and ignores repeats during execution',async()=>{
 const f=await fixture();try{
  f.api.choosePhase('autonomous');
  const key=()=>f.document.dispatchEvent(new f.dom.window.KeyboardEvent('keydown',{key:'V',metaKey:true,shiftKey:true,bubbles:true,cancelable:true}));
  key();await new Promise(r=>setTimeout(r,20));
  assert.equal(f.calls.filter(c=>c.command==='manifest_import_clipboard').length,1);
  assert.equal(f.api.activeId(),'root-session');
  assert.match(f.document.querySelector('#content').textContent,/今回の依頼/);
  key();await new Promise(r=>setTimeout(r,10));
  assert.equal(f.calls.filter(c=>c.command==='manifest_import_clipboard').length,1);
 }finally{f.close();}
 });
