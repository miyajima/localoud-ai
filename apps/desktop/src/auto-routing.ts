import {fillModelSelect,fillReasoningSelect,modelForTarget,readTarget} from './model-controls';
import type {ModelChoice,ModelTarget} from './model-controls';

export type AgentProfile = {id:string;name:string;target:ModelTarget};
export type RouteAgentAssignment = {route:string;agent_id:string};
export type AutoSettings = {
  classifier:ModelTarget;
  levels:{level:number;target:ModelTarget}[];
  fallback:ModelTarget|null;
  planner_default:ModelTarget|null;
  reviewer_default:ModelTarget|null;
  agents:AgentProfile[];
  route_agents:RouteAgentAssignment[];
  confirm_before_run:boolean;
};
export type AutoPreview = {
  id:string;
  revision:number;
  level:number|null;
  risk:string|null;
  confidence:number|null;
  target:ModelTarget|null;
  route_key:string|null;
  agent_name:string|null;
  reason:string;
  blocked:string|null;
  used_fallback:boolean;
  needs_plan:boolean;
  confirm_before_run:boolean;
  manual_override:boolean;
};

type Call = <T=unknown>(command:string,args?:Record<string,unknown>)=>Promise<T>;
type Dependencies = {
  call:Call;
  models:()=>ModelChoice[];
  refresh:()=>Promise<void>;
  busy:()=>boolean;
  run:(preview:AutoPreview)=>void;
  plan:(preview:AutoPreview)=>void;
  notice:(text:string)=>void;
};

const levels=[
  ['ごく簡単','誤字・文言・明確な置換'],
  ['簡単','局所的な修正、手順が明確'],
  ['標準','複数ファイルの調査・変更・テスト'],
  ['難しい','原因不明の不具合、設計判断'],
  ['非常に難しい','大きな設計変更、複雑な依存関係'],
] as const;
const routeIds=['classifier','level_1','level_2','level_3','level_4','level_5','planner','reviewer','fallback'] as const;
type RouteId=(typeof routeIds)[number];
const routePrefixes:Record<RouteId,string>={
  classifier:'classifier',
  level_1:'level-1',
  level_2:'level-2',
  level_3:'level-3',
  level_4:'level-4',
  level_5:'level-5',
  planner:'planner-default',
  reviewer:'reviewer-default',
  fallback:'fallback',
};
const defaultNames:Record<RouteId,string>={
  classifier:'Router',
  level_1:'Quick',
  level_2:'Builder',
  level_3:'Developer',
  level_4:'Engineer',
  level_5:'Architect',
  planner:'Planner',
  reviewer:'Reviewer',
  fallback:'Fallback',
};

let deps:Dependencies;
let current:AutoPreview|null=null;
let revising=false;
let saving=false;
let agentSequence=0;
let draftAgents:AgentProfile[]=[];
let draftAssignments=new Map<string,string>();

const select=(id:string)=>document.getElementById(id) as HTMLSelectElement;
const dialog=(id:string)=>document.getElementById(id) as HTMLDialogElement;
const cloneTarget=(target:ModelTarget):ModelTarget=>({...target});

