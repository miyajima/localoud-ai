import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {JSDOM} from 'jsdom';
import ts from 'typescript';

const source=(await readFile(new URL('../src/auto-routing.ts',import.meta.url),'utf8')).replaceAll("'./model-controls'",JSON.stringify(new URL('../src/model-controls.ts',import.meta.url).href));
const {outputText}=ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ES2022}});
const routing=await import('data:text/javascript;base64,'+Buffer.from(outputText).toString('base64'));
const local={provider:'local',profile_id:'spark',model:'local',reasoning:null};
const remote={provider:'codex',profile_id:'codex',model:'remote',reasoning:'low'};
const models=[
  {key:'spark',label:'Local',model:'local',local:true,reasoning:[],default_reasoning:null},
  {key:'codex:remote',label:'Remote',model:'remote',local:false,reasoning:['low','high'],default_reasoning:'low'},
];
const routeIds=['classifier','level_1','level_2','level_3','level_4','level_5','planner','reviewer','fallback'];
const config={
  classifier:local,
  levels:[1,2,3,4,5].map(level=>({level,target:level===1?local:remote})),
  fallback:null,
  planner_default:remote,
  reviewer_default:remote,
  agents:[
    {id:'agent-router',name:'Router',target:local},
    {id:'agent-builder',name:'Builder',target:remote},
  ],
  route_agents:routeIds.map(route=>({route,agent_id:['classifier','level_1'].includes(route)?'agent-router':'agent-builder'})),
  confirm_before_run:false,
};

function setup(call,run=()=>{}){
  const dom=new JSDOM('<!doctype html><body></body>');
  globalThis.document=dom.window.document;
  dom.window.HTMLDialogElement.prototype.showModal=function(){this.open=true;};
  dom.window.HTMLDialogElement.prototype.close=function(){this.open=false;};
  routing.setupAutoRouting({call,models:()=>models,refresh:async()=>{},busy:()=>false,run,plan:()=>{},notice:()=>{}});
  return dom.window;
}
const flush=()=>new Promise(resolve=>setImmediate(resolve));

test('agents can be added, edited, assigned and saved without role names',async()=>{
  const calls=[];
  const window=setup(async(command,args)=>{
    calls.push({command,args});
    if(command==='auto_settings')return structuredClone(config);
  });
  await routing.openAutoSettings();
  document.getElementById('add-agent').click();
  assert.equal(document.querySelectorAll('.agent-row').length,3);
  document.getElementById('agent-2-name').value='Sakura';
  document.getElementById('agent-2-model').value='codex:remote';
  document.getElementById('agent-2-model').dispatchEvent(new window.Event('change'));
  document.getElementById('agent-2-reasoning').value='high';
  document.getElementById('level-3-agent').value=document.querySelectorAll('.agent-row')[2].dataset.agentId;
  document.getElementById('auto-fallback-mode').value='model';
  document.getElementById('auto-fallback-mode').dispatchEvent(new window.Event('change'));
  document.getElementById('auto-confirm').checked=true;
  document.getElementById('auto-settings-form').dispatchEvent(new window.Event('submit',{cancelable:true}));
  await flush();
  const saved=calls.find(call=>call.command==='set_auto_settings').args.config;
  assert.equal(saved.agents.length,3);
  assert.equal(saved.agents[2].name,'Sakura');
  assert.equal(saved.agents[2].target.reasoning,'high');
  assert.equal(saved.levels[2].target.reasoning,'high');
  assert.equal(saved.fallback.model,'remote');
  assert.equal(saved.confirm_before_run,true);
  assert.equal('identities' in saved,false);
});

test('deleting an assigned agent rewires its routes to a remaining agent',async()=>{
  setup(async command=>command==='auto_settings'?structuredClone(config):undefined);
  await routing.openAutoSettings();
  document.querySelector('.agent-delete').click();
  assert.equal(document.querySelectorAll('.agent-row').length,1);
  assert.equal(document.getElementById('classifier-agent').value,'agent-builder');
  assert.match(document.getElementById('agent-change-status').textContent,/割り当て/);
});

test('blocked previews cannot execute and a difficulty change carries the current revision',async()=>{
  const calls=[];
  let executions=0;
  const preview={id:'p',revision:2,level:4,target:remote,reason:'needs design',blocked:'plan required',needs_plan:true,used_fallback:false,confirm_before_run:true,manual_override:false,agent_name:'Architect'};
  const window=setup(async(command,args)=>{calls.push({command,args});return {...preview,revision:3,level:1};},()=>executions++);
  routing.showAutoPreview(preview);
  document.getElementById('route-run').click();
  assert.equal(executions,0);
  const level=document.getElementById('route-level');
  level.value='1';
  level.dispatchEvent(new window.Event('change'));
  await flush();
  assert.equal(calls[0].command,'revise_auto_route');
  assert.equal(calls[0].args.revision,2);
  assert.equal(calls[0].args.level,1);
  assert.equal(document.getElementById('route-run').disabled,true);
});

test('preview shows the resolved agent name',()=>{
  const preview={id:'p',revision:0,level:3,target:remote,route_key:'level_3',agent_name:'Sakura',reason:'standard',blocked:null,needs_plan:false,used_fallback:false,confirm_before_run:true,manual_override:false};
  setup(async()=>preview);
  routing.showAutoPreview(preview);
  assert.equal(document.getElementById('route-assignment').textContent,'Sakura');
});
