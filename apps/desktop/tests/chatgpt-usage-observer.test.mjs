import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import {JSDOM} from 'jsdom';
const source=await readFile(new URL('../src-tauri/src/chatgpt-usage-observer.js',import.meta.url),'utf8');
test('loaded history does not count; observed completion records response model once',()=>{
 const dom=new JSDOM('<button data-testid="model-switcher-dropdown-button">Astra</button><div data-message-author-role="assistant" data-message-id="old"></div>',{url:'https://chatgpt.com/c/test'});
 let tick,now=0;const reported=[];const context={document:dom.window.document,window:dom.window,location:{hostname:'chatgpt.com',set href(value){reported.push(new URL(value));}},URLSearchParams,Date:{now:()=>now},MutationObserver:class{observe(){}},setInterval:f=>tick=f};
 Object.defineProperty(dom.window.document,'readyState',{value:'complete'});
 vm.runInNewContext(source,context);tick();now=2000;tick();assert.equal(reported.length,0);
 const stop=dom.window.document.createElement('button');stop.dataset.testid='stop-button';dom.window.document.body.append(stop);tick();
 dom.window.document.body.insertAdjacentHTML('beforeend','<div data-message-author-role="assistant" data-message-id="new" data-message-model-slug="gpt-6-sol"></div>');
 now=5000;stop.remove();tick();now=6300;tick();tick();
 assert.equal(reported.length,1);assert.equal(reported[0].searchParams.get('model'),'sol');assert.equal(reported[0].searchParams.get('duration'),'3000');assert.equal(reported[0].searchParams.get('id'),'new');
 dom.window.close();
});
