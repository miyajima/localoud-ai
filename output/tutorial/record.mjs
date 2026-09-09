import { chromium } from '/Users/miyajimakazuhiro/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright/index.mjs';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
const dir=path.dirname(fileURLToPath(import.meta.url));
const scenes=[
 {chapter:'01  画面の役割',title:'右で相談、左で実行',file:'01-plan.png',crop:[0,0,1156,768],body:'右側のChatGPTで計画とレビュー。左側のLocaloudでファイルの変更と検証を行います。',caption:'この動画は、完了済みタスクの実画面を使った操作解説です。',duration:7},
 {chapter:'02  計画を作る',title:'プロジェクトを選ぶ',file:'01-plan.png',crop:[0,65,610,640],body:'左で対象プロジェクトを選び、「新しいタスク」を押します。操作は「Manifestを実行」を選びます。',caption:'前提：ChatGPTのLocaloudプラグインとMCP接続が設定済み。',duration:7},
 {chapter:'02  計画を作る',title:'ChatGPTに依頼する',file:'01-plan.png',crop:[612,82,540,665],body:'Localoudを選択し、対象プロジェクト・作業内容・変更範囲・合格条件を伝えます。',caption:'「MCPで確認し、Task ManifestをJSONコードブロック1個で返して」と依頼します。',duration:8},
 {chapter:'03  計画を取り込む',title:'JSON全体をコピー',file:'02-copy.png',crop:[612,425,540,290],body:'ChatGPTの回答にあるJSON枠の右上、「コピー」を押します。説明文を混ぜずに取り込めます。',caption:'Task Manifestは、Localoudに渡す作業指示書です。',duration:7},
 {chapter:'03  計画を取り込む',title:'貼り付けて実行',file:'04-agents.png',crop:[145,605,460,162],body:'左下の「貼り付けて実行」を1回押します。入力欄へ先に貼り付ける必要はありません。',caption:'新規の取り込みでは「取り込み中…」の後に取り込み結果と状態を表示します。',duration:7},
 {chapter:'03  計画を取り込む',title:'取り込み済みの場合',file:'03-import.png',crop:[145,90,460,220],body:'同じManifestは二重実行されず、保存済みのタスクが開きます。この実演は完了済みの計画を再度取り込んだ場面です。',caption:'「取り込み済み」は成功した既存タスクを開いたという通知です。',duration:8},
 {chapter:'04  実行結果を確認',title:'workerの担当を見る',file:'04-agents.png',crop:[145,285,460,326],body:'Agentsで担当、難易度、使用モデル、各workerの状態を確認します。今回は2つの担当が完了しています。',caption:'ここは保存済みの実行結果です。新しいworkerは起動していません。',duration:7},
 {chapter:'04  実行結果を確認',title:'検証の根拠を読む',file:'06-result.png',crop:[145,245,460,350],body:'Chatには保存済みの検証結果が残ります。今回の例では、ファイル内容のバイト一致を確認しています。',caption:'「コマンド成功」と「依頼した条件を満たすこと」は別々に確認します。',duration:7},
 {chapter:'05  ChatGPTでレビュー',title:'タスクIDを渡す',file:'07-review.png',crop:[613,85,540,248],body:'実装が統合されたら、Chatに表示されたタスクIDを使ってChatGPTへレビューを依頼します。',caption:'「MCPで最新成果物・差分・検証結果をレビューし、Review Manifestを返して」',duration:8},
 {chapter:'05  ChatGPTでレビュー',title:'レビュー結果をコピー',file:'07-review.png',crop:[613,341,540,274],body:'ChatGPTが返したReview Manifestも、JSON枠のコピーを使います。今回の判定は pass（合格）です。',caption:'この画面は、ChatGPTが実際にMCPで確認したレビュー結果です。',duration:7},
 {chapter:'06  レビューを戻して完了',title:'同じボタンで戻す',file:'04-agents.png',crop:[145,605,460,162],body:'レビューJSONをコピーした状態で、もう一度「貼り付けて実行」を押します。計画とレビューで同じボタンを使います。',caption:'新規の合格レビューを取り込むと「レビュー合格・タスク完了」と表示されます。',duration:7},
 {chapter:'06  レビューを戻して完了',title:'完了を確かめる',file:'08-complete.png',crop:[145,75,460,175],body:'画面上部が「完了」になれば、このタスクの実行とレビューは終了です。修正指示がある場合は、修正後に再レビューします。',caption:'操作は「計画をコピー → 実行 → レビューをコピー → 完了」の順です。',duration:8},
];
for(const s of scenes)s.src='data:image/png;base64,'+(await readFile(path.join(dir,s.file))).toString('base64');
const html=`<!doctype html><html lang="ja"><meta charset="utf-8"><style>
*{box-sizing:border-box}body{margin:0;background:#101b2b;color:#eef4ff;font-family:'Hiragino Sans','Noto Sans CJK JP',sans-serif}header{height:90px;display:flex;align-items:center;justify-content:space-between;padding:0 40px;border-bottom:1px solid #354459}header strong{font-size:28px}header span{font-size:17px;color:#b9c8da}#stage{position:absolute;left:30px;top:115px;width:1000px;height:650px;overflow:hidden;background:#edf1f7;border-radius:18px;box-shadow:0 14px 44px #0005}#shot{position:absolute;max-width:none;transition:transform .8s ease}aside{position:absolute;left:1070px;top:150px;width:326px}#chapter{font-size:18px;color:#8cbaff;margin-bottom:24px}h1{font-size:34px;line-height:1.45;margin:0 0 28px}#body{font-size:24px;line-height:1.9;color:#d8e4f3}#subtitle{position:absolute;left:30px;right:30px;top:793px;min-height:80px;background:#233249;border-radius:12px;padding:18px 24px;font-size:24px;line-height:1.6;text-align:center}#bar{position:absolute;left:0;bottom:0;height:5px;background:#8cbaff;transition:width .5s}#cover{position:absolute;inset:0;background:#101b2b;z-index:4;display:flex;flex-direction:column;justify-content:center;align-items:center}#cover small{color:#8cbaff;font-size:22px;letter-spacing:3px}#cover h2{font-size:54px;margin:28px}#cover p{font-size:23px;color:#bccbde}#cover[hidden]{display:none}</style>
<header><strong>Localoud 操作ガイド</strong><span>実画面による解説 ・ 日本語字幕 ・ 接続設定後の使い方</span></header><div id="stage"><img id="shot" alt="Localoudの実画面"></div><aside><div id="chapter"></div><h1></h1><div id="body"></div></aside><div id="subtitle" role="status"></div><div id="bar"></div><div id="cover"><small>LOCALoud WALKTHROUGH</small><h2>計画から、レビュー完了まで</h2><p>実画面を使った操作解説 / 過去の実行結果を含みます</p></div>
<script>window.showScene=async(s,i,n)=>{document.querySelector('#shot').src=s.src;await document.querySelector('#shot').decode();const [x,y,w,h]=s.crop;const scale=Math.min(1000/w,650/h);const im=document.querySelector('#shot');im.style.width=(1156*scale)+'px';im.style.height=(768*scale)+'px';im.style.left=((1000-w*scale)/2-x*scale)+'px';im.style.top=((650-h*scale)/2-y*scale)+'px';document.querySelector('#chapter').textContent=s.chapter;document.querySelector('h1').textContent=s.title;document.querySelector('#body').textContent=s.body;document.querySelector('#subtitle').textContent=s.caption;document.querySelector('#bar').style.width=((i+1)/n*100)+'%';};</script>`;
await writeFile(path.join(dir,'presentation.html'),html);
const browser=await chromium.launch({headless:true});
const diagnostics={console:[],pageerror:[],requestfailed:[]};
try{
 const pre=await browser.newPage({viewport:{width:1440,height:900}});await pre.setContent(html);await pre.evaluate(s=>window.showScene(s,0,12),scenes[0]);await pre.locator('#cover').evaluate(e=>e.hidden=true);await pre.getByRole('heading',{name:scenes[0].title,exact:true}).waitFor();await pre.screenshot({path:path.join(dir,'qa/preflight.png')});await pre.close();
 const ctx=await browser.newContext({viewport:{width:1440,height:900},recordVideo:{dir:path.join(dir,'raw'),size:{width:1440,height:900}}});const page=await ctx.newPage();
 page.on('console',m=>{if(['warning','error'].includes(m.type()))diagnostics.console.push(m.text())});page.on('pageerror',e=>diagnostics.pageerror.push(String(e)));page.on('requestfailed',r=>diagnostics.requestfailed.push(r.url()));
 await page.setContent(html);await page.waitForTimeout(3500);let chapter='';const logs=[];
 for(let i=0;i<scenes.length;i++){
  const s=scenes[i];if(s.chapter!==chapter){await page.locator('#cover').evaluate((e,s)=>{e.hidden=false;e.querySelector('h2').textContent=s.chapter;e.querySelector('p').textContent=s.title},s);await page.waitForTimeout(1800);chapter=s.chapter;}
  await page.evaluate(({s,i,n})=>window.showScene(s,i,n),{s,i,n:scenes.length});await page.waitForTimeout(600);await page.locator('#cover').evaluate(e=>e.hidden=true);await page.getByRole('heading',{name:s.title,exact:true}).waitFor();await page.waitForTimeout(s.duration*1000);await page.screenshot({path:path.join(dir,'qa',String(i+1).padStart(2,'0')+'.png')});logs.push({chapter:s.chapter,title:s.title,duration:s.duration,source:s.file});console.log('recorded',i+1,s.title);
 }
 const video=page.video();await ctx.close();await video.saveAs(path.join(dir,'localoud-guide.webm'));await writeFile(path.join(dir,'execution-logs.json'),JSON.stringify(logs,null,2));await writeFile(path.join(dir,'browser-diagnostics.json'),JSON.stringify(diagnostics,null,2));
}finally{await browser.close()}
