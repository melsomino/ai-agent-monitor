// Exercise the actual shared preview settings in hidden WebKit, without user data or network access.
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
   const pause=ms=>new Promise(r=>setTimeout(r,ms)),assert=(value,label)=>{if(!value)throw Error(label);};
   const action=name=>{const button=document.querySelector(`[data-setting-action="${name}"]`);assert(button,'Missing '+name);button.click();};
   const tab=name=>document.querySelector(`[data-pane="${name}"]`).click();
   const events=[];window.fetch=()=>{throw Error('Preview attempted network access');};
   window.aam.destroy();window.aam=window.AAMMonitor.mount(document.getElementById('aam-ui-preview'),event=>events.push(event));
   const snapshot={version:'preview',theme:'dark',sessions:[],applications:[{id:'vscode',name:'VS Code',icon:''},{id:'terminal',name:'Terminal',icon:''}],usage:[]};
   window.aam.setSnapshot(snapshot);
   document.querySelector('[data-command="github"]').click();assert(events.at(-1)?.type==='link'&&events.at(-1).url==='https://github.com/melsomino/ai-agent-monitor','GitHub action opens the fixed project URL');
   window.aam.setSnapshot({...snapshot,versions:{current:'v0.3.0',available:{version:'v0.4.0',url:'https://github.com/melsomino/ai-agent-monitor/releases/tag/v0.4.0'}}});
   const release=document.querySelector('.aam-update-link'),version=document.querySelector('.aam-version');
   assert(!release.hidden&&version.hidden&&release.textContent==='v0.4.0 available','Only new release shown');
   release.click();assert(events.at(-1).url.endsWith('/releases/tag/v0.4.0'),'Release opens through host');
   window.aam.setSnapshot({...snapshot,versions:{current:'v0.3.0',available:{version:'v0.4.0',url:'https://untrusted.example/'}}});assert(release.hidden&&!release.hasAttribute('href')&&!version.hidden,'Untrusted release destinations have no link');
   window.aam.setSnapshot({...snapshot,versions:{current:'v0.3.0',available:null,checked_at:100}});assert(release.hidden&&version.textContent==='v0.3.0 ✓','Current version is a single checked line');window.aam.setSnapshot(snapshot);assert(version.title==='Not checked yet'&&!version.textContent.includes('✓'),'Unknown state never claims up to date');
   document.querySelector('[data-command="settings"]').click();await pause(30);
   assert(document.querySelectorAll('[role="tab"]').length===5,'Five settings tabs');
   assert(document.activeElement.classList.contains('aam-settings-screen'),'No automatic Back keyboard focus');
   const login=document.querySelector('[data-preference="login"]');login.click();
   assert(events.some(e=>e.type==='preferences'&&e.preferences.login===true),'Login preference');
   tab('applications');document.querySelector('[data-application="vscode"]').click();
   assert(events.some(e=>e.type==='preferences'&&e.preferences.applications.vscode===false),'Application visibility preference');
   tab('agents');action('check-agents');assert(document.querySelector('[data-setting-action="check-agents"]').disabled,'Agent setup busy');await pause(850);
   assert(document.querySelector('.aam-agent-actions').textContent.includes('Setup complete'),'Agent setup completion');
   tab('telegram');const original=document.querySelector('.aam-bot-connected strong').textContent;
   action('change-bot');assert(document.querySelector('.aam-bot-form').textContent.includes('current bot stays connected'),'Replacement warning');
   assert(!document.querySelector('[data-setting-action="activate-bot"]'),'No disabled Open bot before connect');
   action('cancel-bot');assert(document.querySelector('.aam-bot-connected strong').textContent===original,'Cancel restores connected bot');
   action('change-bot');let token=document.querySelector('.aam-bot-token input');token.value='invalid';token.dispatchEvent(new Event('input',{bubbles:true}));document.querySelector('.aam-bot-form').requestSubmit();
   assert(document.querySelector('.aam-bot-status').textContent.startsWith('Invalid token'),'Invalid token state');
   token=document.querySelector('.aam-bot-token input');token.value='123456789:ABCDEFGHIJKLMNOPQRSTUVWXYZ_abcd';token.dispatchEvent(new Event('input',{bubbles:true}));document.querySelector('.aam-bot-form').requestSubmit();
   assert(document.querySelector('.aam-bot-token input').disabled,'Checking token disables editing');await pause(850);
   assert(document.querySelector('[data-setting-action="activate-bot"]'),'Waiting for activation exposes Open bot');
   action('cancel-bot');assert(document.querySelector('.aam-bot-connected strong').textContent===original,'Cancel during activation preserves old bot');
   action('unlink-bot');assert(!document.querySelector('.aam-bot-form').textContent.includes('current bot stays connected'),'First setup has no replacement warning');
   token=document.querySelector('.aam-bot-token input');token.value='123456789:ABCDEFGHIJKLMNOPQRSTUVWXYZ_abcd';token.dispatchEvent(new Event('input',{bubbles:true}));document.querySelector('.aam-bot-form').requestSubmit();await pause(850);action('activate-bot');
   assert(document.querySelector('.aam-bot-connected strong'),'Activation completes setup');
   for(const theme of ['light','dark']) {
    snapshot.theme=theme;window.aam.setSnapshot(snapshot);tab('web');
    const running=()=>document.querySelector('[data-web="running"]');
    assert(!document.querySelector('[data-web="remote"]')&&!document.querySelector('.aam-web-qr'),'One access switch, initially off');assert(!document.querySelector('.aam-web-pairing')&&!document.querySelector('[data-setting-action="renew-web"]')&&!document.querySelector('.aam-web-addresses'),'Off hides the entire connection section');running().click();
    assert(document.querySelectorAll('[data-web-address]').length===2,'Inline local addresses');
    assert(document.querySelector('.aam-web-qr')&&document.querySelector('[data-web-remaining]').textContent==='120 sec','QR and countdown');
    document.querySelector('[data-web-address="http://127.0.0.1:8765/"]').click();action('copy-web');
    assert(events.at(-1).type==='copy'&&events.at(-1).text.startsWith('http://127.0.0.1:8765/#pair=preview-only-'),'Copy uses selected address');assert(document.querySelector('.aam-web-link-url').textContent==='http://127.0.0.1:8765/','Visible link omits pairing nonce');const copied=events.at(-1).text;action('open-web');assert(events.at(-1).type==='link'&&events.at(-1).url===copied,'Clickable URL uses same full pairing link');assert(document.querySelector('.aam-web-link-panel').compareDocumentPosition(document.querySelector('.aam-web-pairing'))&Node.DOCUMENT_POSITION_FOLLOWING,'Link and countdown precede QR');
    assert([...document.querySelectorAll('.aam-web-route')].some(row=>row.textContent.includes('Cloudflare')&&row.textContent.includes('connecting…')),'Tunnel starts automatically');assert(document.querySelectorAll('.aam-web-route').length===5,'All connection methods have one row');assert(document.querySelector('[data-setting-action="toggle-ngrok"]').previousElementSibling.textContent==='ngrok','Disclosure immediately follows provider name');await pause(850);
    assert(document.querySelectorAll('[data-web-address]').length>=3,'Remote address added');const addresses=document.querySelector('.aam-web-addresses');assert(addresses.scrollHeight<=addresses.clientHeight+1,'Address list must not scroll internally');
    assert(document.querySelector('[data-web-address]:checked').dataset.webAddress.startsWith('https://'),'New HTTPS origin selected');
    const section=()=>document.querySelector('[data-setting-action="toggle-ngrok"]');
    if(section().getAttribute('aria-expanded')!=='true')action('toggle-ngrok');
    if(document.querySelector('.aam-ngrok-token input')){
      const dashboard=document.querySelector('[data-setting-action="ngrok-dashboard"]');
      assert(dashboard.querySelector('.aam-token-key')&&dashboard.previousElementSibling.matches('input'),'Key link immediately follows token input');
      action('ngrok-dashboard');assert(events.at(-1).type==='link'&&events.at(-1).url.includes('dashboard.ngrok.com'),'Dashboard action opens browser');
      const hint=document.querySelector('.aam-ngrok-storage');assert(hint.textContent==='Will be stored securely in macOS Keychain','Future storage caption');assert(Math.abs(hint.getBoundingClientRect().left-document.querySelector('.aam-ngrok-token input').getBoundingClientRect().left)<1,'Storage caption aligns with token input');
      const input=document.querySelector('.aam-ngrok-token input');input.value='invalid';input.dispatchEvent(new Event('input',{bubbles:true}));action('connect-ngrok');await pause(850);
      assert(document.querySelector('.aam-ngrok-status').textContent.includes('connection failed'),'Failed ngrok disclosure status');
      assert(document.querySelector('.aam-ngrok-saved').textContent.includes('stored securely in macOS Keychain')&&!document.querySelector('.aam-ngrok-token input'),'Saved token is never exposed');
      assert(document.querySelector('[data-setting-action="connect-ngrok"]').textContent==='Reconnect','Failed connection exposes Reconnect');
      action('connect-ngrok');await pause(850);
    }
    assert(document.querySelector('.aam-ngrok-status').textContent==='ready','ngrok connected disclosure');
    assert(!document.querySelector('[data-setting-action="connect-ngrok"]'),'Connected saved token has no Connect button');
    action('change-ngrok-token');const replacement=document.querySelector('.aam-ngrok-token input');assert(replacement&&document.querySelector('[data-setting-action="connect-ngrok"]').disabled,'Change opens empty replacement field');assert(document.querySelector('label[for="aam-ngrok-token"]').getBoundingClientRect().bottom<=replacement.getBoundingClientRect().top,'Token label sits above field');action('cancel-ngrok-token');assert(document.querySelector('.aam-ngrok-saved'),'Cancel preserves saved connection');
    assert(document.querySelectorAll('[data-web-address]').length>=4,'Both remote addresses and local addresses coexist');
    document.querySelector('[data-web-address="https://ngrok-preview.example/"]').click();
    assert(document.querySelector('.aam-web-instructions').textContent.includes('through ngrok'),'ngrok privacy instruction');
    const cf=document.querySelector('[data-web-address="https://monitor-preview.example/"]');assert(cf,'Cloudflare remains available');cf.click();assert(document.querySelector('.aam-web-instructions').textContent.includes('through Cloudflare'),'Selected route controls privacy instruction');
    document.querySelector('[data-web-address="https://ngrok-preview.example/"]').click();
    action('toggle-ngrok');assert(document.querySelector('#aam-ngrok-details').hidden,'Details collapse');action('toggle-ngrok');
    const tunaSection=()=>document.querySelector('[data-setting-action="toggle-tuna"]');
    if(tunaSection().getAttribute('aria-expanded')!=='true')action('toggle-tuna');
    const tunaInput=document.querySelector('.aam-tuna-token input');
    if(tunaInput){
      assert(document.querySelector('[data-setting-action="connect-tuna"]').disabled,'Tuna Connect waits for API key');
      action('tuna-dashboard');assert(events.at(-1).url.startsWith('https://my.tuna.am/api_keys'),'Tuna account link');
      tunaInput.value='synthetic-tuna-api-key';tunaInput.dispatchEvent(new Event('input',{bubbles:true}));action('connect-tuna');await pause(850);
    }
    assert(document.querySelector('.aam-tuna-status').textContent==='ready','Tuna connects independently');
    assert(document.querySelectorAll('[data-web-address]').length===5,'Three tunnel addresses plus LAN and loopback');
    document.querySelector('[data-web-address="https://tuna-preview.example/"]').click();action('copy-web');assert(events.at(-1).text.startsWith('https://tuna-preview.example/#pair='),'Tuna pairing link');
    assert(document.querySelector('.aam-web-instructions').textContent.includes('through Tuna'),'Tuna privacy instruction');
    action('change-tuna-token');assert(document.querySelector('.aam-tuna-token input'),'Tuna replacement input');action('cancel-tuna-token');assert(document.querySelector('.aam-tuna-saved'),'Cancel preserves Tuna credential');
    const before=document.querySelector('.aam-web-qr path').getAttribute('d');action('renew-web');
    assert(document.querySelector('.aam-web-qr path').getAttribute('d')!==before,'QR renewal changes matrix');
    const now=performance.now.bind(performance);performance.now=()=>now()+121000;const old=document.querySelector('.aam-web-qr path').getAttribute('d');await pause(1500);
    assert(document.querySelector('.aam-web-qr path').getAttribute('d')!==old,'Expired QR renews automatically');
    assert(document.querySelector('[data-web-remaining]').textContent.match(/^11[89] sec$|^120 sec$/),'Countdown restarts');
    assert(document.documentElement.scrollWidth<=innerWidth,'Settings overflow horizontally');
    assert(document.querySelectorAll('[data-web-address]').length===5,'Local addresses remain available with all tunnels');
    running().click();assert(!document.querySelector('.aam-web-qr')&&!document.querySelector('[data-web-address]'),'Stop removes all access paths');
    running().click();running().click();await pause(850);assert(!document.querySelector('[data-web-address]'),'Stop during tunnel startup stays stopped');
   }
   tab('general');document.querySelector('[data-pane="general"]').dispatchEvent(new KeyboardEvent('keydown',{key:'End',bubbles:true}));
   assert(document.querySelector('[data-pane="web"]').getAttribute('aria-selected')==='true','Keyboard navigation reaches fifth tab');
   document.querySelector('[data-web="running"]').click();await pause(850);
   return 'Five panes, preferences, Telegram replacement/cancel/activation, Web toggles, address selection, QR renewal, keyboard and themes passed';
  """,arguments:[:],in:nil,in:.page) {result in
   switch result {
    case .failure(let error):print("FAIL \(error)");exit(1)
    case .success(let value):print("PASS \(value)");web.takeSnapshot(with:nil) {image,error in
     if let tiff=image?.tiffRepresentation,let bitmap=NSBitmapImageRep(data:tiff),let png=bitmap.representation(using:.png,properties:[:]){try? png.write(to:URL(fileURLWithPath:"/tmp/aam-settings-preview.png"))}
     exit(0)
    }
   }
  }
 }
}
let check=Check();web.navigationDelegate=check
let root=URL(fileURLWithPath:FileManager.default.currentDirectoryPath,isDirectory:true)
web.loadFileURL(root.appendingPathComponent("src/web/index.html"),allowingReadAccessTo:root)
DispatchQueue.main.asyncAfter(deadline:.now()+25){print("Timed out");exit(1)}
app.run()
