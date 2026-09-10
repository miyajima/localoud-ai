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
 const dom=new JSDOM('<div id="app"><aside></aside><main><header></header><nav></nav></main></div>',{url:'https://localoud.fixture'});
 dom.window.localStorage.setItem('localoud-browser-hidden','false');
 dom.window.HTMLDialogElement.prototype.showModal=function(){this.setAttribute('open','');};
 dom.window.HTMLDialogElement.prototype.show=function(){this.setAttribute('open','');};
 dom.window.HTMLDialogElement.prototype.close=function(){this.removeAttribute('open');this.dispatchEvent(new dom.window.Event('close'));};
 const calls=[];
 const setup=vm.runInNewContext(js+';setupBrowserWorkspace',{window:dom.window,document:dom.window.document,HTMLElement:dom.window.HTMLElement,localStorage:dom.window.localStorage,MutationObserver:dom.window.MutationObserver,ResizeObserver:class{observe(){}},isTauri:()=>true});
 const ui=setup(async(command,args)=>calls.push({command,args}));
 return {dom,calls,ui,doc:dom.window.document,settle:()=>new Promise(r=>setImmediate(r)),visible:()=>calls.at(-1)?.args?.bounds.visible};
}
test('ChatGPT starts closed and preserves one native webview across modal visits',async()=>{
 const f=fixture();try{
  await f.settle();assert.equal(f.visible(),false);
  f.ui.showBrowser();await f.settle();assert.equal(f.visible(),true);
  assert.equal(f.calls.at(-1).args.bounds.focus,true);
  assert.equal(f.doc.querySelector('#app').inert,true);assert.equal(f.doc.querySelector('#browser-overlay').hidden,false);
  assert.equal(f.doc.querySelector('#browser-panel').getAttribute('role'),'dialog');
  assert.equal(f.doc.querySelector('#browser-toggle').getAttribute('aria-expanded'),'true');
  const count=f.calls.length;f.ui.showBrowser();f.ui.setSidebarHidden(false);await f.settle();assert.equal(f.calls.length,count);
  const other=f.doc.createElement('dialog');f.doc.body.append(other);other.showModal();await f.settle();assert.equal(f.visible(),false);
  other.close();await f.settle();assert.equal(f.visible(),true);
  f.doc.querySelector('#browser-close').click();await f.settle();assert.equal(f.visible(),false);
  assert.equal(f.calls.at(-1).args.bounds.focus,true);
  assert.equal(f.doc.querySelector('#app').inert,false);assert.equal(f.doc.querySelector('#browser-overlay').hidden,true);
  assert.equal(f.doc.querySelector('#browser-toggle').getAttribute('aria-expanded'),'false');
  f.ui.showBrowser();f.ui.showWorkspace();await f.settle();assert.equal(f.visible(),false);
  assert.ok(f.calls.every(c=>c.command==='browser_layout'));
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
