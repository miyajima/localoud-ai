import {test} from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';
import {fillModelSelect,fillReasoningSelect,readTarget} from '../src/model-controls.ts';
const models=[{key:'spark',label:'Local',model:'local/8bit',local:true,reasoning:[],default_reasoning:null,is_default:false},{key:'codex:remote',label:'Remote',model:'remote',local:false,reasoning:['low','high'],default_reasoning:'low',is_default:true}];
function controls(){const dom=new JSDOM('<select id="model"></select><select id="effort"></select>');return [dom.window.document.querySelector('#model'),dom.window.document.querySelector('#effort')];}
test('selected reasoning belongs to the selected model and unsupported values never fall back silently',()=>{
 const [model,effort]=controls();fillModelSelect(model,models,{provider:'codex',model:'remote',reasoning:'high'});fillReasoningSelect(effort,models[1],'high');
 assert.deepEqual(readTarget(model,effort,models),{provider:'codex',model:'remote',reasoning:'high'});
 fillReasoningSelect(effort,models[1],'ultra');assert.equal(readTarget(model,effort,models),null);assert.equal(effort.value,'ultra');
});
test('local replacement cannot take the place of a saved model with a different ID',()=>{
 const [model,effort]=controls();fillModelSelect(model,models,{provider:'local',model:'old/model',reasoning:null});fillReasoningSelect(effort,models[0]);
 assert.equal(readTarget(model,effort,models),null);
 fillModelSelect(model,models,{provider:'local',model:'local/8bit',reasoning:null});assert.equal(effort.disabled,true);assert.equal(readTarget(model,effort,models).reasoning,null);
});
test('provider default is distinct from an explicit reasoning value and labels are text',()=>{
 const [model,effort]=controls();fillModelSelect(model,[{...models[1],label:'<img src=x>'}],{provider:'codex',model:'remote',reasoning:null});fillReasoningSelect(effort,models[1]);
 assert.equal(model.querySelector('img'),null);assert.equal(readTarget(model,effort,models).reasoning,null);assert.match(effort.selectedOptions[0].textContent,/既定/);
});

test('a retired browser target never resolves to a Codex model with the same ID',()=>{
 const [model,effort]=controls();fillModelSelect(model,models,{provider:'chatgpt',model:'remote',reasoning:null});fillReasoningSelect(effort,models[1]);assert.equal(readTarget(model,effort,models),null);assert.ok(model.selectedOptions[0].disabled);
});

test('the same model ID on two API profiles remains profile-qualified and resolves exactly',()=>{
 const providers=[
  {key:'api:alpha:same',label:'Alpha / Same',model:'same',profile_id:'alpha',local:false,reasoning:['high'],tools:true,default_reasoning:null,is_default:false},
  {key:'api:beta:same',label:'Beta / Same',model:'same',profile_id:'beta',local:false,reasoning:['high'],tools:true,default_reasoning:null,is_default:false},
 ];
 const [model,effort]=controls();
 fillModelSelect(model,providers,{provider:'api',profile_id:'beta',model:'same',reasoning:'high'});
 fillReasoningSelect(effort,providers[1],'high');
 assert.equal(model.selectedOptions[0].textContent,'Beta / Same');
 assert.deepEqual(readTarget(model,effort,providers),{provider:'api',profile_id:'beta',model:'same',reasoning:'high'});
 assert.deepEqual(Array.from(model.options).filter(option=>option.value).map(option=>option.textContent),['Alpha / Same','Beta / Same']);
});