function write(prefix:string,target:ModelTarget|null){
  fillModelSelect(select(`${prefix}-model`),deps.models(),target);
  fillReasoningSelect(select(`${prefix}-reasoning`),modelForTarget(deps.models(),target),target?.reasoning||null);
}
function read(prefix:string){
  const target=readTarget(select(`${prefix}-model`),select(`${prefix}-reasoning`),deps.models());
  if(!target)throw new Error('利用できるモデルと reasoning を選択してください。');
  return target;
}
function controls(prefix:string){
  return `<div class="target-controls"><label for="${prefix}-model">モデル</label><select id="${prefix}-model"></select><label for="${prefix}-reasoning">Reasoning</label><select id="${prefix}-reasoning"></select></div>`;
}
function assignmentSelect(route:RouteId,label:string){
  const prefix=routePrefixes[route];
  return `<label class="sr-only" for="${prefix}-agent">${label}のエージェント</label><select id="${prefix}-agent" class="agent-assignment"></select>`;
}
function routeTarget(config:AutoSettings,route:RouteId):ModelTarget{
  if(route==='classifier')return config.classifier;
  if(route.startsWith('level_')){
    const level=Number(route.slice(6));
    return config.levels.find(row=>row.level===level)?.target||config.classifier;
  }
  if(route==='planner')return config.planner_default||config.levels.find(row=>row.level===5)?.target||config.classifier;
  if(route==='reviewer')return config.reviewer_default||config.planner_default||config.levels.find(row=>row.level===5)?.target||config.classifier;
  return config.fallback||config.levels.find(row=>row.level===3)?.target||config.classifier;
}
function normalizeAgents(config:AutoSettings){
  if(config.agents?.length&&config.route_agents?.length===routeIds.length){
    return {
      agents:config.agents.map(agent=>({...agent,target:cloneTarget(agent.target)})),
      assignments:new Map(config.route_agents.map(assignment=>[assignment.route,assignment.agent_id])),
    };
  }
  const agents:AgentProfile[]=[];
  const assignments=new Map<string,string>();
  for(const route of routeIds){
    const target=cloneTarget(routeTarget(config,route));
    const name=defaultNames[route];
    const existing=agents.find(agent=>agent.name===name&&JSON.stringify(agent.target)===JSON.stringify(target));
    const agent=existing||{id:`agent-${route.replace('_','-')}`,name,target};
    if(!existing)agents.push(agent);
    assignments.set(route,agent.id);
  }
  return {agents,assignments};
}
function agentRow(agent:AgentProfile,index:number){
  const prefix=`agent-${index}`;
  return `<section class="agent-row" data-agent-id="${agent.id}">
    <div class="agent-identity">
      <label for="${prefix}-name">エージェント名</label>
      <div class="agent-name-line">
        <input id="${prefix}-name" required maxlength="80" autocomplete="off">
        <button type="button" class="agent-delete">削除</button>
      </div>
    </div>
    ${controls(prefix)}
  </section>`;
}
function captureAgents(requireNames=false){
  const next=draftAgents.map((agent,index)=>{
    const name=(document.getElementById(`agent-${index}-name`) as HTMLInputElement)?.value.trim()||'';
    if(requireNames&&!name)throw new Error(`${index+1}件目のエージェント名を入力してください。`);
    const model=readTarget(select(`agent-${index}-model`),select(`agent-${index}-reasoning`),deps.models());
    if(!model)throw new Error(`${name||index+1}のモデルを選択してください。`);
    return {...agent,name,target:model};
  });
  draftAgents=next;
  return next;
}
function updateAssignmentOptions(){
  for(const route of routeIds){
    const field=select(`${routePrefixes[route]}-agent`);
    const previous=draftAssignments.get(route)||field.value;
    field.replaceChildren(...draftAgents.map(agent=>{
      const option=document.createElement('option');
      option.value=agent.id;
      option.textContent=`${agent.name||'名称未設定'} · ${agent.target.model}`;
      return option;
    }));
    const selected=draftAgents.some(agent=>agent.id===previous)?previous:draftAgents[0]?.id||'';
    field.value=selected;
    draftAssignments.set(route,selected);
  }
}
function setAgentStatus(text:string){
  document.getElementById('agent-change-status')!.textContent=text;
}
function renderAgents(focusId?:string){
  const list=document.getElementById('agent-list')!;
  list.innerHTML=draftAgents.map(agentRow).join('');
  draftAgents.forEach((agent,index)=>{
    const prefix=`agent-${index}`;
    const name=document.getElementById(`${prefix}-name`) as HTMLInputElement;
    name.value=agent.name;
    name.addEventListener('input',()=>{
      draftAgents[index].name=name.value;
      updateAssignmentOptions();
      remove.setAttribute('aria-label',`${name.value.trim()||index+1}を削除`);
    });
    write(prefix,agent.target);
    select(`${prefix}-model`).addEventListener('change',()=>fillReasoningSelect(
      select(`${prefix}-reasoning`),
      deps.models().find(model=>model.key===select(`${prefix}-model`).value),
    ));
    const remove=list.querySelectorAll<HTMLButtonElement>('.agent-delete')[index];
    remove.setAttribute('aria-label',`${agent.name||index+1}を削除`);
    remove.addEventListener('click',()=>{
      captureAgents();
      if(draftAgents.length===1){
        document.getElementById('auto-settings-error')!.textContent='少なくとも1件のエージェントが必要です。';
        return;
      }
      const removed=draftAgents[index];
      draftAgents.splice(index,1);
      for(const route of routeIds){
        if(draftAssignments.get(route)===removed.id)draftAssignments.set(route,draftAgents[0].id);
      }
      renderAgents();
      updateAssignmentOptions();
      setAgentStatus(`${removed.name||'エージェント'}を削除し、必要な割り当てを${draftAgents[0].name}へ変更しました。`);
    });
  });
  document.getElementById('agent-count')!.textContent=`${draftAgents.length}件`;
  if(focusId){
    const index=draftAgents.findIndex(agent=>agent.id===focusId);
    (document.getElementById(`agent-${index}-name`) as HTMLInputElement|null)?.focus();
  }
}
function addAgent(){
  captureAgents();
  if(draftAgents.length>=50){
    document.getElementById('auto-settings-error')!.textContent='エージェントは50件まで追加できます。';
    return;
  }
  const last=draftAgents.at(-1);
  if(!last)throw new Error('エージェントを読み込めませんでした。');
  const id=`agent-custom-${Date.now().toString(36)}-${++agentSequence}`;
  draftAgents.push({id,name:`Agent ${draftAgents.length+1}`,target:cloneTarget(last.target)});
  renderAgents(id);
  updateAssignmentOptions();
  setAgentStatus('新しいエージェントを追加しました。名前とモデルを設定してください。');
}
function updateFallback(){
  document.getElementById('auto-fallback-controls')!.hidden=select('auto-fallback-mode').value==='stop';
}
function showPreview(){
  if(!current)return;
  select('route-level').value=current.level?String(current.level):'';
  write('route',current.target);
  document.getElementById('route-assignment')!.textContent=current.agent_name||'エージェント未設定';
  document.getElementById('route-reason')!.textContent=current.reason;
  document.getElementById('route-state')!.textContent=current.blocked||(current.used_fallback?'設定した代替エージェントを使用します。':current.manual_override?'手動で調整した振り分けです。':'このエージェントのモデルで実行します。');
  (document.getElementById('route-run') as HTMLButtonElement).disabled=revising||!!current.blocked||!current.target;
  (document.getElementById('route-plan') as HTMLButtonElement).hidden=!current.needs_plan;
  (document.getElementById('route-fields') as HTMLFieldSetElement).disabled=revising;
}
async function revise(args:{level?:number;target?:ModelTarget}){
  if(!current||revising)return;
  const previous=current;
  revising=true;
  showPreview();
  document.getElementById('route-error')!.textContent='';
  try{
    const next=await deps.call<AutoPreview>('revise_auto_route',{routeId:previous.id,revision:previous.revision,level:args.level??null,target:args.target??null});
    if(dialog('auto-preview').open)current=next;
  }catch(error){
    document.getElementById('route-error')!.textContent=String(error);
  }finally{
    revising=false;
    if(dialog('auto-preview').open)showPreview();
  }
}
export function showAutoPreview(preview:AutoPreview){
  current=preview;
  document.getElementById('route-error')!.textContent='';
  showPreview();
  dialog('auto-preview').showModal();
}
export async function openAutoSettings(){
  if(deps.busy()||saving)return;
  const panel=dialog('auto-settings-dialog');
  panel.showModal();
  document.getElementById('auto-settings-error')!.textContent='';
  setAgentStatus('');
  const fields=document.getElementById('auto-settings-fields') as HTMLFieldSetElement;
  fields.disabled=true;
  try{
    await deps.refresh();
    const config=await deps.call<AutoSettings>('auto_settings');
    if(!panel.open)return;
    const normalized=normalizeAgents(config);
    draftAgents=normalized.agents;
    draftAssignments=normalized.assignments;
    renderAgents();
    updateAssignmentOptions();
    select('auto-fallback-mode').value=config.fallback?'model':'stop';
    (document.getElementById('auto-confirm') as HTMLInputElement).checked=config.confirm_before_run;
    updateFallback();
    fields.disabled=false;
  }catch(error){
    document.getElementById('auto-settings-error')!.textContent=String(error);
  }
}
export function setupAutoRouting(value:Dependencies){
  deps=value;
  document.body.insertAdjacentHTML('beforeend',`<dialog id="auto-settings-dialog" class="auto-dialog auto-settings-dialog">
    <form id="auto-settings-form">
      <header class="auto-dialog-heading">
        <h2>エージェントの振り分け</h2>
        <p>エージェントごとにモデルを決め、仕事の難易度や工程へ割り当てます。</p>
      </header>
      <fieldset id="auto-settings-fields">
        <legend class="sr-only">エージェントの振り分け設定</legend>
        <section class="auto-section" aria-labelledby="agent-list-title">
          <div class="auto-section-heading">
            <div><h3 id="agent-list-title">エージェント</h3><p>名前と実行モデルを一か所で管理します。</p></div>
            <div class="agent-list-actions"><span id="agent-count"></span><button id="add-agent" type="button">エージェントを追加</button></div>
          </div>
          <div id="agent-list" class="agent-list"></div>
          <p id="agent-change-status" class="agent-status" role="status"></p>
        </section>
        <section class="auto-section" aria-labelledby="assignment-title">
          <div class="auto-section-heading"><div><h3 id="assignment-title">振り分け</h3><p>同じエージェントを複数の用途へ割り当てられます。</p></div></div>
          <div class="auto-table-wrap"><table class="auto-table assignment-table">
            <thead><tr><th>用途</th><th>エージェント</th></tr></thead>
            <tbody>
              <tr><th scope="row">難易度判定<small>依頼を5段階に分類</small></th><td>${assignmentSelect('classifier','難易度判定')}</td></tr>
              ${levels.map(([title,description],index)=>`<tr><th scope="row"><span class="level-number">${index+1}</span>${title}<small>${description}</small></th><td>${assignmentSelect(`level_${index+1}` as RouteId,title)}</td></tr>`).join('')}
              <tr><th scope="row">計画<small>実装前の分解と方針作成</small></th><td>${assignmentSelect('planner','計画')}</td></tr>
              <tr><th scope="row">レビュー<small>別セッションで成果物を確認</small></th><td>${assignmentSelect('reviewer','レビュー')}</td></tr>
            </tbody>
          </table></div>
        </section>
        <section class="auto-section fallback-section" aria-labelledby="fallback-title">
          <div class="auto-section-heading"><div><h3 id="fallback-title">代替実行</h3><p>判定できない、またはモデルを利用できない場合。</p></div></div>
          <div class="fallback-controls">
            <div><label for="auto-fallback-mode">扱い</label><select id="auto-fallback-mode"><option value="stop">停止して自分で選ぶ</option><option value="model">別のエージェントで続ける</option></select></div>
            <div id="auto-fallback-controls"><label for="fallback-agent">代替エージェント</label>${assignmentSelect('fallback','代替実行')}</div>
          </div>
        </section>
        <label class="auto-check"><input id="auto-confirm" type="checkbox">実行前に振り分け結果を確認する</label>
        <p class="muted auto-safety-note">エージェントを変更しても、安全制限・計画ゲート・別セッションレビューは維持されます。</p>
      </fieldset>
      <p id="auto-settings-error" role="alert"></p>
      <div class="dialog-actions auto-settings-actions"><button id="auto-settings-close" type="button">閉じる</button><button type="submit" class="primary">変更を保存</button></div>
    </form>
  </dialog>
  <dialog id="auto-preview" class="auto-dialog">
    <h2>進め方の確認</h2>
    <p id="route-assignment" class="route-assignment"></p>
    <p>難易度とモデルを変更できます。変更しても依頼内容の制限は維持されます。</p>
    <fieldset id="route-fields">
      <label for="route-level">難易度</label>
      <select id="route-level"><option value="" disabled>未判定</option>${levels.map(([title],index)=>`<option value="${index+1}">${index+1} · ${title}</option>`).join('')}</select>
      ${controls('route')}
    </fieldset>
    <p id="route-state" role="status"></p>
    <details open><summary>判定理由</summary><p id="route-reason" class="preserve-lines"></p></details>
    <p id="route-error" role="alert"></p>
    <div class="dialog-actions"><button id="route-close">閉じる</button><button id="route-plan" hidden>計画を作成</button><button id="route-run" class="primary">この設定で実行</button></div>
  </dialog>`);

  document.getElementById('add-agent')!.addEventListener('click',addAgent);
  for(const route of routeIds){
    select(`${routePrefixes[route]}-agent`).addEventListener('change',event=>{
      draftAssignments.set(route,(event.currentTarget as HTMLSelectElement).value);
    });
  }
  select('auto-fallback-mode').addEventListener('change',updateFallback);
  document.getElementById('auto-settings-close')!.onclick=()=>{if(!saving)dialog('auto-settings-dialog').close();};
  dialog('auto-settings-dialog').addEventListener('cancel',event=>{if(saving)event.preventDefault();});
  document.getElementById('auto-settings-form')!.addEventListener('submit',async event=>{
    event.preventDefault();
    if(saving)return;
    const fields=document.getElementById('auto-settings-fields') as HTMLFieldSetElement;
    try{
      const agents=captureAgents(true);
      const route_agents=routeIds.map(route=>({route,agent_id:select(`${routePrefixes[route]}-agent`).value}));
      const targetFor=(route:RouteId)=>{
        const id=route_agents.find(assignment=>assignment.route===route)?.agent_id;
        const agent=agents.find(value=>value.id===id);
        if(!agent)throw new Error('すべての用途にエージェントを割り当ててください。');
        return cloneTarget(agent.target);
      };
      const config:AutoSettings={
        classifier:targetFor('classifier'),
        levels:levels.map((_,index)=>({level:index+1,target:targetFor(`level_${index+1}` as RouteId)})),
        fallback:select('auto-fallback-mode').value==='model'?targetFor('fallback'):null,
        planner_default:targetFor('planner'),
        reviewer_default:targetFor('reviewer'),
        agents,
        route_agents,
        confirm_before_run:(document.getElementById('auto-confirm') as HTMLInputElement).checked,
      };
      saving=true;
      fields.disabled=true;
      document.getElementById('auto-settings-error')!.textContent='';
      await deps.call('set_auto_settings',{config});
      dialog('auto-settings-dialog').close();
      deps.notice('エージェントの振り分けを保存しました。次の判定から反映します。');
    }catch(error){
      document.getElementById('auto-settings-error')!.textContent=String(error);
    }finally{
      saving=false;
      fields.disabled=false;
    }
  });
  select('route-level').addEventListener('change',()=>void revise({level:Number(select('route-level').value)}));
  select('route-model').addEventListener('change',()=>{
    fillReasoningSelect(select('route-reasoning'),deps.models().find(model=>model.key===select('route-model').value));
    try{void revise({target:read('route')});}catch(error){document.getElementById('route-error')!.textContent=String(error);}
  });
  select('route-reasoning').addEventListener('change',()=>{
    try{void revise({target:read('route')});}catch(error){document.getElementById('route-error')!.textContent=String(error);}
  });
  document.getElementById('route-close')!.onclick=()=>dialog('auto-preview').close();
  document.getElementById('route-plan')!.onclick=()=>{dialog('auto-preview').close();if(current)deps.plan(current);};
  document.getElementById('route-run')!.onclick=()=>{
    if(!current||revising||current.blocked||!current.target)return;
    const chosen=current;
    current=null;
    dialog('auto-preview').close();
    deps.run(chosen);
  };
}
