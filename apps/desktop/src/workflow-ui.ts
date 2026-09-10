import type {TaskFocus} from './task-focus';
import type {WorkerStep,Progress} from './worker-progress';
const esc=(s:string)=>s.replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]!));
export const tabLabels:Record<string,string>={Chat:'概要',Plan:'計画',Diff:'変更',Agents:'担当',Terminal:'ログ',Context:'参照情報',Usage:'使用量'};
export type Flow = {project:string;title:string;status:string;kind:'manifest'|'direct'|'legacy';hasTask:boolean;planningRequested:boolean;reviewRequested:boolean;iteration?:number;id?:string;version?:string|null;busy:boolean;importing:boolean;stopping:boolean;writing?:boolean;running?:boolean};
export function flowStage(f:Flow){
 if(f.kind==='legacy')return -1;
 if(f.kind==='direct')return !f.hasTask?0:f.status==='completed'?2:1;
 if(!f.hasTask)return f.planningRequested?1:0;
 return f.status==='completed'?4:['awaiting_review','review_rejected'].includes(f.status)?3:2;
}
export function flowMarkup(f:Flow){
 const index=flowStage(f),stages=f.kind==='direct'?['依頼','実行','完了']:['依頼','計画','実行','レビュー','完了'];
 const halted=['failed','interrupted','reconciliation_required'].includes(f.status);
 const action=(key:string,text:string,primary=false,disabled=false)=>`<button data-flow-action="${key}" ${key==='import'?'title="コピーした計画・レビューを取り込んで次へ（⌘⇧V / Ctrl+Shift+V）"':''} class="${primary?'primary':''}" ${f.busy||disabled?'disabled':''}>${text}</button>`;
 let next='',actions='';
 if(!f.project){next='作業するプロジェクトを選んで始めます。';actions=action('project','フォルダを開く',true);}
 else if(f.kind==='legacy'){next='旧方式の保存記録です。新しい依頼は新しいタスクで始めます。';}
 else if(f.kind==='direct'){
  next=!f.hasTask?'依頼先を選び、やりたいことを書いて送信。おまかせなら内容に応じて担当を決めます。':halted?'作業が中断しています。状態を確認して再開してください。':index===2?'実行が終了しました。変更と結果を確認し、続きの指示を入力できます。':f.running===false?'続きの指示を入力できます。':'依頼した内容を実行しています。進捗は概要、詳細はログで確認できます。';
  if(halted)actions=action('resume','再開・状態を確認',true);
  else if(f.hasTask&&index===1&&f.running!==false)actions=action('stop',f.stopping?'停止中…':'停止',false,f.stopping);
 }else if(!f.hasTask){
  next=f.planningRequested?'依頼をコピーしました。ChatGPTの入力欄に貼り付けて相談し、返ってきた計画をコピーしてください。':'作業画面で依頼を準備すると、対象プロジェクトの情報も一緒にコピーできます。';
  actions=action('write',f.planningRequested?'依頼を編集':'依頼を書く',!f.planningRequested)+action('import',f.importing?'取り込み中…':'コピーした計画を確認',f.planningRequested);
 }else if(halted){next='実行が中断しています。記録を確認し、状態を確認して再開してください。';actions=action('resume','再開・状態を確認',true);}
 else if(index===2){next=f.status==='queued'?'計画を取り込みました。workerの開始を待っています。':f.status==='stopping'?'停止処理中です。実行状態の確定を待っています。':f.iteration&&f.iteration>1?'修正内容を実装・検証しています。完了すると再レビューへ進みます。':'計画に沿って実装・検証しています。完了するとレビューへ進みます。';actions=action('stop',f.stopping||f.status==='stopping'?'停止中…':'停止',false,f.stopping||f.status==='stopping');}
 else if(index===3){
  next=f.status==='review_rejected'?'レビューは未合格です。指摘をChatGPTで確認し、修正指示または追加レビューを受け取ります。':f.reviewRequested?'レビュー依頼をコピー済みです。ChatGPTへ貼り付け、返ってきたレビュー結果を取り込んでください。':'実装・検証が終わりました。ChatGPTへレビューを依頼し、返ってきた結果を取り込みます。';
  const review=action('review',f.reviewRequested?'レビュー依頼を再コピー':'レビュー依頼をコピー',!f.reviewRequested,!f.version);
  const receive=action('import',f.importing?'取り込み中…':'コピーした結果を確認',f.reviewRequested);
  actions=f.reviewRequested?receive+review:review+receive;
  next+=' 内容を確認し、修正の実行または完了へ進めます。';
 }else{next='作業フォルダの成果物がレビューに合格しました。変更内容と保存先を確認できます。';actions=action('new','次のタスク',false);}
 return `<div class="flow-title" data-flow-kind="${f.kind}"><span>${esc(f.project||'プロジェクト未選択')}${f.kind==='direct'?' · タスク':f.kind==='legacy'?' · 保存記録':''}</span><strong title="${esc(f.title)}">${esc(f.title||'新しいタスク')}</strong>${f.id?`<small title="${esc(f.id)}">ID ${esc(f.id.slice(0,8))}${f.iteration&&f.iteration>1?' · 実行 '+f.iteration+'回目':''}</small>`:''}</div><div class="flow-body">${f.iteration&&f.iteration>1?`<p class="flow-cycle">${f.iteration}回目の実行 · 実装とレビューを繰り返して確認</p>`:''}${index>=0?`<ol class="flow-stages" aria-label="作業の工程">${stages.map((stage,i)=>`<li class="${i<index||f.hasTask&&index===stages.length-1?'done':''} ${i===index?'current':''} ${i===index&&halted?'halted':''}" ${i===index?'aria-current="step"':''}><span>${i+1}</span>${stage}</li>`).join('')}</ol>`:''}<div class="flow-next"><p role="status">${esc(next)}</p><div class="flow-actions" aria-label="現在の工程の操作">${actions}</div></div></div>`;
}
export function welcomeMarkup(hasProject:boolean,planning:boolean,request?:string){
 return `<section class="read-panel welcome-flow"><span class="welcome-kicker">${planning?'PLAN WITH CHATGPT':'NEW TASK'}</span><h2>${!hasProject?'作業するフォルダを開く':planning?'まず、進め方を相談する。':'何を進めますか？'}</h2><p>${!hasProject?'プロジェクトを選んだら、やりたいことを書くだけで始められます。':planning?'依頼を準備してChatGPTへ。返ってきた計画を確認してから、実行へ進みます。':'小さな修正も、調査が必要な依頼も。やりたいことをそのまま書いてください。'}</p>${!hasProject?'<button id="welcome-add" class="primary">フォルダを開く</button>':planning?`<div class="planning-receipt">${request?`<details id="pending-planning-request"><summary>前回コピーした依頼</summary><p class="request-goal">${esc(request)}</p></details><button data-flow-action="chat">ChatGPTで続きを相談</button>`:''}<button data-flow-action="import">${request?'コピーした計画を確認':'すでに計画がある場合は取り込む'}</button></div>`:''}</section>`;
}
export type ExecutionRecord = {focus:TaskFocus;steps:WorkerStep[];artifactPath?:string;review?:{summary:string;findings:string[];verdict:string};scope?:string[]};
const list=(items:string[]|undefined)=>items?.length?`<ul>${items.map(s=>`<li>${esc(s)}</li>`).join('')}</ul>`:'<p class="muted">記録されていません。</p>';
export function executionPlan(r:ExecutionRecord,label:(s:string)=>string){
 return `<section class="read-panel"><h2>取り込んだ計画</h2><p>${esc(r.focus.goal)}</p><h3>完了の条件</h3>${list(r.focus.acceptance)}<ol class="plan-steps">${r.steps.map(s=>`<li><div class="read-row"><strong>${esc(s.step.title)}</strong><span>${esc(label(s.status))}</span></div><p>${esc(s.step.goal||'')}</p><p class="muted">対象: ${esc(s.step.owned_paths?.join('、')||'記録なし')}</p><p class="muted">前提: ${esc(s.step.dependencies?.map(key=>r.steps.find(d=>d.step.key===key)?.step.title||key).join('、')||'先行作業なし')}</p><details id="plan-step-${esc(s.step.key)}"><summary>完了条件と実行設定</summary>${list(s.step.acceptance)}<p>難易度 ${s.step.level} · ${esc(s.target.model)}${s.target.reasoning?' / '+esc(s.target.reasoning):''}</p></details></li>`).join('')}</ol><p class="muted">計画の相談・修正はChatGPTのチャットで行います。</p></section>`;
}
export function executionContext(r:ExecutionRecord){
 return `<section class="read-panel"><h2>担当に渡した情報</h2><p class="muted">このタスクの実行時に保存した指示です。</p>${r.steps.map(s=>{
 let capsule:Record<string,unknown>|undefined;try{capsule=s.capsule?JSON.parse(s.capsule):undefined;}catch{/* Some local worker prompts are plain text. */}
 return `<article class="read-card"><h3>${esc(s.step.title)}</h3>${!s.capsule?'<p class="muted">まだ指示を送っていません。</p>':`<p>${esc(s.step.goal||r.focus.goal)}</p><p class="muted">担当範囲: ${esc(s.step.owned_paths?.join('、')||'記録なし')}</p>${typeof capsule?.revision_instruction==='string'?`<h4>今回の修正指示</h4><p>${esc(capsule.revision_instruction)}</p>`:''}<details id="context-${esc(s.step.key)}"><summary>実際に渡した指示の全文</summary><pre class="read-output" tabindex="0" data-scroll-key="context-${esc(s.step.key)}">${esc(capsule?Object.entries(capsule).map(([k,v])=>`${k}\n${typeof v==='string'?v:JSON.stringify(v,null,2)}`).join('\n\n'):s.capsule)}</pre></details>`}</article>`;
 }).join('')}</section>`;
}
export function executionUsage(r:ExecutionRecord){
 return `<section class="read-panel"><h2>このタスクの使用量</h2><p class="muted">各担当の最新の保存記録です。Codexはセッション累計（以前の実行を含む場合があります）、ローカルは生成1回分です。時間はCodexのターン経過時間、ローカルの処理時間です。「—」は未取得。契約枠の残量ではありません。</p><table><thead><tr><th>担当 / モデル</th><th>入力</th><th>出力</th><th>時間</th></tr></thead><tbody>${r.steps.map(s=>{
 const u=s.usage?.at(-1)||{};
 const values=(typeof u.usage==='object'&&u.usage!==null?u.usage:u) as Record<string,unknown>;
 const number=(...keys:string[])=>{const v=keys.map(k=>values[k]).find(v=>typeof v==='number');return typeof v==='number'?String(v):'—';};
 return `<tr><td>${esc(s.step.title)}<small>${esc(s.target.model)}</small></td><td>${number('total_input_tokens','prompt_tokens','input_tokens')}</td><td>${number('total_output_tokens','completion_tokens','output_tokens')}</td><td>${typeof values.latency_ms==='number'?(values.latency_ms/1000).toFixed(1)+' s':'—'}</td></tr>`;
 }).join('')}</tbody></table></section>`;
}
export function executionOverview(r:ExecutionRecord,p:Progress|undefined,label:(s:string)=>string){
 const complete=r.steps.filter(s=>s.status==='completed').length;
 return `<section class="read-panel"><h2>今回の依頼</h2><p class="request-goal">${esc(r.focus.goal)}</p><div class="read-row"><h3>進捗</h3><span>${complete} / ${r.steps.length} 担当完了</span></div>${p?.error?`<p class="progress-error">進捗を更新できませんでした。保存済みの表示です。${esc(p.error)}</p>`:''}<ul class="progress-summary">${r.steps.map(s=>{const w=p?.workers.find(w=>w.step.step.key===s.step.key);const latest=[...(w?.streams.values()||[])].join('\n')||w?.events.filter(e=>['message_completed','item_started','item_completed','error'].includes(e.event.kind)).at(-1)?.event.text||s.output||'';return `<li><div class="read-row"><strong>${esc(s.step.title)}</strong><span>${esc(label(s.status))}</span></div>${s.error?`<p class="progress-error">${esc(s.error)}</p>`:''}<p class="latest-progress">${esc(latest.split('\n').filter(Boolean).slice(0,3).join('\n')||'開始待ち')}</p></li>`;}).join('')}</ul>${r.review?`<article class="review-summary ${r.review.verdict==='pass'?'review-passed':'review-attention'}"><h3>レビュー結果 · ${esc(({pass:'合格',fail:'修正が必要',inconclusive:'判定保留'} as Record<string,string>)[r.review.verdict]||r.review.verdict)}</h3><p>${esc(r.review.summary)}</p><details id="review-findings"><summary>指摘・確認事項（${r.review.findings.length}件）</summary>${list(r.review.findings)}</details></article>`:''}<details id="overview-acceptance"><summary>完了条件（${r.focus.acceptance?.length||0}件）</summary>${list(r.focus.acceptance)}</details><details id="overview-ids"><summary>作業ID・保存先</summary><p>Task ID: ${esc(r.focus.id)}</p>${r.focus.manifestId?`<p>実行依頼 ID: ${esc(r.focus.manifestId)}</p>`:''}${r.focus.reviewId?`<p>レビュー結果 ID: ${esc(r.focus.reviewId)}</p>`:''}<p>${r.artifactPath?'成果物: '+esc(r.artifactPath):'成果物は準備中です。'}</p></details></section>`;
}
