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
   if(!window.__socketExtensions.includes('permessage-deflate'))throw Error('WebSocket compression was not negotiated');
   if(location.hash)throw Error('Pairing token retained in URL');
   if(document.cookie.includes('aam_session'))throw Error('Cookie accessible to JavaScript');
   if(document.querySelector('.aam-open-title').textContent!=='Phone test')throw Error('Wrong session');
   if(document.documentElement.scrollWidth>390)throw Error('Horizontal overflow on phone');
   document.querySelector('.aam-open').click();
   await until(()=>document.querySelector('.aam-open-title')?.textContent==='Updated over diff'&&window.__socketPatches>0,'Live patch was not applied');
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
   const draft='Please continue with all checks and provide a detailed summary. '.repeat(15);
   input.focus();input.value=draft;input.dispatchEvent(new Event('input',{bubbles:true}));await pause(50);
   if(input.getBoundingClientRect().height<=compact+36)throw Error('Composer did not grow with wrapping');
   if(input.scrollHeight>input.clientHeight+1||document.documentElement.scrollWidth>390)throw Error('Composer clipped text or overflowed horizontally');
   input.setSelectionRange(5,10);demo.sessions[0].title='Updated draft test';window.aam.setSnapshot(demo);await pause(50);
   input=document.querySelector('.aam-compose-input');
   if(input.value!==draft||input.selectionStart!==5||input.selectionEnd!==10||document.activeElement!==input)throw Error('Composer resize/redraw lost draft, selection or focus');
   input.value='Short';input.dispatchEvent(new Event('input',{bubbles:true}));await pause(50);
   if(input.getBoundingClientRect().height!==compact)throw Error('Composer did not shrink');
   return 'Pairing, compressed WebSocket, diffs, history, approval, real multiline message send/retry and growing drafts passed';
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
web.load(URLRequest(url:URL(string:link)!))
DispatchQueue.main.asyncAfter(deadline:.now()+25){print("Timed out");exit(1)}
app.run()
