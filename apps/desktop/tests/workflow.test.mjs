import {tabLabels,flowMarkup,welcomeMarkup,executionOverview,executionPlan,executionContext,executionUsage} from '../src/workflow-ui.ts';
import {focusMarkup,planningRequest,reviewRequest,recordMarkup} from '../src/task-focus.ts';
import {syncWorkers,applyWorkerEvent,mergeActivity,progressMarkup,changesMarkup} from '../src/worker-progress.ts';
import {taskGraph,bindTaskGraphs} from '../src/task-graph.ts';
import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import {JSDOM} from 'jsdom';
import ts from 'typescript';
import {appendModelOptions,fillReasoningSelect,profileForChoice,readTarget,resolvedModel,modelForTarget} from '../src/model-controls.ts';
const source=(await readFile(new URL('../src/main.ts',import.meta.url),'utf8')).replace(/^import .*;\s*$/gm,'');
const js=ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ES2022}}).outputText.replace(/export\s*\{\s*\};?/g,'');
const reviewJs=ts.transpileModule((await readFile(new URL('../src/manifest-review.ts',import.meta.url),'utf8')).replace(/^import .*;\s*$/gm,'').replace('export function','function'),{compilerOptions:{target:ts.ScriptTarget.ES2022}}).outputText;
async function fixture({saved,existing=false,connected=true,duplicate=false,importStatus,importGate,importError,multipleProjects=false,browserOpen=true,autoPreview,planTarget,routeError,startupError,profiles=[],apiModels=[]}={}){
 const dom=new JSDOM('<div id="app"></div>',{url:'https://workflow.fixture'});
 if(saved)dom.window.localStorage.setItem('astra-ui-v1',saved);
 dom.window.HTMLDialogElement.prototype.showModal=function(){this.setAttribute('open','');};dom.window.HTMLDialogElement.prototype.close=function(){this.removeAttribute('open');this.dispatchEvent(new dom.window.Event('close'));};
 const calls=[];const clipboard=[];const navigator={clipboard:{writeText:async text=>{if(state.copyError)throw Error(state.copyError);clipboard.push(text);}}};
 const project={id:'project',name:'Fixture',root:'/fixture'};
 const root={id:'root-session',project_id:project.id,provider:'workflow',provider_thread_id:'workflow:root',title:'入力を保持する',status:'idle'};
 const state={projects:multipleProjects?[project,{id:'other',name:'別プロジェクト',root:'/other'}]:[project],threads:existing?[root]:[],providerProfiles:profiles.map(profile=>({...profile})),running:false,failReview:false,messages:existing?[{role:'user',text:'保存済みの依頼',key:'old-user'},{role:'assistant',text:'保存済みのプラン',key:'old-plan',label:'プラン · ChatGPT'}]:[],targets:existing?{plan:{model:'gpt-6-astra',reasoning:'high',chatgpt:true},review:{model:'gpt-6-astra',reasoning:'high',chatgpt:true},implement:{model:'codex-implementation',reasoning:'high',chatgpt:false}}:{},phase:'plan',model:'gpt-6-astra'};
 if(startupError)state.fail={projects:startupError};
 const invoke=async(command,args)=>{
  calls.push({command,args});
  if(state.gates?.[command])await state.gates[command];
  if(state.fail?.[command])throw Error(state.fail[command]);
  if(state.responses&&Object.hasOwn(state.responses,command))return state.responses[command];
  switch(command){
   case 'available_models':return {models:[{key:'local:fixture',label:'Local fixture',model:'local-fixture',local:true,reasoning:[],default_reasoning:null},{key:'codex:gpt-6-astra',label:'Astra on Codex',model:'gpt-6-astra',local:false,reasoning:['high'],default_reasoning:null},{key:'codex:codex-implementation',label:'Implementation',model:'codex-implementation',local:false,reasoning:['high','low'],default_reasoning:null},...apiModels],warnings:[]};
   case 'preview_auto_route':return autoPreview||{id:'route-1',revision:0,target:{provider:'codex',model:'codex-implementation',reasoning:'high'},reason:'routine',needs_plan:false,blocked:null,confirm_before_run:false};
   case 'auto_settings':return {levels:[{level:5,target:planTarget||{provider:'codex',model:'gpt-6-astra',reasoning:'high'}}]};
   case 'create_routed_task':{if(routeError)throw Error(routeError);const thread={id:'direct-task',project_id:project.id,provider:'codex',provider_thread_id:'provider-direct',title:args.request.text,status:'idle'};state.threads.push(thread);return {thread,target:{model:'codex-implementation',reasoning:'high'},route:{decision:{executor:'codex',reason:'explicit'}}};}
   case 'interrupt_turn':case 'send_composed_turn':case 'send_turn':case 'steer_turn':return;
   case 'resume_task':return {active_turn:null,messages:[]};
   case 'event_history':return [];
   case 'model_usage':return args?.threadId?[{model:'session-model',prompt_tokens:1,latency_ms:1000}]:[{model:'global-model',prompt_tokens:99,latency_ms:2000}];
   case 'manage_project':return null;
   case 'worker_insights':return {review:null,recovery:null};
   case 'repo_diff':return '';
   case 'inspect_context':return null;
   case 'memory_candidates':return {candidates:[]};
   case 'local_model_settings':return {display_name:'Local fixture',model_id:'fixture',endpoint:'http://127.0.0.1:9999',quantization_bits:4};
   case 'codex_binary':return '/usr/local/bin/codex';
   case 'astra_settings':return {mode:'disabled',model:'gpt-6-astra',reasoning:null};
   case 'set_astra_settings':return null;
   case 'projects':return state.projects||[project];
   case 'threads':return state.threads;
   case 'sync_archived_sessions':return [];
   case 'archived_threads':return state.archived||[];
   case 'thread_models':return state.threads.some(t=>t.provider==='codex')?{'direct-task':'codex-implementation'}:{};
   case 'thread_reasoning':return {};
   case 'thread_model_targets':return state.threads.some(t=>t.provider==='codex')?{'direct-task':{provider:'codex',profile_id:'codex',model:'codex-implementation',reasoning:null}}:{};
   case 'provider_profiles':return state.providerProfiles;
   case 'set_provider_profile':{const index=state.providerProfiles.findIndex(profile=>profile.id===args.profile.id);const saved={...args.profile,credential_present:false};if(index<0)state.providerProfiles.push(saved);else state.providerProfiles[index]=saved;return apiModels.filter(model=>model.profile_id===saved.id).map(model=>({id:model.model}));}
   case 'run_provider_tool_canary':return {supported:true,profile_id:args.profileId,model_id:args.modelId};
   case 'set_thread_model_target':return null;
   case 'workflow_snapshot':return {status:'legacy_read_only',running:state.running,messages:state.messages,phase:state.phase,model:state.model,targets:state.targets,handoff:state.phase==='review'?'要件・プラン・実装結果・作業差分':'これまでの会話'};
   case 'workflow_stop':state.running=false;return;
   case 'workflow_role_settings':return {plan:{model:'planner',reasoning:'medium'},review:{model:'reviewer',reasoning:'pro'}};
   case 'manifest_preview_clipboard':return state.preview||{kind:'task',project_id:'project',request:'確認する依頼',scope:['src/example.ts'],acceptance:['検証が通る'],steps:[{title:'変更する',goal:'内容を修正する',owned_paths:['src/example.ts'],acceptance:['検証が通る']}]};
   case 'manifest_import_text':if(importGate)await importGate;if(importError)throw Error(importError);state.threads=[{...root,provider:'autonomous',status:importStatus||(duplicate?'interrupted':'queued')}];state.running=!duplicate;return {thread:state.threads[0],imported:!duplicate};
   case 'autonomous_snapshot':return {thread:state.threads.find(t=>t.id===args.threadId)||state.threads[0],messages:[{role:'user',text:'依頼だけ'}],steps:state.steps||[],children:state.children||[],final_diff:null,artifact:null,...state.snapshot};
   case 'autonomous_activity':if(state.activityError)throw Error(state.activityError);return state.activity||{workers:[],changes:[]};
   case 'autonomous_stop':state.running=false;state.threads[0].status='interrupted';return;
   default:throw Error('Unexpected command: '+command);
  }
 };
 const noop=()=>{};
 const context={tabLabels,flowMarkup,welcomeMarkup,executionOverview,executionPlan,executionContext,executionUsage,focusMarkup,planningRequest,reviewRequest,recordMarkup,syncWorkers,applyWorkerEvent,mergeActivity,progressMarkup,changesMarkup,taskGraph,bindTaskGraphs,navigator,window:dom.window,document:dom.window.document,localStorage:dom.window.localStorage,console,Option:dom.window.Option,setTimeout,clearTimeout,setInterval:()=>0,clearInterval:noop,nativeInvoke:invoke,isTauri:()=>true,listen:async()=>noop,appendModelOptions,fillReasoningSelect,profileForChoice,readTarget,resolvedModel,modelForTarget,markdown:s=>s,diffMarkup:s=>s,statusLabel:s=>s,planPreview:()=>'',setupSettingsNavigation:()=>({mcp:dom.window.document.createElement('div'),select:noop,setMcpLoader:noop}),setupManifestReview:()=>noop,setupMcpSettings:()=>noop,setupChatGptUsage:noop,setupBrowserWorkspace:()=>{const bar=dom.window.document.createElement('section');bar.id='workflow-bar';dom.window.document.body.append(bar);return {showBrowser:noop,showWorkspace:noop,showCopiedRequestGuide:noop,isBrowserOpen:()=>browserOpen,setSidebarHidden:noop,setWorkflow:(html,action)=>{bar.innerHTML=html;bar.querySelectorAll('[data-flow-action]').forEach(b=>b.onclick=()=>action(b.dataset.flowAction));}}},setupMemory:noop,memoryPanel:()=>'',bindMemory:noop,setupAutoRouting:noop,showAutoPreview:preview=>calls.push({command:'showAutoPreview',args:preview}),openAutoSettings:noop,setupComposer:()=>({syncContext:noop,getMentions:()=>[],setMentions:noop,clear:noop,refreshThread:async()=>{},seedHistory:async()=>{},remember:async()=>{},beforeSend:async()=>true})};
 for(const name of ['HTMLElement','HTMLButtonElement','HTMLSelectElement','HTMLInputElement','HTMLTextAreaElement','HTMLDialogElement','HTMLDetailsElement','HTMLAnchorElement','HTMLOptionElement','URL'])context[name]=dom.window[name];
 context.setupManifestReview=vm.runInNewContext(reviewJs+';setupManifestReview',context);
 const api=await vm.runInNewContext(`(async()=>{${js}\nawait refreshModels();return {render,newTask,choosePhase,send,sendAutonomous,selectThread,refreshWorkflow,refreshAutonomous,refreshDiff,refreshUsage,refreshContext,refreshThreads,applyEvent,selectedView,activeId:()=>activeThread};})()`,context);
 return {dom,calls,clipboard,state,api,select:dom.window.document.querySelector('#preference'),input:dom.window.document.querySelector('#task-input'),document:dom.window.document,close:()=>dom.window.close()};
}
test('failed startup exposes a read-only retry without repeating archive synchronization',async()=>{
 const f=await fixture({startupError:'接続エラー'});try{
  assert.match(f.document.querySelector('#error').textContent,/一覧を読み込めません/);
  delete f.state.fail.projects;f.document.querySelector('#retry-error').click();await new Promise(r=>setTimeout(r,0));
  assert.equal(f.document.querySelector('#project-title').textContent,'Fixture');
  assert.equal(f.calls.filter(c=>c.command==='sync_archived_sessions').length,1);
 }finally{f.close();}
});
test('typing and clearing a direct request updates send availability immediately',async()=>{
 const f=await fixture();try{
  const send=f.document.querySelector('#send');assert.equal(send.disabled,true);
  f.input.value='検索を改善する';f.input.dispatchEvent(new f.dom.window.Event('input',{bubbles:true}));assert.equal(send.disabled,false);
  f.input.value='  ';f.input.dispatchEvent(new f.dom.window.Event('input',{bubbles:true}));assert.equal(send.disabled,true);
 }finally{f.close();}
});
test('manual tab navigation keeps focus in the tab rail until activation',async()=>{
 const f=await fixture();try{
  assert.equal(f.document.querySelector('#app').firstElementChild.id,'skip-to-content');
  const first=f.document.querySelector('[data-tab="Chat"]');first.focus();
  for(let i=0;i<2;i++)f.document.activeElement.dispatchEvent(new f.dom.window.KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true,cancelable:true}));
  assert.equal(f.document.activeElement.dataset.tab,'Diff');
  assert.equal(first.getAttribute('aria-selected'),'true');
  f.document.activeElement.click();
  assert.equal(f.document.activeElement.dataset.tab,'Diff');
  assert.equal(f.document.querySelector('#content').getAttribute('aria-labelledby'),'tab-Diff');
  assert.equal(f.document.querySelector('#content').hasAttribute('aria-live'),false);
 }finally{f.close();}
});
async function directFixture(){const f=await fixture();f.state.threads=[{id:'direct-task',project_id:'project',provider:'codex',provider_thread_id:'provider-direct',title:'検証する',status:'completed'}];await f.api.refreshThreads();await f.api.selectThread('direct-task');return f;}
test('a read distinguishes loading from empty and preserves previous data on failure',async()=>{
 const f=await directFixture();let release;try{
  f.state.gates={model_usage:new Promise(r=>release=r)};
  f.document.querySelector('[data-tab="Usage"]').click();
  assert.equal(f.document.querySelector('#content').getAttribute('aria-busy'),'true');
  assert.match(f.document.querySelector('#content').textContent,/読み込み中/);
  assert.doesNotMatch(f.document.querySelector('#content').textContent,/0件/);
  release();await f.api.refreshUsage();
  assert.match(f.document.querySelector('#content').textContent,/session-model/);
  f.state.fail={model_usage:'一時的な接続エラー'};await f.api.refreshUsage();
  assert.match(f.document.querySelector('#content').textContent,/前回の表示を残しています/);
  assert.match(f.document.querySelector('#content').textContent,/session-model/);
  assert.ok(f.document.querySelector('#retry-pane'));
  delete f.state.fail.model_usage;await f.api.refreshUsage();
  assert.equal(f.document.querySelector('#retry-pane'),null);
 }finally{release?.();f.close();}
});
test('a late failed read cannot replace or attach an error to a different task',async()=>{
 const f=await directFixture();let release;try{
  f.state.gates={repo_diff:new Promise(r=>release=r)};f.state.fail={repo_diff:'古いタスクの読み取り失敗'};
  f.document.querySelector('[data-tab="Diff"]').click();const pending=f.api.refreshDiff('direct-task');
  f.api.newTask();const before=f.document.querySelector('#content').textContent;
  release();await pending;
  assert.equal(f.document.querySelector('#content').textContent,before);
  assert.equal(f.document.querySelector('#error').hidden,true);
 }finally{release?.();f.close();}
});
test('saved execution plans expose a scoped retry when snapshot refresh fails',async()=>{
 const f=await fixture();try{
  f.state.steps=[{step:{key:'check',title:'検証用の保存済み工程',level:2},status:'completed',target:{model:'fixture'},verification:[]}];
  await f.api.sendAutonomous('fixture-only-plan','project');f.state.fail={autonomous_snapshot:'一時的な読み取りエラー'};
  f.document.querySelector('[data-tab="Plan"]').click();await new Promise(r=>setTimeout(r,0));
  assert.ok(f.document.querySelector('#retry-pane'));assert.match(f.document.querySelector('#content').textContent,/計画を読み込めません/);
  assert.match(f.document.querySelector('#content').textContent,/検証用の保存済み工程/);
  delete f.state.fail.autonomous_snapshot;f.document.querySelector('#retry-pane').click();await new Promise(r=>setTimeout(r,0));
  assert.equal(f.document.querySelector('#retry-pane'),null);
 }finally{f.close();}
});
test('rerender retains focus on a message action without announcing the entire transcript',async()=>{
 const f=await directFixture();try{
  f.api.selectedView().messages=[{role:'assistant',key:'reply',text:'作業結果'}];f.api.render();
  f.document.querySelector('[data-copy-message="0"]').focus();f.api.render();
  assert.equal(f.document.activeElement.dataset.copyMessage,'0');
  assert.equal(f.document.querySelector('#content').hasAttribute('aria-live'),false);
 }finally{f.close();}
});
test('project disclosure retains focus and unchanged task trees are not replaced',async()=>{
 const f=await fixture();try{
  const button=f.document.querySelector('[data-project-toggle]');button.focus();button.click();
  assert.equal(f.document.activeElement.dataset.projectToggle,'project');
  const current=f.document.activeElement;f.api.render();assert.equal(f.document.activeElement,current);
 }finally{f.close();}
});
test('provider profile settings show missing credentials and preserve revisions on edit',async()=>{
 const profile={id:'anthropic-main',name:'Anthropic Main',protocol:'anthropic_messages',base_url:'https://api.anthropic.com/v1',locality:'cloud',credential_env:'ANTHROPIC_API_KEY',max_concurrency:2,enabled:true,revision:3,credential_present:false};
 const f=await fixture({profiles:[profile],apiModels:[{key:'api:anthropic-main:claude',label:'Anthropic Main / Claude',model:'claude',profile_id:'anthropic-main',protocol:'anthropic_messages',local:false,reasoning:['high'],tools:true,default_reasoning:null,is_default:false}]});try{
  f.document.querySelector('#settings').click();await new Promise(resolve=>setTimeout(resolve,5));
  assert.match(f.document.querySelector('#provider-profile-list').textContent,/ANTHROPIC_API_KEY: 未設定/);
  f.document.querySelector('[data-provider-edit="anthropic-main"]').click();
  assert.equal(f.document.querySelector('#provider-id').readOnly,true);
  assert.equal(f.document.querySelector('#provider-revision').value,'3');
  f.document.querySelector('#provider-name').value='Anthropic Updated';
  f.document.querySelector('#provider-profile-form').dispatchEvent(new f.dom.window.Event('submit',{bubbles:true,cancelable:true}));
  await new Promise(resolve=>setTimeout(resolve,10));
  const save=f.calls.find(call=>call.command==='set_provider_profile');
  assert.equal(save.args.profile.revision,4);
  assert.equal(save.args.profile.credential_env,'ANTHROPIC_API_KEY');
  assert.match(f.document.querySelector('#provider-profile-list').textContent,/Anthropic Updated/);
 }finally{f.close();}
});

