import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import {JSDOM} from 'jsdom';
import ts from 'typescript';
import {appendModelOptions,fillReasoningSelect,readTarget} from '../src/model-controls.ts';
const source=(await readFile(new URL('../src/main.ts',import.meta.url),'utf8')).replace(/^import .*;\s*$/gm,'');
const js=ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ES2022}}).outputText.replace(/export\s*\{\s*\};?/g,'');
async function fixture({saved,existing=false,connected=true}={}){
 const dom=new JSDOM('<div id="app"></div>',{url:'https://workflow.fixture'});
 if(saved)dom.window.localStorage.setItem('astra-ui-v1',saved);
 const calls=[];
 const project={id:'project',name:'Fixture',root:'/fixture'};
 const root={id:'root-session',project_id:project.id,provider:'workflow',provider_thread_id:'workflow:root',title:'入力を保持する',status:'idle'};
 const state={threads:existing?[root]:[],running:false,failReview:false,messages:existing?[{role:'user',text:'保存済みの依頼',key:'old-user'},{role:'assistant',text:'保存済みのプラン',key:'old-plan',label:'プラン · ChatGPT'}]:[],targets:existing?{plan:{model:'gpt-6-astra',reasoning:'high',chatgpt:true},review:{model:'gpt-6-astra',reasoning:'high',chatgpt:true},implement:{model:'codex-implementation',reasoning:'high',chatgpt:false}}:{},phase:'plan',model:'gpt-6-astra'};
 const invoke=async(command,args)=>{
  calls.push({command,args});
  switch(command){
   case 'available_models':return {models:[{key:'codex:gpt-6-astra',label:'Astra on Codex',model:'gpt-6-astra',local:false,reasoning:['high'],default_reasoning:null},{key:'codex:codex-implementation',label:'Implementation',model:'codex-implementation',local:false,reasoning:['high','low'],default_reasoning:null}],warnings:[]};
   case 'chatgpt_status':return {models:connected?[{id:'gpt-6-astra',label:'Astra',efforts:['high']}]:[],connected,token:'fixture',extension_path:'/fixture/extensions/chatgpt',error:null};
   case 'projects':return [project];
   case 'threads':return state.threads;
   case 'archived_threads':return [];
   case 'thread_models':return {};
   case 'thread_reasoning':return {};
   case 'workflow_run':{
    const r=args.request;if(state.running)throw Error('still running');if(r.phase==='review'&&state.failReview)throw Error('ChatGPT is disconnected');
    state.phase=r.phase;state.model=r.model;state.threads=[root];state.targets[r.phase]={model:r.model,reasoning:r.reasoning,chatgpt:r.provider==='chatgpt'};
    state.messages.push({role:'user',text:r.text,key:`u-${calls.length}`},{role:'assistant',text:r.phase==='plan'?'入力を保持するプラン':r.phase==='implement'?'実装結果: tests passed':'レビュー結果: pass',label:r.phase==='implement'?'実装 · Codex':'ChatGPT',key:`a-${calls.length}`});return root;
   }
   case 'workflow_snapshot':return {running:state.running,messages:state.messages,phase:state.phase,model:state.model,targets:state.targets,handoff:state.phase==='review'?'要件・プラン・実装結果・作業差分':'これまでの会話'};
   case 'workflow_stop':state.running=false;return;
   default:throw Error('Unexpected command: '+command);
  }
 };
 const noop=()=>{};
 const context={window:dom.window,document:dom.window.document,localStorage:dom.window.localStorage,console,Option:dom.window.Option,setTimeout,clearTimeout,setInterval:()=>0,clearInterval:noop,nativeInvoke:invoke,isTauri:()=>true,listen:async()=>noop,appendModelOptions,fillReasoningSelect,readTarget,markdown:s=>s,diffMarkup:s=>s,statusLabel:s=>s,planPreview:()=>'',setupMemory:noop,memoryPanel:()=>'',bindMemory:noop,setupAutoRouting:noop,showAutoPreview:noop,openAutoSettings:noop,setupComposer:()=>({syncContext:noop,getMentions:()=>[],setMentions:noop,clear:noop,refreshThread:async()=>{},seedHistory:async()=>{},remember:async()=>{},beforeSend:async()=>true})};
 for(const name of ['HTMLElement','HTMLButtonElement','HTMLSelectElement','HTMLInputElement','HTMLTextAreaElement','HTMLDialogElement','HTMLDetailsElement','HTMLAnchorElement','HTMLOptionElement'])context[name]=dom.window[name];
 const api=await vm.runInNewContext(`(async()=>{${js}\nawait refreshModels();return {choosePhase,send,selectThread,refreshWorkflow,selectedView,activeId:()=>activeThread};})()`,context);
 return {dom,calls,state,api,select:dom.window.document.querySelector('#preference'),input:dom.window.document.querySelector('#task-input'),document:dom.window.document,close:()=>dom.window.close()};
}
test('one session supports ChatGPT plan → explicit Codex implementation → ChatGPT review with distinct same-ID model paths',async()=>{
 const f=await fixture();try{
  f.api.choosePhase('plan');assert.equal(f.select.value,'chatgpt:gpt-6-astra');assert.equal(f.select.disabled,false);f.input.value='入力を保持してください';await f.api.send();
  assert.equal(f.api.activeId(),'root-session');assert.equal(f.calls.filter(c=>c.command==='workflow_run').length,1);
  f.api.choosePhase('implement');assert.equal(f.calls.filter(c=>c.command==='workflow_run').length,1,'phase selection must not execute');
  assert.ok([...f.select.options].every(o=>!o.value.startsWith('chatgpt:')));f.select.value='codex:codex-implementation';await f.api.send();
  f.api.choosePhase('review');assert.equal(f.select.value,'chatgpt:gpt-6-astra');await f.api.send();
  const requests=f.calls.filter(c=>c.command==='workflow_run').map(c=>c.args.request);
  assert.deepEqual(requests.map(r=>r.phase),['plan','implement','review']);assert.deepEqual(requests.map(r=>r.provider),['chatgpt','codex','chatgpt']);assert.deepEqual(requests.map(r=>r.threadId),[null,'root-session','root-session']);assert.deepEqual(requests.map(r=>r.model),['gpt-6-astra','codex-implementation','gpt-6-astra']);
  assert.equal(f.document.querySelectorAll('[data-thread]').length,1);const text=f.document.querySelector('#content').textContent;for(const expected of ['入力を保持するプラン','実装結果: tests passed','レビュー結果: pass','要件・プラン・実装結果・作業差分'])assert.ok(text.includes(expected),expected);
  assert.ok(!f.calls.some(c=>['chatgpt_start_tunnel','chatgpt_approve','create_routed_task','final_review','send_turn'].includes(c.command)));
 }finally{f.close();}
});
test('reload restores the phase/model and resumes the workflow; running phases cannot switch and stop uses workflow transport',async()=>{
 const saved=JSON.stringify({project:'project',thread:'root-session',drafts:{'root-session':{text:'保存した下書き',mode:'workflow-implement',preference:'codex:codex-implementation',reasoning:'high'}}});
 const f=await fixture({saved,existing:true});try{
  assert.equal(f.input.value,'保存した下書き');assert.equal(f.select.value,'codex:codex-implementation');assert.equal(f.document.querySelector('#task-mode').value,'workflow-implement');
  assert.ok(f.calls.some(c=>c.command==='workflow_snapshot'&&c.args.resume===true));
  f.state.running=true;await f.api.refreshWorkflow('root-session');f.api.choosePhase('review');assert.equal(f.document.querySelector('#task-mode').value,'workflow-implement');assert.equal(f.document.querySelector('#operation').disabled,true);
  f.document.querySelector('#stop').click();await new Promise(resolve=>setImmediate(resolve));assert.ok(f.calls.some(c=>c.command==='workflow_stop'));assert.ok(!f.calls.some(c=>c.command==='interrupt_turn'));assert.equal(f.input.value,'保存した下書き');
 }finally{f.close();}
});
test('a failed ChatGPT review preserves the request and never falls back to Codex',async()=>{
 const f=await fixture({existing:true});try{
  await f.api.selectThread('root-session');f.api.choosePhase('review');f.state.failReview=true;f.input.value='この観点でレビューしてください';await f.api.send();
  assert.equal(f.input.value,'この観点でレビューしてください');assert.match(f.document.querySelector('#error').textContent,/ChatGPT is disconnected/);assert.equal(f.calls.filter(c=>c.command==='workflow_run').length,1);assert.ok(!f.calls.some(c=>['create_routed_task','send_turn','final_review','send_composed_turn'].includes(c.command)));
 }finally{f.close();}
});

