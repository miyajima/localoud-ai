import {test} from 'node:test';
import assert from 'node:assert/strict';
import {syncWorkers,applyWorkerEvent,mergeActivity,progressMarkup,changesMarkup} from '../src/worker-progress.ts';
const snapshot=(iteration=1)=>({iteration,steps:[{step:{key:'edit',title:'File edit',level:2,owned_paths:['new.txt']},status:'running',target:{model:'worker',reasoning:null},agent_name:'Sakura',child_id:'child',verification:[]}],children:[{id:'child',provider_thread:{id:'provider'},turn:{id:'current'}}]});
const event=(sequence,kind,text,turn_id='current')=>({sequence,event:{thread_id:'provider',turn_id,item_id:'message',kind,text}});
test('current-turn progress replaces streamed text with the saved final message without duplicates',()=>{
 const p=syncWorkers(undefined,snapshot());const w=p.workers[0];
 applyWorkerEvent(w,event(1,'message_delta','editing '),true);
 applyWorkerEvent(w,event(2,'message_delta','new.txt'),true);
 applyWorkerEvent(w,event(3,'message_completed','stale','other'),true);
 assert.equal(w.streams.get('message'),'editing new.txt');assert.equal(w.events.length,0);
 const activity={workers:[{child_id:'child',events:[event(4,'message_completed','new.txt updated')]}],changes:[]};
 mergeActivity(p,activity,false);mergeActivity(p,activity,false);
 assert.equal(w.events.length,1);assert.equal(w.streams.size,0);
 assert.match(progressMarkup(p,s=>s),/new.txt updated/);
 assert.match(progressMarkup(p,s=>s),/Sakura · 難易度 2/);
 assert.equal(syncWorkers(p,snapshot(2)).workers[0].events.length,0);
});
test('worker text and paths are escaped, including streamed markup',()=>{
 const p=syncWorkers(undefined,snapshot());applyWorkerEvent(p.workers[0],event(1,'message_delta','<img src=x onerror=alert(1)>'),true);
 assert.ok(!progressMarkup(p,s=>s).includes('<img'));
 const diff=changesMarkup([{key:'edit',title:'<b>unsafe</b>',path:'/tmp/<path>',diff:null,error:'cannot read <script>'}],s=>s);
 assert.ok(!diff.includes('<script>'));assert.match(diff,/差分を取得できませんでした/);
});
