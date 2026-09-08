// All loopback requests originate in the extension worker; the page never receives the token.
let polling=false;
async function api(path,body){const {token}=await chrome.storage.local.get('token');if(!token)throw new Error('拡張の接続キーを設定してください。');const response=await fetch('http://127.0.0.1:8791/'+path,{method:'POST',headers:{'Content-Type':'application/json','Authorization':'Bearer '+token},body:JSON.stringify(body)});if(!response.ok)throw new Error('Localoud接続: '+response.status);return response.status===204?{}:response.json();}
chrome.runtime.onMessage.addListener((message,sender,respond)=>{
 if(sender.id!==chrome.runtime.id||!sender.tab||!sender.url?.startsWith('https://chatgpt.com/'))return;
 (async()=>{
   if(message.kind==='heartbeat'){
     if(polling)return {};polling=true;
     try{const reply=await api('poll',{models:message.models});if(reply.command)await dispatch(reply.command);return {};}finally{polling=false;}
   }
   if(message.kind==='result')return api('result',message.payload);
   if(message.kind==='snapshot')return api('snapshot',message.payload);
   if(message.kind==='binding'){const data=await chrome.storage.session.get('binding:'+sender.tab.id);return data['binding:'+sender.tab.id]||null;}
   throw new Error('Unknown message');
 })().then(respond,e=>respond({error:String(e)}));return true;
});
async function dispatch(command){
 try{
   const saved=await chrome.storage.session.get('session:'+command.session);let tabId=saved['session:'+command.session];let tab;
   if(tabId){try{tab=await chrome.tabs.get(tabId);}catch{}}
   if(command.kind==='stop'){if(!tab)throw new Error('対象のChatGPTタブが見つかりません。');await chrome.tabs.sendMessage(tab.id,command);return;}
   if(!tab){tab=await chrome.tabs.create({url:command.conversation?'https://chatgpt.com/c/'+encodeURIComponent(command.conversation):'https://chatgpt.com/',active:true});}
   await chrome.storage.session.set({['session:'+command.session]:tab.id,['binding:'+tab.id]:{session:command.session,turn:command.turn,conversation:command.conversation}});
   // Wait for the content script, not a fixed page-load delay. Delivery occurs once only.
   let ready=false;for(let attempt=0;attempt<60;attempt++){try{const reply=await chrome.tabs.sendMessage(tab.id,{kind:'ready'});if(reply.ready){ready=true;break;}}catch{}await new Promise(resolve=>setTimeout(resolve,500));}
   if(!ready)throw new Error('ChatGPTにログインし、ページの読み込みを完了してください。');
   await chrome.tabs.sendMessage(tab.id,command);
 }catch(e){await api('result',{id:command.id,error:String(e)});}
}
