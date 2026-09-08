export type ModelChoice = {key:string;label:string;model:string;local:boolean;reasoning:string[];default_reasoning:string|null;is_default:boolean};
export type ModelTarget = {provider:'local'|'codex';model:string;reasoning:string|null};
function option(select:HTMLSelectElement,label:string,value:string,disabled=false){const o=select.ownerDocument.createElement('option');o.textContent=label;o.value=value;o.disabled=disabled;select.add(o);return o;}
export function appendModelOptions(select:HTMLSelectElement,models:ModelChoice[]){
  for(const [label,local] of [['ローカル',true],['Codex',false]] as const){
    const choices=models.filter(m=>m.local===local);if(!choices.length)continue;
    const group=select.ownerDocument.createElement('optgroup');group.label=label;
    for(const model of choices){
      const item=select.ownerDocument.createElement('option');
      item.value=model.key;item.textContent=model.model==='gpt-6-astra'&&!model.local?'GPT-6-Astra':model.label;
      group.append(item);
    }
    select.append(group);
  }
  const group=select.ownerDocument.createElement('optgroup');group.label='ChatGPT（未接続）';
  const item=select.ownerDocument.createElement('option');item.textContent='GPT-6-Astra(ChatGPT)';item.value='unavailable:chatgpt:gpt-6-astra';item.disabled=true;
  group.append(item);select.append(group);
}
export function modelForTarget(models:ModelChoice[],target:ModelTarget|null|undefined){return target?models.find(m=>m.model===target.model&&m.local===(target.provider==='local')):undefined;}
export function fillModelSelect(select:HTMLSelectElement,models:ModelChoice[],target:ModelTarget|null){
  select.replaceChildren();option(select,'モデルを選択','');
  appendModelOptions(select,models);
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
