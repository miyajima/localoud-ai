export type ChatGptModel='astra'|'sol';
export type ChatGptUsage={week:string;day:string;astra:number;sol:number;solToday:number};
const dateKey=(date:Date)=>`${date.getFullYear()}-${String(date.getMonth()+1).padStart(2,'0')}-${String(date.getDate()).padStart(2,'0')}`;
export function periods(now=new Date()){
 const monday=new Date(now);monday.setHours(0,0,0,0);monday.setDate(monday.getDate()-(monday.getDay()+6)%7);
 return {week:dateKey(monday),day:dateKey(now)};
}
export function normalizeUsage(value:ChatGptUsage|undefined,now=new Date()):ChatGptUsage{
 const current=periods(now);
 if(!value||value.week!==current.week)return {...current,astra:0,sol:0,solToday:0};
 return {...value,...current,solToday:value.day===current.day?value.solToday:0};
}
export function addUsage(value:ChatGptUsage,model:ChatGptModel,now=new Date()):ChatGptUsage{
 const next=normalizeUsage(value,now);return model==='astra'?{...next,astra:next.astra+1}:{...next,sol:next.sol+1,solToday:next.solToday+1};
}
export type CompletedTurn={id:string;model:ChatGptModel|'unknown';duration:number};
export function setupChatGptUsage(container:HTMLElement){
 const key='localoud-chatgpt-usage-v1';
 let usage:ChatGptUsage|undefined,records:Record<string,CompletedTurn>={},error='';
 try{
  const raw=localStorage.getItem(key);if(raw){const parsed=JSON.parse(raw);if(typeof parsed.week!=='string'||typeof parsed.day!=='string'||!['astra','sol','solToday'].every(k=>Number.isSafeInteger(parsed[k])&&parsed[k]>=0))throw new Error('invalid count');usage=parsed;}
  const combined=localStorage.getItem('localoud-chatgpt-auto-v1');
  if(combined){
   const parsed=JSON.parse(combined),u=parsed.usage,r=parsed.records;
   if(!u||typeof u.week!=='string'||typeof u.day!=='string'||!['astra','sol','solToday'].every(k=>Number.isSafeInteger(u[k])&&u[k]>=0)||!r||typeof r!=='object'||Array.isArray(r)||!Object.values(r).every((v:any)=>v&&typeof v.id==='string'&&['astra','sol','unknown'].includes(v.model)&&Number.isFinite(v.duration)))throw new Error('invalid automatic count');
   usage=u;records=r;
  }
 }catch{usage=undefined;records={};error='保存済みカウントを読み取れません。';}
 function render(){
  usage=normalizeUsage(usage);const total=usage.astra+usage.sol;
  container.innerHTML=`<div class="quota-summary"><span>週 ${total} / 200 · Sol 今日 ${usage.solToday} / 170</span><strong>自動記録</strong></div><small class="quota-note">内蔵ChatGPTで検知した完了ターンを自動計上。他での利用は含みません。</small><p>Astra ${usage.astra} / Sol ${usage.sol} · 自動計測時間 ${(Object.values(records).reduce((sum,r)=>sum+r.duration,0)/1000).toFixed(1)}秒 · モデル未取得 ${Object.values(records).filter(r=>r.model==='unknown').length}件</p><small>過去の手動記録を含みます。週は月曜0時、日は毎日0時（端末時刻）で区切ります。OpenAIの正式な残量ではありません。</small>${error?'<p role="alert">'+error+'</p>':''}`;
 }
 render();window.setInterval(render,30000);
 return (turn:CompletedTurn)=>{
  if(error||Object.hasOwn(records,turn.id))return;
  if(!/^[a-zA-Z0-9_-]{1,128}$/.test(turn.id)||!['astra','sol','unknown'].includes(turn.model)||!Number.isFinite(turn.duration)||turn.duration<0)return;
  const nextUsage=turn.model==='unknown'?normalizeUsage(usage):addUsage(normalizeUsage(usage),turn.model);
  // One atomic storage entry owns both deduplication and current counters.
  const nextRecords={...records,[turn.id]:turn};
  try{localStorage.setItem('localoud-chatgpt-auto-v1',JSON.stringify({usage:nextUsage,records:nextRecords}));usage=nextUsage;records=nextRecords;}catch{error='カウントを保存できません。';}
  render();
 };
}
