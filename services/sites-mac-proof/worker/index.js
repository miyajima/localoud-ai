// Private transport diagnostic. No files, shell, HTTP-to-socket relay or global sockets.
const A='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',B='bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb';
const json=(value,status=200)=>new Response(JSON.stringify(value),{status,headers:{'content-type':'application/json','cache-control':'no-store'}});
function authorized(request,expected){const actual=request.headers.get('x-localoud-proof-token');if(typeof expected!=='string'||expected.length<32||typeof actual!=='string'||actual.length!==expected.length)return false;let diff=0;for(let i=0;i<actual.length;i++)diff|=actual.charCodeAt(i)^expected.charCodeAt(i);return diff===0;}
function socket(request,env,url){
 if(request.method!=='GET')return json({error:'method_not_allowed'},405);
 if(request.headers.get('upgrade')?.toLowerCase()!=='websocket')return json({error:'websocket_required'},426);
 const origin=request.headers.get('origin');if(origin&&origin!==url.origin)return json({error:'invalid_origin'},403);
 const device=url.searchParams.get('device'),key=device===A?'PROOF_DEVICE_A_TOKEN':device===B?'PROOF_DEVICE_B_TOKEN':null;
 if(!key||!authorized(request,env[key]))return json({error:'forbidden'},403);
 const pair=new WebSocketPair(),client=pair[0],server=pair[1],connection=crypto.randomUUID();server.accept();
 server.addEventListener('message',event=>{
  if(typeof event.data!=='string'||event.data.length>1024){server.close(1008,'invalid_message');return;}
  let data;try{data=JSON.parse(event.data);}catch{server.close(1008,'invalid_json');return;}
  if(!data||data.type!=='ping'||typeof data.nonce!=='string'||!/^[a-f0-9]{32}$/.test(data.nonce)||Object.keys(data).some(k=>!['type','nonce'].includes(k))){server.close(1008,'invalid_probe');return;}
  server.send(JSON.stringify({type:'pong',nonce:data.nonce,device,connection}));
 });
 server.send(JSON.stringify({type:'ready',device,connection}));
 return new Response(null,{status:101,webSocket:client});
}
export default {async fetch(request,env){
 const url=new URL(request.url);
 if(url.pathname==='/health'&&request.method==='GET')return json({service:'localoud-sites-mac-proof',worker_websocket_pair:typeof WebSocketPair==='function',production_ready:false,phase:'transport_only',probe_protocol:'nonce-v1'});
 if(url.pathname==='/ws')return socket(request,env,url);
 if(url.pathname==='/mcp'&&request.method==='POST'){
  // Only a bounded, read-only diagnostic tool is published.
  const size=Number(request.headers.get('content-length'));if(!Number.isFinite(size)||size<0||size>4096)return json({error:'body_too_large'},413);
  let text='';const reader=request.body?.getReader();if(!reader)return json({error:'invalid_body'},400);
  try{const decoder=new TextDecoder();let bytes=0;while(true){const {value,done}=await reader.read();if(done)break;bytes+=value.byteLength;if(bytes>4096){await reader.cancel();return json({error:'body_too_large'},413);}text+=decoder.decode(value,{stream:true});}text+=decoder.decode();}catch{return json({error:'invalid_body'},400);}
  let rpc;try{rpc=JSON.parse(text);}catch{return json({error:'invalid_json'},400);}
  if(!rpc||rpc.jsonrpc!=='2.0')return json({error:'invalid_rpc'},400);if(rpc.id===undefined)return new Response(null,{status:202});
  let result;if(rpc.method==='server/discover')result={supportedVersions:['2026-07-28'],capabilities:{tools:{}}};
  else if(rpc.method==='tools/list')result={tools:[{name:'transport_health',description:'Read transport diagnostic availability. No file or command access.',inputSchema:{type:'object',properties:{},additionalProperties:false},annotations:{readOnlyHint:true,destructiveHint:false,openWorldHint:false}}]};
  else if(rpc.method==='tools/call'&&rpc.params?.name==='transport_health')result={content:[{type:'text',text:JSON.stringify({phase:'transport_only',production_ready:false})}]};
  else return json({jsonrpc:'2.0',id:rpc.id,error:{code:-32601,message:'Method not found'}});
  return json({jsonrpc:'2.0',id:rpc.id,result});
 }
 if(url.pathname==='/'&&request.method==='GET')return new Response('<!doctype html><html lang="ja"><meta charset="utf-8"><title>Localoud 接続検証</title><main><h1>Localoud Sites–Mac接続検証</h1><p>本人限定の通信試験です。ファイル編集・コマンド実行機能はありません。</p><p>WebSocket接続、接続維持、再接続と往復遅延を検証します。</p><p>今回は通信路のみの診断です。別のHTTP要求からMacへ操作を届ける中継は含みません。</p></main></html>',{headers:{'content-type':'text/html;charset=utf-8','cache-control':'no-store','content-security-policy':"default-src 'none'; frame-ancestors 'none'; base-uri 'none'"}});
 return json({error:'not_found'},404);
}};
