export type TaskFocus = {id:string;projectId:string;title:string;goal:string;status:string;artifactVersion?:string|null;manifestId?:string;reviewId?:string;verdict?:string;acceptance?:string[]};
export function planningRequest(project:{id:string;name:string},goal:string,files:string[]=[]){
 return `Localoudで実行する計画を相談したいです。\nProject: ${project.name}\nProject ID: ${project.id}\n依頼: ${goal.trim()}${files.length?'\n対象ファイル: '+files.join(', '):''}\n\nLocaloud MCPのproject_getで対象と現在の版を確認し、handoff_schemaを参照してください。必要な箇所だけ取得し、目的・変更範囲・完了条件を短く整理してください。不明点があれば相談し、実行できる計画が固まったらTaskManifestを1つのJSONコードブロックで返してください。MCPに接続できない場合はその旨を伝え、IDや版を推測しないでください。`;
}
const esc=(s:string)=>s.replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]!));
export function nextAction(status:string){
 if(status==='completed')return 'レビュー合格。変更ファイルと結果を確認できます。';
 if(status==='awaiting_review')return '次はChatGPTでレビュー。依頼文をコピーしてChatGPTの会話へ貼り付けてください。';
 if(status==='review_rejected')return 'レビューは未合格です。指摘を確認し、ChatGPTで修正レビューを作成してください。';
 if(['interrupted','failed','reconciliation_required'].includes(status))return '作業が中断しています。「再開・状態を確認」で状態を確認してください。';
 if(status==='queued')return '実行を受け付けました。workerの開始を待っています。';
 if(status==='stopping')return '停止処理中です。状態が確定するまでお待ちください。';
 return 'Localoudで実行中。進捗と変更は下のタブで確認できます。';
}
export function reviewRequest(t:TaskFocus){
 return `Localoud MCPを使い、次のタスクをレビューしてください。\n目的: ${t.goal}\nProject ID: ${t.projectId}\nTask ID: ${t.id}\nArtifact version: ${t.artifactVersion}\n\ntask_get、git_diff、file_read、verification_getで現在の成果物と受け入れ条件を照合してください。対象版が変わっていればレビューを確定せず、その旨を伝えてください。handoff_schemaを参照してReviewManifestを1つのJSONコードブロックで返してください。JSONの前には目的・Task ID・レビュー結果を短く表示してください。`;
}
export function focusMarkup(t:TaskFocus,label:(s:string)=>string){
 return `<div class="focus-heading"><strong title="${esc(t.goal)}">${esc(t.title)}</strong><span>${esc(label(t.status))}</span></div><p class="focus-next">${esc(nextAction(t.status))}</p><div class="focus-actions"><button data-focus-id title="${esc(t.id)}">ID ${esc(t.id.slice(0,8))} · コピー</button>${t.status==='awaiting_review'&&t.artifactVersion?'<button data-focus-review>レビュー依頼をコピー</button>':''}</div>`;
}
export function recordMarkup(text:string,key:string,index:number,render:(s:string)=>string){
 let data;try{data=JSON.parse(text.replace(/^```(?:json)?\s*\n?|\n?```$/g,'').trim());}catch{/* Plain saved record. */}
 const title=data?.kind==='review'?'レビュー結果':data?.kind==='task'?'実行依頼':key==='verification-summary'?'検証結果':key==='review-target'?'レビュー対象の詳細':key==='autonomous-artifact'?'成果物の保存先':'作業記録';
 const id=typeof data?.manifest_id==='string'?data.manifest_id:'';
 return `<details class="task-record" id="record-${esc(key)}"><summary>${title}${id?` · ID ${esc(id)}`:''}</summary><button class="copy-message" data-copy-message="${index}">全文をコピー</button><div class="message-text markdown">${render(text)}</div></details>`;
}
