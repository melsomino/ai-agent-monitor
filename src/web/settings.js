/* Preview-only settings. Uses synthetic state; never starts services or contacts Telegram. */
(() => {
  'use strict';
  const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  const art = {
    general:'<path d="m9 3 1-2h4l1 2 3 2h2l2 4-1 2v3l1 2-2 4h-2l-3 2-1 2h-4l-1-2-3-2H4l-2-4 1-2v-3L2 9l2-4h2Z"/><circle cx="12" cy="12" r="3"/>',
    applications:'<rect x="3" y="3" width="7" height="7" rx="2"/><rect x="14" y="3" width="7" height="7" rx="2"/><rect x="3" y="14" width="7" height="7" rx="2"/><rect x="14" y="14" width="7" height="7" rx="2"/>',
    agents:'<rect x="4" y="6" width="16" height="14" rx="4"/><path d="M12 2v4M1 11v4m22-4v4M8 12h.01M16 12h.01M9 16h6"/>',
    telegram:'<path d="m22 2-7 20-4-9-9-4 20-7ZM11 13 22 2"/>',
    web:'<circle cx="12" cy="12" r="9"/><ellipse cx="12" cy="12" rx="4" ry="9"/><path d="M3 12h18M5 6h14M5 18h14"/>',
    back:'<path d="m14 6-6 6 6 6"/>',
  };
  const icon = name => `<span class="aam-icon" aria-hidden="true"><svg viewBox="0 0 24 24">${art[name]}</svg></span>`;
  // QR_FIXTURES: valid QR matrices for reserved example addresses, not real pairing credentials.
  const qrFixtures = {"local":["111111101010111011000100101111111100000100101100010001000001000001101110101110110001001110001011101101110100100100000000001101011101101110100011010101010111101011101100000101000010101010100001000001111111101010101010101010101111111000000000110111110111001100000000101000110101010000001011100100101000010010011011001000101011001010110100101111000010000001101001101000101010111010111010100001111001110010100000010101010101000010001001010011011010000000100111111011101000110110111100110000111100001100111001010110000100001010001000000101110010101101010011011101010000111000010011000100011011001001001010111111001111100111101001111111110010001010110100011101111011010101101111010111111110000000001000111000111101010101010001101001111101110011011010101110010001001001010010101101111011011110100101111010111000101010001011111110111000000001010011001101110100010000111111101100100100011000101011111100000100000001111010100100010011101110100100110101000101111110001101110100111111000000000100011010101110101100010100110111100111011100000100111100000000000010011000111111101100111100110010010100101","111111101101111100000011101111111100000100111011010110000101000001101110100100001001110110101011101101110101011000111000110001011101101110101111110010010001101011101100000101000101101101100101000001111111101010101010101010101111111000000001001111001111110100000000100010111011010111001100111111001011010001111100001111101100001101011111110011111010111001010001010100010010100010000010011001000001000001101011010010010010000101001110010001010101000111100000111100010100101111100100001000000100110101000011101110111100110010110000001010100101101010010100011010010111111011110110000011011100001110010011100111111111011111010001000011010010010000001100100101000011010000101000111000111001000111001101010010101010010010010110101110000101101010000010010110101001110001101001010101000011100110011101111101111111101101001100111111111000000001010100001010110100010111111111101000010100100000101011000100000100101001000010011100011011101110101101111010000010111111001101110100110001000111000011011101101110100100101100001111011111100100000100001000111000111010100000111111101010111011110101010011101"],"remote":["111111101011010011111101001111111100000101101010000100111001000001101110100011100011010000101011101101110101001001110010000001011101101110100110111001001100101011101100000100110111000111100101000001111111101010101010101010101111111000000001000101001101011100000000101101110010101001110100101001011011100000100000010011011001101111011111110010101011001011101111011011011011010011000101000010101001111100110101000000100011110011000000100010111100011110000110101010100100101010000111111110101110000000010010100010010001100111011100001111101111001100010100011011100110100011001000101001111111011011001001100110111111001010100110100000101001001010000000010100010010010111100110101000110101000101101100111010111111101110001001100101001011101100110010100011010010011011001000011101010010001100011010101011111101010101010010111110010000000001010010111010111100011000111111101100110010010101101010010100000101001110100010101100011110101110100000101101000111111110111101110101101011000101011100100101101110101100111000000101001101100100000100110110111100000111110001111111101100001101101101000011100","111111100111010011111101001111111100000100110111101001010101000001101110101101010101100110001011101101110101111001110010000001011101101110101000110100100001001011101100000101001101110000010001000001111111101010101010101010101111111000000001101100100010110000000000101111100101011111000010001111100111110000000000010011011001101111000010111111000110100110000010110011111001000101110011110100011111011100111111100000100011110011000110100011010001110011101011000111010110100100010001001000011000110000110001100110010001100111011100100010110010100001111001110110001100000001111100011111001001101101001001100110100111001010100110100111011000100111101101111001111111000001110110010110000011110011011111110000111110101110001001100101100010110000011111001110111111110100111010100011100100111010101100100011111100010101010010111110010000000001011111010111010100010101111111100000000100100011101010100100000101011101100010101100011110101110101011011000101010111111010101110101011101110011101010010011101110101101011000000101001101100100000100010011010001101010011100111111101010111011011011110101010"],"loopback":["111111101110100011100111101111111100000100101000111010110101000001101110100010010010010010101011101101110101000001101101100001011101101110101001111000111000101011101100000101111000011110000101000001111111101010101010101010101111111000000001001101111100111000000000100010111001011000010100111111001111000011100111100000001100001110001100110111000101001111010001010100010011001101001110101001000011101100110101011000111000000111001010000001011000010010110110111100110110111100001010010001000101110111010011011010000011110110011100000010111110001111001100011011110100101011011010001011111100001101111000110001011010111001010001010101101001011101000010010101000001110010101001110010010011000101001101001010001100000111000011111110000010100100101100001110000101110001000011010110111000100001110010110110100100100011110100111111111000000001010011000011010100010110111111101010011111011110101011000100000100011010001100101100011011101110101100000000111000111111001101110100001111010010111011011101101110100111001010110010011110100100000100100000001111110010100000111111101011100100101010000011001","111111101110100011100111101111111100000100111000111010110101000001101110100010010010010010101011101101110101100101101101100001011101101110101010111000111000101011101100000101111000011110000101000001111111101010101010101010101111111000000001001001111100111000000000100010111001011000010100111111001111010011100111100000001100001110101101110011000101001111010001010100000011111101001110101001000011101101110111011000111000100111001011000000010000010010111110111100111110110101101010010001000101110110110010011110000011110110011100001010101111001111001100011011110100101011011010001011111100001101111000110111001010111001010001010101111001011110000010010101000001010101101011000010010011000101001111001000010000000111000011111110001010110101001100001110000101110001000001010010111000100001110010110110100100100011110100111111111000000001110001000011010100010110111111101110010111011110101011000100000100001011001100101100011011101110101010111000111000111111001101110100001111010010111011011101101110100110101010110010011110100100000100101100001111110010100000111111101011100100101010000011001"]};
  window.AAMSettings = {
    mount(product, emit, layout) {
      const menu=product.querySelector('.aam-menu'),screen=document.createElement('section');
      screen.className='aam-settings-screen';screen.hidden=true;screen.id='aam-settings-screen';screen.tabIndex=-1;
      screen.setAttribute('aria-label','Settings');menu.append(screen);
      const gear=product.querySelector('[data-command="settings"]');gear.setAttribute('aria-controls',screen.id);gear.setAttribute('aria-expanded','false');
      const tabs=[['general','General'],['applications','Applications'],['agents','Agents'],['telegram','Telegram'],['web','Web']];
      const apps=new Map(),preferences={login:false,animate:true,shortcut:'',applications:{}};
      let pane='general',snapshot={},scroll=0,origin=null,timer=null,agentTimer=null,remoteTimer=null,destroyed=false;
      let bot='aiagmon_demo_bot',replacing=false,username='aiagmon_demo_bot',tokenDraft='',botStage='idle',botNote='',agentBusy=false,agentNote='';
      const web={running:false,remote:'off',selected:'http://192.0.2.10:8765/',generation:0,deadline:0};
      const button=(label,action,extra='')=>`<button type="button" class="aam-action" data-setting-action="${action}" ${extra}>${label}</button>`;
      const checkbox=(id,label,detail,checked)=>`<label class="aam-setting-row aam-setting-checkbox"><input type="checkbox" data-preference="${id}" ${checked?'checked':''}><span><strong>${label}</strong><small>${detail}</small></span></label>`;
      function general() {
        return `<div class="aam-settings-section">${checkbox('login','Launch at login','Open AI Agent Monitor automatically',preferences.login)}${checkbox('animate','Menu bar animation','Animate the eyes while an agent is working',preferences.animate)}</div><div class="aam-setting-row aam-shortcut-row"><span><strong>Keyboard shortcut</strong><small data-shortcut-note>Open the sessions menu from any app</small></span>${button(esc(preferences.shortcut)||'Record shortcut','record','aria-label="Record keyboard shortcut"')}${button('×','clear-shortcut',`aria-label="Clear keyboard shortcut" ${preferences.shortcut?'':'disabled'}`)}</div>`;
      }
      function applications() {
        return `<h2 class="aam-settings-section-title">Show sessions in the menu</h2><div class="aam-settings-section">${[...apps.values()].map(app=>`<label class="aam-setting-row aam-setting-app"><img src="${esc(app.icon)}" alt="" width="24" height="24"><strong>${esc(app.name)}</strong><input type="checkbox" role="switch" aria-label="Show ${esc(app.name)} sessions" data-application="${esc(app.id)}" ${preferences.applications[app.id]===false?'':'checked'}></label>`).join('')||'<p class="aam-settings-help">No applications detected yet.</p>'}</div>`;
      }
      function agents() {
        return `<h2 class="aam-settings-section-title">Detected agents</h2><div class="aam-settings-section"><div class="aam-agent-setting"><div><strong>Codex</strong><span class="aam-agent-ready">✓ No setup required</span></div><p>Direct local connection; no hooks required.</p></div><div class="aam-agent-setting"><div><strong>Claude Code</strong><span class="aam-agent-ready">✓ Hooks installed</span></div><p>Restart Claude Code to load the hooks.</p></div></div><div class="aam-agent-actions"><p class="aam-settings-help" role="status">${esc(agentNote)}</p>${button(agentBusy?'Setting up…':'Check & set up','check-agents',agentBusy?'disabled':'')}</div>`;
      }
      function telegram() {
        if(bot&&!replacing)return `<h2>Telegram is connected</h2><div class="aam-bot-connected"><strong title="@${esc(bot)}">@${esc(bot)}</strong>${button('Test','test-bot')}${button('Open','open-bot')}</div>${botNote?`<p class="aam-settings-help" role="status">${esc(botNote)}</p>`:''}<div class="aam-settings-actions">${button('Change bot','change-bot')}${button('Unlink bot','unlink-bot')}</div>`;
        const busy=botStage==='checking';
        return `<h2>Set up Telegram integration</h2><p class="aam-settings-intro">Set up your personal Telegram bot in a few simple steps.</p><p class="aam-settings-help">We chose personal Telegram bots over shared ones to protect your privacy. Your bot connects directly to the app running on your Mac, so your conversations never pass through our servers — only Telegram’s.</p><p class="aam-settings-help">Already have a bot? Just paste its token below.</p><div class="aam-settings-step"><strong>1. Open BotFather and send /newbot.</strong>${button('Open','botfather')}</div><div class="aam-settings-section"><h3>2. Choose your bot’s name and username</h3><div class="aam-bot-field"><label for="aam-bot-name">Bot name:</label><input id="aam-bot-name" value="AI Agent Monitor" readonly>${button('Copy','copy-name')}</div><div class="aam-bot-field"><label for="aam-bot-user">Bot username:</label><input id="aam-bot-user" value="${esc(username)}" readonly>${button('↻','regenerate','aria-label="Generate another bot username"')}${button('Copy','copy-username')}</div><small>Must be unique. Regenerate if the username is taken.</small></div><form class="aam-bot-form"><h3>3. Copy the token from BotFather and paste it below.</h3><p class="aam-settings-help">Find it in BotFather’s last message.</p><div class="aam-bot-token"><input type="password" autocomplete="off" aria-label="Bot token" placeholder="Paste the entire token from BotFather" value="${esc(tokenDraft)}" required ${busy?'disabled':''}>${button('Paste','paste-token',busy?'disabled':'')}</div><small>Your bot token will be stored securely in macOS Keychain.</small>${bot?'<p class="aam-settings-help">Your current bot stays connected until you start the new bot.</p>':''}<p class="aam-bot-status aam-settings-help" role="status">${esc(botNote)}</p><div class="aam-settings-actions aam-bot-actions"><button type="submit" class="aam-action" ${busy?'disabled':''}>Connect</button>${botStage==='waiting'?button('Open bot','activate-bot'):''}${bot||replacing||botStage!=='idle'?button('Cancel','cancel-bot'):''}</div></form>`;
      }
      function urls() {return [...(web.remote==='connected'?['https://monitor-preview.example/']:[]),'http://192.0.2.10:8765/','http://127.0.0.1:8765/'];}
      function qrMarkup() {
        // These two demo codes alternate only to illustrate renewal, not to implement authentication.
        const grid=qrFixtures[web.selected.startsWith('https:')?'remote':web.selected.includes('127.0.0.1')?'loopback':'local'][web.generation%2],size=Math.sqrt(grid.length);
        const cells=[];for(let i=0;i<grid.length;i++)if(grid[i]==='1')cells.push(`M${i%size+4} ${Math.floor(i/size)+4}h1v1h-1z`);
        return `<svg class="aam-web-qr" viewBox="0 0 ${size+8} ${size+8}" role="img" aria-label="Demo pairing QR code for ${esc(web.selected)}"><rect width="100%" height="100%" fill="white"/><path d="${cells.join('')}" fill="black"/></svg>`;
      }
      function webPane() {
        const remoteNote=web.remote==='starting'?'Connecting to Cloudflare…':'';
        const remote=web.selected.startsWith('https:');
        const header=`<div class="aam-web-title"><h2>Web access</h2><input type="checkbox" role="switch" data-web="running" aria-label="Enable web access" ${web.running?'checked':''}></div><p class="aam-settings-intro">Open your monitor in a browser on your phone.</p>`;
        if(!web.running)return header;
        return `${header}${remoteNote?`<p class="aam-settings-help aam-web-remote-note" role="status">${remoteNote}</p>`:''}<div class="aam-web-addresses" role="group" aria-label="Connection address">${web.running?urls().map(url=>`<label><input type="radio" name="aam-web-address" data-web-address="${esc(url)}" ${web.selected===url?'checked':''}><span class="aam-web-address-url" title="${esc(url)}">${esc(url)}</span><small>${url.startsWith('https:')?'Cloudflare tunnel':url.includes('127.0.0.1')?'This computer':'Local network'}</small></label>`).join(''):''}</div><div class="aam-web-pairing"><div>${web.running?qrMarkup():''}<div class="aam-web-countdown" ${web.running?'':'hidden'}><progress max="120" value="120" aria-label="Pairing link time remaining"></progress><span data-web-remaining>120 sec</span></div></div><div class="aam-web-instructions"><p>${!web.running?'Web access is off. Turn it on to connect your phone.':remote?'Scan with your phone’s camera.<br><br>Works outside your Wi-Fi. Traffic passes through Cloudflare.<br><br>The QR code renews automatically.':'Scan with your phone’s camera.<br><br>Same Wi-Fi network required.<br><br>Single-use link. The QR code renews automatically.'}</p>${button('New QR code','renew-web',web.running?'':'disabled')}${button('Copy link','copy-web',web.running?'':'disabled')}</div></div><p class="aam-settings-help">0 paired browsers</p><small>Turning Web access off disconnects all browsers and stops the tunnel.</small>`;
      }
      function render(focusTab=false) {
        if(destroyed)return;
        screen.innerHTML=`<header class="aam-settings-header">${button(icon('back')+'Sessions','back','aria-label="Back to sessions"')}<span>Settings</span><small>${esc(snapshot.version||'')}</small></header><nav class="aam-settings-tabs" role="tablist" aria-label="Settings sections">${tabs.map(([id,title])=>`<button type="button" role="tab" id="aam-settings-tab-${id}" data-pane="${id}" aria-controls="aam-settings-content" aria-selected="${pane===id}" tabindex="${pane===id?0:-1}">${icon(id)}<span>${title}</span></button>`).join('')}</nav><div class="aam-settings-body" id="aam-settings-content" role="tabpanel" aria-labelledby="aam-settings-tab-${pane}">${({general,applications,agents,telegram,web:webPane})[pane]()}<p class="aam-settings-feedback" role="status" aria-live="polite"></p></div>`;
        if(focusTab)screen.querySelector(`[data-pane="${pane}"]`).focus({preventScroll:true});
        updateCountdown();layout();
      }
      function feedback(text) {const node=screen.querySelector('.aam-settings-feedback');if(node)node.textContent=text;layout();}
      function notify() {emit({type:'preferences',preferences:structuredClone(preferences)});}
      function renewWeb() {web.generation++;web.deadline=performance.now()+120000;}
      function updateCountdown() {
        if(screen.hidden||pane!=='web'||!web.running)return;
        const seconds=Math.max(0,Math.ceil((web.deadline-performance.now())/1000));
        if(!seconds){renewWeb();render();return;}
        const progress=screen.querySelector('.aam-web-countdown progress'),remaining=screen.querySelector('[data-web-remaining]');
        if(progress)progress.value=Math.min(120,(web.deadline-performance.now())/1000);
        if(remaining)remaining.textContent=`${seconds} sec`;
      }
      const clock=setInterval(updateCountdown,250);
      function close() {
        if(screen.hidden)return;screen.hidden=true;product.classList.remove('is-settings');gear.setAttribute('aria-expanded','false');
        product.querySelector('.aam-list').inert=false;product.querySelector('.aam-list').scrollTop=scroll;layout();origin?.focus({preventScroll:true});
      }
      function finishBot() {bot=username||'aiagmon_demo_bot';replacing=false;botStage='idle';tokenDraft='';botNote='';if(pane==='telegram')render();}
      screen.addEventListener('click',event=>{
        const tab=event.target.closest('[data-pane]');if(tab){pane=tab.dataset.pane;screen.scrollTop=0;render(true);return;}
        const control=event.target.closest('[data-setting-action]');if(!control||control.disabled)return;const action=control.dataset.settingAction;
        if(action==='back')close();
        if(action==='clear-shortcut'){preferences.shortcut='';notify();render();}
        if(action==='record'){control.textContent='Press shortcut…';control.dataset.recording='true';screen.querySelector('[data-shortcut-note]').textContent='Press a shortcut. Escape cancels.';}
        if(action==='change-bot'){replacing=true;botStage='idle';botNote='';tokenDraft='';render();}
        if(action==='cancel-bot'){clearTimeout(timer);botStage='idle';botNote='';tokenDraft='';replacing=false;render();}
        if(action==='unlink-bot'){clearTimeout(timer);bot=null;replacing=false;botStage='idle';botNote='';tokenDraft='';render();}
        if(action==='regenerate'){username=`aiagmon_demo_${Math.random().toString(36).slice(2,6)}_bot`;screen.querySelector('#aam-bot-user').value=username;}
        if(action==='copy-name'||action==='copy-username')emit({type:'copy',text:action==='copy-name'?'AI Agent Monitor':username});
        if(action==='paste-token') {
          const fallback=()=>feedback('Use Command-V or Control-V to paste your token.');
          if(!navigator.clipboard?.readText)fallback();else navigator.clipboard.readText().then(text=>{tokenDraft=text;const input=screen.querySelector('.aam-bot-token input');if(input){input.value=text;input.focus();}}).catch(fallback);
        }
        if(action==='check-agents'){agentBusy=true;agentNote='Checking agents…';render();clearTimeout(agentTimer);agentTimer=setTimeout(()=>{agentBusy=false;agentNote='Setup complete.';if(pane==='agents')render();},800);}
        if(action==='test-bot'){botNote='Test notification sent.';render();}
        if(action==='open-bot'||action==='botfather')feedback(action==='botfather'?'Telegram → BotFather → /newbot (preview).':'Bot opened (preview).');
        if(action==='activate-bot'){clearTimeout(timer);finishBot();}
        if(action==='renew-web'){renewWeb();render();}
        if(action==='copy-web')emit({type:'copy',text:`${web.selected}#pair=preview-only-${web.generation%2?'b':'a'}`});
      });
      screen.addEventListener('change',event=>{
        const input=event.target;
        if(input.dataset.preference)preferences[input.dataset.preference]=input.checked;
        if(input.dataset.application)preferences.applications[input.dataset.application]=input.checked;
        if(input.dataset.preference||input.dataset.application)notify();
        if(input.dataset.web==='running'){
          web.running=input.checked;clearTimeout(remoteTimer);web.selected='http://192.0.2.10:8765/';
          web.remote=web.running?'starting':'off';
          if(web.running){
            renewWeb();
            remoteTimer=setTimeout(()=>{web.remote='connected';web.selected='https://monitor-preview.example/';renewWeb();if(pane==='web')render();},800);
          } else web.deadline=0;
          render();
        }
        if(input.dataset.webAddress){web.selected=input.dataset.webAddress;render();}
      });
      screen.addEventListener('input',event=>{if(event.target.matches('.aam-bot-token input'))tokenDraft=event.target.value;});
      screen.addEventListener('submit',event=>{
        event.preventDefault();event.stopPropagation();if(botStage==='checking')return;
        tokenDraft=screen.querySelector('.aam-bot-token input').value;
        if(!/^\d{6,}:[\w-]{20,}$/.test(tokenDraft.trim())){botNote='Invalid token. Paste the entire token from BotFather.';render();return;}
        botStage='checking';botNote='Checking token…';render();clearTimeout(timer);
        timer=setTimeout(()=>{botStage='waiting';botNote='Waiting for bot activation…';if(pane==='telegram'){render();feedback('Bot opened (preview). Start it to finish connecting.');}},800);
      });
      screen.addEventListener('keydown',event=>{
        if(event.target.dataset.recording){
          if(event.key==='Escape'){event.preventDefault();render();}
          else if((event.metaKey||event.ctrlKey||event.altKey)&&!['Meta','Control','Alt','Shift'].includes(event.key)){event.preventDefault();preferences.shortcut=[event.ctrlKey?'⌃':'',event.altKey?'⌥':'',event.shiftKey?'⇧':'',event.metaKey?'⌘':'',event.key.toUpperCase()].join('');notify();render();}
          else if(!['Meta','Control','Alt','Shift'].includes(event.key))screen.querySelector('[data-shortcut-note]').textContent='Use Command, Control or Option with another key.';
          event.stopPropagation();return;
        }
        if(event.key==='Escape'){event.preventDefault();close();}
        if(event.key==='Tab'){
          const targets=[...screen.querySelectorAll('button:not(:disabled),input:not(:disabled)')].filter(node=>node.tabIndex>=0&&node.getClientRects().length);
          if(event.shiftKey&&event.target===targets[0]){event.preventDefault();targets.at(-1)?.focus();}
          else if(!event.shiftKey&&(event.target===targets.at(-1)||event.target===screen)){event.preventDefault();targets[0]?.focus();}
        }
        if(event.target.closest('[role="tab"]')&&['ArrowLeft','ArrowRight','Home','End'].includes(event.key)){
          event.preventDefault();let at=tabs.findIndex(([id])=>id===pane);at=event.key==='Home'?0:event.key==='End'?tabs.length-1:(at+(event.key==='ArrowRight'?1:tabs.length-1))%tabs.length;pane=tabs[at][0];render(true);
        }
        event.stopPropagation();
      });
      return {
        open(){if(!screen.hidden)return;origin=gear;scroll=product.querySelector('.aam-list').scrollTop;product.classList.add('is-settings');product.querySelector('.aam-list').inert=true;screen.hidden=false;screen.scrollTop=0;gear.setAttribute('aria-expanded','true');render();screen.focus({preventScroll:true});},
        close,
        update(value){snapshot=value;if(value.preferences)Object.assign(preferences,structuredClone(value.preferences));apps.clear();for(const app of value.applications||value.sessions.map(s=>({id:s.appId,name:s.app,icon:s.icon})))apps.set(app.id,app);},
        active:()=>!screen.hidden,
        destroy(){destroyed=true;clearTimeout(timer);clearTimeout(agentTimer);clearTimeout(remoteTimer);clearInterval(clock);screen.remove();},
      };
    },
  };
})();
