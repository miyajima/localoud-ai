export type ModelChoice = {key:string;label:string;model:string;local:boolean;reasoning:string[];default_reasoning:string|null;is_default:boolean};
export type ModelTarget = {provider:'local'|'codex';model:string;reasoning:string|null};
function option(select:HTMLSelectElement,label:string,value:string,disabled=false){const o=select.ownerDocument.createElement('option');o.textContent=label;o.value=value;o.disabled=disabled;select.add(o);return o;}
export function modelForTarget(models:ModelChoice[],target:ModelTarget|null|undefined){return target?models.find(m=>m.model===target.model&&m.local===(target.provider==='local')):undefined;}
export function fillModelSelect(select:HTMLSelectElement,models:ModelChoice[],target:ModelTarget|null){
  select.replaceChildren();option(select,'モデルを選択','');
  for(const model of models)option(select,model.label+(model.local?' · ローカル':''),model.key);
  const match=modelForTarget(models,target);
  if(match)select.value=match.key;
  else if(target){const unavailable=`unavailable:${target.provider}:${target.model}`;option(select,`${target.model}（利用不可）`,unavailable,true);select.value=unavailable;}
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
  const model=models.find(m=>m.key===modelSelect.value), effort=reasoningSelect.value||null;
  if(!model || (effort&&(model.local||!model.reasoning.includes(effort))))return null;
  return {provider:model.local?'local':'codex',model:model.model,reasoning:effort};
}