test('generic provider tool canary runs only after the explicit billed-call confirmation',async()=>{
 const profile={id:'generic',name:'Generic',protocol:'open_ai_chat',base_url:'https://generic.example/v1',locality:'cloud',credential_env:'GENERIC_API_KEY',max_concurrency:1,enabled:true,revision:1,credential_present:true};
 const model={key:'api:generic:model',label:'Generic / Model',model:'model',profile_id:'generic',protocol:'open_ai_chat',local:false,reasoning:[],tools:false,default_reasoning:null,is_default:false};
 const f=await fixture({profiles:[profile],apiModels:[model]});try{
  f.document.querySelector('#settings').click();await new Promise(resolve=>setTimeout(resolve,5));
  const button=f.document.querySelector('[data-provider-canary="generic"]');
  f.dom.window.confirm=()=>false;button.click();await new Promise(resolve=>setTimeout(resolve,0));
  assert.equal(f.calls.filter(call=>call.command==='run_provider_tool_canary').length,0);
  f.dom.window.confirm=()=>true;button.click();await new Promise(resolve=>setTimeout(resolve,10));
  const call=f.calls.find(item=>item.command==='run_provider_tool_canary');
  assert.equal(call.args.profileId,'generic');assert.equal(call.args.modelId,'model');
 }finally{f.close();}
});

