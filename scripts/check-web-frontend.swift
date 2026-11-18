// Hidden WebKit smoke check against an isolated Rust test server; no user data.
import AppKit
import WebKit
let app=NSApplication.shared;app.setActivationPolicy(.prohibited)
let config=WKWebViewConfiguration();config.websiteDataStore = .nonPersistent()
config.userContentController.addUserScript(WKUserScript(source:"window.__socketMessages=0;window.__socketPatches=0;window.__socketExtensions='';const Base=window.WebSocket;window.WebSocket=class extends Base{constructor(...args){super(...args);this.addEventListener('open',()=>window.__socketExtensions=this.extensions);this.addEventListener('message',event=>{window.__socketMessages++;if(JSON.parse(event.data).type==='patch')window.__socketPatches++;});}};",injectionTime:.atDocumentStart,forMainFrameOnly:true))
let web=WKWebView(frame:NSRect(x:0,y:0,width:390,height:844),configuration:config)
let window=NSWindow(contentRect:web.frame,styleMask:.borderless,backing:.buffered,defer:false);window.contentView=web
final class Check:NSObject,WKNavigationDelegate{
 var started=false
 func webView(_ web:WKWebView,didFinish navigation:WKNavigation!){
  if started{return};started=true
  web.callAsyncJavaScript("""
   const pause=ms=>new Promise(r=>setTimeout(r,ms));
   const until=async(test,label)=>{for(let i=0;i<100;i++){if(test())return;await pause(100);}throw Error(label);};
   await until(()=>document.querySelector('.aam-session')&&window.__socketMessages>0,'No real WebSocket snapshot');
   await until(()=>document.querySelector('.aam-mesh-raster'),'Filtered SVG backdrop was not rasterized');
   const mesh=document.querySelector('.aam-mesh-raster'),meshImage=mesh.style.backgroundImage;
   if(!meshImage.includes('data:image/png')||getComputedStyle(mesh).filter!=='none')throw Error('Backdrop still requires a live blur surface');
   if(!window.__socketExtensions.includes('permessage-deflate'))throw Error('WebSocket compression was not negotiated');
   if(location.hash)throw Error('Pairing token retained in URL');
   if(document.cookie.includes('aam_session'))throw Error('Cookie accessible to JavaScript');
   if(document.querySelector('.aam-open-title').textContent!=='Phone test')throw Error('Wrong session');
   if(document.documentElement.scrollWidth>390)throw Error('Horizontal overflow on phone');
   document.querySelector('.aam-open').click();
   await until(()=>document.querySelector('.aam-open-title')?.textContent==='Updated over diff'&&window.__socketPatches>0,'Live patch was not applied');
   const resetToggle=document.querySelector('[data-command="reset-credits"]');
   if(!resetToggle||resetToggle.getAttribute('aria-expanded')!=='false'||!document.querySelector('.aam-reset-list').hidden)throw Error('Reset inventory must start collapsed');
   for(let attempt=0;attempt<2;attempt++){
   resetToggle.click();
   if(!document.querySelector('.aam-resets-spinner')||document.querySelector('.aam-reset-row'))throw Error('Old reset rows flashed on disclosure');
   if(attempt===1){await pause(300);if(!document.querySelector('.aam-resets-spinner')||document.querySelector('.aam-reset-row'))throw Error('Unrelated revision revealed stale reset rows');}
   await until(()=>document.querySelector('.aam-resets-spinner')&&!document.querySelector('.aam-reset-list').hidden,'Missing reset loading spinner');
   await until(()=>document.querySelector('.aam-reset-count')?.textContent==='3 available','Reset result did not arrive');
   if(document.querySelectorAll('.aam-reset-row').length!==1||document.querySelector('.aam-resets-spinner'))throw Error('Reset count/details or loading resolution incorrect');
   if(!document.querySelector('.aam-reset-expiry').textContent.trim())throw Error('Missing reset expiration');
   resetToggle.click();if(!document.querySelector('.aam-reset-list').hidden)throw Error('Reset disclosure did not close');
   }
   let command=false;const original=window.fetch;window.fetch=async(...args)=>{if(args[0]==='/api/command'&&JSON.parse(args[1]?.body||'{}').type==='history')throw Error('History used HTTP');const response=await original(...args);if(args[0]==='/api/command'&&response.ok)command=true;return response;};
   document.querySelector('.aam-previous').click();
   await until(()=>document.querySelector('.aam-messages')?.textContent.includes('Earlier over WebSocket'),'WebSocket history was not displayed');
   const scope=document.querySelector('.aam-scope-toggle'),code=document.getElementById(scope.getAttribute('aria-controls'));
   if(!code.hidden||scope.tabIndex!==-1)throw Error('Scope must start hidden and outside keyboard traversal');
   const action=document.querySelector('[data-response="similar"]'),a=action.getBoundingClientRect(),b=scope.getBoundingClientRect();
   if(b.left<a.right||Math.abs((a.top+a.bottom-b.top-b.bottom)/2)>1)throw Error('Scope disclosure must be beside and centered on its action');
   scope.click();await pause(50);
   if(code.hidden||code.textContent!=='cargo test'||scope.getAttribute('aria-expanded')!=='true')throw Error('Scope was not revealed');
   if(command)throw Error('Disclosure submitted an approval');
   scope.click();if(!code.hidden)throw Error('Scope was not collapsed');
   document.querySelector('[data-response="allow"]').click();
   await until(()=>command,'Approval was not submitted');
   await until(()=>document.querySelector('.aam-compose-input')&&!document.querySelector('.aam-question'),'Composer not enabled after approval');
   const message='Please continue.\\nInclude all checks.';
   let compose=document.querySelector('.aam-compose-input');compose.value=message;compose.dispatchEvent(new Event('input',{bubbles:true}));
   compose.closest('form').querySelector('button').click();
   await until(()=>document.getElementById('aam-remote-notice').textContent.includes('Test delivery failure')&&!document.querySelector('.aam-compose-input').disabled,'Failed message did not restore the composer');
   compose=document.querySelector('.aam-compose-input');if(compose.value!==message)throw Error('Failed message lost its multiline draft');
   compose.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true}));
   await until(()=>document.querySelector('.aam-compose-input')?.value===''&&!document.querySelector('.aam-compose-input').disabled,'Successful retry did not clear the draft');
   const demo={sessions:[{id:'composer-test',title:'Draft test',appId:'test',app:'Test',status:'idle',expanded:true,activities:[],directory:'/demo',history:{messages:[]}}],usage:[]};
   window.aam.setSnapshot(demo);await pause(50);
   let input=document.querySelector('.aam-compose-input');const compact=input.getBoundingClientRect().height;
   const inset=document.getElementById('aam-ui-preview').classList.contains('aam-inset-composer');
   if(inset&&input.closest('form').querySelector('button').getClientRects().length)throw Error('Empty composer shows Send');
   const draft='Please continue with all checks and provide a detailed summary. '.repeat(15);
   input.focus();input.value=draft;input.dispatchEvent(new Event('input',{bubbles:true}));await pause(50);
   if(input.getBoundingClientRect().height<=compact+36)throw Error('Composer did not grow with wrapping');
   if(input.scrollHeight>input.clientHeight+1||document.documentElement.scrollWidth>390)throw Error('Composer clipped text or overflowed horizontally');
   input.setSelectionRange(5,10);demo.sessions[0].title='Updated draft test';window.aam.setSnapshot(demo);await pause(50);
   if(document.querySelector('.aam-compose-input')!==input)throw Error('Snapshot replaced the active editor');
   input=document.querySelector('.aam-compose-input');
   if(input.value!==draft||input.selectionStart!==5||input.selectionEnd!==10||document.activeElement!==input)throw Error('Composer resize/redraw lost draft, selection or focus');
   input.value='Short';input.dispatchEvent(new Event('input',{bubbles:true}));await pause(50);
   if(input.getBoundingClientRect().height!==(inset?44:compact))throw Error('Composer did not shrink');
   if(inset){
    const form=input.closest('form'),button=form.querySelector('button');
    const field=input.getBoundingClientRect(),send=button.getBoundingClientRect();
    if(!form.classList.contains('is-inline')||Math.abs((field.top+field.bottom-send.top-send.bottom)/2)>1)throw Error('Short composer Send is not centered');
    input.value='X'.repeat(50);input.dispatchEvent(new Event('input',{bubbles:true}));await pause(50);
    if(form.classList.contains('is-inline')||parseFloat(getComputedStyle(input).paddingBottom)<44)throw Error('Long composer has no lower Send row');
    input.value='A\\nB';input.dispatchEvent(new Event('input',{bubbles:true}));await pause(50);
    if(form.classList.contains('is-inline'))throw Error('Explicit newline uses inline Send');
    input.value='';input.dispatchEvent(new Event('input',{bubbles:true}));await pause(50);
    if(input.getBoundingClientRect().height!==32||button.getClientRects().length)throw Error('Empty composer did not return to compact mode');
   }
   const article=document.querySelector('.aam-session'),clip=article.querySelector('.aam-clip'),latest=article.querySelector('.aam-latest');
   const observer=new MutationObserver(()=>{});observer.observe(document.querySelector('.aam-product'),{childList:true,subtree:true});
   for(let i=0;i<5;i++)window.aam.setSnapshot(demo);
   if(observer.takeRecords().some(record=>record.addedNodes.length||record.removedNodes.length))throw Error('Identical snapshots rebuilt DOM content');
   const toggle=article.querySelector('.aam-toggle');toggle.click();await pause(40);
   const transition=article.querySelector('.aam-detail').getAnimations()[0];
   demo.sessions[0].status='finished';demo.sessions[0].activities=[{id:'message-one',type:'message',role:'agent',html:'<p>Fresh output</p>',startedAt:'2026-01-01T00:00:00Z'}];
   window.aam.setSnapshot(demo);
   if(document.querySelector('.aam-session')!==article||article.querySelector('.aam-clip')!==clip||article.querySelector('.aam-latest')!==latest)throw Error('Update replaced a session or its disclosure document');
   if(transition&&article.querySelector('.aam-detail').getAnimations()[0]!==transition)throw Error('Snapshot restarted the disclosure animation');
   if(!latest.textContent.includes('Fresh output'))throw Error('In-place update lost agent output');
   if(document.querySelector('.aam-mesh-raster')!==mesh||mesh.style.backgroundImage!==meshImage)throw Error('Disclosure/update regenerated background artwork');
   await pause(220);toggle.click();await pause(40);
   demo.sessions[0].status='idle';window.aam.setSnapshot(demo);
   if(article.querySelector('.aam-compose-input')!==input)throw Error('Disclosure/update replaced composer');
   observer.disconnect();
   const diagnosticMode=document.getElementById('aam-diagnostics-mode');
   if(diagnosticMode){
    const choose=value=>{diagnosticMode.value=value;diagnosticMode.dispatchEvent(new Event('change'));};
    choose('freeze');await pause(50);
    const frozenArticle=document.querySelector('.aam-session');
    choose('repeat');await pause(800);
    if(document.querySelector('.aam-session')!==frozenArticle)throw Error('Diagnostic repeat rebuilt the session');
    if(!document.getElementById('aam-diagnostics-stats').textContent.includes('sync errors 0'))throw Error('Diagnostics reported a stream error');
    if(document.querySelector('.aam-menu').getBoundingClientRect().top<document.getElementById('aam-diagnostics').getBoundingClientRect().bottom-1)throw Error('Diagnostic panel covers the monitor');
    if(document.documentElement.scrollWidth>390)throw Error('Diagnostic panel overflows the phone viewport');
    choose('live');
   }
   const makeSession=(id,appId,title)=>({id,appId,app:appId,title,status:'idle',authoritative:true,expanded:true,activities:[],directory:'/demo',history:{messages:[]}});
   const first=makeSession('order-a','vscode','Task 2'),second=makeSession('order-b','vscode','Task 10'),third=makeSession('order-c','terminal','A');
   const ordered={sessions:[first,second,third],usage:[]};window.aam.setSnapshot(ordered);await pause(50);
   const secondNode=document.querySelector('[data-session="order-b"]'),editor=secondNode.querySelector('.aam-compose-input');
   editor.focus();editor.value='Keep my draft';editor.dispatchEvent(new Event('input',{bubbles:true}));editor.setSelectionRange(2,5);
   second.title='Task 1';ordered.sessions=[second,first,third];window.aam.setSnapshot(ordered);
   if(document.querySelector('.aam-session')!==secondNode)throw Error('Renamed session did not move to incoming sorted position');
   if(secondNode.querySelector('.aam-compose-input')!==editor||editor.value!=='Keep my draft'||document.activeElement!==editor||editor.selectionStart!==2)throw Error('Reordering lost the editor or its draft/focus/selection');
   ordered.sessions=[third,second,first];window.aam.setSnapshot(ordered);
   if(document.querySelector('.aam-session').dataset.session!=='order-c'||document.querySelector('[data-session="order-b"]')!==secondNode)throw Error('Application group reorder failed or rebuilt sessions');
   return 'Pairing, compressed WebSocket, diffs, history, approval, real multiline message send/retry growing drafts and persistent DOM during disclosure updates passed';
  """,arguments:[:],in:nil,in:.page){result in
   switch result{case .failure(let error):print("FAIL \(error)");exit(1);case .success(let value):
    print("PASS \(value)");web.takeSnapshot(with:nil){image,error in
     if let tiff=image?.tiffRepresentation,let bitmap=NSBitmapImageRep(data:tiff),let png=bitmap.representation(using:.png,properties:[:]){try? png.write(to:URL(fileURLWithPath:"/tmp/aam-web-phone.png"))}
     exit(0)
    }
   }
  }
 }
}
let check=Check();web.navigationDelegate=check
let link=ProcessInfo.processInfo.environment["AAM_WEB_TEST_URL"]!
var components=URLComponents(string:link)!
if ProcessInfo.processInfo.environment["AAM_WEB_TEST_DIAGNOSTICS"] == "1" {
 var items=components.queryItems ?? [];items.append(URLQueryItem(name:"diagnostics",value:"1"));components.queryItems=items
}
web.load(URLRequest(url:components.url!))
DispatchQueue.main.asyncAfter(deadline:.now()+25){print("Timed out");exit(1)}
app.run()
