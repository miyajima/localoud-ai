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
