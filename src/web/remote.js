/* HTTP host; the preview continues to use its isolated mock host. */
(() => {
 'use strict';
 let socket, retry, snapshot, stopped=false, requestSerial=0,synchronized=false;
 const pending=new Map(),commands=new Map(),historyRequests=new Map();
 const notice=document.createElement('p');notice.id='aam-remote-notice';notice.role='status';notice.hidden=true;document.body.append(notice);
 const gate=document.createElement('section');gate.id='aam-remote-gate';gate.hidden=true;
 const title=document.createElement('h1');title.textContent='Connect to AI Agent Monitor';
 const detail=document.createElement('p'),again=document.createElement('button');again.textContent='Try again';again.onclick=()=>start();gate.append(title,detail,again);document.body.append(gate);
 const theme=()=>matchMedia('(prefers-color-scheme: light)').matches?'light':'dark';
 const info=text=>{notice.textContent=text;notice.hidden=!text;};
 const locked=text=>{gate.hidden=false;detail.textContent=text;window.aam.setSnapshot({sessions:[],usage:[],theme:theme()});};
 async function api(path,body){
  const response=await fetch(path,{method:body===undefined?'GET':'POST',credentials:'same-origin',cache:'no-store',headers:body===undefined?{}:{'Content-Type':'application/json'},body:body===undefined?undefined:JSON.stringify(body)});
  const data=await response.json();if(response.status===401)locked(data.error);if(!response.ok)throw Error(data.error||'Request failed.');return data;
 }
 function render(value){snapshot=value;gate.hidden=true;window.aam.setSnapshot({...value,theme:theme(),sessions:value.sessions.map(s=>({...s,authoritative:s.authoritative&&!pending.has(s.id),history:historyRequests.has(s.id)?{...s.history,loading:true}:s.history}))});}
 function offline(){synchronized=false;if(snapshot)window.aam.setSnapshot({...snapshot,error:'Disconnected. Reconnecting…',sessions:snapshot.sessions.map(s=>({...s,authoritative:false,activities:s.activities.map(a=>a.type==='request'?{...a,request:{...a.request,enabled:false}}:a)}))});}
 function rejectCommands(ws){for(const [id,command] of commands){if(command.socket!==ws)continue;commands.delete(id);clearTimeout(command.timer);command.reject(Error('Connection lost. The history request was not retried.'));}}
 function connect(){
  const ws=new WebSocket(`${location.protocol==='https:'?'wss:':'ws:'}//${location.host}/api/events`);socket=ws;
  const stream=new window.AAMStream();let resync=false,resyncTimer;
  ws.onmessage=event=>{
   if(socket!==ws)return;
   try{
    const message=JSON.parse(event.data);
    if(message.type==='command_result'){
     const command=commands.get(message.id);if(!command||command.socket!==ws)return;
     commands.delete(message.id);clearTimeout(command.timer);
     if(message.protocol===1&&message.accepted===true)command.resolve();else command.reject(Error(message.error||'History request failed.'));
     return;
    }
    if(resync&&message.type!=='snapshot')return;
    render(stream.receive(message));synchronized=true;resync=false;clearTimeout(resyncTimer);info('');
   }catch{
    offline();info('Resynchronizing…');
    if(resync){ws.close();return;}
    resync=true;
    if(ws.readyState===WebSocket.OPEN)ws.send(JSON.stringify({type:'resync',protocol:1}));else ws.close();
    resyncTimer=setTimeout(()=>ws.close(),5000);
   }
  };
  ws.onclose=()=>{clearTimeout(resyncTimer);rejectCommands(ws);if(stopped||socket!==ws)return;offline();info('Disconnected. Reconnecting…');retry=setTimeout(start,2000);};ws.onerror=()=>ws.close();
 }
 async function start(){
  clearTimeout(retry);stopped=false;const serial=++requestSerial;
  synchronized=false;if(socket){rejectCommands(socket);socket.onclose=null;socket.close();socket=null;}info('Connecting…');
  try{const value=await api('/api/snapshot');if(serial!==requestSerial||stopped)return;render(value);connect();}
  catch(error){if(serial!==requestSerial||stopped)return;info(error.message);if(gate.hidden){offline();retry=setTimeout(start,3000);}}
 }
 function operationId(){const bytes=new Uint8Array(16);crypto.getRandomValues(bytes);return [...bytes].map(b=>b.toString(16).padStart(2,'0')).join('');}
 async function requestHistory(session){
  if(historyRequests.has(session))return await historyRequests.get(session);
  const ws=socket;
  if(!synchronized||!ws||ws.readyState!==WebSocket.OPEN)throw Error('Not connected. Wait for the monitor to reconnect.');
  const id=operationId();
  const promise=new Promise((resolve,reject)=>{
   const timer=setTimeout(()=>{commands.delete(id);reject(Error('History request timed out. Check the session before retrying.'));},10000);
   commands.set(id,{socket:ws,resolve,reject,timer});
   try{ws.send(JSON.stringify({type:'history',protocol:1,id,session}));}catch(error){commands.delete(id);clearTimeout(timer);reject(error);}
  });
  historyRequests.set(session,promise);if(snapshot)render(snapshot);
  try{return await promise;}finally{historyRequests.delete(session);if(snapshot&&synchronized)render(snapshot);}
 }
 async function send(event){
  try{
   if(['ready','size','expansion'].includes(event.type))return;
   if(event.type==='copy'){
    if(navigator.clipboard)await navigator.clipboard.writeText(event.text);else{const input=document.createElement('textarea');input.value=event.text;document.body.append(input);input.select();document.execCommand('copy');input.remove();}return;
   }
   if(event.type==='link'){window.open(event.url,'_blank','noopener,noreferrer');return;}
   if(event.type==='quit'){stopped=true;synchronized=false;++requestSerial;clearTimeout(retry);if(socket){rejectCommands(socket);socket.onclose=null;socket.close();}await api('/api/logout',{});locked('Disconnected. Scan a new QR code in Settings → Web to reconnect.');info('');return;}
   if(event.type==='close'){document.activeElement?.blur();return;}
   if(event.type==='message'){
    if(pending.has(event.session))return;pending.set(event.session,event.text);if(snapshot)render(snapshot);
    try{await api('/api/command',{...event,id:operationId()});window.aam.messageResult(event.session,event.text,true);}
    catch(error){window.aam.messageResult(event.session,event.text,false);throw error;}
    finally{pending.delete(event.session);if(snapshot)render(snapshot);}return;
   }
   if(event.type==='history'){await requestHistory(event.session);return;}
   if(['submit','seen','open'].includes(event.type))await api('/api/command',event);
  }catch(error){info(error.message);}
 }
 window.aam=window.AAMMonitor.mount(document.getElementById('aam-ui-preview'),send,{settings:false,live:true});
 const quit=document.querySelector('[data-command="quit"]');quit.setAttribute('aria-label','Disconnect this browser');quit.title='Disconnect this browser';
 matchMedia('(prefers-color-scheme: light)').addEventListener('change',()=>{if(snapshot)render(snapshot);});
 document.addEventListener('visibilitychange',()=>{if(!document.hidden&&!stopped)start();});
 (async()=>{
  const token=new URLSearchParams(location.hash.slice(1)).get('pair');if(location.hash)history.replaceState(null,'',location.pathname+location.search);
  if(token){info('Pairing…');try{await api('/api/pair',{token,name:/iPhone/.test(navigator.userAgent)?'iPhone':/Android/.test(navigator.userAgent)?'Android browser':'Web browser'});}catch(error){locked(error.message);info('');return;}}
  await start();
 })();
})();
