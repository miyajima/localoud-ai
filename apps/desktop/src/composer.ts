import {triggerAt,historySuffix,mentionKey,filterEntries,canAcceptSuffix} from './composer-model';
import type {Mention,Entry,Trigger} from './composer-model';
import type {ModelChoice} from './model-controls';
type Call=<T=unknown>(command:string,args?:Record<string,unknown>)=>Promise<T>;
type Catalog={entries:Entry[];modes:string[];warnings:string[]};
type CompletionConfig={enabled:boolean;model:string;reasoning:string};
type Dependencies={call:Call;project:()=>string|null;thread:()=>string|null;busy:()=>boolean;running:()=>boolean;codex:()=>boolean;mode:()=>string;setMode:(mode:string)=>boolean;models:()=>ModelChoice[];changed:()=>void;notice:(text:string)=>void;error:(text:string)=>void};
type Candidate={label:string;detail:string;choose:()=>void};
type Question={request_id:string;questions:{id:string;header:string;question:string;isSecret?:boolean;options?:{label:string;description:string}[]|null}[]};
export function setupComposer(deps:Dependencies){
  const input=document.querySelector<HTMLTextAreaElement>('#task-input')!;
  const assist=document.createElement('div');assist.id='composer-assist';input.after(assist);
  assist.innerHTML='<div id="composer-chips"></div><div id="composer-suffix" hidden></div><div class="composer-tools"><button type="button" id="composer-slash">/ コマンド</button><button type="button" id="composer-at">@ 参照</button><button type="button" id="composer-history">入力履歴</button><button type="button" id="composer-ai">AI補完</button><button type="button" id="composer-settings">補完設定</button><span id="composer-hint">候補は Tab で採用 · Esc で閉じる</span></div><div id="composer-menu" role="listbox" aria-label="入力候補" hidden></div>';
  const interactions=document.createElement('div');interactions.id='composer-interactions';interactions.hidden=true;document.querySelector('#content')!.before(interactions);
  const settings=document.createElement('dialog');settings.id='completion-settings-dialog';settings.innerHTML='<form><h2>入力補完</h2><p>このプロジェクトの入力履歴から、続きの候補を表示します。候補を採用するまで入力は変わりません。</p><label><input type="checkbox" id="completion-enabled"> 履歴で補えない場合はAI補完を使う</label><p>入力履歴を優先し、一致する候補がなければ設定済みのローカルLLMで補完します。ローカルが未接続・未対応・生成失敗の場合はLuna / lowを使います。</p><p>補完に使うのは現在の入力と、このプロジェクトの直近の入力履歴です。Lunaへ切り替える場合はそれらを送信します。リポジトリ本文は送信しません。</p><p id="completion-error" role="alert"></p><div class="dialog-actions"><button type="button" id="completion-close">閉じる</button><button type="submit" class="primary">保存</button></div></form>';document.body.append(settings);
  const menu=assist.querySelector<HTMLDivElement>('#composer-menu')!,suffixPanel=assist.querySelector<HTMLDivElement>('#composer-suffix')!;
  const chips=assist.querySelector<HTMLDivElement>('#composer-chips')!,hint=assist.querySelector<HTMLSpanElement>('#composer-hint')!;
  const byId=<T extends HTMLElement=HTMLElement>(id:string)=>document.getElementById(id) as T;
  let mentions:Mention[]=[],catalog:Catalog|null=null,catalogPromise:Promise<Catalog>|null=null,context='',history:string[]=[],generation=0,composing=false;
  let trigger:Trigger|null=null,category='',candidates:Candidate[]=[],selected=0,suffix='',suffixPrefix='',suffixContext='',dismissed='',aiBusy=false,lastAI=0;
  let timer:ReturnType<typeof setTimeout>|undefined,config:CompletionConfig={enabled:true,model:'local-first',reasoning:'low'},interactionVersion=0,lastInteraction='';
  function key(){return `${deps.project()||''}:${deps.thread()||''}`;}
  function args(){return {projectId:deps.project(),threadId:deps.thread()};}
  function hideMenu(){menu.hidden=true;input.removeAttribute('aria-activedescendant');input.setAttribute('aria-expanded','false');candidates=[];}
  function hideSuffix(){suffix='';suffixPanel.hidden=true;suffixPanel.replaceChildren();}
  function changed(){deps.changed();}
  function label(m:Mention){return m.kind==='skill'?`/${m.name}`:m.kind==='plugin'?`/${m.name}`:m.kind==='agent'?`@agent/${m.name}`:`@${m.name}`;}
  function renderChips(){
    chips.replaceChildren();for(const mention of mentions){const chip=document.createElement('span');chip.className='composer-chip';const text=document.createElement('span');text.textContent=label(mention);chip.append(text);const remove=document.createElement('button');remove.type='button';remove.textContent='×';remove.setAttribute('aria-label',`${label(mention)} を外す`);remove.onclick=()=>{mentions=mentions.filter(m=>mentionKey(m)!==mentionKey(mention));renderChips();changed();};chip.append(remove);chips.append(chip);}
  }
  function add(mention:Mention){if(mentions.length>=20){deps.error('参照は20件までです。');return;}if(!mentions.some(m=>mentionKey(m)===mentionKey(mention)))mentions.push(mention);renderChips();changed();}
  function consumeTrigger(){if(trigger){input.setRangeText('',trigger.start,trigger.end,'end');trigger=null;}hideMenu();hideSuffix();input.focus();changed();}
  function chooseEntry(entry:Entry){consumeTrigger();add(entry.kind==='agent'?{kind:'agent',name:entry.name}:{kind:entry.kind as 'skill'|'plugin'|'file',name:entry.name,path:entry.path});}
  async function getCatalog(){
    if(catalog)return catalog;if(catalogPromise)return catalogPromise;
    const expected=key();catalogPromise=deps.call<Catalog>('composer_catalog',args()).then(value=>{if(key()===expected){catalog=value;if(value.warnings.length)deps.notice(value.warnings.join('\n'));}return value;}).finally(()=>{if(key()===expected)catalogPromise=null;});return catalogPromise;
  }
  function paint(){
    menu.replaceChildren();input.setAttribute('aria-controls','composer-menu');input.setAttribute('aria-expanded','true');
    candidates.forEach((candidate,index)=>{const button=document.createElement('button');button.type='button';button.id=`composer-candidate-${index}`;button.setAttribute('role','option');button.setAttribute('aria-selected',String(index===selected));button.className=index===selected?'selected':'';const title=document.createElement('strong');title.textContent=candidate.label;const detail=document.createElement('small');detail.textContent=candidate.detail;button.append(title,detail);button.onmousedown=e=>e.preventDefault();button.onclick=()=>candidate.choose();menu.append(button);});
    if(!candidates.length){const empty=document.createElement('p');empty.textContent='該当する候補はありません。';menu.append(empty);}
    menu.hidden=false;if(candidates[selected]){input.setAttribute('aria-activedescendant',`composer-candidate-${selected}`);menu.children[selected]?.scrollIntoView({block:'nearest'});}
  }
  function switchMode(mode:string){if(deps.setMode(mode))consumeTrigger();}
  function commands(query:string):Candidate[]{return [
    {label:'/skills',detail:'スキルを選択',choose:()=>{enterCategory('skill');}},
    {label:'/plugins',detail:'インストール済みプラグインを指定',choose:()=>{enterCategory('plugin');}},
    {label:'/plan',detail:'Codexのプランモード',choose:()=>switchMode('plan')},
    {label:'/goal',detail:'入力した目標をCodexのゴールに設定',choose:()=>switchMode('goal')},
    {label:'/implement',detail:'通常の実装モード',choose:()=>switchMode('implement')},
    {label:'/workflow',detail:'Planタブ用の実行計画を作成',choose:()=>switchMode('workflow')},
    {label:'/history',detail:'このプロジェクトの入力履歴',choose:()=>{enterCategory('history');}},
  ].filter(c=>`${c.label} ${c.detail}`.toLowerCase().includes(query.toLowerCase()));}
  async function search(query:string){
    const seq=++generation,expected=key();selected=0;hideSuffix();candidates=[];
    const sigil=trigger?.sigil||'/';
    if(category==='history'){
      candidates=history.filter(h=>h.toLowerCase().includes(query.toLowerCase())).slice(0,20).map(text=>({label:text,detail:'このプロジェクトの入力履歴',choose:()=>{input.value=text;input.setSelectionRange(text.length,text.length);trigger=null;hideMenu();input.focus();changed();}}));paint();return;
    }
    if(!category&&sigil==='/')candidates=commands(query);
    if(!category&&sigil==='@')candidates=[
      {label:'@plan',detail:'Codexのプランモード',choose:()=>switchMode('plan')},
      {label:'@goal',detail:'ゴールを設定して実行',choose:()=>switchMode('goal')},
      {label:'@agent',detail:'サブエージェントを選択',choose:()=>{enterCategory('agent');}},
      {label:'@file',detail:'プロジェクト内のファイルを選択',choose:()=>{enterCategory('file');}},
    ].filter(c=>`${c.label} ${c.detail}`.toLowerCase().includes(query.toLowerCase()));
    paint();
    try {
      const values:Candidate[]=[];
      if(category!=='file'){
        const data=await getCatalog();
        const kinds=category?[category]:sigil==='/'?['skill','plugin']:['agent'];
        for(const kind of kinds)for(const entry of filterEntries(data.entries,query,kind))values.push({label:kind==='agent'?`@agent/${entry.name}`:`/${entry.name}`,detail:`${entry.label} · ${entry.description}`,choose:()=>chooseEntry(entry)});
      }
      if(sigil==='@'&&(category===''||category==='file')){
        const files=await deps.call<string[]>('composer_files',{...args(),query});
        for(const path of files)values.push({label:`@${path}`,detail:'ファイルを参照',choose:()=>chooseEntry({kind:'file',name:path,label:path,path,description:''})});
      }
      if(seq!==generation||key()!==expected)return;
      candidates.push(...values);paint();
    }catch(e){if(seq===generation&&key()===expected){const error=document.createElement('p');error.textContent=String(e);menu.append(error);}}
  }
  function enterCategory(kind:string){
    const sigil=trigger?.sigil||'/';const start=trigger?.start??input.selectionStart,end=trigger?.end??input.selectionEnd;
    input.setRangeText(sigil,start,end,'end');trigger={start,end:start+1,sigil,query:''};category=kind;changed();void search('');
  }
  function open(sigil:'/'|'@',kind=''){
    if(input.disabled)return;hideSuffix();input.focus();const start=input.selectionStart;
    input.setRangeText(sigil,start,input.selectionEnd,'end');trigger={start,end:start+1,sigil,query:''};category=kind;changed();void search('');
  }
  function showSuffix(value:string,source:string,original:string,expected:string){
    if(!value||dismissed===original||!canAcceptSuffix(original,input.value,expected,key(),input.selectionStart)||input.selectionStart!==input.selectionEnd||composing)return;
    suffix=value;suffixPrefix=original;suffixContext=expected;suffixPanel.replaceChildren();const sourceLabel=document.createElement('small');sourceLabel.textContent=source;const text=document.createElement('span');text.textContent=value;const accept=document.createElement('button');accept.type='button';accept.textContent='Tab で採用';accept.onmousedown=e=>e.preventDefault();accept.onclick=acceptSuffix;suffixPanel.append(sourceLabel,text,accept);suffixPanel.hidden=false;
  }
  function acceptSuffix(){if(!suffix||!canAcceptSuffix(suffixPrefix,input.value,suffixContext,key(),input.selectionStart))return;const tail=suffix;hideSuffix();input.setRangeText(tail,input.value.length,input.value.length,'end');changed();input.focus();}
  async function complete(manual=false){
    if(composing||input.disabled||deps.busy()||deps.running()||!menu.hidden||aiBusy)return;
    if(!config.enabled){if(manual)void openSettings();return;}
    if(!manual&&Date.now()-lastAI<12000)return;
    const original=input.value,expected=key();if(original.trim().length<4||original.length>2000||input.selectionStart!==original.length)return;
    aiBusy=true;lastAI=Date.now();hint.textContent='ローカル優先で補完中…';
    try{const value=await deps.call<{suffix:string;source:string;fallback_reason:string|null}>('complete_prompt',{projectId:deps.project(),prefix:original});if(key()===expected){showSuffix(value.suffix,value.source,original,expected);hint.textContent=value.fallback_reason?`${value.source} · ${value.fallback_reason}`:`${value.source} で補完`; }}
    catch(e){if(key()===expected)hint.textContent=String(e);}
    finally{aiBusy=false;if(hint.textContent?.includes('補完中'))hint.textContent='候補は Tab で採用 · Esc で閉じる';}
  }
  function onInput(){
    clearTimeout(timer);dismissed='';hideSuffix();if(composing)return;
    const next=triggerAt(input.value,input.selectionStart);
    if(next){if(trigger?.start!==next.start)category='';trigger=next;void search(next.query);return;}
    trigger=null;category='';generation++;hideMenu();
    const value=historySuffix(input.value,history);if(value){showSuffix(value,'入力履歴',input.value,key());return;}
    timer=setTimeout(()=>void complete(),1400);
  }
  input.addEventListener('input',onInput);
  input.addEventListener('compositionstart',()=>{composing=true;clearTimeout(timer);hideMenu();hideSuffix();});
  input.addEventListener('compositionend',()=>{composing=false;onInput();});
  input.addEventListener('keydown',e=>{
    if(composing||e.isComposing||e.keyCode===229)return;
    const take=()=>{e.preventDefault();e.stopImmediatePropagation();};
    if(e.key==='Escape'){take();dismissed=input.value;generation++;clearTimeout(timer);hideMenu();hideSuffix();return;}
    if(e.ctrlKey&&e.code==='Space'){take();void complete(true);return;}
    if(!menu.hidden&&['ArrowDown','ArrowUp'].includes(e.key)){take();selected=(selected+(e.key==='ArrowDown'?1:-1)+Math.max(1,candidates.length))%Math.max(1,candidates.length);paint();return;}
    if(!menu.hidden&&((e.key==='Enter'&&!e.metaKey&&!e.ctrlKey)||e.key==='Tab')&&candidates[selected]){take();candidates[selected].choose();return;}
    if(e.key==='Tab'&&suffix){take();acceptSuffix();}
  },true);
  input.addEventListener('click',()=>{if(input.selectionStart!==input.value.length)hideSuffix();});
  byId('composer-slash').onclick=()=>open('/');byId('composer-at').onclick=()=>open('@');byId('composer-history').onclick=()=>open('/','history');byId('composer-ai').onclick=()=>void complete(true);byId('composer-settings').onclick=()=>void openSettings();
  byId('completion-close').onclick=()=>settings.close();
  async function openSettings(){
    settings.showModal();byId('completion-error').textContent='';
    try{config=await deps.call<CompletionConfig>('completion_settings');byId<HTMLInputElement>('completion-enabled').checked=config.enabled;}
    catch(e){byId('completion-error').textContent=String(e);}
  }
  settings.querySelector('form')!.onsubmit=async e=>{
    e.preventDefault();const next={enabled:byId<HTMLInputElement>('completion-enabled').checked,model:'local-first',reasoning:'low'};
    try{await deps.call('set_completion_settings',{config:next});config=next;settings.close();hint.textContent=config.enabled?'履歴 → ローカル → Luna / low':'入力履歴から補完';}catch(err){byId('completion-error').textContent=String(err);}
  };
  async function remember(text:string){const project=deps.project();if(!project||!text.trim())return;try{await deps.call('remember_prompt',{projectId:project,text});const loaded=await deps.call<string[]>('prompt_history',{projectId:project});if(deps.project()===project)history=loaded;}catch(e){deps.notice(`入力履歴を保存できませんでした: ${e}`);}}
  async function seedHistory(texts:string[],project:string){
    try{for(const text of texts)await deps.call('remember_prompt',{projectId:project,text});const loaded=await deps.call<string[]>('prompt_history',{projectId:project});if(deps.project()===project)history=loaded;}catch(e){deps.notice(`入力履歴を読み込めませんでした: ${e}`);}
  }
  function syncContext(){
    if(context===key())return;context=key();catalog=null;catalogPromise=null;history=[];generation++;interactionVersion++;lastInteraction='';interactions.hidden=true;interactions.replaceChildren();hideMenu();hideSuffix();clearTimeout(timer);
    const expected=key();if(deps.project())void deps.call<string[]>('prompt_history',{projectId:deps.project()}).then(v=>{if(key()===expected)history=v;}).catch(e=>{if(key()===expected)deps.notice(String(e));});
  }
  async function refreshThread(restoreMode=false){
    const expected=key(),thread=deps.thread(),version=++interactionVersion;
    if(!thread||!deps.codex()){interactions.hidden=true;return;}
    try{
      const state=await deps.call<{mode:string;goal:null|{objective:string;status:string;tokensUsed:number};questions:Question[];warning:string|null}>('composer_thread_state',{threadId:thread});
      if(key()!==expected||version!==interactionVersion)return;
      if(restoreMode&&!deps.running())deps.setMode(state.mode==='plan'?'plan':'implement');
      const serialized=JSON.stringify(state);if(serialized===lastInteraction)return;lastInteraction=serialized;interactions.replaceChildren();interactions.hidden=!state.goal&&!state.questions.length;
      if(state.goal){const row=document.createElement('div');row.className='composer-goal';const text=document.createElement('span');text.textContent=`ゴール · ${state.goal.status} · ${state.goal.objective}`;row.append(text);if(state.goal.status==='active'){const pause=document.createElement('button');pause.textContent='ゴールを一時停止';pause.onclick=async()=>{pause.disabled=true;try{await deps.call('pause_composer_goal',{threadId:thread});lastInteraction='';await refreshThread();}catch(e){deps.error(String(e));pause.disabled=false;}};row.append(pause);}interactions.append(row);}
      for(const request of state.questions){const form=document.createElement('form');form.className='composer-question';const fields:HTMLInputElement[]=[];
        for(const q of request.questions){const label=document.createElement('label');label.textContent=q.question;const field=document.createElement('input');field.type=q.isSecret?'password':'text';field.autocomplete='off';field.dataset.question=q.id;field.setAttribute('aria-label',q.question);label.append(field);form.append(label);fields.push(field);
          for(const option of q.options||[]){const choice=document.createElement('button');choice.type='button';choice.textContent=`${option.label} — ${option.description}`;choice.onclick=()=>{field.value=option.label;};form.append(choice);}}
        const error=document.createElement('p');error.setAttribute('role','alert');form.append(error);const send=document.createElement('button');send.type='submit';send.className='primary';send.textContent='回答する';form.append(send);
        form.onsubmit=async event=>{event.preventDefault();const answers:Record<string,string[]>={};for(const field of fields)if(field.value.trim())answers[field.dataset.question!]=[field.value.trim()];if(Object.keys(answers).length!==fields.length){error.textContent='各質問に回答してください。';return;}send.disabled=true;try{await deps.call('answer_composer_question',{threadId:thread,requestId:request.request_id,answers});lastInteraction='';await refreshThread();}catch(e){error.textContent=String(e);send.disabled=false;}};interactions.append(form);}
    }catch(e){if(key()===expected)deps.notice(String(e));}
  }
  async function beforeSend():Promise<boolean>{
    if(composing)return false;if(deps.running()&&/^\s*[/@]/.test(input.value)){deps.error("実行中はコマンドを使えません。完了を待ってください。");return false;}hideSuffix();clearTimeout(timer);
    const match=/^\s*([/@])(plan|goal|implement|workflow|skills|plugins|history)(?=\s|$)\s*/.exec(input.value);
    if(match){const command=match[2];if(!['skills','plugins','history'].includes(command)&&!deps.setMode(command))return false;input.value=input.value.slice(match[0].length);changed();
      if(['skills','plugins','history'].includes(command)){open('/',command==='skills'?'skill':command==='plugins'?'plugin':'history');return false;}
      if(!input.value.trim())return false;
    } else if(/^\s*\//.test(input.value)){
      const command=/^\s*\/([^\s]+)\s*/.exec(input.value);const original=input.value,expected=key();const data=await getCatalog();if(key()!==expected||input.value!==original)return false;const found=data.entries.filter(e=>e.kind==='skill'||e.kind==='plugin').filter(e=>e.name===command?.[1]);
      if(found.length!==1){deps.error('スラッシュコマンドを候補から選択してください。');return false;}
      input.value=input.value.slice(command![0].length);add(found[0].kind==='skill'?{kind:'skill',name:found[0].name,path:found[0].path}:{kind:'plugin',name:found[0].name,path:found[0].path});if(!input.value.trim())return false;
    }
    hideMenu();return true;
  }
  void deps.call<CompletionConfig>('completion_settings').then(value=>{config=value;hint.textContent=value.enabled?'履歴 → ローカル → Luna / low':'入力履歴から補完';}).catch(e=>deps.notice(String(e)));
  return {getMentions:()=>mentions.map(m=>({...m})),setMentions:(items:Mention[])=>{mentions=items;renderChips();},clear:()=>{mentions=[];renderChips();hideMenu();hideSuffix();changed();},syncContext,refreshThread,remember,seedHistory,beforeSend,openSettings};
}
