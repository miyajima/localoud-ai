import {test} from 'node:test';
import assert from 'node:assert/strict';
import {flowStage,flowMarkup,executionUsage} from '../src/workflow-ui.ts';
const base={project:'Project',title:'Task',kind:'manifest',hasTask:true,planningRequested:false,reviewRequested:false,busy:false,importing:false,stopping:false};
test('workflow maps observed states, including interrupted execution and review rework',()=>{
 for(const [status,stage] of [['queued',2],['running',2],['failed',2],['interrupted',2],['awaiting_review',3],['review_rejected',3],['completed',4]])assert.equal(flowStage({...base,status}),stage);
 assert.match(flowMarkup({...base,status:'interrupted'}),/data-flow-action="resume"/);
 assert.ok(!flowMarkup({...base,status:'running'}).includes('data-flow-action="import"'));
 assert.ok(!flowMarkup({...base,status:'completed'}).includes('data-flow-action="review"'));
 assert.equal(flowStage({...base,kind:'direct',status:'idle'}),1);
 assert.equal(flowStage({...base,kind:'direct',hasTask:false,planningRequested:true}),0);
 assert.equal(flowStage({...base,status:'running',iteration:2}),2);
 assert.match(flowMarkup({...base,status:'running',iteration:2}),/修正内容を実装/);
 assert.match(flowMarkup({...base,status:'awaiting_review',iteration:2}),/2回目の実行/);
 assert.equal(flowStage({...base,kind:'direct',status:'completed'}),2);
 assert.equal(flowStage({...base,kind:'legacy'}),-1);
 assert.equal(flowStage({...base,hasTask:false,planningRequested:true}),1);
 assert.equal(flowStage({...base,hasTask:false}),0);
});

test('usage renders the latest cumulative provider report once per worker',()=>{
 const html=executionUsage({steps:[{step:{title:'Worker'},target:{model:'codex'},usage:[{total_input_tokens:10,total_output_tokens:2},{total_input_tokens:25,total_output_tokens:6}]},{step:{title:'Local'},target:{model:'local'},usage:[{usage:{prompt_tokens:3,completion_tokens:4,latency_ms:1500}}]}]});
 assert.equal((html.match(/<tr>/g)||[]).length,3);
 assert.match(html,/<td>25<\/td><td>6<\/td>/);
 assert.match(html,/<td>3<\/td><td>4<\/td><td>1.5 s<\/td>/);
 assert.match(html,/セッション累計/);
});
