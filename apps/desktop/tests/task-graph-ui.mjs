// Browser-only acceptance harness. Every native command is intercepted; no LLM,
// account, file-picker, setting or repository mutation reaches the real backend.
import {chromium} from 'playwright';
import AxeBuilder from '@axe-core/playwright';
import {mkdir, mkdtemp, writeFile} from 'node:fs/promises';
import assert from 'node:assert/strict';
import {createServer} from 'vite';

const output=process.env.UI_AUDIT_OUTPUT||await mkdtemp('/private/tmp/localoud-ui-audit-');
await mkdir(output,{recursive:true});
const server=process.env.UI_AUDIT_URL?undefined:await createServer({cacheDir:`${output}/vite-cache`,server:{host:'127.0.0.1',port:4179,strictPort:false}});
await server?.listen();
const url=process.env.UI_AUDIT_URL||server.resolvedUrls.local[0];
assert.equal(new URL(url).hostname,'127.0.0.1','Audit must use an isolated local frontend');
const browser=await chromium.launch({headless:true}).catch(async error=>{await server?.close();throw error;});
const results=[],checks=[];
const check=(name,passed)=>{assert.ok(passed,name);checks.push(name);};
async function fixture(options={},kind='direct') {
 const context=await browser.newContext({viewport:{width:1280,height:900},...options});
 await context.route('**/*',route=>new URL(route.request().url()).origin===new URL(url).origin?route.continue():route.abort());
 await context.addInitScript(kind=>{
  const project={id:'fixture',name:'Localoud',root:'/workspace/localoud'};
  const thread={id:'task-1',project_id:project.id,provider:kind==='autonomous'?'autonomous':'codex',provider_thread_id:'provider-1',title:'検索結果の読みやすさを改善する',status:'running'};
  const model={key:'codex:fixture-model',label:'Codex fixture',model:'fixture-model',local:false,reasoning:['medium','high'],default_reasoning:'medium'};
  window.audit={calls:[],fail:{},delay:{},callbacks:{},listeners:{}};
  window.isTauri=true;
  window.__TAURI_INTERNALS__={transformCallback:callback=>{const id=Object.keys(window.audit.callbacks).length+1;window.audit.callbacks[id]=callback;return id;},invoke:async(command,args={})=>{
   window.audit.calls.push(command);
   if(window.audit.delay[command])await new Promise(r=>setTimeout(r,window.audit.delay[command]));
   if(window.audit.fail[command])throw Error(window.audit.fail[command]);
   switch(command){
    case 'plugin:event|listen':window.audit.listeners[args.event]=args.handler;return args.handler;
    case 'projects':return [project];
    case 'threads':return [thread];
    case 'available_models':return {models:[model],warnings:[]};
    case 'thread_models':return {'task-1':model.model};
    case 'thread_model_targets':return {};
    case 'thread_reasoning':return {'task-1':'medium'};
    case 'archived_threads':case 'sync_archived_sessions':case 'event_history':case 'prompt_history':case 'task_graph':return [];
    case 'completion_settings':return {enabled:false};
    case 'composer_thread_state':return {mode:'implement',goal:null,questions:[],warning:null};
    case 'composer_catalog':return {entries:[],modes:[],warnings:[]};
    case 'browser_layout':case 'browser_reload':case 'remember_prompt':case 'set_astra_settings':return null;
    case 'read_task':case 'resume_task':return {active_turn:null,messages:[{role:'user',text:'検索結果の見出しと説明を読みやすくしてください。キーボード操作も維持してください。'},{role:'assistant',text:'## 変更内容\n\n見出しと説明の間隔を整え、キーボードのフォーカスを維持しました。\n\n- 検索結果を読みやすく表示\n- 長いURLも画面内で折り返し\n- テストで基本操作を確認\n\n```ts\nconst label = "検索結果";\n```\n\nテスト用の表示です。実際のファイルは変更していません。'}]};
    case 'worker_insights':return {review:null,recovery:null};
    case 'repo_diff':return 'diff --git a/search.ts b/search.ts\n--- a/search.ts\n+++ b/search.ts\n@@ -1 +1 @@\n-const spacing = 4;\n+const spacing = 12;';
    case 'model_usage':return [{project_id:'fixture',project_name:'Localoud',provider:'codex',model:'fixture-model',prompt_tokens:1200,completion_tokens:340,cached_tokens:null,latency_ms:1600}];
    case 'inspect_context':return null;
    case 'memory_candidates':return {candidates:[]};
    case 'local_model_settings':return {display_name:'Local fixture',model_id:'fixture-local',endpoint:'http://127.0.0.1:9999',quantization_bits:4};
    case 'provider_profiles':return [];
    case 'mcp_settings':case 'set_mcp_settings':return {config:args.config||{enabled:true,project_ids:['fixture'],connection:{kind:'secure_tunnel',tunnel_id:'fixture-tunnel'}},running:true,error:null,local_endpoint:'http://127.0.0.1:8792/mcp'};
    case 'codex_binary':return '/usr/local/bin/codex';
    case 'astra_settings':return {mode:'disabled',model:'fixture-model',reasoning:null};
    case 'autonomous_snapshot':return {thread,messages:[{role:'user',text:thread.title}],steps:[['source','共通仕様を整理',[],'completed'],['ui','検索画面を改善',['source'],'running'],['index','検索処理を改善',['source'],'running'],['verify','変更を統合して検証',['ui','index'],'pending']].map(([key,title,dependencies,status])=>({step:{key,title,dependencies,goal:title+'。既存の機能とキーボード操作を維持します。',level:2,owned_paths:['src/'+key],acceptance:['関連テストが成功する']},status,target:{model:'fixture-model',reasoning:'medium'},verification:[]})),children:[{id:'child-1',provider_thread:{id:'child-provider'}}],artifact_version:'fixture-revision',source_head:'fixture-base',iteration:1,final_diff:null,artifact:{path:'/workspace/fixture-worktree'},manifest:{manifest_id:'fixture-manifest',request:thread.title,acceptance:['読みやすさとキーボード操作を両立する']},review:{manifest_id:'fixture-review',verdict:'pass',summary:'画面検証用の保存記録です。実際のLLM実行は行っていません。',findings:[]}};
    case 'manifest_preview_clipboard':return {kind:'task',project_id:'fixture',request:'検索の体験を改善する',scope:['src'],acceptance:['検索品質と操作性を維持'],steps:[['source','共通仕様を整理',[]],['ui','検索画面を改善',['source']],['index','検索処理を改善',['source']],['verify','変更を統合して検証',['ui','index']]].map(([key,title,dependencies])=>({key,title,dependencies,goal:title,level:2,owned_paths:['src/'+key],acceptance:['テストが成功する']}))};
    case 'autonomous_activity':return {workers:[{child_id:'child-1',events:[{sequence:1,event:{kind:'item_completed',text:'表示の調整と検証を完了しました。',details:{type:'command',command:'npm test',exit_code:0}}}]}],changes:[{key:'implementation',title:'検索結果を改善',path:'/workspace/fixture-worktree',diff:'diff --git a/search.ts b/search.ts\n--- a/search.ts\n+++ b/search.ts\n@@ -1 +1 @@\n-const spacing = 4;\n+const spacing = 12;',error:null}]};
    default:throw Error('AUDIT BLOCKED native command: '+command);
   }
  }};
 },kind);
 const page=await context.newPage();
 const errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto(url);
 await page.locator('#project-title').filter({hasText:'Localoud'}).waitFor();
 await page.waitForFunction(()=>!document.querySelector('#refresh-models').disabled);
 return {context,page,errors};
}
async function inspect(page,name){
 await page.evaluate(()=>Promise.all(document.getAnimations().map(a=>a.finished.catch(()=>{}))));
 const sidebarOverlaps=await page.evaluate(()=>{
  const rows=[...document.querySelectorAll('#app>aside>:not(.sidebar-bottom)')].filter(e=>e.getClientRects().length).map(e=>({id:e.id||e.className,rect:e.getBoundingClientRect()}));
  return rows.slice(1).filter((row,i)=>row.rect.top<rows[i].rect.bottom-1).map(row=>row.id);
 });
 check(`${name}: sidebar rows do not overlap`,sidebarOverlaps.length===0);
 const axe=await new AxeBuilder({page}).withTags(['wcag2a','wcag2aa','wcag21aa','wcag22aa']).analyze();
 const layout=await page.evaluate(()=>{
  const visible=e=>{const r=e.getBoundingClientRect();return r.width&&r.height&&r.bottom>0&&r.top<innerHeight&&r.right>0&&r.left<innerWidth&&getComputedStyle(e).visibility==='visible'&&!e.closest('[inert]');};
  return {width:innerWidth,scrollWidth:document.documentElement.scrollWidth,smallText:[...document.querySelectorAll('button,p,small,label,summary,span')].filter(e=>visible(e)&&e.textContent.trim()&&parseFloat(getComputedStyle(e).fontSize)<12).slice(0,35).map(e=>({selector:e.id||e.className,text:e.textContent.slice(0,60),size:getComputedStyle(e).fontSize})),smallTargets:matchMedia('(pointer:coarse)').matches?[...document.querySelectorAll('button,summary,input,select')].filter(e=>visible(e)&&!e.disabled&&e.type!=='checkbox').flatMap(e=>{const r=e.getBoundingClientRect();return r.width<43.5||r.height<43.5?[{id:e.id||e.className,width:r.width,height:r.height}]:[]}):[]};
 });
 await page.screenshot({path:`${output}/${name}.png`,fullPage:true});
 const size=page.viewportSize();await page.setViewportSize({width:size.width,height:2200});
 await page.locator('.task-graph').screenshot({path:`${output}/${name}-detail.png`});
 await page.setViewportSize(size);
 const violations=axe.violations.map(v=>({id:v.id,impact:v.impact,nodes:v.nodes.map(n=>({target:n.target,summary:n.failureSummary}))}));
 results.push({name,...layout,violations});
 console.log(name,JSON.stringify({violations:violations.map(v=>[v.id,v.nodes.length]),smallText:layout.smallText.length,smallTargets:layout.smallTargets.length,overflow:layout.scrollWidth-layout.width}));
}

