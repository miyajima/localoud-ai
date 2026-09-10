import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import ts from 'typescript';
import {JSDOM} from 'jsdom';

const source=(await readFile(new URL('../src/manifest-review.ts',import.meta.url),'utf8')).replace('export function','function');
const js=ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022}}).outputText;
const task=()=>({kind:'task',project_id:'project',request:'確認した依頼',scope:['src/task.ts'],acceptance:['入力が残る'],steps:[{title:'入力を保持',goal:'下書きを保存',owned_paths:['src/task.ts'],acceptance:['画面を戻しても入力が残る']}]});
const settle=()=>new Promise(resolve=>setImmediate(resolve));
function deferred(){let resolve;const promise=new Promise(r=>{resolve=r;});return {promise,resolve};}
function fixture(read,commit){
 const dom=new JSDOM('<body></body>');
 dom.window.HTMLDialogElement.prototype.showModal=function(){this.setAttribute('open','');};
 dom.window.HTMLDialogElement.prototype.close=function(){this.removeAttribute('open');};
 const setup=vm.runInNewContext(js+';setupManifestReview',{document:dom.window.document});
 const imports=[],reads=[];let returned=0;
 const open=setup(async(command,args)=>{reads.push({command,args});return read();},async(text,id)=>{imports.push({text,id});if(commit)await commit();});
 const doc=dom.window.document;
 return {imports,reads,doc,dom,open:()=>open({id:'project',name:'Project'},()=>returned++),returned:()=>returned,confirm:()=>doc.querySelector('#manifest-confirm'),cancel:()=>doc.querySelector('#manifest-cancel'),dialog:()=>doc.querySelector('dialog'),close:()=>dom.window.close()};
}
test('preview is read-only, escaped, cancellable and discards a late clipboard response',async()=>{
 const gate=deferred(),f=fixture(()=>gate.promise);try{
  const pending=f.open();assert.equal(f.confirm().disabled,true);
  f.cancel().click();gate.resolve(task());await pending;
  assert.equal(f.dialog().open,false);assert.equal(f.returned(),1);assert.equal(f.imports.length,0);
  assert.equal(f.confirm().disabled,true);
 }finally{f.close();}
 const payload=task();payload.request='<img src=x onerror=alert(1)>';
 const safe=fixture(()=>payload);try{
  await safe.open();assert.equal(safe.doc.querySelector('img'),null);assert.match(safe.doc.querySelector('#manifest-preview').textContent,/<img/);
  safe.cancel().click();assert.equal(safe.imports.length,0);assert.equal(safe.returned(),1);
 }finally{safe.close();}
});
test('confirmation imports the displayed payload once even if the clipboard changes',async()=>{
 const payload=task(),gate=deferred(),f=fixture(()=>payload,()=>gate.promise);try{
  await f.open();const displayed=JSON.parse(f.doc.querySelector('#manifest-preview pre').textContent);
  assert.equal(f.imports.length,0);payload.request='後からコピーした別の依頼';
  f.confirm().click();f.confirm().click();f.cancel().click();await settle();
  assert.equal(f.imports.length,1);assert.deepEqual(JSON.parse(f.imports[0].text),displayed);assert.equal(f.imports[0].id,'project');
  assert.equal(f.reads.length,1);assert.equal(f.dialog().open,true);assert.equal(f.returned(),0);
  gate.resolve();await settle();assert.equal(f.dialog().open,false);
 }finally{gate.resolve();f.close();}
});
test('mismatched projects and invalid clipboard content never enable execution',async()=>{
 for(const read of [()=>({...task(),project_id:'another'}),()=>{throw Error('JSON形式が不正');}]){
  const f=fixture(read);try{await f.open();assert.equal(f.confirm().disabled,true);assert.ok(f.doc.querySelector('#manifest-error').textContent);f.confirm().click();assert.equal(f.imports.length,0);}finally{f.close();}
 }
});
test('review confirmation describes its effect and failed imports retain the same payload for retry',async()=>{
 for(const [verdict,changes,label] of [['pass',[],'合格を記録して完了'],['fail',[{step_key:'one',instruction:'修正する'}],'この修正を実行'],['inconclusive',[],'結果を記録']]){
  let attempts=0;const f=fixture(()=>({kind:'review',project_id:'project',task_id:'task-id',verdict,summary:'確認結果',findings:[],changes}),()=>{if(++attempts===1)throw Error('接続失敗');});try{
   await f.open();assert.equal(f.confirm().textContent,label);f.confirm().click();await settle();
   assert.equal(f.dialog().open,true);assert.equal(f.confirm().disabled,false);assert.match(f.doc.querySelector('#manifest-error').textContent,/接続失敗/);
   f.confirm().click();await settle();assert.equal(f.dialog().open,false);assert.equal(f.imports[0].text,f.imports[1].text);assert.equal(f.reads.length,1);
  }finally{f.close();}
 }
});