test('changing an existing session model saves an exact next-turn API target without sending',async()=>{
 const profile={id:'anthropic-main',name:'Anthropic Main',protocol:'anthropic_messages',base_url:'https://api.anthropic.com/v1',locality:'cloud',credential_env:'ANTHROPIC_API_KEY',max_concurrency:1,enabled:true,revision:1,credential_present:true};
 const model={key:'api:anthropic-main:claude',label:'Anthropic Main / Claude',model:'claude',profile_id:'anthropic-main',protocol:'anthropic_messages',local:false,reasoning:['high'],tools:true,default_reasoning:null,is_default:false};
 const f=await fixture({profiles:[profile],apiModels:[model]});try{
  f.state.threads=[{id:'direct-task',project_id:'project',provider:'codex',provider_thread_id:'provider-direct',title:'Direct',status:'completed'}];
  await f.api.refreshThreads();await f.api.selectThread('direct-task');
  const sends=f.calls.filter(call=>call.command==='send_composed_turn').length;
  f.select.value=model.key;f.select.dispatchEvent(new f.dom.window.Event('change',{bubbles:true}));await new Promise(resolve=>setTimeout(resolve,10));
  const call=f.calls.find(item=>item.command==='set_thread_model_target');
  assert.deepEqual(call.args.target,{provider:'api',profile_id:'anthropic-main',model:'claude',reasoning:null});
  assert.equal(f.calls.filter(item=>item.command==='send_composed_turn').length,sends);
  assert.match(f.document.querySelector('#notice').textContent,/次のターン/);
 }finally{f.close();}
});

