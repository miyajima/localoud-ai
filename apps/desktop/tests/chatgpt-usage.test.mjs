import {test} from 'node:test';
import assert from 'node:assert/strict';
import {normalizeUsage,addUsage,periods,setupChatGptUsage} from '../src/chatgpt-usage.ts';
import {JSDOM} from 'jsdom';
test('Monday boundary resets the shared weekly count; midnight resets only Sol daily',()=>{
 const sunday=new Date(2026,8,13,23,59),monday=new Date(2026,8,14,0,0);
 let s=normalizeUsage(undefined,sunday);s=addUsage(s,'astra',sunday);s=addUsage(s,'sol',sunday);
 assert.equal(s.astra+s.sol,2);assert.equal(s.solToday,1);assert.equal(periods(sunday).week,'2026-09-07');
 assert.deepEqual(normalizeUsage(s,monday),{week:'2026-09-14',day:'2026-09-14',astra:0,sol:0,solToday:0});
 const tuesday=new Date(2026,8,15,0,0);s=addUsage(normalizeUsage(undefined,monday),'sol',monday);
 assert.equal(normalizeUsage(s,tuesday).sol,1);assert.equal(normalizeUsage(s,tuesday).solToday,0);
});
test('completed turns count once, persist across reload, and keep unknown models separate',()=>{
 const dom=new JSDOM('<div id="usage"></div>',{url:'https://fixture.local'});globalThis.window=dom.window;globalThis.localStorage=dom.window.localStorage;
 try{
 const container=dom.window.document.querySelector('#usage');let record=setupChatGptUsage(container);
 record({id:'a',model:'astra',duration:5000});record({id:'a',model:'astra',duration:5000});record({id:'s',model:'sol',duration:3000});
 assert.match(container.textContent,/週 2 \/ 200.*Sol 今日 1 \/ 170/);
 record=setupChatGptUsage(container);record({id:'a',model:'astra',duration:5000});
 record({id:'u',model:'unknown',duration:1000});
 assert.match(container.textContent,/週 2 \/ 200/);assert.match(container.textContent,/モデル未取得 1件/);
 assert.equal(Object.keys(JSON.parse(localStorage.getItem('localoud-chatgpt-auto-v1')).records).length,3);
 assert.equal(container.querySelector('[data-count]'),null);
 }finally{dom.window.close();}
});
