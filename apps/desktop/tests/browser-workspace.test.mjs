import {focusMarkup,reviewRequest} from '../src/task-focus.ts';
import {flowMarkup} from '../src/workflow-ui.ts';
import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import ts from 'typescript';
import {JSDOM} from 'jsdom';
const source=(await readFile(new URL('../src/browser-workspace.ts',import.meta.url),'utf8')).replace(/^import .*;\s*$/gm,'').replace('export function','function');
const js=ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022}}).outputText;
function fixture(){
 const dom=new JSDOM('<div id="app"><aside></aside><main><header><button id="sidebar-toggle"></button></header><nav></nav></main></div>',{url:'https://localoud.fixture'});
 dom.window.document.querySelector('#sidebar-toggle').addEventListener('click',()=>dom.window.document.querySelector('#app').classList.toggle('mobile-sidebar-open'));
 dom.window.localStorage.setItem('localoud-browser-hidden','false');
 dom.window.HTMLDialogElement.prototype.showModal=function(){this.setAttribute('open','');};
 dom.window.HTMLDialogElement.prototype.show=function(){this.setAttribute('open','');};
 dom.window.HTMLDialogElement.prototype.close=function(){this.removeAttribute('open');this.dispatchEvent(new dom.window.Event('close'));};
 const calls=[];
 const setup=vm.runInNewContext(js+';setupBrowserWorkspace',{window:dom.window,document:dom.window.document,HTMLElement:dom.window.HTMLElement,localStorage:dom.window.localStorage,MutationObserver:dom.window.MutationObserver,ResizeObserver:class{observe(){}},isTauri:()=>true});
 const ui=setup(async(command,args)=>calls.push({command,args}));
 return {dom,calls,ui,doc:dom.window.document,settle:()=>new Promise(r=>setImmediate(r)),visible:()=>calls.at(-1)?.args?.bounds.visible};
}
test('ChatGPT uses a non-modal work surface and switches native webviews by session',async()=>{
 const f=fixture();try{
  await f.settle();assert.equal(f.visible(),false);
  f.ui.setSession('session-a','最初のセッション');await f.settle();
  f.ui.showBrowser();await f.settle();assert.equal(f.visible(),true);
  assert.equal(f.calls.at(-1).args.bounds.focus,true);
  const firstBrowser=f.calls.at(-1).args.browserId;assert.match(firstBrowser,/^web-/);
  assert.notEqual(f.doc.querySelector('#app').inert,true);assert.equal(f.doc.querySelector('#browser-overlay').hidden,false);
  assert.equal(f.doc.querySelector('#browser-overlay').parentElement.tagName,'MAIN');
  assert.equal(f.doc.querySelector('#browser-panel').getAttribute('role'),'region');
  assert.equal(f.doc.querySelector('#browser-panel').hasAttribute('aria-modal'),false);
  assert.match(f.doc.querySelector('#browser-context').textContent,/最初のセッション/);
  assert.equal(f.doc.querySelector('#browser-toggle').getAttribute('aria-expanded'),'true');
  const count=f.calls.length;f.ui.showBrowser();f.ui.setSidebarHidden(false);await f.settle();assert.equal(f.calls.length,count);
  f.doc.querySelector('#browser-sessions').click();await f.settle();assert.equal(f.visible(),false);
  f.doc.querySelector('#browser-sessions').click();await f.settle();assert.equal(f.visible(),true);
  f.ui.setSession('session-b','次のセッション');await f.settle();
  assert.equal(f.visible(),true);assert.notEqual(f.calls.at(-1).args.browserId,firstBrowser);
  assert.match(f.doc.querySelector('#browser-context').textContent,/次のセッション/);
  f.ui.setSession('session-a','最初のセッション');await f.settle();assert.equal(f.calls.at(-1).args.browserId,firstBrowser);
  const other=f.doc.createElement('dialog');f.doc.body.append(other);other.showModal();await f.settle();assert.equal(f.visible(),false);
  other.close();await f.settle();assert.equal(f.visible(),true);
  f.doc.querySelector('#browser-close').click();await f.settle();assert.equal(f.visible(),false);
  assert.equal(f.calls.at(-1).args.bounds.focus,true);
  assert.notEqual(f.doc.querySelector('#app').inert,true);assert.equal(f.doc.querySelector('#browser-overlay').hidden,true);
  assert.equal(f.doc.querySelector('#browser-toggle').getAttribute('aria-expanded'),'false');
  f.ui.showBrowser();f.ui.showWorkspace();await f.settle();assert.equal(f.visible(),false);
  assert.ok(f.calls.every(c=>c.command==='browser_layout'));
 }finally{f.dom.window.close();}
});
test('a planning webview follows the Localoud session created from that draft',async()=>{
 const f=fixture();try{
  f.ui.setSession('new:project','新しいタスク');f.ui.showBrowser();await f.settle();
  const draftBrowser=f.calls.at(-1).args.browserId;
  f.ui.moveSession('new:project','created-session');
  f.ui.setSession('created-session','取り込んだタスク');await f.settle();
  assert.equal(f.calls.at(-1).args.browserId,draftBrowser);
  assert.match(f.doc.querySelector('#browser-context').textContent,/取り込んだタスク/);
  f.ui.setSession('new:project','次の新しいタスク');await f.settle();
  assert.notEqual(f.calls.at(-1).args.browserId,draftBrowser);
 }finally{f.dom.window.close();}
});
test('handoff actions retain selected task and lock during import',async()=>{
 const f=fixture(),actions=[];try{
  const flow={project:'Project',title:'対象の依頼',id:'selected-task',version:'v1',kind:'manifest',hasTask:true,status:'awaiting_review',reviewRequested:true,busy:false};
  f.ui.setWorkflow(flowMarkup(flow),action=>actions.push(action));f.ui.showBrowser();await f.settle();
  const handoff=f.doc.querySelector('#browser-handoff');assert.equal(handoff.hidden,false);assert.match(handoff.textContent,/対象の依頼/);
  handoff.querySelector('[data-flow-action="import"]').click();assert.deepEqual(actions,['import']);
  f.ui.setWorkflow(flowMarkup({...flow,busy:true,importing:true}),action=>actions.push(action));
  handoff.querySelector('[data-flow-action="import"]').click();assert.deepEqual(actions,['import']);
  f.ui.setWorkflow(flowMarkup({...flow,kind:'direct'}),()=>{});assert.equal(handoff.hidden,true);
  f.ui.hideBrowser();await f.settle();assert.equal(f.visible(),false);
  const divider=f.doc.querySelector('#sidebar-divider');divider.dispatchEvent(new f.dom.window.KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true}));assert.equal(f.dom.window.localStorage.getItem('localoud-sidebar-width'),'250');
 }finally{f.dom.window.close();}
});

