export type ModelChoice = {key:string;label:string;model:string;profile_id?:string;protocol?:string;local:boolean;reasoning:string[];tools?:boolean;default_reasoning:string|null;is_default:boolean};
export function resolvedModel(model:ModelChoice|undefined,_effort:string|null):ModelChoice|undefined{return model;}
export type ModelTarget = {provider:'local'|'codex'|'api';profile_id?:string|null;model:string;reasoning:string|null};
export function profileForChoice(model:ModelChoice){return model.profile_id||(model.local?'spark':'codex');}
function option(select:HTMLSelectElement,label:string,value:string,disabled=false){const o=select.ownerDocument.createElement('option');o.textContent=label;o.value=value;o.disabled=disabled;select.add(o);return o;}
export function appendModelOptions(select:HTMLSelectElement,models:ModelChoice[]){
  for(const [label,match] of [['ローカル',(m:ModelChoice)=>profileForChoice(m)==='spark'],['Codex',(m:ModelChoice)=>profileForChoice(m)==='codex'],['API providers',(m:ModelChoice)=>!['spark','codex'].includes(profileForChoice(m))]] as const){
    const choices=models.filter(match);if(!choices.length)continue;
    const group=select.ownerDocument.createElement('optgroup');group.label=label;
    for(const model of choices){
      const item=select.ownerDocument.createElement('option');
      const duplicate=choices.some(other=>other.model!==model.model&&other.label===model.label);
      item.value=model.key;item.textContent=(model.model==='gpt-6-astra'&&!model.local?'GPT-6-Astra':model.label)+(duplicate?` · ${model.model}`:'');
      group.append(item);
    }
    select.append(group);
  }

}
export function modelForTarget(models:ModelChoice[],target:ModelTarget|null|undefined){
  if(!target||!['local','codex','api'].includes(target.provider))return undefined;
  const profile=target.profile_id||(target.provider==='local'?'spark':target.provider==='codex'?'codex':null);
  return models.find(m=>m.model===target.model&&(!profile||profileForChoice(m)===profile));
}
export function fillModelSelect(select:HTMLSelectElement,models:ModelChoice[],target:ModelTarget|null){
  select.replaceChildren();option(select,'モデルを選択','');
  appendModelOptions(select,models);
  const match=modelForTarget(models,target);
  if(match)select.value=match.key;
  else if(target){const unavailable=`unavailable:${target.profile_id||target.provider}:${target.model}`;option(select,`${target.profile_id?target.profile_id+' / ':''}${target.model}（利用不可）`,unavailable,true);select.value=unavailable;}
}
export function fillReasoningSelect(select:HTMLSelectElement,model:ModelChoice|undefined,desired:string|null=null){
  select.replaceChildren();
  option(select,!model?'モデルを選択':model.local?'非対応（固定）':`既定${model.default_reasoning?`（${model.default_reasoning}）`:''}`,'');
  if(model&&!model.local)for(const value of model.reasoning)option(select,value,value);
  if(desired){
    if(!model || model.local || !model.reasoning.includes(desired))option(select,`${desired}（利用不可）`,desired,true);
    select.value=desired;
  }
  select.disabled=!model||model.local||!model.reasoning.length;
}
export function readTarget(modelSelect:HTMLSelectElement,reasoningSelect:HTMLSelectElement,models:ModelChoice[]):ModelTarget|null {
  const choice=models.find(m=>m.key===modelSelect.value), effort=reasoningSelect.value||null;
  const model=resolvedModel(choice,effort);
  if(!model || (effort&&(model.local||!model.reasoning.includes(effort))))return null;
  const profile=profileForChoice(model),target:ModelTarget={provider:profile==='spark'?'local':profile==='codex'?'codex':'api',model:model.model,reasoning:effort};
  if(model.profile_id)target.profile_id=model.profile_id;
  return target;
}
