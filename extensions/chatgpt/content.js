/* global CLF_DOM */
let sentText='',models,working=false,binding=null,lastSignature='',observedGenerating=false,idleSince=0,finished=false;
const tell=(kind,payload)=>chrome.runtime.sendMessage({kind,payload});
async function inspect(){if(working||CLF_DOM.generating())throw new Error('応答が終了してからモデルを確認してください。');const result=await CLF_DOM.inspectModelSettings();if(!result?.length)throw new Error('モデル選択欄を確認できません。ChatGPTにログインしてモデルメニューを確認してください。');models=result;return result;}
chrome.runtime.onMessage.addListener((command,sender,reply)=>{
 if(sender.id!==chrome.runtime.id)return;
 if(command.kind==='ready'){reply({ready:!!CLF_DOM.composer()});return;}
 if(command.kind==='inspect'){inspect().then(()=>reply({ok:true}),e=>reply({error:String(e)}));return true;}
 if(command.kind==='send'||command.kind==='stop'||command.kind==='resume'){reply({received:true});void execute(command);}
});
async function execute(command){
 try{
  if(command.kind==='resume'){if(CLF_DOM.conversationId()!==command.conversation)throw new Error('別の会話が開かれています。');binding={session:command.session,turn:command.turn,conversation:command.conversation};finished=false;lastSignature='';idleSince=Date.now();sentText='';await tell('result',{id:command.id,result:{resumed:true}});return;}
  if(command.kind==='stop'){if(!binding||binding.session!==command.session)throw new Error('会話の対応を確認できないため停止しませんでした。');if(!CLF_DOM.generating())throw new Error('停止対象の生成を確認できません。');if(!CLF_DOM.stopGeneration(()=>binding?.session===command.session&&CLF_DOM.conversationId()===binding.conversation))throw new Error('停止ボタンを確認できません。');for(let i=0;i<40&&CLF_DOM.generating();i++)await new Promise(resolve=>setTimeout(resolve,250));if(CLF_DOM.generating())throw new Error('ChatGPTの停止完了を確認できません。画面を確認してください。');finished=true;await tell('result',{id:command.id,result:{stopped:true}});return;}
  if(working||CLF_DOM.generating())throw new Error('ChatGPTタブで実行中の応答があります。');
  if((CLF_DOM.conversationId()||null)!==(command.conversation||null))throw new Error('別の会話が開かれています。送信しませんでした。');
  if(CLF_DOM.composer()?.textContent?.trim()||CLF_DOM.hasComposerAttachments())throw new Error('ChatGPT入力欄に下書きがあります。内容を確認してください。');
  working=true;const originalConversation=CLF_DOM.conversationId();const current=()=>CLF_DOM.conversationId()===originalConversation;
  if(!await CLF_DOM.selectModelSettings(command.model,command.effort,current))throw new Error('指定したChatGPTモデルとReasoningを確認できません。');
  if(!current()||!CLF_DOM.insertPrompt(command.text))throw new Error('入力を安全に挿入できませんでした。');
  binding={session:command.session,turn:command.turn,conversation:command.conversation};observedGenerating=false;idleSince=0;finished=false;lastSignature='';
  if(!await CLF_DOM.send({stillCurrent:()=>!command.conversation||CLF_DOM.conversationId()===command.conversation}))throw new Error('ChatGPTが送信を受け付けたことを確認できません。画面を確認してください。自動再送はしません。');
  sentText=command.text;binding.conversation=CLF_DOM.conversationId();await tell('result',{id:command.id,result:{accepted:true}});
 }catch(e){await tell('result',{id:command.id,error:String(e)});}finally{working=false;}
}
async function tick(){
 try{
  await chrome.runtime.sendMessage({kind:'heartbeat',models});
  if(!binding)binding=await chrome.runtime.sendMessage({kind:'binding'});
  if(!binding||finished||working)return;
  const conversation=CLF_DOM.conversationId();if(!conversation)return;
  if(binding.conversation&&binding.conversation!==conversation)return;
  const generating=CLF_DOM.generating();observedGenerating ||= generating;
  const messages=CLF_DOM.messages().filter(m=>['user','assistant'].includes(m.role)).map(({role,text})=>({role,text}));
  const signature=JSON.stringify(messages);if(generating||signature!==lastSignature)idleSince=Date.now();
  const compact=value=>String(value||'').replace(/\s+/g,' ').trim();const lastUser=messages.findLastIndex(m=>m.role==='user');const hasAnswer=lastUser>=0&&(!sentText||compact(messages[lastUser].text)===compact(sentText))&&messages.slice(lastUser+1).some(m=>m.role==='assistant'&&m.text.trim());
  const errors=CLF_DOM.errors().map(e=>e.text).filter(Boolean);const failure=!generating&&errors.length?errors.join('\n'):null;const done=!!failure||(!generating&&hasAnswer&&idleSince>0&&Date.now()-idleSince>2500);
  if(signature!==lastSignature||done){await tell('snapshot',{...binding,conversation,messages,finished:done,error:failure});lastSignature=signature;if(done)finished=true;}
 }catch{/* The desktop reports disconnected state; no prompt is retried. */}
 finally{setTimeout(tick,1500);}
}
void tick();