try{
 const f=await fixture({},'autonomous');
 if(!process.env.GRAPH_PREVIEW_ONLY){
 await f.page.locator('[data-thread="task-1"]').click();
 await f.page.locator('.task-graph').waitFor();
 check('concurrent workers', (await f.page.locator('.graph-heading').innerText()).includes('2件を実行中'));
 await f.page.locator('#execution-graph-node-3').click();
 await f.page.waitForTimeout(1700);
 check('selection persists after polling',await f.page.locator('#execution-graph-node-3').getAttribute('aria-expanded')==='true');
 await inspect(f.page,'graph-desktop-light');
 await f.page.emulateMedia({colorScheme:'dark',reducedMotion:'reduce'});
 await inspect(f.page,'graph-desktop-dark');
 await f.page.setViewportSize({width:390,height:844});
 await inspect(f.page,'graph-narrow-dark');
 await f.page.setViewportSize({width:1280,height:900});
 await f.page.emulateMedia({colorScheme:'light'});
 await f.page.evaluate(()=>document.documentElement.style.fontSize='200%');
 await inspect(f.page,'graph-enlarged-text');
 await f.page.evaluate(()=>document.documentElement.style.fontSize='');
 await f.page.locator('[data-tab="Plan"]').click();
 await inspect(f.page,'graph-plan');
 }
 await f.page.locator('#new').click();
 await f.page.locator('#task-mode').evaluate(e=>{e.value='autonomous';e.dispatchEvent(new Event('change',{bubbles:true}));});
 await f.page.locator('[data-flow-action="import"]:visible').first().click();
 await f.page.locator('#preview-graph-node-0').waitFor();
 check('preview never dispatches workers',!(await f.page.evaluate(()=>window.audit.calls)).includes('manifest_import_text'));
 check('desktop preview shows all dependency columns',await f.page.locator('#manifest-preview .graph-scroll').evaluate(e=>e.scrollWidth<=e.clientWidth));
 await inspect(f.page,'graph-preview-desktop');
 await f.page.setViewportSize({width:390,height:844});
 await inspect(f.page,'graph-preview-narrow');
 check('no browser errors',f.errors.length===0);
 await writeFile(output+(process.env.GRAPH_PREVIEW_ONLY?'/preview-verdict.json':'/graph-report.json'),JSON.stringify({checks,results},null,2));
 check('no page overflow',results.every(r=>r.scrollWidth<=r.width));
 check('no accessibility violations',results.every(r=>r.violations.length===0));
 console.log('Graph evidence: '+output);
 await f.context.close();
}finally{await browser.close();await server?.close();}
