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
  const thread={id:'task-1',project_id:project.id,provider:kind==='autonomous'?'autonomous':'codex',provider_thread_id:'provider-1',title:'検索結果の読みやすさを改善する',status:'completed'};
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
    case 'auto_settings':return {classifier:{provider:'codex',profile_id:'codex',model:model.model,reasoning:'medium'},levels:[1,2,3,4,5].map(level=>({level,target:{provider:'codex',profile_id:'codex',model:model.model,reasoning:'medium'}})),fallback:null,planner_default:{provider:'codex',profile_id:'codex',model:model.model,reasoning:'high'},reviewer_default:{provider:'codex',profile_id:'codex',model:model.model,reasoning:'high'},agents:[{id:'agent-sakura',name:'Sakura',target:{provider:'codex',profile_id:'codex',model:model.model,reasoning:'medium'}},{id:'agent-reviewer',name:'Reviewer',target:{provider:'codex',profile_id:'codex',model:model.model,reasoning:'high'}}],route_agents:['classifier','level_1','level_2','level_3','level_4','level_5','planner','fallback'].map(route=>({route,agent_id:'agent-sakura'})).concat({route:'reviewer',agent_id:'agent-reviewer'}),confirm_before_run:true};
    case 'thread_models':return {'task-1':model.model};
    case 'thread_model_targets':return {};
    case 'thread_reasoning':return {'task-1':'medium'};
    case 'archived_threads':case 'sync_archived_sessions':case 'event_history':case 'prompt_history':case 'task_graph':return [];
    case 'completion_settings':return {enabled:false};
    case 'composer_thread_state':return {mode:'implement',goal:null,questions:[],warning:null};
    case 'composer_catalog':return {entries:[],modes:[],warnings:[]};
    case 'browser_layout':case 'browser_reload':case 'remember_prompt':case 'set_astra_settings':return null;
    case 'resume_task':return {active_turn:null,messages:[{role:'user',text:'検索結果の見出しと説明を読みやすくしてください。キーボード操作も維持してください。'},{role:'assistant',text:'## 変更内容\n\n見出しと説明の間隔を整え、キーボードのフォーカスを維持しました。\n\n- 検索結果を読みやすく表示\n- 長いURLも画面内で折り返し\n- テストで基本操作を確認\n\n```ts\nconst label = "検索結果";\n```\n\nテスト用の表示です。実際のファイルは変更していません。'}]};
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
    case 'autonomous_snapshot':return {thread,messages:[{role:'user',text:thread.title}],steps:[{step:{key:'implementation',title:'検索結果を改善',goal:'見出しと説明の余白を整える',level:3,owned_paths:['src/search.ts'],acceptance:['キーボード操作を維持'],dependencies:[]},status:'completed',target:{model:'fixture-model',reasoning:'medium'},agent_name:'Sakura',child_id:'child-1',capsule:'保存済みの実行指示です。',verification:[{status:'pass',command:'npm test',exit_code:0,target_revision:'fixture-revision',log_ref:'fixture-log',output:'テスト成功（画面検証用データ）'}],usage:[{prompt_tokens:1200,completion_tokens:340,latency_ms:1600}]}],children:[{id:'child-1',provider_thread:{id:'child-provider'}}],artifact_version:'fixture-revision',source_head:'fixture-base',iteration:1,final_diff:null,artifact:{path:'/workspace/fixture-worktree'},manifest:{manifest_id:'fixture-manifest',request:thread.title,acceptance:['読みやすさとキーボード操作を両立する']},review:{manifest_id:'fixture-review',verdict:'pass',summary:'画面検証用の保存記録です。実際のLLM実行は行っていません。',findings:[]}};
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
 const violations=axe.violations.map(v=>({id:v.id,impact:v.impact,nodes:v.nodes.map(n=>({target:n.target,summary:n.failureSummary}))}));
 results.push({name,...layout,violations});
 console.log(name,JSON.stringify({violations:violations.map(v=>[v.id,v.nodes.length]),smallText:layout.smallText.length,smallTargets:layout.smallTargets.length,overflow:layout.scrollWidth-layout.width}));
}
try{
 for(const mode of ['light','dark']){
  const f=await fixture({colorScheme:mode});
  await inspect(f.page,`desktop-${mode}-welcome`);
  await f.page.locator('#compose-more').evaluate(element=>element.open=true);
  await f.page.locator('#auto-settings-button').click();
  await f.page.locator('#agent-list .agent-row').first().waitFor();
  await f.page.locator('#add-agent').click();
  check(`${mode}: agent can be added`,await f.page.locator('#agent-list .agent-row').count()===3);
  await inspect(f.page,`desktop-${mode}-agent-settings`);
  await f.page.locator('#auto-settings-close').click();
  await f.page.locator('[data-thread="task-1"]').click();
  await f.page.locator('.message').first().waitFor();
  await inspect(f.page,`desktop-${mode}-task`);
  await f.page.locator('[data-tab="Usage"]').click();
  await f.page.locator('td').first().waitFor();
  await inspect(f.page,`desktop-${mode}-usage`);
  await f.page.locator('#settings').click();
  await f.page.locator('#local-name').inputValue();
  await inspect(f.page,`desktop-${mode}-settings`);
  await f.page.locator('[data-settings-page="mcp"]').click();
  await f.page.locator('#mcp-projects input').waitFor();
  check('MCP checkbox and label align',await f.page.locator('#mcp-projects label').evaluate(label=>{const a=label.querySelector('input').getBoundingClientRect(),b=label.querySelector('span').getBoundingClientRect();return Math.abs(a.y+a.height/2-b.y-b.height/2)<2;}));
  await inspect(f.page,`desktop-${mode}-settings-mcp`);
  await f.page.locator('#mcp-projects input').uncheck();
  await f.page.locator('[data-settings-page="usage"]').click();
  check('Usage identifies its project',await f.page.locator('#total-usage tbody td').first().textContent()==='Localoud');
  await inspect(f.page,`desktop-${mode}-settings-usage`);
  await f.page.locator('[data-settings-page="mcp"]').click();
  check('Switching settings preserves unpublished checkbox edits',!await f.page.locator('#mcp-projects input').isChecked());
  await f.page.locator('[data-settings-page="connection"]').click();
  await f.page.locator('#settings-cancel').click();
  for(const tab of ['Plan','Diff','Agents','Terminal','Context']){
   await f.page.locator(`[data-tab="${tab}"]`).click();
   await f.page.waitForFunction(()=>document.querySelector('#content').getAttribute('aria-busy')!=='true');
   await inspect(f.page,`desktop-${mode}-${tab.toLowerCase()}`);
  }
  await f.page.locator('#browser-toggle').click();await inspect(f.page,`desktop-${mode}-browser-shell`);
  await f.page.keyboard.press('Escape');
  await f.page.locator('#command-menu').click();await inspect(f.page,`desktop-${mode}-commands`);
  assert.deepEqual(f.errors,[]);
  await f.context.close();
 }
 for(const mode of ['light','dark']){
  const f=await fixture({colorScheme:mode},'autonomous');
  await f.page.locator('[data-thread="task-1"]').click();
  await f.page.locator('#overview-acceptance').waitFor();
  for(const tab of ['Chat','Plan','Diff','Agents','Terminal','Context','Usage']){
   await f.page.locator(`[data-tab="${tab}"]`).click();
   await f.page.waitForFunction(()=>document.querySelector('#content').getAttribute('aria-busy')!=='true');
   if(tab==='Agents')check(`${mode}: configured agent is visible`,await f.page.locator('#content').evaluate(e=>e.textContent.includes('Sakura · 難易度 3')));
   await inspect(f.page,`execution-${mode}-${tab.toLowerCase()}`);
  }
  assert.deepEqual(f.errors,[]);await f.context.close();
 }
 for(const viewport of [{width:375,height:812},{width:812,height:375}]){
  const f=await fixture({viewport,hasTouch:true,colorScheme:'dark',reducedMotion:'reduce'});
  await inspect(f.page,`narrow-${viewport.width}-welcome`);
  await f.page.locator('#compose-more').evaluate(element=>element.open=true);
  await f.page.locator('#auto-settings-button').click();
  await f.page.locator('#agent-list .agent-row').first().waitFor();
  await inspect(f.page,`narrow-${viewport.width}-agent-settings`);
  await f.page.locator('#auto-settings-close').click();
  if(viewport.width===375){
   await f.page.locator('#sidebar-toggle').click();
   check('375px navigation opens with focus inside',await f.page.evaluate(()=>document.activeElement.id==='sidebar-close'&&document.querySelector('main').inert));
   await inspect(f.page,'narrow-375-navigation');
   await f.page.keyboard.press('Escape');
   check('Escape returns focus to navigation trigger',await f.page.locator('#sidebar-toggle').evaluate(e=>e===document.activeElement));
   await f.page.locator('#sidebar-toggle').click();
  }
  await f.page.locator('[data-thread="task-1"]').click();
  await f.page.locator('.message').first().waitFor();
  await inspect(f.page,`narrow-${viewport.width}-task`);
  await f.page.locator('#task-input').fill('テスト用の下書き');
  check(`${viewport.width}px typing enables send`,await f.page.locator('#send').isEnabled());
  await f.page.locator('#send').scrollIntoViewIfNeeded();
  check(`${viewport.width}px send remains reachable`,await f.page.locator('#send').evaluate(e=>{const r=e.getBoundingClientRect();return r.top>=0&&r.bottom<=innerHeight;}));
  if(viewport.width===375)await f.page.locator('#sidebar-toggle').click();
  await f.page.locator('#new').click();
  await f.page.evaluate(()=>{document.documentElement.style.fontSize='200%';});
  await inspect(f.page,`text200-${viewport.width}-welcome`);
  await f.page.locator('#task-input').fill('拡大文字でも操作できます');
  check(`${viewport.width}px 200% typing enables send`,await f.page.locator('#send').isEnabled());
  await f.page.locator('#send').scrollIntoViewIfNeeded();
  await inspect(f.page,`text200-${viewport.width}-composer`);
  check(`${viewport.width}px 200% send remains reachable`,await f.page.locator('#send').evaluate(e=>{const r=e.getBoundingClientRect();return r.top>=0&&r.bottom<=innerHeight&&r.right<=innerWidth;}));
  if(viewport.width===375){await f.page.evaluate(()=>{document.documentElement.style.fontSize='100%';});await f.page.locator('#sidebar-toggle').click();await f.page.locator('#settings').click();await inspect(f.page,'narrow-375-settings');}
  await f.context.close();
 }
 const f=await fixture({reducedMotion:'reduce'});const p=f.page;
 await p.keyboard.press('Tab');check('Skip link is the first keyboard target',await p.locator('#skip-to-content').evaluate(e=>e===document.activeElement));
 await p.keyboard.press('Enter');check('Skip link reaches the content',await p.locator('#content').evaluate(e=>e===document.activeElement));
 await p.locator('[data-thread="task-1"]').click();await p.locator('.message').first().waitFor();
 await p.locator('[data-tab="Chat"]').focus();await p.keyboard.press('ArrowRight');await p.keyboard.press('ArrowRight');
 check('Two arrow presses retain tab focus without activating',await p.evaluate(()=>document.activeElement.dataset.tab==='Diff'&&document.querySelector('[data-tab="Chat"]').getAttribute('aria-selected')==='true'));
 await p.keyboard.press('Enter');await p.keyboard.press('Tab');check('Tab reaches the selected panel',await p.locator('#content').evaluate(e=>e===document.activeElement));
 await p.locator('[data-tab="Chat"]').click();await p.locator('#task-input').focus();await p.keyboard.press('Tab');
 check('Composer without a suggestion does not trap Tab',await p.evaluate(()=>document.activeElement.id!=='task-input'));
 await p.evaluate(()=>{window.audit.delay.model_usage=250;});await p.locator('[data-tab="Usage"]').click();
 check('Loading is not presented as zero usage',await p.locator('#content').evaluate(e=>e.textContent.includes('読み込み中')&&!e.textContent.includes('0件')));
 await p.locator('td').first().waitFor();await p.evaluate(()=>{window.audit.fail.model_usage='fixture unavailable';});
 await p.locator('[data-tab="Usage"]').click();await p.locator('#retry-pane').waitFor();
 check('Failed refresh preserves the last successful data',await p.locator('#content').evaluate(e=>e.textContent.includes('前回の表示')&&e.textContent.includes('fixture-model')));
 await inspect(p,'usage-failed');await p.evaluate(()=>{delete window.audit.fail.model_usage;});await p.locator('#retry-pane').click();await p.locator('#retry-pane').waitFor({state:'detached'});
 check('Retry clears the error after a successful read',await p.locator('#content').evaluate(e=>!e.textContent.includes('読み込めません')));
 await p.locator('#settings').click();await p.waitForFunction(()=>!document.querySelector('#settings-form [type="submit"]').disabled);
 await p.locator('#local-name').fill('');await p.locator('#local-endpoint').fill('invalid');await p.locator('#settings-form [type="submit"]').click();
 check('Multiple invalid fields focus the linked error summary',await p.evaluate(()=>document.activeElement.id==='settings-error'&&document.querySelectorAll('#settings-error a').length===2));
 await inspect(p,'settings-invalid');
 await p.emulateMedia({colorScheme:'dark'});await inspect(p,'settings-invalid-dark');
 await p.locator('#local-name').fill('Local fixture');await p.locator('#local-endpoint').fill('http://127.0.0.1:9999');await p.evaluate(()=>{window.audit.delay.set_astra_settings=300;});
 await p.locator('#settings-form [type="submit"]').click();await p.locator('#settings-form').dispatchEvent('submit');
 await p.locator('#connection-settings').waitFor({state:'hidden'});
 check('Repeated submit calls the mocked save exactly once',await p.evaluate(()=>window.audit.calls.filter(c=>c==='set_astra_settings').length===1));
 await p.locator('[data-tab="Chat"]').click();
 const stream=await p.evaluate(async()=>{
  let contentChanges=0,treeChanges=0;
  const contentObserver=new MutationObserver(()=>contentChanges++),treeObserver=new MutationObserver(()=>treeChanges++);
  contentObserver.observe(document.querySelector('#content'),{childList:true});treeObserver.observe(document.querySelector('#projects'),{childList:true});
  document.querySelector('[data-copy-message="0"]').focus();
  const start=performance.now(),listener=window.audit.callbacks[window.audit.listeners['hub-event']];
  for(let i=1;i<=100;i++)listener({payload:{sequence:i,event:{thread_id:'provider-1',turn_id:'turn-1',item_id:'stream-1',kind:'message_delta',text:`${i} `}}});
  await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));
  contentObserver.disconnect();treeObserver.disconnect();
  return {contentChanges,treeChanges,elapsedMs:performance.now()-start,focus:document.activeElement.dataset.copyMessage,final:document.querySelector('#content').textContent.includes('99 100')};
 });
 check('100 streaming updates are rendered in one batch',stream.contentChanges===1&&stream.treeChanges===0&&stream.final);
 check('Streaming preserves the focused message action',stream.focus==='0');
 results.push({name:'stream-burst',...stream,violations:[],smallText:[],smallTargets:[],width:1280,scrollWidth:1280});
 const nativeError=await p.locator('#error').textContent();
 check(`The fixture made no unexpected native calls: ${nativeError}`,!nativeError.includes('AUDIT BLOCKED'));
 assert.deepEqual(f.errors,[]);await f.context.close();
}finally{await writeFile(`${output}/report.json`,JSON.stringify({screens:results,checks},null,2));await browser.close();await server?.close();}
if(!process.argv.includes('--baseline')){
 assert.equal(results.reduce((n,r)=>n+r.violations.length,0),0,'Accessibility violations: see report.json');
 assert.ok(results.every(r=>r.scrollWidth<=r.width&&r.smallText.length===0&&r.smallTargets.length===0),'Layout or target-size violations: see report.json');
}
console.log(`Evidence: ${output}`);
