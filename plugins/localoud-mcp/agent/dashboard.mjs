import { presentApprovals, approvalNotice } from './presentation.mjs';

export function dashboard(csrf, html) {
  return html(`<h1 data-csrf="${csrf}">Localoud Mac 承認</h1>
<p>編集とコマンドは、このMacで承認するまで実行されません。</p>
<p id="connection" class="status"></p>
<section aria-label="操作の承認"><h2>操作の承認</h2>
<p>対象と内容を確認してください。コマンドはMac利用者の権限で動き、プロジェクト外にも影響する可能性があります。</p>
<p id="message" role="status"></p><div id="jobs"></div></section>
<details id="history" hidden><summary id="history-summary">操作履歴</summary><div id="history-list"></div></details>
<details id="pairing"><summary>ChatGPTと接続（初回・追加の接続）</summary><section>
<p>ChatGPTの接続画面に表示されたコードを入力します。</p>
<input id="code" size="42" aria-label="接続コード"><button id="preview">接続内容を確認</button>
<pre id="pair-details"></pre><button id="pair-allow" hidden>この接続を許可</button></section></details>`, `
const csrf=${JSON.stringify(csrf)};
const presentApprovals=${presentApprovals.toString()};
const approvalNotice=${approvalNotice.toString()};
let preview, rendered, pairingInitialized=false;
const message=t=>document.getElementById('message').textContent=t;
const toolLabel=t=>({file_edit:'ファイル編集',file_create:'ファイル作成',command_run:'コマンド実行'}[t]||t);
const statusLabel=s=>({expired:'期限切れ（実行していません）',denied:'拒否（実行していません）',completed:'処理結果あり',failed:'処理失敗',running:'実行中',indeterminate:'結果不明'}[s]||'状態を確認してください');
async function api(path,data){
  const r=await fetch(path,{method:'POST',headers:{'content-type':'application/json','x-localoud-csrf':csrf},body:JSON.stringify(data)});
  const j=await r.json();if(!r.ok)throw new Error(j.error||'操作できません');return j;
}
document.getElementById('preview').onclick=async()=>{
  try{preview=await api('/api/pair-preview',{code:document.getElementById('code').value.trim()});
    document.getElementById('pair-details').textContent=JSON.stringify(preview.details,null,2);
    document.getElementById('pair-allow').hidden=false;
  }catch{message('接続コードを確認してください');}
};
document.getElementById('pair-allow').onclick=async()=>{
  try{await api('/api/pair-approve',preview);document.getElementById('pair-allow').hidden=true;
    document.getElementById('pairing').open=false;document.getElementById('pair-details').textContent='';document.getElementById('code').value='';
    message('Macの確認が完了しました。ChatGPTの接続画面で「接続を許可」を押してください。');
  }catch{message('接続を承認できませんでした');}
};
function card(job){
  const node=document.createElement('article'),title=document.createElement('h3'),root=document.createElement('p'),detail=document.createElement('pre'),deadline=document.createElement('small');
  title.textContent=toolLabel(job.tool)+(job.arguments.path?'：'+job.arguments.path:'');
  root.textContent='対象フォルダー：'+job.target_root;
  detail.textContent=JSON.stringify(job.arguments,null,2);
  detail.className='operation-detail';detail.tabIndex=0;detail.setAttribute('aria-label','操作内容（スクロールできます）');
  deadline.textContent='承認期限：'+new Date(job.expires_at).toLocaleString();
  node.append(title,root,detail,deadline,document.createElement('br'));
  for(const [path,label] of [['approve','承認して実行'],['deny','拒否']]){
    const button=document.createElement('button');button.textContent=label;if(path==='deny')button.className='deny';
    button.onclick=async()=>{
      button.disabled=true;
      try{const result=await api('/api/'+path,{operation_id:job.operation_id,digest:job.digest});message(approvalNotice(result.status).message);await refresh();}
      catch{message('処理できませんでした。操作を再送せず、状態を確認してください');await refresh();}
      finally{if(button.isConnected)button.disabled=false;}
    };
    node.append(button);
  }
  return node;
}
async function refresh(){
  try{
    const r=await fetch('/api/state');if(!r.ok)return;const s=await r.json();
    document.getElementById('connection').textContent=s.online?'クラウドに接続中':'クラウドに未接続';
    const view=presentApprovals([...(s.jobs||[]),...(s.history||[])],Date.now());
    if(!pairingInitialized){document.getElementById('pairing').open=!view.pending.length&&!view.history.length;pairingInitialized=true;}
    const key=JSON.stringify(view);if(key===rendered)return;rendered=key;
    const root=document.getElementById('jobs');root.replaceChildren();
    for(const job of view.pending)root.append(card(job));
    if(!view.pending.length)root.textContent=view.history.some(job=>job.status==='expired')
      ?'承認待ちの操作はありません。期限切れの申請は実行されません。ChatGPTから新しい申請を送ってください。'
      :'承認待ちの操作はありません。ChatGPTから編集やコマンドを申請すると、ここに承認ボタンが表示されます。';
    const history=document.getElementById('history'),list=document.getElementById('history-list');history.hidden=!view.history.length;
    document.getElementById('history-summary').textContent='処理中・操作履歴（'+view.history.length+'件）';list.replaceChildren();
    for(const job of view.history){const row=document.createElement('p');row.textContent=toolLabel(job.tool)+' — '+statusLabel(job.status)+' / '+job.operation_id;list.append(row);}
  }catch{message('状態を取得できません。接続を確認してください。');}
}
refresh();setInterval(refresh,2000);
`);
}