test('copied request instructions precede ChatGPT and navigation stays accessible',async()=>{
 const f=fixture();try{
  f.ui.showCopiedRequestGuide('plan');await f.settle();
  assert.equal(f.doc.querySelector('#chatgpt-paste-guide').open,true);
  assert.equal(f.doc.querySelector('#browser-overlay').hidden,true);
  assert.match(f.doc.querySelector('#chatgpt-return-guide').textContent,/回答のコピーボタンで全体をコピー/);
  assert.match(f.doc.querySelector('#chatgpt-return-guide').textContent,/ChatGPTの回答を確認/);
  f.doc.querySelector('#guide-open').click();await f.settle();
  assert.equal(f.doc.querySelector('#chatgpt-paste-guide').open,false);
  assert.equal(f.visible(),true);
  const browserId=f.calls.at(-1).args.browserId;
  f.doc.querySelector('#browser-back').click();await f.settle();
  f.doc.querySelector('#browser-home').click();await f.settle();
  assert.ok(f.calls.some(c=>c.command==='browser_back'&&c.args.browserId===browserId));
  assert.ok(f.calls.some(c=>c.command==='browser_home'&&c.args.browserId===browserId));
  f.ui.showCopiedRequestGuide('review');
  assert.match(f.doc.querySelector('#chatgpt-return-guide').textContent,/コピーした結果を確認/);
  f.doc.querySelector('#guide-close').click();await f.settle();
  assert.equal(f.doc.querySelector('#browser-overlay').hidden,true);
  const event=new f.dom.window.MouseEvent('contextmenu',{bubbles:true,cancelable:true});
  f.doc.body.dispatchEvent(event);assert.equal(event.defaultPrevented,true);
 }finally{f.dom.window.close();}
});
