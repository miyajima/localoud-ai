import {test} from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';
import {setupMcpSettings} from '../src/mcp-settings.ts';

test('MCP edit permissions are explicit, saved separately and cleared when a project is unpublished', async () => {
  const dom = new JSDOM('<main id="container"></main>');
  globalThis.document = dom.window.document;
  const config = {enabled:true, project_ids:['p1','p2'], connection:{kind:'unconfigured'}};
  let submitted;
  const call = async (command,args) => {
    if (command === 'set_mcp_settings') {submitted=args.config; return {config:submitted,running:true,error:null,local_endpoint:'http://127.0.0.1:8792/mcp'};}
    return {config,running:true,error:null,local_endpoint:'http://127.0.0.1:8792/mcp'};
  };
  const open = setupMcpSettings(call,()=>[{id:'p1',name:'One',root:'/one'},{id:'p2',name:'Two',root:'/two'}],undefined,{container:document.querySelector('main'),open(){},close(){}});
  await open();
  const write = document.querySelectorAll('#mcp-write-projects input');
  assert.equal(write.length,2);
  assert.equal(write[0].checked,false);
  write[0].checked=true;
  document.querySelector('form').dispatchEvent(new dom.window.Event('submit',{cancelable:true}));
  await new Promise(resolve=>setImmediate(resolve));
  assert.deepEqual(submitted.write_project_ids,['p1']);
  const read=document.querySelector('#mcp-projects input[value="p1"]');
  read.checked=false; read.dispatchEvent(new dom.window.Event('change'));
  assert.equal(write[0].checked,false);
  assert.equal(write[0].disabled,true);
  document.querySelector('form').dispatchEvent(new dom.window.Event('submit',{cancelable:true}));
  await new Promise(resolve=>setImmediate(resolve));
  assert.deepEqual(submitted.write_project_ids,[]);
  assert.deepEqual(submitted.project_ids,['p2']);
});

test('existing project edit permissions are restored without treating new projects as writable', async () => {
  const dom = new JSDOM('<main></main>'); globalThis.document=dom.window.document;
  const config={enabled:true,project_ids:['p1'],write_project_ids:['p1'],connection:{kind:'unconfigured'}};
  const open=setupMcpSettings(async()=>({config,running:true,error:null,local_endpoint:'local'}),()=>[{id:'p1',name:'One',root:'/one'},{id:'p2',name:'Two',root:'/two'}],undefined,{container:document.querySelector('main'),open(){},close(){}});
  await open();
  const boxes=document.querySelectorAll('#mcp-write-projects input');
  assert.equal(boxes[0].checked,true); assert.equal(boxes[1].checked,false); assert.equal(boxes[1].disabled,true);
});
