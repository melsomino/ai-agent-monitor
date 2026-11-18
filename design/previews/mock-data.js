/* Demo fixtures and simulated host only. The complete UI lives in src/web. */
(() => {
const applicationIcons = {"VS Code": "data:image/png;base64,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", "Terminal": "data:image/png;base64,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", "Codex": "data:image/png;base64,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"};
const sessions = [
    {id:'monitor', app:'VS Code', title:'ai agent monitor', status:'working', progress:'Running cargo test --offline --all-targets', model:'GPT-6.1-Sol Medium', message:'Updated event handling. Response delivery and session states have been verified.', directory:'~/dev/ai-agent-monitor', branch:'main'},
    {id:'delivery', app:'VS Code', title:'Task Board', status:'request', request:'approval', model:'GPT-6.1-Sol Medium', message:'<h3>Delivery queue</h3><p>Fixed <strong>duplicate delivery</strong> of responses.</p><ul><li>Local checks passed.</li><li>Changes in <code>delivery.rs</code> are ready.</li></ul><p>Run the project tests to complete verification.</p>', directory:'~/demo/task-board', branch:'feature/task-lists', history:'Separated response delivery from state updates. Each request now retains its own identifier.'},
    {id:'notes-studio', app:'VS Code', title:'Notes Studio', status:'request', request:'questions', message:'The editor is ready. I will confirm the scope of verification and where to send the results.', directory:'~/demo/notes-studio', branch:'feature/note-editor'},
    {id:'snapshot', app:'VS Code', title:'Project Dashboard', status:'finished', message:'All tests pass. The changes are ready for review.', directory:'~/demo/project-dashboard', branch:'main'},
    {id:'connection', app:'VS Code', title:'Restoring the connection after an agent restart and synchronizing active requests', status:'working', progress:'Thinking: Checking pending requests after reconnection', message:'Reconnection has been verified. Pending requests retained their identifiers.', directory:'~/dev/ai-agent-monitor', branch:'fix/reconnect'},
    {id:'permissions', app:'VS Code', title:'Handling permission requests for terminal commands', status:'idle', message:'Approval applies only to the current request. Retries are protected against duplicate delivery.', directory:'~/dev/ai-agent-monitor', branch:'fix/permissions'},
    {id:'metrics', app:'Terminal', title:'Return to metrics', status:'working', progress:'Running cargo test --offline --all-targets', message:'Comparing measurements before and after the index changes.', directory:'~/demo/metrics-service', branch:'feature/metrics'},
    {id:'processes', app:'Terminal', title:'Check running processes', status:'idle', message:'No unexpected processes are running. There is only one monitor instance.', directory:'~/dev/ai-agent-monitor', branch:'main'},
    {id:'restore', app:'Codex', title:'Restoring the application', status:'idle', message:'State persistence works. The next launch will restore open documents.', directory:'~/demo/desktop-workspace', branch:'main'},
    {id:'cargo', app:'Codex', title:'Set up the local Cargo.lock', status:'idle', message:'Dependencies are pinned. The build is reproducible with the local Cargo.lock.', directory:'~/dev/ai-agent-monitor', branch:'main'}
  ];
 const frame=document.getElementById('monitor');
 const previewWindow=document.getElementById('preview-window');
 const windowChrome=document.getElementById('preview-window-chrome');
 const pin=document.getElementById('preview-pin');
 let detached=false,windowDrag=null,zoomed=false;
 function placeWindow(left,top){
   const width=previewWindow.offsetWidth,height=previewWindow.offsetHeight;
   previewWindow.style.left=`${Math.max(8,Math.min(left,innerWidth-width-8))}px`;
   previewWindow.style.top=`${Math.max(34,Math.min(top,innerHeight-height-8))}px`;
 }
 function showWindow(){previewWindow.hidden=false;frame.contentWindow.postMessage({channel:'aam-host',visibility:true},'*');}
 pin.addEventListener('click',()=>{
   const bounds=previewWindow.getBoundingClientRect();detached=!detached;zoomed=false;
   previewWindow.classList.toggle('is-detached',detached);
   frame.contentWindow.postMessage({channel:'aam-host',detached},'*');
   pin.setAttribute('aria-pressed',String(detached));
   const label=detached?'Attach window to menu bar':'Detach window from menu bar';pin.title=label;pin.setAttribute('aria-label',label);
   previewWindow.setAttribute('aria-label',detached?'AI Agent Monitor detached window':'AI Agent Monitor menu window');
   previewWindow.querySelector('.preview-traffic-lights').hidden=!detached;
   previewWindow.querySelector('.preview-window-title').hidden=!detached;
   previewWindow.style.width='';previewWindow.style.height='';
   if(detached)placeWindow(bounds.left+32,54);else{previewWindow.style.left='';previewWindow.style.top='';}
 });
 windowChrome.addEventListener('pointerdown',event=>{
   if(!detached||event.button!==0||event.target.closest('button'))return;
   const bounds=previewWindow.getBoundingClientRect();windowDrag={id:event.pointerId,x:event.clientX-bounds.left,y:event.clientY-bounds.top};
   windowChrome.setPointerCapture(event.pointerId);previewWindow.classList.add('is-dragging');event.preventDefault();
 });
 windowChrome.addEventListener('pointermove',event=>{if(windowDrag?.id===event.pointerId)placeWindow(event.clientX-windowDrag.x,event.clientY-windowDrag.y);});
 function stopWindowDrag(){windowDrag=null;previewWindow.classList.remove('is-dragging');}
 windowChrome.addEventListener('pointerup',stopWindowDrag);windowChrome.addEventListener('pointercancel',stopWindowDrag);windowChrome.addEventListener('lostpointercapture',stopWindowDrag);
 previewWindow.querySelectorAll('[data-window]').forEach(button=>button.addEventListener('click',()=>{
   if(button.dataset.window==='zoom'){
     zoomed=!zoomed;previewWindow.style.width=zoomed?'min(660px,calc(100vw - 16px))':'';previewWindow.style.height=zoomed?'calc(100dvh - 62px)':'';
     const bounds=previewWindow.getBoundingClientRect();placeWindow(bounds.left,bounds.top);
   }else{previewWindow.hidden=true;frame.contentWindow.postMessage({channel:'aam-host',visibility:false},'*');}
 }));
 window.addEventListener('resize',()=>{if(detached){const bounds=previewWindow.getBoundingClientRect();placeWindow(bounds.left,bounds.top);}});

 // Synthetic update fixture; this is not a claim that v0.4.0 has been published.
 const versions={current:'v0.3.0 · preview',available:{version:'v0.4.0',url:'https://github.com/melsomino/ai-agent-monitor/releases/tag/v0.4.0'},checked_at:Math.floor(Date.now()/1000)};
 const newerRelease=versions.available;
 let theme='dark',context=1,updateTick=0,updateTimer=null;
 let remaining=85;
 let preferences={login:false,animate:true,shortcut:'',applications:{}};
 const state=sessions.map(s=>({...s,appId:s.app,icon:applicationIcons[s.app],html:s.message,
   fullDirectory:s.directory,completion:s.status==='finished'?1:null,
   expanded:s.id==='delivery', history:{messages:[],loading:false,exhausted:false}}));
 const questionList=[{id:'checks',title:'Which checks should run?',choices:[{label:'Only changed modules',description:''},{label:'The full test suite',description:''}],freeform:true,secret:false},{id:'destination',title:'Where should the results be sent?',choices:[{label:'This chat',description:''},{label:'A pull request',description:''}],freeform:true,secret:false}];
 for(const s of state) {
  if(s.request==='approval')s.request={context:String(context++),body:'Allow running the full test suite before publishing the changes?',command:'cargo test --locked --workspace --all-targets --all-features --no-fail-fast -- --test-threads=1 --nocapture',enabled:true,questions:[],actions:[{id:'allow',label:'Allow once',detail:'',primary:true},{id:'similar',label:'Allow similar',detail:'cargo test',primary:false},{id:'deny',label:'Deny',detail:'',primary:false}]};
  else if(s.request==='questions')s.request={context:String(context++),body:'',enabled:true,questions:questionList,actions:[]};
  else s.request=null;
 }
 let activitySerial=0;
 function activity(type,payload) {const time=new Date().toISOString();return {id:`fixture:${++activitySerial}`,type,startedAt:time,updatedAt:time,...payload};}
 const pending = s => s.activities.find(a=>a.type==='request'&&a.status==='pending');
 function trimActivities(s) {
   let ordinary=s.activities.filter(a=>a.type!=='request'||a.status!=='pending').length;
   while(ordinary>30 && s.activities.length) {
     if(s.activities[0].type==='request'&&s.activities[0].status==='pending')break;
     s.activities.shift();ordinary--;
   }
 }
 const addMessage = (s,html) => {s.activities.push(activity('message',{role:'agent',html}));trimActivities(s);};
 for(const s of state) {
   s.activities=[];
   addMessage(s,s.html);
   if(s.progress)s.activities.push(activity('action',{text:s.progress,current:true,status:'running'}));
   if(s.request)s.activities.push(activity('request',{request:s.request,status:'pending'}));
   delete s.html;delete s.message;delete s.progress;delete s.request;
 }
 let resetCreditTimer=null,resetCreditRequest=0;
 const resetCreditRows=[{title:'Full reset',expiresAt:Date.parse('2026-11-30T19:47:00Z')/1000},{title:'Full reset',expiresAt:Date.parse('2026-12-02T22:22:00Z')/1000},{title:'Full reset',expiresAt:Date.parse('2026-12-13T18:17:00Z')/1000}];
 let usageResets={generation:0,phase:'idle',availableCount:null,credits:null};
 function snapshot(){return {version:versions.current,versions,theme,animate:preferences.animate,preferences:structuredClone(preferences),applications:[...new Map(state.map(s=>[s.appId,{id:s.appId,name:s.app,icon:s.icon}])).values()],sessions:state.filter(s=>preferences.applications[s.appId]!==false),usage:[{label:'Weekly',remaining,reset:'Nov 11'}],usageResets};}
 function publish(){
   const current=snapshot();
   for(const [selector,status] of [['request','request'],['finished','finished'],['working','working']]) {
     const badge=document.querySelector(`.preview-bar .${selector}`),count=current.sessions.filter(s=>s.status===status).length;
     badge.textContent=String(count);badge.hidden=count===0;
   }
   frame.contentWindow.postMessage({channel:'aam-host',snapshot:current},'*');
 }
 window.addEventListener('message',event=>{
  if(event.source!==frame.contentWindow||event.data?.channel!=='aam-monitor')return;
  const m=event.data.message,s=state.find(s=>s.id===m.session);
  if(m.type==='ready'){publish();frame.contentWindow.postMessage({channel:'aam-host',visibility:true},'*');}
  if(m.type==='preview-pin')pin.click();
  if(m.type==='reset-credits'){
    const request=++resetCreditRequest;clearTimeout(resetCreditTimer);
    usageResets={generation:request,phase:'loading',availableCount:null,credits:null};publish();
    resetCreditTimer=setTimeout(()=>{if(request!==resetCreditRequest)return;usageResets={generation:request,phase:'ready',availableCount:resetCreditRows.length,credits:resetCreditRows};publish();},2600);
  }

  if(m.type==='submit'&&s&&m.context===pending(s)?.request.context){pending(s).status='answered';s.status=m.action==='deny'?'idle':'working';addMessage(s,m.action==='deny'?'<p>Run canceled. Waiting for the next command.</p>':'<p>Response sent. Continuing work.</p>');publish();}
  if(m.type==='history'&&s){s.history.messages=[{html:'<p>Separated response delivery from state updates. Each request now retains its own identity and offered choices.</p>'}];s.history.exhausted=true;publish();}
  if(m.type==='message'&&s&&!pending(s)&&['idle','finished'].includes(s.status)) {s.status='working';addMessage(s,'<p>Message received. Continuing with the next phase.</p>');publish();}
  if(m.type==='seen'&&s?.status==='finished'){s.status='idle';publish();}
  if(m.type==='copy')navigator.clipboard?.writeText(m.text).catch(()=>{});
  if(m.type==='preferences'){preferences=m.preferences;publish();}
  if(m.type==='open'&&s)publish();
  if(m.type==='link'&&/^(https?:|mailto:)/i.test(m.url))window.open(m.url,'_blank','noopener');
  if(m.type==='quit'||m.type==='close'){previewWindow.hidden=true;frame.contentWindow.postMessage({channel:'aam-host',visibility:false},'*');}
 });
 // Exercise the real renderer with changing content and stable activity/request IDs.
 const quota=document.getElementById('preview-quota');
 const quotaValue=document.getElementById('preview-quota-value');
 quota.addEventListener('input',()=>{
   remaining=Number(quota.value);
   quotaValue.value=`${remaining}%`;
   quota.setAttribute('aria-valuetext',`${remaining}% remaining`);
   publish();
 });
 const updates=document.getElementById('preview-live-updates');
 function simulateUpdate(){
   updateTick++;
   const session=state.find(s=>s.status==='working'&&!pending(s)&&preferences.applications[s.appId]!==false);
   if(session){
     const action=session.activities.findLast(a=>a.type==='action'&&a.current);
     if(action){action.text=`Running checks · ${updateTick} completed`;action.updatedAt=new Date().toISOString();}
     const message=session.activities.findLast(a=>a.type==='message'&&a.role==='agent');
     if(message){message.html=`<p>Live preview: checking updates, disclosure animation and editor stability. Update ${updateTick}.</p>`;message.updatedAt=new Date().toISOString();}
   }

   if(updateTick%12===0){
     const idle=state.find(s=>s.id==='snapshot');
     if(idle&&!pending(idle)&&['idle','finished'].includes(idle.status)){
       idle.status=idle.status==='idle'?'finished':'idle';
       idle.completion=idle.status==='finished'?updateTick:null;
     }
   }
   publish();
 }
 function syncUpdates(){
   clearInterval(updateTimer);updateTimer=null;
   if(updates.checked&&!document.hidden)updateTimer=setInterval(simulateUpdate,250);
 }
 updates.addEventListener('change',syncUpdates);
 document.addEventListener('visibilitychange',syncUpdates);
 window.addEventListener('pagehide',()=>{clearInterval(updateTimer);updateTimer=null;clearTimeout(resetCreditTimer);});
 window.addEventListener('pageshow',syncUpdates);
 document.querySelectorAll('.preview-theme button[data-theme]').forEach(button=>button.addEventListener('click',()=>{theme=button.dataset.theme;document.body.dataset.theme=theme;document.querySelectorAll('.preview-theme button[data-theme]').forEach(b=>b.setAttribute('aria-pressed',String(b===button)));publish();}));
 document.getElementById('preview-update').addEventListener('change',event=>{versions.available=event.currentTarget.checked?newerRelease:null;versions.checked_at=Math.floor(Date.now()/1000);publish();});
 document.getElementById('show-monitor').addEventListener('click',showWindow);
})();
