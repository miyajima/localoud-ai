import {test} from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';
import {graphLayers,taskGraph,bindTaskGraphs} from '../src/task-graph.ts';
const node=(key,dependencies=[],status='pending')=>({step:{key,title:key,dependencies,owned_paths:[`src/${key}`]},status});
const fork=()=>[node('start',[],'completed'),node('left',['start'],'running'),node('right',['start'],'running'),node('join',['left','right'])];
test('graph preserves fork/join and rejects invalid or disconnected cyclic dependencies',()=>{
 assert.deepEqual(graphLayers(fork()).map(l=>l.map(n=>n.step.key)),[['start'],['left','right'],['join']]);
 for(const nodes of [[node('a',['missing'])],[node('a'),node('a')],[node('a'),node('b',['c']),node('c',['b'])]])assert.throws(()=>graphLayers(nodes));
 assert.deepEqual(graphLayers([]),[]);
 assert.match(taskGraph([]),/計画の作業と依存関係/);
});
test('preview is not running; actual concurrency and blocked predecessors are readable without color',()=>{
 const nodes=fork();
 const preview=taskGraph(nodes,{preview:true});
 assert.match(preview,/並列に進められる分岐あり/);assert.ok(!preview.includes('2件を実行中'));
 const html=taskGraph(nodes);
 assert.match(html,/2件を実行中/);assert.match(html,/1 \/ 4件完了/);assert.match(html,/完了待ち: left、right/);
 nodes[1].status='failed';nodes[1].error='<script>bad</script>';
 assert.match(taskGraph(nodes,{status:'failed'}),/&lt;script&gt;/);
 assert.match(taskGraph([node('a')],{status:'interrupted'}),/タスクが停止中/);
 assert.match(taskGraph([node('a',[],'completed')]),/全作業の実行完了/);
});
test('preview graph can expose per-step execution target controls',()=>{
 const html=taskGraph([{step:{key:'build',title:'ビルド',level:3,owned_paths:['src']},status:'pending'}],{preview:true,targetControls:true});
 assert.match(html,/data-step-model="build"/);assert.match(html,/data-step-reasoning="build"/);assert.match(html,/モデル／リーズニング/);
});
test('selecting a node exposes its full detail without losing the graph or invoking work',()=>{
 const nodes=fork();nodes[3].step.title='合流 <img src=x> 長い作業名';
 const dom=new JSDOM(taskGraph(nodes));
 try{
  const doc=dom.window.document;bindTaskGraphs(doc);
  doc.querySelector('#execution-graph-node-3').click();
  assert.equal(doc.querySelectorAll('details[open]').length,1);
  assert.equal(doc.querySelector('#execution-graph-detail-3').open,true);
  assert.equal(doc.querySelector('#execution-graph-node-3').getAttribute('aria-expanded'),'true');
  assert.equal(doc.querySelector('img'),null);
  assert.equal(doc.querySelectorAll('.graph-edge').length,8);
  assert.match(doc.querySelector('details[open]').textContent,/完了待ち: left、right/);
 }finally{dom.window.close();}
});