test('session actions are available from the selected header and each sidebar row',async()=>{
 const f=await directFixture();try{
  const rowAction=f.document.querySelector('[data-thread-menu="direct-task"]');
  assert.ok(rowAction);assert.match(rowAction.getAttribute('aria-label'),/セッション操作/);
  rowAction.click();assert.ok(f.document.querySelector('#lifecycle-dialog').hasAttribute('open'));
  f.document.querySelector('#lifecycle-close').click();
  const open=()=>f.document.querySelector('#session-actions').click();
  const remove=()=>Array.from(f.document.querySelectorAll('#lifecycle-actions button')).find(b=>b.textContent.includes('セッションを削除'));
  open();remove().click();assert.equal(f.calls.filter(c=>c.command==='manage_session').length,0);
  assert.match(f.document.querySelector('#lifecycle-description').textContent,/検証する/);
  f.document.querySelector('#lifecycle-close').click();open();
  assert.equal(remove().dataset.confirmed,undefined);remove().click();
  f.state.responses={manage_session:null};f.state.threads=[];remove().click();await new Promise(r=>setTimeout(r,0));
  assert.equal(f.calls.filter(c=>c.command==='manage_session').length,1);
  assert.equal(f.calls.find(c=>c.command==='manage_session').args.threadId,'direct-task');
  assert.equal(f.calls.find(c=>c.command==='manage_session').args.action,'delete');
 }finally{f.close();}
});
test('settings errors are linked to fields and repeated submit cannot duplicate a save',async()=>{
 const f=await fixture();let release;try{
  f.document.querySelector('#settings').click();await new Promise(r=>setTimeout(r,0));
  const form=f.document.querySelector('#settings-form'),name=f.document.querySelector('#local-name'),endpoint=f.document.querySelector('#local-endpoint');
  name.value='';endpoint.value='invalid';form.dispatchEvent(new f.dom.window.Event('submit',{bubbles:true,cancelable:true}));
  assert.equal(name.getAttribute('aria-invalid'),'true');assert.equal(endpoint.getAttribute('aria-invalid'),'true');
  assert.equal(f.document.activeElement.id,'settings-error');assert.equal(f.document.querySelectorAll('#settings-error a').length,2);
  assert.ok(!f.calls.some(c=>c.command.startsWith('set_')));
  f.document.querySelector('#settings-cancel').click();f.document.querySelector('#settings').click();await new Promise(r=>setTimeout(r,0));
  assert.equal(name.getAttribute('aria-invalid'),null);assert.equal(f.document.querySelector('#local-endpoint-error').textContent,'');
  name.value='Local fixture';endpoint.value='http://127.0.0.1:9999';
  f.state.gates={set_astra_settings:new Promise(r=>release=r)};
  form.dispatchEvent(new f.dom.window.Event('submit',{bubbles:true,cancelable:true}));
  form.dispatchEvent(new f.dom.window.Event('submit',{bubbles:true,cancelable:true}));
  await new Promise(r=>setTimeout(r,0));
  assert.equal(form.getAttribute('aria-busy'),'true');assert.equal(f.calls.filter(c=>c.command==='set_astra_settings').length,1);
  release();await new Promise(r=>setTimeout(r,0));assert.equal(form.getAttribute('aria-busy'),'false');
 }finally{release?.();f.close();}
});
test('startup never reads clipboard or invokes the retired browser transport',async()=>{
 const f=await fixture();try{assert.ok(!f.calls.some(c=>/chatgpt|workflow_run|manifest_import/.test(c.command)));assert.equal(f.document.querySelector('#chatgpt-dialog'),null);}finally{f.close();}
});
test('explicit paste button imports a Manifest once and tracks worker state',async()=>{
 const f=await fixture();try{f.api.choosePhase('autonomous');await f.api.sendAutonomous('captured-plan','project');const calls=f.calls.filter(c=>c.command==='manifest_import_text');assert.equal(calls.length,1);assert.equal(calls[0].args.projectId,'project');assert.equal(f.api.selectedView().running,true);assert.equal(f.input.hidden,true);}finally{f.close();}
});
test('legacy records are readable but cannot be resent by changing composer mode',async()=>{
 const f=await fixture({existing:true});try{await f.api.selectThread('root-session');f.api.choosePhase('autonomous');await f.api.sendAutonomous('captured-plan','project');assert.ok(f.calls.some(c=>c.command==='workflow_snapshot'));assert.ok(!f.calls.some(c=>/manifest_import|workflow_run|send_turn|chatgpt_send/.test(c.command)));assert.match(f.document.querySelector('#error').textContent,/閲覧専用/);}finally{f.close();}
});

