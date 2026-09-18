/** A view of the scheduler's saved dependency graph, never an execution authority. */
export type GraphStep = {key:string;title:string;dependencies?:string[];goal?:string;owned_paths?:string[];level?:number};
export type GraphNode = {step:GraphStep;status:string;target?:{model:string;reasoning?:string|null};error?:string|null};
const esc=(s:string)=>s.replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]!));
const active=(s:string)=>['running','dispatching','integrating','inProgress'].includes(s);
const attention=(s:string)=>['failed','interrupted','cancelled','reconciliation_required','needs_attention'].includes(s);
const labels:Record<string,string>={running:'実行中',dispatching:'準備中',integrating:'統合中',inProgress:'実行中',completed:'完了',failed:'失敗',interrupted:'中断',cancelled:'中止',reconciliation_required:'状態確認が必要',needs_attention:'確認が必要',blocked:'依存待ち',pending:'待機',queued:'実行待ち',ready:'実行待ち'};

export function graphLayers(nodes:GraphNode[]):GraphNode[][] {
 const keys=new Set(nodes.map(n=>n.step.key));
 if(keys.size!==nodes.length||nodes.some(n=>(n.step.dependencies||[]).some(d=>!keys.has(d))))throw Error('依存関係に重複した作業IDまたは存在しない作業があります。');
 const done=new Set<string>(),layers:GraphNode[][]=[];
 while(done.size<nodes.length){
  const layer=nodes.filter(n=>!done.has(n.step.key)&&(n.step.dependencies||[]).every(d=>done.has(d)));
  if(!layer.length)throw Error('依存関係が循環しています。計画を確認してください。');
  layers.push(layer);layer.forEach(n=>done.add(n.step.key));
 }
 return layers;
}
export function taskGraph(nodes:GraphNode[],options:{preview?:boolean;status?:string;id?:string;targetControls?:boolean}={}):string {
 const id=options.id||'execution-graph',preview=!!options.preview;
 if(!nodes.length)return '<section class="task-graph"><h3>タスクグラフ</h3><p>計画の作業と依存関係が届くと、実行順序を表示します。</p></section>';
 let layers:GraphNode[][];
 try{layers=graphLayers(nodes);}catch(e){return `<section class="task-graph"><h3>タスクグラフ</h3><p class="progress-error">${esc((e as Error).message)}</p></section>`;}
 const running=nodes.filter(n=>active(n.status)),done=nodes.filter(n=>n.status==='completed').length;
 const parallel=layers.some(l=>l.length>1);
 const stalled=attention(options.status||'')||options.status==='stopping';
 const initial=running[0]||nodes.find(n=>attention(n.status))||nodes.find(n=>n.status!=='completed')||nodes[0];
 const state=(n:GraphNode)=>preview?'実行前':labels[n.status]||n.status;
 const tone=(n:GraphNode)=>preview?'waiting':active(n.status)?'active':attention(n.status)?'attention':n.status==='completed'?'done':'waiting';
 const reason=(n:GraphNode)=>{
  const dependencies=(n.step.dependencies||[]).map(k=>nodes.find(d=>d.step.key===k)!);
  if(preview)return dependencies.length?`先に完了する作業: ${dependencies.map(d=>d.step.title).join('、')}`:'先行作業なし。最初の実行候補です。';
  if(active(n.status))return 'この担当を処理中です。実行先の空きを待つ時間を含みます。';
  if(n.status==='completed')return 'この作業は完了しています。';
  if(attention(n.status))return n.error||'作業の状態を確認してください。';
  const waiting=dependencies.filter(d=>d.status!=='completed');
  if(waiting.length)return `完了待ち: ${waiting.map(d=>d.step.title).join('、')}`;
  return stalled?'タスクが停止中です。状態を確認して再開してください。':running.length?'依存先は完了。現在の実行組が終わると、次の実行候補になります。':'依存先は完了。開始を待っています。';
 };
 const rows=Math.max(...layers.map(l=>l.length)),height=rows*112+40,width=layers.length*264-40;
 const positions=new Map<string,{x:number;y:number}>();
 layers.forEach((layer,col)=>layer.forEach((node,row)=>positions.set(node.step.key,{x:col*264,y:40+(rows-layer.length)*56+row*112})));
 const edges=nodes.flatMap(n=>(n.step.dependencies||[]).map(key=>{
  const from=positions.get(key)!,to=positions.get(n.step.key)!,x=from.x+224,y=from.y+44,end=to.y+44;
  return `<path class="graph-edge ${!preview&&active(n.status)?'graph-edge-active':''}" d="M${x},${y} C${x+20},${y} ${to.x-20},${end} ${to.x-5},${end}"/><path class="graph-edge ${!preview&&active(n.status)?'graph-edge-active':''}" d="m${to.x-10},${end-4} 5,4 -5,4"/>`;
 })).join('');
 const summary=preview?`${nodes.length}件の作業 · ${parallel?'並列に進められる分岐あり':'順番に実行'}`:`${running.length?`${running.length}件を実行中`:stalled?'実行を停止・要確認':done===nodes.length?'全作業の実行完了':'次の実行を待機'} · ${done} / ${nodes.length}件完了`;
 const guide=options.targetControls?'作業カードを選択すると、完了条件と実行先（モデル／リーズニング）を設定できます。':'矢印は先に完了する作業から次の作業へ。';
 return `<section class="task-graph" aria-label="タスクグラフ"><div class="graph-heading"><h3>タスクグラフ</h3><span ${preview?'':'role="status"'}>${esc(summary)}</span></div><p class="graph-guide">${guide} 同じ列は依存関係上の並列候補です。</p><div class="graph-scroll" tabindex="0" role="region" aria-label="実行順序。横にスクロールして全体を確認" data-scroll-key="${esc(id)}"><div class="graph-canvas" style="width:${width/16}rem;height:${height/16}rem"><svg aria-hidden="true" viewBox="0 0 ${width} ${height}" preserveAspectRatio="none">${edges}</svg>${layers.map((_,i)=>`<span class="graph-column" style="left:${i*264/16}rem">${i===0?'開始':'依存先の完了後'}${layers.length>1?` · ${i+1}`:''}</span>`).join('')}${nodes.map((n,index)=>{
 const p=positions.get(n.step.key)!;
 return `<button type="button" id="${esc(id)}-node-${index}" class="graph-node graph-${tone(n)}" style="left:${p.x/16}rem;top:${p.y/16}rem" data-graph-node aria-controls="${esc(id)}-detail-${index}" aria-expanded="${n===initial}" title="${esc(n.step.title+' · '+state(n))}"><span class="graph-state">${esc(state(n))}</span><strong>${esc(n.step.title)}</strong></button>`;
 }).join('')}</div></div><p class="graph-policy">最大3件ずつ実行。同じ対象への変更は順番に進めます。実行先の同時実行上限によって待機する場合があります。</p><div class="graph-details">${nodes.map((n,index)=>{const stepKey=n.step.key||String(index);return `<details id="${esc(id)}-detail-${index}" ${n===initial?'open':''}><summary>${esc(n.step.title)} · ${esc(state(n))}</summary><p class="graph-reason">${esc(reason(n))}</p>${n.step.goal?`<p>${esc(n.step.goal)}</p>`:''}<p class="graph-meta">対象: ${esc(n.step.owned_paths?.join('、')||'未記録')}${n.target?`<br>モデル: ${esc(n.target.model)}${n.target.reasoning?` / リーズニング: ${esc(n.target.reasoning)}`:''}`:''}</p>${options.targetControls?`<div class="graph-target-controls" data-graph-target="${esc(stepKey)}"><strong>この作業の実行先</strong><label>モデル<select data-step-model="${esc(stepKey)}"><option value="">難易度 ${n.step.level??'別'} の設定を使用</option></select></label><label>リーズニング<select data-step-reasoning="${esc(stepKey)}" disabled><option value="">既定</option></select></label></div>`:''}</details>`;}).join('')}</div></section>`;
}
export function bindTaskGraphs(root:ParentNode){
 root.querySelectorAll<HTMLElement>('.task-graph').forEach(graph=>{
  const buttons=Array.from(graph.querySelectorAll<HTMLButtonElement>('[data-graph-node]'));
  const details=Array.from(graph.querySelectorAll<HTMLDetailsElement>('.graph-details details'));
  const sync=()=>buttons.forEach(b=>b.setAttribute('aria-expanded',String(details.find(d=>d.id===b.getAttribute('aria-controls'))?.open||false)));
  buttons.forEach(b=>b.onclick=()=>{details.forEach(d=>d.open=d.id===b.getAttribute('aria-controls'));sync();});
  details.forEach(d=>d.addEventListener('toggle',sync));sync();
  const viewport=graph.querySelector<HTMLElement>('.graph-scroll');
  const selected=buttons.find(b=>b.getAttribute('aria-expanded')==='true');
  if(viewport&&selected&&viewport.scrollWidth>viewport.clientWidth)viewport.scrollLeft=Math.max(0,selected.offsetLeft-(viewport.clientWidth-selected.offsetWidth)/2);
 });
}
