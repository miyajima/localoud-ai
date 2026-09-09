import { isTauri } from '@tauri-apps/api/core';
import './browser-workspace.css';

type Invoke = <T = unknown>(command: string, args?: Record<string, unknown>) => Promise<T>;

export function setupBrowserWorkspace(invoke: Invoke, visibilityChanged?: (visible:boolean)=>void) {
  const app = document.querySelector<HTMLElement>('#app')!;
  const main = app.querySelector<HTMLElement>('main')!;
  const header = main.querySelector('header')!;
  const shell=document.createElement('section');shell.id='workflow-shell';
  const workflow=document.createElement('section');workflow.id='workflow-bar';workflow.setAttribute('aria-label','選択中のタスクの工程');
  shell.append(workflow);
  for(const id of ['error','notice']){const feedback=document.getElementById(id);if(feedback)shell.append(feedback);}
  header.after(shell);
  const toggle=document.createElement('button');toggle.id='browser-toggle';toggle.type='button';toggle.setAttribute('aria-controls','browser-panel');header.prepend(toggle);
  const panel=document.createElement('section');panel.id='browser-panel';panel.setAttribute('aria-label','ChatGPT');
  panel.innerHTML='<div class="browser-toolbar"><strong>ChatGPT</strong><span>計画・相談・レビュー</span><button id="browser-reload" type="button" title="ChatGPTを再読み込み" aria-label="ChatGPTを再読み込み">↻</button></div><div id="chatgpt-usage"></div><div id="browser-surface"><p id="browser-error">ChatGPTを開いています…</p></div>';
  const separator=(id:string,label:string)=>{const el=document.createElement('div');el.id=id;el.tabIndex=0;el.setAttribute('role','separator');el.setAttribute('aria-label',label);el.setAttribute('aria-orientation','vertical');return el;};
  const sidebarDivider=separator('sidebar-divider','プロジェクト一覧の幅');
  const divider=separator('browser-divider','ChatGPTとLocaloudの幅');
  main.before(sidebarDivider,panel,divider);
  document.body.classList.add('browser-open');
  const stored=(key:string,fallback:number)=>{const n=Number(localStorage.getItem(key)??fallback);return Number.isFinite(n)?n:fallback;};
  let sidebarWidth=Math.max(160,Math.min(420,stored('localoud-sidebar-width',234)));
  let fraction=Math.max(.3,Math.min(.65,stored('localoud-browser-width',.46)));
  let collapsed=localStorage.getItem('localoud-browser-hidden')==='true';
  let selected:'browser'|'workspace'='workspace';
  let sidebarHidden=false,pending=false,again=false;
  const narrow=()=>window.innerWidth-(sidebarHidden?38:sidebarWidth+6)<730;
  function setWidth(){
    app.style.setProperty('--sidebar-width',`${sidebarWidth}px`);
    app.style.setProperty('--browser-track',`${fraction}fr`);
    app.style.setProperty('--workspace-track',`${1-fraction}fr`);
    for(const [el,min,max,value] of [[sidebarDivider,160,420,sidebarWidth],[divider,30,65,Math.round(fraction*100)]] as const){el.setAttribute('aria-valuemin',String(min));el.setAttribute('aria-valuemax',String(max));el.setAttribute('aria-valuenow',String(value));}
    app.classList.toggle('compact-panes',narrow());
  }
  function reflect(){
    setWidth();app.classList.toggle('chatgpt-hidden',collapsed);app.dataset.workspacePane=selected;
    const showing=!collapsed&&(!narrow()||selected==='browser');
    toggle.textContent=showing?'◧ ChatGPT':'◧ ChatGPT';toggle.setAttribute('aria-expanded',String(showing));
    toggle.setAttribute('aria-label',showing?'ChatGPTを非表示':'ChatGPTを表示');toggle.title=(showing?'ChatGPTを非表示':'ChatGPTを表示')+' ⌘⇧B';
    schedule();
  }
  async function layout(){
    const message=panel.querySelector('#browser-error')!;
    if(!isTauri()){message.textContent='ChatGPTはデスクトップアプリで表示できます。';return;}
    if(pending){again=true;return;}pending=true;
    const rect=panel.querySelector('#browser-surface')!.getBoundingClientRect();
    const visible=!app.classList.contains('resizing-panes')&&!collapsed&&(!narrow()||selected==='browser')&&!document.querySelector('dialog[open]');
    try{await invoke('browser_layout',{bounds:{x:Math.max(0,rect.x),y:Math.max(0,rect.y),width:Math.max(1,rect.width),height:Math.max(1,rect.height),viewport_height:window.innerHeight,visible}});message.textContent='';}
    catch(e){message.textContent=`ブラウザを開けませんでした: ${String(e)}`;}
    finally{pending=false;if(again){again=false;schedule();}}
  }
  function schedule(){void layout();}
  function collapse(value:boolean){collapsed=value;localStorage.setItem('localoud-browser-hidden',String(value));if(!value)selected='browser';reflect();visibilityChanged?.(!value);}
  function toggleBrowser(){collapse(!collapsed&&(!narrow()||selected==='browser'));}
  toggle.addEventListener('click',toggleBrowser);
  window.addEventListener('keydown',e=>{if((e.metaKey||e.ctrlKey)&&e.shiftKey&&e.code==='KeyB'){e.preventDefault();toggleBrowser();}});
  panel.querySelector('#browser-reload')!.addEventListener('click',()=>{void invoke('browser_reload').catch(e=>{panel.querySelector('#browser-error')!.textContent=String(e);});});
  const saveWidths=()=>{localStorage.setItem('localoud-sidebar-width',String(sidebarWidth));localStorage.setItem('localoud-browser-width',String(fraction));};
  for(const el of [sidebarDivider,divider]){
    el.addEventListener('pointerdown',e=>{
      el.setPointerCapture(e.pointerId);
      // Native child webviews otherwise consume drag events across the divider.
      app.classList.add('resizing-panes');void invoke('browser_layout',{bounds:{x:0,y:0,width:1,height:1,viewport_height:window.innerHeight,visible:false}});
      const move=(event:PointerEvent)=>{if(el===sidebarDivider)sidebarWidth=Math.max(160,Math.min(420,event.clientX-app.getBoundingClientRect().x));else{const start=sidebarHidden?38:sidebarWidth+6;fraction=Math.max(.3,Math.min(.65,(event.clientX-start)/(window.innerWidth-start-6)));}setWidth();};
      const stop=()=>{saveWidths();app.classList.remove('resizing-panes');reflect();el.removeEventListener('pointermove',move);el.removeEventListener('pointerup',stop);el.removeEventListener('pointercancel',stop);};
      el.addEventListener('pointermove',move);el.addEventListener('pointerup',stop);el.addEventListener('pointercancel',stop);
    });
    el.addEventListener('keydown',e=>{if(!['ArrowLeft','ArrowRight'].includes(e.key))return;e.preventDefault();const direction=e.key==='ArrowLeft'?-1:1;if(el===sidebarDivider)sidebarWidth=Math.max(160,Math.min(420,sidebarWidth+direction*16));else fraction=Math.max(.3,Math.min(.65,fraction+direction*.02));saveWidths();reflect();});
  }
  new MutationObserver(schedule).observe(app,{subtree:true,attributes:true,attributeFilter:['open']});
  new ResizeObserver(()=>{if(!app.classList.contains('resizing-panes'))schedule();}).observe(panel.querySelector('#browser-surface')!);
  window.addEventListener('resize',reflect);
  reflect();
  return {
    showWorkspace:()=>{selected='workspace';reflect();},
    showBrowser:()=>collapse(false),
    hideBrowser:()=>collapse(true),
    isBrowserOpen:()=>!collapsed,
    setSidebarHidden:(value:boolean)=>{sidebarHidden=value;app.classList.toggle('sidebar-hidden',value);const button=app.querySelector('#sidebar-toggle');button?.setAttribute('aria-expanded',String(!value));button?.setAttribute('aria-label',value?'プロジェクト一覧を表示':'プロジェクト一覧を閉じる');reflect();},
    setWorkflow:(markup:string,action:(name:string)=>void)=>{
      if(workflow.innerHTML===markup)return;workflow.innerHTML=markup;
      workflow.querySelectorAll<HTMLButtonElement>('[data-flow-action]').forEach(button=>button.addEventListener('click',()=>action(button.dataset.flowAction!)));schedule();
    }
  };
}