test('duplicate paste opens the interrupted task with resume visible and no new execution',async()=>{
 const f=await fixture({duplicate:true});try{f.api.choosePhase('autonomous');await f.api.sendAutonomous('captured-plan','project');assert.equal(f.api.activeId(),'root-session');assert.equal(f.api.selectedView().running,false);assert.equal(f.api.selectedView().needsResume,true);assert.ok(f.document.querySelector('#workflow-bar [data-flow-action="resume"]'));assert.match(f.document.querySelector('#notice').textContent,/保存済みタスクを開きました/);assert.ok(!f.calls.some(c=>/autonomous_resume|send_turn/.test(c.command)));}finally{f.close();}
});

test('paste immediately signals progress, prevents duplicate clicks and confirms review completion',async()=>{
 let release;const gate=new Promise(resolve=>{release=resolve;});
 const f=await fixture({importStatus:'completed',importGate:gate});try{
  f.api.choosePhase('autonomous');const pending=f.api.sendAutonomous('captured-plan','project');
  assert.match(f.document.querySelector('#workflow-bar [data-flow-action="import"]').textContent,/取り込み中/);
  assert.equal(f.document.querySelector('#workflow-bar [data-flow-action="import"]').disabled,true);
  assert.match(f.document.querySelector('#notice').textContent,/取り込み中/);
  await assert.rejects(f.api.sendAutonomous('captured-plan','project'),/現在は取り込めません/);assert.equal(f.calls.filter(c=>c.command==='manifest_import_text').length,1);
  release();await pending;
  assert.match(f.document.querySelector('#notice').textContent,/レビュー合格・タスク完了/);
  assert.equal(f.document.querySelector('#status').textContent,'completed');
  assert.equal(f.document.querySelector('#send').getAttribute('aria-busy'),'false');
 }finally{release();f.close();}
});
test('failed import replaces progress with error and enables retry',async()=>{
 const f=await fixture({importError:'ManifestのJSON形式が不正です'});try{
  f.api.choosePhase('autonomous');await assert.rejects(f.api.sendAutonomous('captured-plan','project'),/JSON形式が不正/);
  assert.match(f.document.querySelector('#error').textContent,/JSON形式が不正/);
  assert.equal(f.document.querySelector('#notice').hidden,true);
  assert.equal(f.document.querySelector('#workflow-bar [data-flow-action="import"]').disabled,false);
  assert.equal(f.document.querySelector('#workflow-bar [data-flow-action="import"]').textContent,'コピーした計画を確認');
 }finally{f.close();}
});

