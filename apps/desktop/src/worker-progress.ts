export type ActivityEvent = {sequence:number;event:{thread_id:string|null;turn_id:string|null;item_id:string|null;kind:string;text:string;details?:{type:string;command?:string;exit_code?:number|null}|null}};
export type WorkerStep = {step:{key:string;title:string;level:number;owned_paths?:string[];goal?:string;dependencies?:string[];acceptance?:string[]};status:string;target:{model:string;reasoning:string|null};agent_name?:string;child_id?:string|null;output?:string|null;error?:string|null;capsule?:string|null;usage?:Record<string,unknown>[];verification:{status?:string;command?:string;exit_code?:number|null;target_revision?:string;log_ref?:string;output?:string}[]};
export type WorktreeChange = {key:string;title:string;path:string;diff:string|null;error:string|null};
export type Activity = {workers:{child_id:string;events:ActivityEvent[]}[];changes:WorktreeChange[]};
export type WorkerView = {step:WorkerStep;providerThread?:string;turn?:string;events:ActivityEvent[];streams:Map<string,string>};
export type Progress = {iteration:number;workers:WorkerView[];changes:WorktreeChange[];updatedAt?:number;error?:string};
const esc=(s:string)=>s.replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]!));
export function syncWorkers(old:Progress|undefined,snapshot:{iteration?:number;steps:WorkerStep[];children?:{id:string;provider_thread?:{id:string}|null;turn?:{id:string}|null}[]}):Progress {
 const iteration=snapshot.iteration||0;
 return {iteration,changes:old?.iteration===iteration?old.changes:[],workers:snapshot.steps.map(step=>{
  const child=snapshot.children?.find(c=>c.id===step.child_id);
  const prior=old?.iteration===iteration?old.workers.find(w=>w.step.step.key===step.step.key&&w.turn===child?.turn?.id):undefined;
  return {step,providerThread:child?.provider_thread?.id,turn:child?.turn?.id,events:prior?.events||[],streams:prior?.streams||new Map()};
 }),updatedAt:old?.updatedAt,error:old?.error};
}
export function applyWorkerEvent(worker:WorkerView,record:ActivityEvent,live=false){
 const e=record.event;
 if(worker.turn&&e.turn_id&&worker.turn!==e.turn_id)return;
 const key=e.item_id||e.turn_id||'message';
 if(live&&e.kind==='message_delta')worker.streams.set(key,((worker.streams.get(key)||'')+e.text).slice(-16000));
 if(e.kind==='message_completed')worker.streams.delete(key);
 if(['message_completed','item_started','item_completed','error','turn_started','turn_completed','diff'].includes(e.kind)&&!worker.events.some(r=>r.sequence===record.sequence)){
  worker.events.push(record);worker.events.sort((a,b)=>a.sequence-b.sequence);worker.events=worker.events.slice(-60);
 }
 if(worker.streams.size>8)worker.streams.delete(worker.streams.keys().next().value!);
}
export function mergeActivity(progress:Progress,activity:Activity,includeChanges:boolean){
 for(const source of activity.workers){const worker=progress.workers.find(w=>w.step.child_id===source.child_id);if(worker)for(const record of source.events)applyWorkerEvent(worker,record);}
 if(includeChanges)progress.changes=activity.changes;
 progress.updatedAt=Date.now();progress.error=undefined;
}
function eventMarkup(record:ActivityEvent,prefix:string){
 const e=record.event, d=e.details;
 const label=e.kind==='message_completed'?'進捗メッセージ':e.kind==='error'?'エラー':e.kind==='item_started'?'開始':e.kind==='item_completed'?'結果':e.kind==='turn_completed'?'実行終了':e.kind==='turn_started'?'実行開始':'ファイル変更';
 const command=d?.type==='command'?d.command:undefined;
 const title=command||e.text.split('\n')[0]||label;
 return `<details class="worker-event" id="${esc(prefix)}-event-${record.sequence}"><summary><span>${label}${d?.type==='command'&&e.kind==='item_completed'?` · 終了コード ${d.exit_code??'未取得'}`:''}</span><code title="${esc(title)}">${esc(title)}</code></summary><pre tabindex="0" aria-label="出力・枠内をスクロール" data-scroll-key="${esc(prefix)}-output-${record.sequence}">${command?esc(command)+'\n\n':''}${esc(e.text.slice(0,6000))}${e.text.length>6000?'\n（表示は先頭6000文字まで）':''}</pre></details>`;
}
export function progressMarkup(progress:Progress|undefined,label:(s:string)=>string,allEvents=false){
 if(!progress)return '<p class="muted">workerの進捗を取得中…</p>';
 return `<section class="worker-progress"><div class="progress-heading"><h2>${allEvents?'workerの実行ログ':'workerの進捗'}</h2><small>${progress.updatedAt?'更新 '+new Date(progress.updatedAt).toLocaleTimeString('ja-JP'):'取得中…'}</small></div>${progress.error?`<p class="progress-error" role="status">進捗の更新に失敗しました。保存済みの表示を維持し、再試行します。${esc(progress.error)}</p>`:''}${progress.workers.map(w=>{
 const messages=w.events.filter(r=>['message_completed','item_started','item_completed','error'].includes(r.event.kind));
 const latest=messages.at(-1),streams=[...w.streams.values()].join('\n');
 const logEvents=w.events.filter(r=>!(['item_started','item_completed'].includes(r.event.kind)&&['reasoning','agentMessage'].includes(r.event.text.trim())&&!r.event.details?.command));
 return `<article class="worker-card"><header><strong>${esc(w.step.step.title)}</strong><span>${esc(label(w.step.status))}</span></header><p class="worker-model">${w.step.agent_name?`${esc(w.step.agent_name)} · `:''}難易度 ${w.step.step.level} · ${esc(w.step.target.model)}${w.step.target.reasoning?' / '+esc(w.step.target.reasoning):''}</p><p class="worker-paths">対象: ${esc(w.step.step.owned_paths?.join('、')||'未取得')}</p>${w.step.error?`<p class="progress-error">${esc(w.step.error)}</p>`:''}${!allEvents&&streams?`<pre class="worker-stream" tabindex="0" aria-label="進捗・枠内をスクロール" data-scroll-key="stream-${esc(w.step.step.key)}">${esc(streams)}</pre>`:!allEvents&&latest?eventMarkup(latest,`${w.step.step.key}-latest`):allEvents?'':`<p class="muted">${w.step.status==='running'?'実行中です。新しい進捗通知を待っています。':'進捗通知はまだありません。'}</p>`}${allEvents?`<details id="worker-log-${esc(w.step.step.key)}" open><summary>実行履歴（直近${logEvents.length}件）</summary>${logEvents.map(e=>eventMarkup(e,w.step.step.key)).join('')}</details>`:''}${allEvents&&w.step.verification.length?`<details id="verification-${esc(w.step.step.key)}"><summary>Localoudの検証記録（${w.step.verification.length}件）</summary>${w.step.verification.map((v,i)=>`<div class="verification-record"><p>${esc(v.status||'未判定')} · ${esc(v.command||'ファイル照合')} · 終了コード ${v.exit_code??'対象外／未取得'}</p>${v.output?`<pre tabindex="0" data-scroll-key="verification-${esc(w.step.step.key)}-${i}">${esc(v.output)}</pre>`:''}</div>`).join('')}</details>`:''}${!allEvents&&w.step.output?`<details id="worker-result-${esc(w.step.step.key)}"><summary>workerの完了報告</summary><pre tabindex="0" aria-label="完了報告・枠内をスクロール" data-scroll-key="result-${esc(w.step.step.key)}">${esc(w.step.output)}</pre></details>`:''}</article>`;
 }).join('')||'<p>実行準備中です。</p>'}</section>`;
}
export function changesMarkup(changes:WorktreeChange[],renderDiff:(s:string)=>string){
 return `<div class="worker-changes"><h2>変更ファイルと差分</h2><p class="muted">実際の作業フォルダを参照しています。実行中はworker別、統合後は統合成果物を表示します。各差分はその作業フォルダの基準revisionとの比較です。</p>${changes.map(c=>`<section><h3>${esc(c.title)}</h3><p class="worker-paths">${esc(c.path)}</p>${c.error?`<p class="progress-error">差分を取得できませんでした: ${esc(c.error)}</p>`:c.diff?c.diff.split(/(?=^diff --git )/m).filter(Boolean).map((patch,i)=>`<details id="change-${esc(c.key)}-${i}" open><summary>${esc(patch.split('\n').find(l=>l.startsWith('+++ '))?.slice(4).replace(/^b\//,'')||patch.split('\n')[0])}</summary><div class="patch-scroll" tabindex="0" aria-label="差分・枠内をスクロール" data-scroll-key="patch-${esc(c.key)}-${i}">${renderDiff(patch)}</div></details>`).join(''):'<p>この作業フォルダに変更はありません。</p>'}</section>`).join('')||'<p>作業フォルダの準備中です。</p>'}</div>`;
}