test('disconnected ChatGPT still permits explicit Codex planning and review',async()=>{
 const f=await fixture({connected:false});try{
  f.api.choosePhase('plan');assert.equal(f.select.value,'codex:gpt-6-astra');assert.ok([...f.document.querySelectorAll('#workflow-steps button')].some(b=>b.textContent==='ChatGPTを接続'));assert.equal(f.document.querySelector('#send').disabled,false);assert.match(f.document.querySelector('#model-hint').textContent,/Codexでプラン/);f.input.value='計画してください';await f.api.send();f.api.choosePhase('review');f.input.value='差分をレビュー';await f.api.send();assert.deepEqual(f.calls.filter(c=>c.command==='workflow_run').map(c=>[c.args.request.phase,c.args.request.provider]),[['plan','codex'],['review','codex']]);assert.ok(f.document.querySelector('#chatgpt-dialog'));assert.equal(f.document.querySelector('#chatgpt-tunnel'),null);
 }finally{f.close();}
});

test('primary operations are only plan, implement and review, with the actual execution path shown separately',async()=>{
 const f=await fixture();try{
  assert.deepEqual([...f.document.querySelector('#operation').options].filter(o=>!o.hidden).map(o=>o.textContent),['プラン','実装','レビュー']);assert.equal(f.document.querySelector('#task-mode').hidden,true);
  f.api.choosePhase('plan');assert.equal(f.document.querySelector('#execution-source').textContent,'ChatGPT');
  f.api.choosePhase('implement');assert.equal(f.document.querySelector('#execution-source').textContent,'Codex');
  f.api.choosePhase('review');assert.equal(f.document.querySelector('#execution-source').textContent,'ChatGPT');
  assert.equal(f.calls.filter(c=>c.command==='workflow_run').length,0);
 }finally{f.close();}
});

test('plan and review allow both providers with identical model IDs and remember each phase separately',async()=>{
 const f=await fixture();try{
  f.api.choosePhase('plan');assert.ok([...f.select.options].some(o=>o.value==='codex:gpt-6-astra'));assert.ok([...f.select.options].some(o=>o.value==='chatgpt:gpt-6-astra'));
  f.select.value='codex:gpt-6-astra';f.select.dispatchEvent(new f.dom.window.Event('change'));f.input.value='Codexでプラン';await f.api.send();
  assert.equal(f.document.querySelector('#execution-source').textContent,'Codex');
  f.api.choosePhase('review');f.select.value='chatgpt:gpt-6-astra';f.input.value='ChatGPTでレビュー';await f.api.send();
  f.api.choosePhase('plan');assert.equal(f.select.value,'codex:gpt-6-astra');
  f.api.choosePhase('review');assert.equal(f.select.value,'chatgpt:gpt-6-astra');
  assert.deepEqual(f.calls.filter(c=>c.command==='workflow_run').map(c=>c.args.request.provider),['codex','chatgpt']);
 }finally{f.close();}
});
