import {fillModelSelect,fillReasoningSelect,modelForTarget,readTarget} from './model-controls';
import type {ModelChoice,ModelTarget} from './model-controls';
export type AutoSettings = {classifier:ModelTarget;levels:{level:number;target:ModelTarget}[];fallback:ModelTarget|null;confirm_before_run:boolean};
export type AutoPreview = {id:string;revision:number;level:number|null;risk:string|null;confidence:number|null;target:ModelTarget|null;reason:string;blocked:string|null;used_fallback:boolean;needs_plan:boolean;confirm_before_run:boolean;manual_override:boolean};
type Call = <T=unknown>(command:string,args?:Record<string,unknown>)=>Promise<T>;
type Dependencies = {call:Call;models:()=>ModelChoice[];refresh:()=>Promise<void>;busy:()=>boolean;run:(preview:AutoPreview)=>void;plan:()=>void;notice:(text:string)=>void};
let deps:Dependencies;
let current:AutoPreview|null=null, revising=false, saving=false;
const levels=[['ごく簡単','誤字・文言・明確な置換'],['簡単','局所的な修正、手順が明確'],['標準','複数ファイルの調査・変更・テスト'],['難しい','原因不明の不具合、設計判断'],['非常に難しい','大きな設計変更、複雑な依存関係']];
const select=(id:string)=>document.getElementById(id) as HTMLSelectElement;
const dialog=(id:string)=>document.getElementById(id) as HTMLDialogElement;
function write(prefix:string,target:ModelTarget|null){fillModelSelect(select(`${prefix}-model`),deps.models(),target);fillReasoningSelect(select(`${prefix}-reasoning`),modelForTarget(deps.models(),target),target?.reasoning||null);}
function read(prefix:string){const target=readTarget(select(`${prefix}-model`),select(`${prefix}-reasoning`),deps.models());if(!target)throw new Error('利用できるモデルと reasoning を選択してください。');return target;}
function controls(prefix:string){return `<div class="target-controls"><label for="${prefix}-model">モデル</label><select id="${prefix}-model"></select><label for="${prefix}-reasoning">Reasoning</label><select id="${prefix}-reasoning"></select></div>`;}
function updateFallback(){(document.getElementById('auto-fallback-controls') as HTMLElement).hidden=select('auto-fallback-mode').value==='stop';}
function showPreview(){
  if(!current)return;
  select('route-level').value=current.level?String(current.level):'';
  write('route',current.target);
  document.getElementById('route-reason')!.textContent=current.reason;
  document.getElementById('route-state')!.textContent=current.blocked || (current.used_fallback?'設定した代替モデルを使用します。':current.manual_override?'手動で調整した振り分けです。':'このモデルと reasoning で実行します。');
  (document.getElementById('route-run') as HTMLButtonElement).disabled=revising||!!current.blocked||!current.target;
  (document.getElementById('route-plan') as HTMLButtonElement).hidden=!current.needs_plan;
  (document.getElementById('route-fields') as HTMLFieldSetElement).disabled=revising;
}
async function revise(args:{level?:number;target?:ModelTarget}){
  if(!current||revising)return;
  const previous=current;revising=true;showPreview();document.getElementById('route-error')!.textContent='';
  try{const next=await deps.call<AutoPreview>('revise_auto_route',{routeId:previous.id,revision:previous.revision,level:args.level??null,target:args.target??null});if(dialog('auto-preview').open)current=next;}
  catch(e){document.getElementById('route-error')!.textContent=String(e);}
  finally{revising=false;if(dialog('auto-preview').open)showPreview();}
}
export function showAutoPreview(preview:AutoPreview){current=preview;document.getElementById('route-error')!.textContent='';showPreview();dialog('auto-preview').showModal();}
export async function openAutoSettings(){
  if(deps.busy()||saving)return;
  const panel=dialog('auto-settings-dialog');panel.showModal();document.getElementById('auto-settings-error')!.textContent='';
  const fields=document.getElementById('auto-settings-fields') as HTMLFieldSetElement;fields.disabled=true;
  try{
    await deps.refresh();
    const config=await deps.call<AutoSettings>('auto_settings');
    if(!panel.open)return;
    write('classifier',config.classifier);
    for(let i=1;i<=5;i++)write(`level-${i}`,config.levels.find(row=>row.level===i)?.target||null);
    select('auto-fallback-mode').value=config.fallback?'model':'stop';
    write('fallback',config.fallback||config.levels.find(row=>row.level===3)?.target||null);
    (document.getElementById('auto-confirm') as HTMLInputElement).checked=config.confirm_before_run;
    updateFallback();fields.disabled=false;
  }catch(e){document.getElementById('auto-settings-error')!.textContent=String(e);}
}
export function setupAutoRouting(value:Dependencies){
  deps=value;
  document.body.insertAdjacentHTML('beforeend',`<dialog id="auto-settings-dialog" class="auto-dialog"><form id="auto-settings-form"><h2>Auto の振り分け設定</h2><p>依頼文と対象ファイル名から難易度を推定し、設定したモデルへ振り分けます。モデルは複数の段階に割り当てられます。</p><fieldset id="auto-settings-fields"><legend class="sr-only">振り分け設定</legend><h3>難易度を判定するモデル</h3>${controls('classifier')}<h3>難易度ごとの実行モデル</h3><div class="auto-table-wrap"><table class="auto-table"><thead><tr><th>難易度・目安</th><th>モデルと Reasoning</th></tr></thead><tbody>${levels.map(([title,description],i)=>`<tr><th scope="row">${i+1} · ${title}<small>${description}</small></th><td>${controls(`level-${i+1}`)}</td></tr>`).join('')}</tbody></table></div><h3>判定できない・モデルが利用できない場合</h3><label for="auto-fallback-mode">扱い</label><select id="auto-fallback-mode"><option value="stop">停止して自分で選ぶ</option><option value="model">指定した代替モデルを使う</option></select><div id="auto-fallback-controls">${controls('fallback')}</div><label class="auto-check"><input id="auto-confirm" type="checkbox">実行前に振り分け結果を確認する</label><p class="muted">ローカル編集の範囲や、計画が必要な依頼の扱いは維持されます。初期値は難易度1がローカル、それ以外は接続先の既定モデルです。接続できない場合はローカルを表示します。</p><button type="submit" class="primary">保存する</button></fieldset><p id="auto-settings-error" role="alert"></p><div class="dialog-actions"><button id="auto-settings-close" type="button">閉じる</button></div></form></dialog><dialog id="auto-preview" class="auto-dialog"><h2>Auto の振り分け結果</h2><p>難易度とモデルを変更できます。変更しても依頼内容の制限は維持されます。</p><fieldset id="route-fields"><label for="route-level">難易度</label><select id="route-level"><option value="" disabled>未判定</option>${levels.map(([title],i)=>`<option value="${i+1}">${i+1} · ${title}</option>`).join('')}</select>${controls('route')}</fieldset><p id="route-state" role="status"></p><details open><summary>判定理由</summary><p id="route-reason" class="preserve-lines"></p></details><p id="route-error" role="alert"></p><div class="dialog-actions"><button id="route-close">閉じる</button><button id="route-plan" hidden>プランモードに切り替え</button><button id="route-run" class="primary">この設定で実行</button></div></dialog>`);
  for(const prefix of ['classifier','fallback',...levels.map((_,i)=>`level-${i+1}`)])select(`${prefix}-model`).addEventListener('change',()=>fillReasoningSelect(select(`${prefix}-reasoning`),deps.models().find(m=>m.key===select(`${prefix}-model`).value)));
  select('auto-fallback-mode').addEventListener('change',updateFallback);
  document.getElementById('auto-settings-close')!.onclick=()=>{if(!saving)dialog('auto-settings-dialog').close();};
  dialog('auto-settings-dialog').addEventListener('cancel',event=>{if(saving)event.preventDefault();});
  document.getElementById('auto-settings-form')!.addEventListener('submit',async event=>{
    event.preventDefault();if(saving)return;
    const fields=document.getElementById('auto-settings-fields') as HTMLFieldSetElement;
    try{
      const config:AutoSettings={classifier:read('classifier'),levels:levels.map((_,i)=>({level:i+1,target:read(`level-${i+1}`)})),fallback:select('auto-fallback-mode').value==='model'?read('fallback'):null,confirm_before_run:(document.getElementById('auto-confirm') as HTMLInputElement).checked};
      saving=true;fields.disabled=true;document.getElementById('auto-settings-error')!.textContent='';
      await deps.call('set_auto_settings',{config});dialog('auto-settings-dialog').close();deps.notice('Auto の振り分け設定を保存しました。次の判定から反映します。');
    }catch(e){document.getElementById('auto-settings-error')!.textContent=String(e);}finally{saving=false;fields.disabled=false;}
  });
  select('route-level').addEventListener('change',()=>void revise({level:Number(select('route-level').value)}));
  select('route-model').addEventListener('change',()=>{fillReasoningSelect(select('route-reasoning'),deps.models().find(m=>m.key===select('route-model').value));try{void revise({target:read('route')});}catch(e){document.getElementById('route-error')!.textContent=String(e);}});
  select('route-reasoning').addEventListener('change',()=>{try{void revise({target:read('route')});}catch(e){document.getElementById('route-error')!.textContent=String(e);}});
  document.getElementById('route-close')!.onclick=()=>dialog('auto-preview').close();
  document.getElementById('route-plan')!.onclick=()=>{dialog('auto-preview').close();deps.plan();};
  document.getElementById('route-run')!.onclick=()=>{if(!current||revising||current.blocked||!current.target)return;const chosen=current;current=null;dialog('auto-preview').close();deps.run(chosen);};
}
