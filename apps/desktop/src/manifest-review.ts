import {taskGraph,bindTaskGraphs} from './task-graph.ts';
type Target = {provider:'local'|'codex'|'api';profile_id?:string|null;model:string;reasoning:string|null};
type Choice = {key:string;label:string;model:string;profile_id?:string;local:boolean;reasoning:string[]};
type Step = {dependencies?:string[];key:string;title:string; goal:string; level:number; owned_paths:string[]; acceptance:string[];target_override?:Target|null};
type Manifest = {kind:'task'; project_id:string; request:string; scope:string[]; acceptance:string[]; steps:Step[]} | {kind:'review'; project_id:string; task_id:string; verdict:'pass'|'fail'|'inconclusive'; summary:string; findings:string[]; changes:{step_key:string;instruction:string}[]};
type Invoke = <T = unknown>(command:string,args?:Record<string,unknown>)=>Promise<T>;
const esc=(s:string)=>s.replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]!));
const list=(items:string[])=>`<ul>${items.map(s=>`<li>${esc(s)}</li>`).join('')}</ul>`;

export function setupManifestReview(call:Invoke,commit:(text:string,projectId:string)=>Promise<void>,models:()=>Choice[]=()=>[]){
 const dialog=document.createElement('dialog');dialog.id='manifest-review';dialog.setAttribute('aria-labelledby','manifest-review-title');
 dialog.innerHTML='<div class="manifest-heading"><span id="manifest-project"></span><h2 id="manifest-review-title">ChatGPTの回答を確認</h2></div><div id="manifest-preview"></div><p id="manifest-error" role="alert"></p><div class="dialog-actions"><button id="manifest-cancel" type="button">戻る</button><button id="manifest-confirm" type="button" class="primary" disabled>この計画で実行</button></div>';
 document.body.append(dialog);
 const preview=dialog.querySelector<HTMLElement>('#manifest-preview')!,error=dialog.querySelector<HTMLElement>('#manifest-error')!,confirm=dialog.querySelector<HTMLButtonElement>('#manifest-confirm')!,cancel=dialog.querySelector<HTMLButtonElement>('#manifest-cancel')!;
 let captured:{manifest:Manifest;projectId:string}|undefined,working=false,generation=0,onCancel=()=>{};
 const profile=(choice:Choice)=>choice.profile_id||(choice.local?'spark':'codex');
 const addOption=(select:HTMLSelectElement,label:string,value:string)=>{const option=document.createElement('option');option.textContent=label;option.value=value;select.add(option);};
 const target=(choice:Choice,reasoning:string|null):Target=>({provider:profile(choice)==='spark'?'local':profile(choice)==='codex'?'codex':'api',profile_id:choice.profile_id||profile(choice),model:choice.model,reasoning});
 const selectedManifest=()=>{
  if(!captured||captured.manifest.kind!=='task')return captured?.manifest;
  return {...captured.manifest,steps:captured.manifest.steps.map((step,index)=>{
   const model=dialog.querySelector<HTMLSelectElement>(`[data-step-model="${index}"]`);
   const reasoning=dialog.querySelector<HTMLSelectElement>(`[data-step-reasoning="${index}"]`);
   const choice=models().find(candidate=>candidate.key===model?.value);
   return {...step,target_override:choice?target(choice,reasoning?.value||null):null};
  })};
 };
 const refreshRaw=()=>{const raw=dialog.querySelector<HTMLElement>('#manifest-raw');const selected=selectedManifest();if(raw&&selected)raw.textContent=JSON.stringify(selected,null,2);};
 const dismiss=()=>{if(working)return;captured=undefined;generation++;dialog.close();onCancel();};
 cancel.onclick=dismiss;
 dialog.addEventListener('cancel',e=>{e.preventDefault();dismiss();});
 confirm.onclick=async()=>{
  if(!captured||working)return;working=true;confirm.disabled=true;cancel.disabled=true;error.textContent='';
  try{const selected=selectedManifest();if(!selected)throw Error('取り込む内容がありません。');await commit(JSON.stringify(selected),captured.projectId);captured=undefined;dialog.close();}
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
    preview.innerHTML=`<p class="manifest-goal">${esc(m.request)}</p>${taskGraph(m.steps.map(step=>({step,status:'pending'})),{preview:true,id:'preview-graph'})}<h3>変更する範囲</h3>${list(m.scope)}<h3>完了の条件</h3>${list(m.acceptance)}<h3>実行する作業 · ${m.steps.length}件</h3><ol>${m.steps.map((s,index)=>`<li><strong>${esc(s.title)}</strong><p>${esc(s.goal)}</p><small>${esc(s.owned_paths.join('、'))}</small><label>実行モデル<select data-step-model="${index}"><option value="">難易度 ${s.level} の設定を使用</option></select></label><label>Reasoning<select data-step-reasoning="${index}" disabled><option value="">既定</option></select></label><details><summary>この作業の完了条件</summary>${list(s.acceptance)}</details></li>`).join('')}</ol><p class="muted">明示したモデルはこの実行だけに固定されます。未指定の作業は難易度別の設定を使います。</p>`;
    bindTaskGraphs(preview);
    m.steps.forEach((step,index)=>{
     const model=dialog.querySelector<HTMLSelectElement>(`[data-step-model="${index}"]`)!;
     const reasoning=dialog.querySelector<HTMLSelectElement>(`[data-step-reasoning="${index}"]`)!;
     for(const choice of models())addOption(model,choice.label,choice.key);
     const saved=models().find(choice=>step.target_override&&profile(choice)===(step.target_override.profile_id||(step.target_override.provider==='local'?'spark':'codex'))&&choice.model===step.target_override.model);
     if(saved)model.value=saved.key;
     const update=()=>{const choice=models().find(candidate=>candidate.key===model.value);reasoning.replaceChildren();addOption(reasoning,'既定','');if(choice)for(const effort of choice.reasoning)addOption(reasoning,effort,effort);reasoning.disabled=!choice||choice.local||!choice.reasoning.length;if(saved===choice&&step.target_override?.reasoning)reasoning.value=step.target_override.reasoning;refreshRaw();};
     model.addEventListener('change',update);reasoning.addEventListener('change',refreshRaw);update();
    });
    confirm.textContent='この計画で実行';
   }else{
    dialog.querySelector('#manifest-review-title')!.textContent='レビュー結果を確認';
    preview.innerHTML=`<p class="manifest-goal">${esc(m.summary)}</p><p>判定: ${{pass:'合格',fail:'修正が必要',inconclusive:'判定保留'}[m.verdict]}</p><p>対象タスク: <code>${esc(m.task_id)}</code></p>${m.findings.length?'<h3>指摘・確認事項</h3>'+list(m.findings):''}${m.changes.length?'<h3>実行する修正</h3>'+list(m.changes.map(c=>`${c.step_key}: ${c.instruction}`)):''}`;
    confirm.textContent=m.verdict==='pass'?'合格を記録して完了':m.changes.length?'この修正を実行':'結果を記録';
   }
   preview.insertAdjacentHTML('beforeend',`<details><summary>取り込むデータの全文</summary><pre id="manifest-raw">${esc(JSON.stringify(m,null,2))}</pre></details>`);
   captured={manifest:JSON.parse(JSON.stringify(m)) as Manifest,projectId:project.id};refreshRaw();confirm.disabled=false;
  }catch(e){if(version===generation&&dialog.open){preview.textContent='実行用JSONを確認できませんでした。ChatGPTの回答にTaskManifestまたはReviewManifestのJSONコードブロックがあることを確認し、回答のコピーボタンでもう一度コピーしてください。計画の文章だけでは実行を開始しません。';error.textContent=String(e);}}
 };
}