test('worker progress survives snapshots, streams child events and reads actual worktree diffs',async()=>{
 const f=await fixture();try{
  f.state.steps=[{step:{key:'one',title:'実装担当',level:2,owned_paths:['new.txt']},status:'running',target:{model:'worker',reasoning:'high'},child_id:'child',verification:[]}];
  f.state.children=[{id:'child',provider_thread:{id:'provider-child'},turn:{id:'turn'}}];
  f.state.activity={workers:[{child_id:'child',events:[{sequence:1,event:{thread_id:'provider-child',turn_id:'turn',item_id:'cmd',kind:'item_started',text:'ファイルを確認',details:{type:'command',command:'cat new.txt'}}}]}],changes:[{key:'one',title:'実装担当',path:'/fixture/worktree',diff:'diff --git a/new.txt b/new.txt\n+++ b/new.txt\n+hello',error:null}]};
  f.api.choosePhase('autonomous');await f.api.sendAutonomous('captured-plan','project');
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
  assert.ok(f.document.querySelector('#retry-activity'));
  delete f.state.activityError;await f.api.refreshAutonomous('root-session');assert.equal(f.api.selectedView().progress.error,undefined);
 }finally{f.close();}
});

test('task purpose stays outside the scroller and logs keep closed state and output position',async()=>{
 const f=await fixture({importStatus:'awaiting_review'});try{
  f.state.snapshot={artifact_version:'version-123',manifest:{manifest_id:'manifest-123',request:'目的を忘れずに修正する'},review:{manifest_id:'review-123',verdict:'inconclusive'},messages:[{role:'user',text:'目的を忘れずに修正する'},{role:'assistant',text:'{"kind":"review","manifest_id":"review-123","private_payload":"long-data"}'}]};
  f.state.steps=[{step:{key:'one',title:'実装担当',level:2},status:'completed',target:{model:'worker'},child_id:'child',verification:[]}];
  f.state.activity={workers:[{child_id:'child',events:[{sequence:1,event:{kind:'item_completed',text:'long output',details:{type:'command',command:'cat file',exit_code:0}}}]}],changes:[]};
  f.api.choosePhase('autonomous');await f.api.sendAutonomous('captured-plan','project');
  const focus=f.document.querySelector('#workflow-bar');assert.equal(f.document.querySelector('#content').contains(focus),false);
  assert.equal(focus.querySelector('[data-flow-action="review"]'),null);assert.equal(f.document.querySelector('#overview-ids').open,false);
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

test('planning keeps the editor and hands off only the request and selected paths',async()=>{
 const f=await fixture();try{
  const content=f.document.querySelector('#content');
  Object.defineProperty(content,'scrollHeight',{get:()=>content.querySelector('#pending-planning-request')?900:0});
  Object.defineProperty(content,'clientHeight',{value:200});
  f.input.value='入力を保持する';f.document.querySelector('#known-files').value='src/search.ts';
  f.api.choosePhase('autonomous');
  assert.equal(f.document.querySelector('main>footer').hidden,false);
  assert.equal(f.document.querySelector('#send').hidden,false);
  assert.equal(f.document.querySelector('#local-scope').hidden,false);
  assert.match(f.document.querySelector('.footnote').textContent,/コピー/);
  assert.ok(!f.document.querySelector('#compose-more').contains(f.document.querySelector('#operation')));
  await f.api.send();
  assert.equal(f.clipboard.length,1);
  assert.equal(content.scrollTop,0,'planning must not inherit conversation auto-scroll');
  assert.match(f.clipboard[0],/Project ID: project/);
  assert.match(f.clipboard[0],/依頼: 入力を保持する/);
  assert.match(f.clipboard[0],/対象ファイル: src\/search.ts/);
  assert.match(f.clipboard[0],/project_get.*handoff_schema/);
  assert.ok(!f.clipboard[0].includes('/fixture'));
  assert.equal(f.input.value,'入力を保持する');
  assert.equal(f.document.querySelector('[aria-current="step"]').textContent,'2計画');
  assert.match(f.document.querySelector('#notice').textContent,/貼り付けて送信/);
  assert.ok(!f.calls.some(c=>/manifest_import|chatgpt_send|send_turn|preview_auto_route|create_routed_task/.test(c.command)));
 }finally{f.close();}
});
test('copy failure leaves the planning request editable without advancing the handoff',async()=>{
 const f=await fixture();try{
  f.api.choosePhase('autonomous');f.input.value='保存する依頼';f.state.copyError='clipboard denied';
  await f.api.send();
  assert.equal(f.clipboard.length,0);assert.equal(f.input.value,'保存する依頼');
  assert.equal(f.document.querySelector('[aria-current="step"]').textContent,'1依頼');
  assert.match(f.document.querySelector('#error').textContent,/コピーできません/);
  assert.equal(f.document.querySelector('#send').disabled,false);
 }finally{f.close();}
});
test('switching to ChatGPT and back retains explicit local or Codex selection and scope',async()=>{
 for(const key of ['local:fixture','codex:codex-implementation']){
  const f=await fixture();try{
   f.select.value=key;f.select.dispatchEvent(new f.dom.window.Event('change'));
   const reasoning=f.document.querySelector('#reasoning');if(!reasoning.disabled){reasoning.value='low';reasoning.dispatchEvent(new f.dom.window.Event('change'));}
   const effort=reasoning.value;f.input.value='依頼の下書き';f.document.querySelector('#known-files').value='src/file.ts';
   f.api.choosePhase('autonomous');f.api.choosePhase('implement');
   assert.equal(f.select.value,key);assert.equal(reasoning.value,effort);
   assert.equal(f.input.value,'依頼の下書き');assert.equal(f.document.querySelector('#known-files').value,'src/file.ts');
   assert.equal(f.document.querySelector('#local-scope').hidden,false);
   assert.ok(!f.calls.some(c=>/preview_auto_route|create_routed_task/.test(c.command)));
  }finally{f.close();}
 }
});
test('every Manifest tab is read-only and shows task-specific plan, context and usage',async()=>{
 const f=await fixture({importStatus:'awaiting_review'});try{
  f.state.steps=[{step:{key:'one',title:'検索を改善',goal:'入力を保持する',level:2,owned_paths:['search.ts'],dependencies:[],acceptance:['入力が残る']},status:'completed',target:{model:'worker'},capsule:JSON.stringify({task:{goal:'入力を保持する'},instructions:'他の作業には変更しない'}),usage:[{usage:{prompt_tokens:17,completion_tokens:9}}],verification:[]}];
  f.state.snapshot={artifact_version:'rev',manifest:{manifest_id:'m1',request:'検索画面',acceptance:['受け入れ条件']}};
  await f.api.sendAutonomous('captured-plan','project');
  for(const tab of ['Chat','Plan','Diff','Agents','Terminal','Context','Usage']){
   f.document.querySelector(`[data-tab="${tab}"]`).click();await new Promise(r=>setTimeout(r,10));
   assert.equal(f.document.querySelectorAll('#content button:not([data-graph-node]),#content textarea,#content select,#content input').length,0,tab+' has an operation');
   if(tab==='Plan'){assert.match(f.document.querySelector('#content').textContent,/入力を保持する/);assert.match(f.document.querySelector('#content').textContent,/受け入れ条件/);}
   if(tab==='Context')assert.match(f.document.querySelector('#content').textContent,/他の作業には変更しない/);
   if(tab==='Usage')assert.match(f.document.querySelector('#content').textContent,/17/);
  }
  assert.ok(!f.calls.some(c=>['task_graph','inspect_context','memory_candidates','model_usage'].includes(c.command)));
  assert.equal(f.document.querySelector('main>footer').hidden,true);
  assert.equal(f.document.querySelectorAll('[data-flow-action="review"]').length,0);
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

test('automated review wait never exposes a manual review handoff',async()=>{
 const f=await fixture({importStatus:'awaiting_review'});try{
  f.state.snapshot={artifact_version:'version-one',manifest:{manifest_id:'m',request:'検索画面を改善'}};
  await f.api.sendAutonomous('captured-plan','project');
  assert.equal(f.document.querySelector('[data-flow-action="review"]'),null);
  assert.equal(f.clipboard.length,0);
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

test('an explicitly scoped write requests the reviewed worktree path',async()=>{
 const f=await fixture();try{
  f.document.querySelector('[data-operation="implement"]').click();
  f.select.value='codex:codex-implementation';f.select.dispatchEvent(new f.dom.window.Event('change'));
  f.document.querySelector('#known-files').value='src/name.ts';
  f.input.value='名前の表示を直して';await f.api.send();
  const request=f.calls.find(c=>c.command==='create_routed_task').args.request;
  assert.equal(request.reviewedWrite,true);
  assert.equal(request.knownFiles.join(','),'src/name.ts');
 }finally{f.close();}
});

test('a hidden ChatGPT pane restores the planning draft without changing its execution choice',async()=>{
 const f=await fixture({browserOpen:false,saved:JSON.stringify({project:'project',drafts:{'new:project':{text:'Keep my draft',mode:'autonomous',preference:'local:fixture'}}})});try{
  assert.equal(f.document.querySelector('#operation [aria-pressed="true"]').dataset.operation,'autonomous');
  assert.equal(f.document.querySelector('main>footer').hidden,false);
  assert.equal(f.input.value,'Keep my draft');assert.equal(f.select.value,'local:fixture');
  assert.equal(f.document.querySelector('[aria-current="step"]').textContent,'1依頼');
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
  assert.equal(f.document.querySelector('#operation').closest('details'),null);
  f.input.value='修正をお願いします';await f.api.send();
  assert.equal(f.calls.filter(c=>c.command==='preview_auto_route').length,1);
  assert.equal(f.calls.find(c=>c.command==='create_routed_task').args.request.autoRouteId,'route-1');
  assert.equal(f.calls.find(c=>c.command==='create_routed_task').args.request.reviewedWrite,false);
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
  assert.equal(f.calls.filter(c=>c.command==='create_routed_task').length,1);
  assert.equal(f.calls.filter(c=>c.command==='send_composed_turn').length,1);
  assert.equal(f.document.querySelector('#task-mode').value,'autonomous');
  assert.match(f.input.value,/計画です/);
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
  assert.equal(f.calls.filter(c=>c.command==='manifest_import_text').length,0);
  assert.equal(f.document.querySelector('#manifest-review').open,true);
  f.document.querySelector('#manifest-confirm').click();await new Promise(r=>setTimeout(r,20));
  assert.equal(f.calls.filter(c=>c.command==='manifest_import_text').length,1);
  assert.equal(f.api.activeId(),'root-session');
  assert.match(f.document.querySelector('#content').textContent,/今回の依頼/);
  key();await new Promise(r=>setTimeout(r,10));
  assert.equal(f.calls.filter(c=>c.command==='manifest_import_text').length,1);
 }finally{f.close();}
 });
