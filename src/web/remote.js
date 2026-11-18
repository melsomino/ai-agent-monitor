/* HTTP host; the preview continues to use its isolated mock host. */
(() => {
 'use strict';
 let socket, retry, snapshot, stopped=false, requestSerial=0,synchronized=false;
 const pending=new Map(),commands=new Map(),historyRequests=new Map();
 const diagnosticMode=new URLSearchParams(location.search).get('diagnostics')==='1';
 let mode='live',frozen=null,repeatTimer=null,connectionState='connecting';
 const metrics={connections:0,closes:0,packets:0,renders:0,syncErrors:0,renderErrors:0};
 let diagnostics,lastDiagnosticError='';
 function drawMetrics(){
  if(!diagnostics)return;
  const message=`${connectionState} · reconnects ${Math.max(0,metrics.connections-1)} · closes ${metrics.closes} · packets ${metrics.packets} · renders ${metrics.renders} · sync errors ${metrics.syncErrors} · render errors ${metrics.renderErrors}`;
  if(diagnostics.stats.textContent!==message)diagnostics.stats.textContent=message;
  diagnostics.error.textContent=lastDiagnosticError;diagnostics.error.hidden=!lastDiagnosticError;
 }
 function diagnosticError(error,rendering=false){
  if(rendering)metrics.renderErrors++;else metrics.syncErrors++;
  lastDiagnosticError=String(error?.message||error||'Unknown error').slice(0,200);drawMetrics();
 }
 function changeMode(next){
  if(mode==='live'&&next!=='live')frozen=snapshot;
  clearInterval(repeatTimer);repeatTimer=null;mode=next;
  if(mode==='live'){frozen=null;if(snapshot)present(snapshot);}
  else if(frozen)present(frozen);
  if(mode==='repeat')repeatTimer=setInterval(()=>{
   if(!document.hidden&&frozen&&!stopped&&gate.hidden){try{present(frozen);}catch(error){diagnosticError(error,true);}}
  },250);
  if(diagnostics)diagnostics.note.textContent=mode==='live'?'Applying live state.':mode==='freeze'?'Data frozen; WebSocket still receives updates. Responses are disabled.':'Repeating one frozen snapshot 4×/s; WebSocket stays live. Responses are disabled.';
  drawMetrics();
 }
 function installDiagnostics(){
  if(!diagnosticMode)return;
  const panel=document.createElement('section');panel.id='aam-diagnostics';panel.setAttribute('aria-label','Web diagnostics');
  const label=document.createElement('label');label.textContent='Diagnostics ';
  const select=document.createElement('select');select.id='aam-diagnostics-mode';
  for(const [value,name] of [['live','Live'],['freeze','Freeze'],['repeat','Repeat 4/s']]){
   const option=document.createElement('option');option.value=value;option.textContent=name;select.append(option);
  }
  select.value=mode;select.onchange=()=>changeMode(select.value);label.append(select);
  const reset=document.createElement('button');reset.type='button';reset.id='aam-diagnostics-reset';reset.textContent='Reset counters';reset.onclick=()=>{
   for(const key of Object.keys(metrics))metrics[key]=0;
   // The current connection is the baseline, not a reconnect after reset.
   if(socket)metrics.connections=1;
   lastDiagnosticError='';drawMetrics();
  };
  const stats=document.createElement('p');stats.id='aam-diagnostics-stats';
  const note=document.createElement('p');note.textContent='Applying live state.';
  const error=document.createElement('p');error.id='aam-diagnostics-error';error.hidden=true;
  panel.append(label,reset,stats,note,error);document.body.prepend(panel);document.body.classList.add('aam-diagnostics-enabled');
  diagnostics={stats,note,error};
  window.addEventListener('error',event=>diagnosticError(event.error||event.message,true));
  window.addEventListener('unhandledrejection',event=>diagnosticError(event.reason,true));
  const size=()=>document.documentElement.style.setProperty('--aam-diagnostics-height',`${panel.offsetHeight}px`);
  new ResizeObserver(size).observe(panel);size();drawMetrics();
 }

 const notice=document.createElement('p');notice.id='aam-remote-notice';notice.role='status';notice.hidden=true;document.body.append(notice);
 const gate=document.createElement('section');gate.id='aam-remote-gate';gate.hidden=true;
 const title=document.createElement('h1');title.textContent='Connect to AI Agent Monitor';
 const detail=document.createElement('p'),again=document.createElement('button');again.textContent='Try again';again.onclick=()=>start();gate.append(title,detail,again);document.body.append(gate);
 const theme=()=>matchMedia('(prefers-color-scheme: light)').matches?'light':'dark';
 const info=text=>{notice.textContent=text;notice.hidden=!text;};
 const locked=text=>{gate.hidden=false;detail.textContent=text;clearInterval(repeatTimer);repeatTimer=null;frozen=null;paint({sessions:[],usage:[],theme:theme()});};
 async function api(path,body){
  const response=await fetch(path,{method:body===undefined?'GET':'POST',credentials:'same-origin',cache:'no-store',headers:body===undefined?{}:{'Content-Type':'application/json'},body:body===undefined?undefined:JSON.stringify(body)});
  const data=await response.json();if(response.status===401)locked(data.error);if(!response.ok)throw Error(data.error||'Request failed.');return data;
 }
 function paint(value){window.aam.setSnapshot(value);metrics.renders++;drawMetrics();}
 function present(value){
  const readonly=mode!=='live';
  paint({...value,theme:theme(),usageResetsEnabled:!readonly,sessions:value.sessions.map(s=>({...s,
   authoritative:s.authoritative&&!readonly&&!pending.has(s.id),
   activities:readonly?s.activities.map(a=>a.type==='request'?{...a,request:{...a.request,enabled:false}}:a):s.activities,
   history:historyRequests.has(s.id)?{...s.history,loading:true}:s.history}))});
 }
 function render(value){
  snapshot=value;gate.hidden=true;
  if(mode==='live')present(value);
  else if(!frozen){frozen=value;present(value);}
  drawMetrics();
 }
 function offline(){
  synchronized=false;connectionState='offline';
  // Transport diagnostics must remain observable even with the data display frozen.
  const value=mode==='live'?snapshot:frozen;
  if(value)paint({...value,usageResetsEnabled:false,error:'Disconnected. Reconnecting…',sessions:value.sessions.map(s=>({...s,authoritative:false,activities:s.activities.map(a=>a.type==='request'?{...a,request:{...a.request,enabled:false}}:a)}))});
  drawMetrics();
 }
 function rejectCommands(ws){for(const [id,command] of commands){if(command.socket!==ws)continue;commands.delete(id);clearTimeout(command.timer);command.reject(Error('Connection lost. The history request was not retried.'));}}
 function connect(){
  const ws=new WebSocket(`${location.protocol==='https:'?'wss:':'ws:'}//${location.host}/api/events`);socket=ws;metrics.connections++;connectionState='connecting';drawMetrics();
  ws.onopen=()=>{if(socket===ws){connectionState='connected';drawMetrics();}};
  const stream=new window.AAMStream();let resync=false,resyncTimer;
  ws.onmessage=event=>{
   if(socket!==ws)return;
   let rendering=false;
   try{
    const message=JSON.parse(event.data);
    if(message.type==='command_result'){
     const command=commands.get(message.id);if(!command||command.socket!==ws)return;
     commands.delete(message.id);clearTimeout(command.timer);
     if(message.protocol===1&&message.accepted===true)command.resolve();else command.reject(Error(message.error||'History request failed.'));
     return;
    }
    if(resync&&message.type!=='snapshot')return;
    const value=stream.receive(message);metrics.packets++;rendering=true;render(value);synchronized=true;connectionState='synchronized';resync=false;clearTimeout(resyncTimer);info('');drawMetrics();
   }catch(error){
    diagnosticError(error,rendering);offline();connectionState='resynchronizing';drawMetrics();info('Resynchronizing…');
    if(resync){ws.close();return;}
    resync=true;
    if(ws.readyState===WebSocket.OPEN)ws.send(JSON.stringify({type:'resync',protocol:1}));else ws.close();
    resyncTimer=setTimeout(()=>ws.close(),5000);
   }
  };
  ws.onclose=()=>{clearTimeout(resyncTimer);rejectCommands(ws);if(stopped||socket!==ws)return;metrics.closes++;offline();info('Disconnected. Reconnecting…');retry=setTimeout(start,2000);};ws.onerror=()=>ws.close();
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
   if(mode!=='live'&&['submit','message','history','seen','open','reset-credits'].includes(event.type)){info('Switch Diagnostics to Live to interact with the agent.');return;}
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
   if(event.type==='reset-credits'){
    try{await api('/api/command',{type:'reset-credits'});}
    catch(error){if(snapshot)paint({...snapshot,theme:theme(),usageResets:{phase:'error',error:error.message}});throw error;}
    return;
   }
   if(event.type==='history'){await requestHistory(event.session);return;}
   if(['submit','seen','open'].includes(event.type))await api('/api/command',event);
  }catch(error){info(error.message);}
 }
 window.aam=window.AAMMonitor.mount(document.getElementById('aam-ui-preview'),send,{settings:false,live:true});
 const quit=document.querySelector('[data-command="quit"]');quit.setAttribute('aria-label','Disconnect this browser');quit.title='Disconnect this browser';
 installDiagnostics();
 matchMedia('(prefers-color-scheme: light)').addEventListener('change',()=>{const value=mode==='live'?snapshot:frozen;if(value)present(value);});
 window.addEventListener('pagehide',()=>{clearInterval(repeatTimer);repeatTimer=null;});
 window.addEventListener('pageshow',()=>{if(diagnosticMode&&mode==='repeat'&&!repeatTimer)changeMode('repeat');});
 document.addEventListener('visibilitychange',()=>{if(!document.hidden&&!stopped)start();});
 (async()=>{
  const token=new URLSearchParams(location.hash.slice(1)).get('pair');if(location.hash)history.replaceState(null,'',location.pathname+location.search);
  if(token){info('Pairing…');try{await api('/api/pair',{token,name:/iPhone/.test(navigator.userAgent)?'iPhone':/Android/.test(navigator.userAgent)?'Android browser':'Web browser'});}catch(error){locked(error.message);info('');return;}}
  await start();
 })();
})();
