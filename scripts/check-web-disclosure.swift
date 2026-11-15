// Exercise shared web disclosure animations in hidden WebKit with synthetic sessions.
import AppKit
import WebKit
let app=NSApplication.shared;app.setActivationPolicy(.prohibited)
let config=WKWebViewConfiguration();config.websiteDataStore = .nonPersistent()
let width=Double(ProcessInfo.processInfo.environment["AAM_PREVIEW_WIDTH"] ?? "460")!
let web=WKWebView(frame:NSRect(x:0,y:0,width:width,height:844),configuration:config)
let window=NSWindow(contentRect:web.frame,styleMask:.borderless,backing:.buffered,defer:false);window.contentView=web
final class Check:NSObject,WKNavigationDelegate {
 var started=false
 func webView(_ web:WKWebView,didFinish navigation:WKNavigation!) {
  if started{return};started=true
  web.callAsyncJavaScript("""
   const pause=ms=>new Promise(r=>setTimeout(r,ms)),assert=(ok,label)=>{if(!ok)throw Error(label);};
   window.aam.destroy();window.aam=window.AAMMonitor.mount(document.getElementById('aam-ui-preview'),()=>{},{settings:false,live:true});
   const menu=document.querySelector('.aam-menu');menu.style.height='100dvh';
   const session=(id,expanded)=>({id,title:id,appId:'test',app:'Test',status:'idle',expanded,authoritative:true,activities:[{id:'text',type:'message',role:'agent',startedAt:'2026-01-01T00:00:00Z',text:'A detailed status message for the disclosure animation. '.repeat(18)}],directory:'/demo',history:{messages:[]}});
   const data={theme:'dark',sessions:[session('first',false),session('second',true)],usage:[]};window.aam.setSnapshot(data);await pause(120);
   const first=document.querySelector('[data-session="first"]'),second=document.querySelector('[data-session="second"]');
   const input=second.querySelector('.aam-compose-input'),list=document.querySelector('.aam-list'),detail=first.querySelector('.aam-detail'),clip=first.querySelector('.aam-clip');
   const draft='Keep this multiline draft and its height stable while another session is opening. '.repeat(12);
   input.value=draft;input.dispatchEvent(new Event('input',{bubbles:true}));await pause(120);
   const inputHeight=input.getBoundingClientRect().height;assert(inputHeight>80,'Draft must be multiline');list.scrollTop=0;
   let zeroResets=0;
   const changes=new MutationObserver(records=>{for(const record of records)if(record.oldValue?.includes('height: 0px'))zeroResets++;});
   changes.observe(input,{attributes:true,attributeFilter:['style'],attributeOldValue:true});
   const toggle=()=>first.querySelector('.aam-toggle').click();
   function seek(ms){
    detail.getBoundingClientRect();
    for(const element of [detail,clip])for(const animation of element.getAnimations()){animation.pause();animation.currentTime=ms;}
   }
   async function sample(open){
    toggle();seek(0);const values=[];
    for(let i=0;i<18;i++){seek(i*16);input.dispatchEvent(new Event('input',{bubbles:true}));values.push(detail.getBoundingClientRect().height);assert(input.getBoundingClientRect().height===inputHeight,'Unrelated draft height jumped');assert(list.scrollTop===0,'Disclosure moved the viewport');}
    await pause(40);
    for(let i=1;i<values.length;i++)assert(open?values[i]>=values[i-1]-.5:values[i]<=values[i-1]+.5,'Disclosure reversed direction mid-animation');
    assert(open?values.at(-1)>100:values.at(-1)<1,'Disclosure did not reach its endpoint: '+values.join(','));
    for(const element of [detail,clip])for(const animation of element.getAnimations())animation.finish();
   }
   await sample(true);toggle();seek(20);assert(getComputedStyle(clip).visibility==='visible','Collapse hid content before the animation ended');seek(200);for(const element of [detail,clip])for(const animation of element.getAnimations())animation.finish();
   await sample(true);await sample(false);
   data.sessions[0].status='finished';window.aam.setSnapshot(data);await pause(60);
   assert(first.querySelector('.aam-detail')===detail,'Completion badge change replaced the detail');
   toggle();seek(40);data.sessions[0].status='idle';window.aam.setSnapshot(data);await pause(60);
   assert(document.querySelector('[data-session="first"]')===first&&first.querySelector('.aam-detail')===detail,'Completion acknowledgment interrupted disclosure');
   toggle();seek(40);toggle();seek(200);for(const element of [detail,clip])for(const animation of element.getAnimations())animation.finish();await pause(60);
   changes.disconnect();assert(zeroResets===0,'Layout repeatedly zeroed the live composer height');assert(input.value===draft,'Disclosure lost the draft');
   input.value='Short';input.dispatchEvent(new Event('input',{bubbles:true}));await pause(40);assert(input.getBoundingClientRect().height<inputHeight,'Composer must still shrink after editing');
   return 'Disclosure endpoints/direction, stable scroll/draft geometry, collapse visibility and interrupted animations passed';
  """,arguments:[:],in:nil,in:.page){result in
   switch result{case .failure(let error):print("FAIL \(error)");exit(1);case .success(let value):print("PASS \(value)");exit(0)}
  }
 }
}
let check=Check();web.navigationDelegate=check
let root=URL(fileURLWithPath:FileManager.default.currentDirectoryPath,isDirectory:true)
web.loadFileURL(root.appendingPathComponent("src/web/index.html"),allowingReadAccessTo:root)
DispatchQueue.main.asyncAfter(deadline:.now()+30){print("Timed out");exit(1)}
app.run()
