import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {JSDOM} from 'jsdom';
import ts from 'typescript';
const source=(await readFile(new URL('../src/auto-routing.ts',import.meta.url),'utf8')).replaceAll("'./model-controls'",JSON.stringify(new URL('../src/model-controls.ts',import.meta.url).href));
const {outputText}=ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ES2022}});
const routing=await import('data:text/javascript;base64,'+Buffer.from(outputText).toString('base64'));
const local={provider:'local',model:'local',reasoning:null},remote={provider:'codex',model:'remote',reasoning:null};
const models=[{key:'spark',label:'Local',model:'local',local:true,reasoning:[],default_reasoning:null},{key:'codex:remote',label:'Remote',model:'remote',local:false,reasoning:['low','high'],default_reasoning:'low'}];
function setup(call,run=()=>{}){
 const dom=new JSDOM('<!doctype html><body></body>');globalThis.document=dom.window.document;
 dom.window.HTMLDialogElement.prototype.showModal=function(){this.open=true;};dom.window.HTMLDialogElement.prototype.close=function(){this.open=false;};
 routing.setupAutoRouting({call,models:()=>models,refresh:async()=>{},busy:()=>false,run,plan:()=>{},notice:()=>{}});
 return dom.window;
}
const flush=()=>new Promise(resolve=>setImmediate(resolve));
test('five model/reasoning mappings, classifier and fallback are saved from visible controls',async()=>{
 const calls=[];const config={classifier:local,levels:[1,2,3,4,5].map(level=>({level,target:level===1?local:remote})),fallback:null,confirm_before_run:false};
 const window=setup(async(command,args)=>{calls.push({command,args});if(command==='auto_settings')return structuredClone(config);});
 await routing.openAutoSettings();
 const field=id=>document.getElementById(id);
 field('level-3-reasoning').value='high';field('auto-fallback-mode').value='model';field('auto-fallback-mode').dispatchEvent(new window.Event('change'));field('fallback-reasoning').value='low';field('auto-confirm').checked=true;
 field('auto-settings-form').dispatchEvent(new window.Event('submit',{cancelable:true}));await flush();
 const saved=calls.find(c=>c.command==='set_auto_settings').args.config;
 assert.equal(saved.levels.length,5);assert.equal(saved.levels[2].target.reasoning,'high');assert.equal(saved.classifier.model,'local');assert.equal(saved.fallback.reasoning,'low');assert.equal(saved.confirm_before_run,true);
});
test('blocked previews cannot execute and a difficulty change carries the current revision',async()=>{
 const calls=[];let executions=0;
 const preview={id:'p',revision:2,level:4,target:remote,reason:'needs design',blocked:'plan required',needs_plan:true,used_fallback:false,confirm_before_run:true,manual_override:false};
 const window=setup(async(command,args)=>{calls.push({command,args});return {...preview,revision:3,level:1};},()=>executions++);
 routing.showAutoPreview(preview);
 document.getElementById('route-run').click();assert.equal(executions,0);
 const level=document.getElementById('route-level');level.value='1';level.dispatchEvent(new window.Event('change'));await flush();
 assert.equal(calls[0].command,'revise_auto_route');assert.equal(calls[0].args.revision,2);assert.equal(calls[0].args.level,1);assert.equal(document.getElementById('route-run').disabled,true);
});
