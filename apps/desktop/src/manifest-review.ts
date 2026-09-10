type Step = {title:string; goal:string; owned_paths:string[]; acceptance:string[]};
type Manifest = {kind:'task'; project_id:string; request:string; scope:string[]; acceptance:string[]; steps:Step[]} | {kind:'review'; project_id:string; task_id:string; verdict:'pass'|'fail'|'inconclusive'; summary:string; findings:string[]; changes:{step_key:string;instruction:string}[]};
type Invoke = <T = unknown>(command:string,args?:Record<string,unknown>)=>Promise<T>;
const esc=(s:string)=>s.replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]!));
const list=(items:string[])=>`<ul>${items.map(s=>`<li>${esc(s)}</li>`).join('')}</ul>`;

export function setupManifestReview(call:Invoke,commit:(text:string,projectId:string)=>Promise<void>){
 const dialog=document.createElement('dialog');dialog.id='manifest-review';dialog.setAttribute('aria-labelledby','manifest-review-title');
 dialog.innerHTML='<div class="manifest-heading"><span id="manifest-project"></span><h2 id="manifest-review-title">コピーした計画を確認</h2></div><div id="manifest-preview"></div><p id="manifest-error" role="alert"></p><div class="dialog-actions"><button id="manifest-cancel" type="button">戻る</button><button id="manifest-confirm" type="button" class="primary" disabled>この計画で実行</button></div>';
 document.body.append(dialog);
 const preview=dialog.querySelector<HTMLElement>('#manifest-preview')!,error=dialog.querySelector<HTMLElement>('#manifest-error')!,confirm=dialog.querySelector<HTMLButtonElement>('#manifest-confirm')!,cancel=dialog.querySelector<HTMLButtonElement>('#manifest-cancel')!;
 let captured:{text:string;projectId:string}|undefined,working=false,generation=0,onCancel=()=>{};
 const dismiss=()=>{if(working)return;captured=undefined;generation++;dialog.close();onCancel();};
 cancel.onclick=dismiss;
 dialog.addEventListener('cancel',e=>{e.preventDefault();dismiss();});
 confirm.onclick=async()=>{
  if(!captured||working)return;working=true;confirm.disabled=true;cancel.disabled=true;error.textContent='';
  try{await commit(captured.text,captured.projectId);captured=undefined;dialog.close();}
  catch(e){error.textContent=String(e);}
  finally{working=false;confirm.disabled=!captured;cancel.disabled=false;}
 };
 return async(project:{id:string;name:string},returnToPrevious:()=>void)=>{
  if(dialog.open||working)return;
  captured=undefined;confirm.disabled=true;cancel.disabled=false;onCancel=returnToPrevious;
  error.textContent='';preview.textContent='クリップボードの内容を確認しています…';
  dialog.querySelector('#manifest-project')!.textContent=project.name;
  dialog.querySelector('#manifest-review-title')!.textContent='コピーした内容を確認';
  dialog.showModal();const version=++generation;
  try{
   const m=await call<Manifest>('manifest_preview_clipboard',{projectId:project.id});
   if(version!==generation||!dialog.open)return;
   if(m.project_id!==project.id)throw Error('選択中のプロジェクトと計画が一致しません。');
   if(m.kind==='task'){
    dialog.querySelector('#manifest-review-title')!.textContent='この計画で進めますか？';
    preview.innerHTML=`<p class="manifest-goal">${esc(m.request)}</p><h3>変更する範囲</h3>${list(m.scope)}<h3>完了の条件</h3>${list(m.acceptance)}<h3>実行する作業 · ${m.steps.length}件</h3><ol>${m.steps.map(s=>`<li><strong>${esc(s.title)}</strong><p>${esc(s.goal)}</p><small>${esc(s.owned_paths.join('、'))}</small><details><summary>この作業の完了条件</summary>${list(s.acceptance)}</details></li>`).join('')}</ol><p class="muted">実行を押すと、設定済みの振り分けに沿って担当が作業を開始します。</p>`;
    confirm.textContent='この計画で実行';
   }else{
    dialog.querySelector('#manifest-review-title')!.textContent='レビュー結果を確認';
    preview.innerHTML=`<p class="manifest-goal">${esc(m.summary)}</p><p>判定: ${{pass:'合格',fail:'修正が必要',inconclusive:'判定保留'}[m.verdict]}</p><p>対象タスク: <code>${esc(m.task_id)}</code></p>${m.findings.length?'<h3>指摘・確認事項</h3>'+list(m.findings):''}${m.changes.length?'<h3>実行する修正</h3>'+list(m.changes.map(c=>`${c.step_key}: ${c.instruction}`)):''}`;
    confirm.textContent=m.verdict==='pass'?'合格を記録して完了':m.changes.length?'この修正を実行':'結果を記録';
   }
   preview.insertAdjacentHTML('beforeend',`<details><summary>取り込むデータの全文</summary><pre>${esc(JSON.stringify(m,null,2))}</pre></details>`);
   captured={text:JSON.stringify(m),projectId:project.id};confirm.disabled=false;
  }catch(e){if(version===generation&&dialog.open){preview.textContent='ChatGPTが返した計画またはレビューのJSONをコピーし、もう一度開いてください。';error.textContent=String(e);}}
 };
}
