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
    copy:'<rect x="8" y="8" width="12" height="13" rx="2"/><path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3"/>',
    regenerate:'<path d="M20 7a8 8 0 1 0 1 7M20 3v5h-5"/>',
    back:'<path d="m14 6-6 6 6 6"/>',
  };
  const tokenDashboardIcon = '<span class="aam-icon aam-token-key" aria-hidden="true"><svg viewBox="0 0 24 24"><circle cx="8" cy="8" r="5"/><path d="m11.5 11.5 9 9M16 16l3-3M19 19l3-3"/></svg></span><span class="aam-icon" aria-hidden="true"><svg viewBox="0 0 24 24"><path d="M7 17 17 7M7 7h10v10"/></svg></span>';
  const icon = name => `<span class="aam-icon" aria-hidden="true"><svg viewBox="0 0 24 24">${art[name]}</svg></span>`;
  // QR_FIXTURES: valid QR matrices for reserved example addresses, not real pairing credentials.
  const qrFixtures = {"tuna":["111111100110000000111011001111111100000100000100110100010001000001101110101100001010011100001011101101110101110000000110011101011101101110101101001110001101001011101100000101101010000010100001000001111111101010101010101010101111111000000001001001010011101000000000101111100011011001010011101111100000000001101111001010011101101111001011100001011111001110010010110100110000101010111111100101011110011000101001001110001010110011001111010010110110101100111001000111111001111011000110101000011000010011011010111001100100100011000100100101101110100101001011110111001101011001100100010111001011101101101000111110011100101100101110110111001000100100110001111010111110001100110101010000100001110111000110100011101010110001101101001001101101111011100000110110010111110100101001000011010011111000101110100010111010101001010010111111000000000001001010011111010100010101111111100101110101001111101010110100000101011000101100111100011100101110101001101110010010111111000101110101110010101100101110010111101110101010111110001011111100100100000100101000100000101010011100111111101110000111000011110101010", "111111100010000000111011001111111100000100010100110100010001000001101110101100001010011100001011101101110101100100000110011101011101101110101101001110001101001011101100000101101010000010100001000001111111101010101010101010101111111000000001001101010011101000000000101111100011011001010011101111100100000001101111001010011101101111101010100001011111001110010010110110111000111010111111100101011110101010100001001110001010110011001110010000100010101100111001000111110101110010100110101000011000010010011010110001100100100011000100101101101110100101001011110111001001011001000110010111001011101101011010111000000100101100101110110001000000010110110001111010111110111101111101010000100001110111000111101011110110110001101101001001101101111011000000110110010111110101001011001111010011111000101110101010111010101001010010111111000000000001101000011111010100010101111111100101111101001111101010110100000101011010101100111100011100101110101001010110010010111111000101110101110111101100101110010111101110101010011110000011111100100100000100100100100010101010011100111111101111000111000011110101010"],"ngrok":["111111100100010001110001101111111100000100110111111001010001000001101110101001010111111111101011101101110101011001110001011101011101101110101100110101100001001011101100000101011111111001110001000001111111101010101010101010101111111000000001011001100100110000000000101111100000110101001011101111100011011000001100010111011001101101000010100000000100101010010010110000000001110101110001101010011110110101100100100000100011110111000010000010001111110001101111000011001111110000001000010100011111110100001000111111010011101011101100000111101101111001010010110111001011111010011001011011011001101101111100111101000101001110110110110100101011011101101100101101111100111010110110000110010010010011000111100001111010101100111001000101101100110000111110101000011100010101111010010001100100100001100101101000100001010111000010111111000000000001000001010111001100010111111111100000100110100101101010110100000101000111110011111100011111101110101110001000100000111111000101110101001101110011101010011011101110101010010000101111101101100100000100101011010001110001011100111111101001000001011011110101010", "111111100000010001110001101111111100000100100111111001010001000001101110101111010111111111101011101101110101001101110001011101011101101110101101010101100001001011101100000101010111111001110001000001111111101010101010101010101111111000000001010001100100110000000000101111100001110101001011101111100111001000101100010111011001101101000000100110000100101010010010110110010001010101110001101010011110100101100110100000100011110111000001100001000011110001101111000011001111101000001000010100011111110100001001111011010011101011101100001111101101111001010010110111001011101010011011011011011001101101001100111101010101001110110110110110111011111111101100101101111100001101110010111110010010010011000110100000100010101100111001000101100100100000111110100000011100010101111001010101100110100001100101101000100001010111000010111111000000000001100011010111001100010111111111100110110110100101101010110100000101100100110011111100011111101110101010000000100000111111000101110101000100110011101010011011101110101011010000101111101101100100000100100011010001110001011100111111101001000001011011110101010"],"local":["111111101010111011000100101111111100000100101100010001000001000001101110101110110001001110001011101101110100100100000000001101011101101110100011010101010111101011101100000101000010101010100001000001111111101010101010101010101111111000000000110111110111001100000000101000110101010000001011100100101000010010011011001000101011001010110100101111000010000001101001101000101010111010111010100001111001110010100000010101010101000010001001010011011010000000100111111011101000110110111100110000111100001100111001010110000100001010001000000101110010101101010011011101010000111000010011000100011011001001001010111111001111100111101001111111110010001010110100011101111011010101101111010111111110000000001000111000111101010101010001101001111101110011011010101110010001001001010010101101111011011110100101111010111000101010001011111110111000000001010011001101110100010000111111101100100100011000101011111100000100000001111010100100010011101110100100110101000101111110001101110100111111000000000100011010101110101100010100110111100111011100000100111100000000000010011000111111101100111100110010010100101","111111101101111100000011101111111100000100111011010110000101000001101110100100001001110110101011101101110101011000111000110001011101101110101111110010010001101011101100000101000101101101100101000001111111101010101010101010101111111000000001001111001111110100000000100010111011010111001100111111001011010001111100001111101100001101011111110011111010111001010001010100010010100010000010011001000001000001101011010010010010000101001110010001010101000111100000111100010100101111100100001000000100110101000011101110111100110010110000001010100101101010010100011010010111111011110110000011011100001110010011100111111111011111010001000011010010010000001100100101000011010000101000111000111001000111001101010010101010010010010110101110000101101010000010010110101001110001101001010101000011100110011101111101111111101101001100111111111000000001010100001010110100010111111111101000010100100000101011000100000100101001000010011100011011101110101101111010000010111111001101110100110001000111000011011101101110100100101100001111011111100100000100001000111000111010100000111111101010111011110101010011101"],"remote":["111111101011010011111101001111111100000101101010000100111001000001101110100011100011010000101011101101110101001001110010000001011101101110100110111001001100101011101100000100110111000111100101000001111111101010101010101010101111111000000001000101001101011100000000101101110010101001110100101001011011100000100000010011011001101111011111110010101011001011101111011011011011010011000101000010101001111100110101000000100011110011000000100010111100011110000110101010100100101010000111111110101110000000010010100010010001100111011100001111101111001100010100011011100110100011001000101001111111011011001001100110111111001010100110100000101001001010000000010100010010010111100110101000110101000101101100111010111111101110001001100101001011101100110010100011010010011011001000011101010010001100011010101011111101010101010010111110010000000001010010111010111100011000111111101100110010010101101010010100000101001110100010101100011110101110100000101101000111111110111101110101101011000101011100100101101110101100111000000101001101100100000100110110111100000111110001111111101100001101101101000011100","111111100111010011111101001111111100000100110111101001010101000001101110101101010101100110001011101101110101111001110010000001011101101110101000110100100001001011101100000101001101110000010001000001111111101010101010101010101111111000000001101100100010110000000000101111100101011111000010001111100111110000000000010011011001101111000010111111000110100110000010110011111001000101110011110100011111011100111111100000100011110011000110100011010001110011101011000111010110100100010001001000011000110000110001100110010001100111011100100010110010100001111001110110001100000001111100011111001001101101001001100110100111001010100110100111011000100111101101111001111111000001110110010110000011110011011111110000111110101110001001100101100010110000011111001110111111110100111010100011100100111010101100100011111100010101010010111110010000000001011111010111010100010101111111100000000100100011101010100100000101011101100010101100011110101110101011011000101010111111010101110101011101110011101010010011101110101101011000000101001101100100000100010011010001101010011100111111101010111011011011110101010"],"loopback":["111111101110100011100111101111111100000100101000111010110101000001101110100010010010010010101011101101110101000001101101100001011101101110101001111000111000101011101100000101111000011110000101000001111111101010101010101010101111111000000001001101111100111000000000100010111001011000010100111111001111000011100111100000001100001110001100110111000101001111010001010100010011001101001110101001000011101100110101011000111000000111001010000001011000010010110110111100110110111100001010010001000101110111010011011010000011110110011100000010111110001111001100011011110100101011011010001011111100001101111000110001011010111001010001010101101001011101000010010101000001110010101001110010010011000101001101001010001100000111000011111110000010100100101100001110000101110001000011010110111000100001110010110110100100100011110100111111111000000001010011000011010100010110111111101010011111011110101011000100000100011010001100101100011011101110101100000000111000111111001101110100001111010010111011011101101110100111001010110010011110100100000100100000001111110010100000111111101011100100101010000011001","111111101110100011100111101111111100000100111000111010110101000001101110100010010010010010101011101101110101100101101101100001011101101110101010111000111000101011101100000101111000011110000101000001111111101010101010101010101111111000000001001001111100111000000000100010111001011000010100111111001111010011100111100000001100001110101101110011000101001111010001010100000011111101001110101001000011101101110111011000111000100111001011000000010000010010111110111100111110110101101010010001000101110110110010011110000011110110011100001010101111001111001100011011110100101011011010001011111100001101111000110111001010111001010001010101111001011110000010010101000001010101101011000010010011000101001111001000010000000111000011111110001010110101001100001110000101110001000001010010111000100001110010110110100100100011110100111111111000000001110001000011010100010110111111101110010111011110101011000100000100001011001100101100011011101110101010111000111000111111001101110100001111010010111011011101101110100110101010110010011110100100000100101100001111110010100000111111101011100100101010000011001"]};
  window.AAMSettings = {
    mount(product, emit, layout) {
      const menu=product.querySelector('.aam-menu'),screen=document.createElement('section');
      screen.className='aam-settings-screen';screen.hidden=true;screen.id='aam-settings-screen';screen.tabIndex=-1;
      screen.setAttribute('aria-label','Settings');menu.append(screen);
      const gear=product.querySelector('[data-command="settings"]');gear.setAttribute('aria-controls',screen.id);gear.setAttribute('aria-expanded','false');
      const tabs=[['general','General'],['applications','Applications'],['agents','Agents'],['telegram','Telegram'],['web','Web']];
      const apps=new Map(),preferences={login:false,animate:true,shortcut:'',applications:{}};
      let pane='general',snapshot={},scroll=0,origin=null,timer=null,agentTimer=null,remoteTimer=null,ngrokTimer=null,tunaTimer=null,destroyed=false;
      let bot='aiagmon_demo_bot',replacing=false,username='aiagmon_demo_bot',tokenDraft='',botStage='idle',botNote='',agentBusy=false,agentNote='';
      const web={running:false,ngrok:'not_configured',ngrokExpanded:false,tokenSaved:false,tokenChanging:false,tokenDraft:'',tuna:'not_configured',tunaExpanded:false,tunaSaved:false,tunaChanging:false,tunaDraft:'',remote:'off',selected:'http://192.0.2.10:8765/',generation:0,deadline:0};
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
      function urls() {return [...(web.remote==='connected'?['https://monitor-preview.example/']:[]),...(web.ngrok==='connected'?['https://ngrok-preview.example/']:[]),...(web.tuna==='connected'?['https://tuna-preview.example/']:[]),'http://192.0.2.10:8765/','http://127.0.0.1:8765/'];}
      function qrMarkup() {
        // These two demo codes alternate only to illustrate renewal, not to implement authentication.
        const grid=qrFixtures[web.selected.includes('tuna-preview')?'tuna':web.selected.includes('ngrok-preview')?'ngrok':web.selected.startsWith('https:')?'remote':web.selected.includes('127.0.0.1')?'loopback':'local'][web.generation%2],size=Math.sqrt(grid.length);
        const cells=[];for(let i=0;i<grid.length;i++)if(grid[i]==='1')cells.push(`M${i%size+4} ${Math.floor(i/size)+4}h1v1h-1z`);
        return `<svg class="aam-web-qr" viewBox="0 0 ${size+8} ${size+8}" role="img" aria-label="Demo pairing QR code for ${esc(web.selected)}"><rect width="100%" height="100%" fill="white"/><path d="${cells.join('')}" fill="black"/></svg>`;
      }
      function accessRow(name,url,status,provider='') {
        const available=urls().includes(url);
        const disclosure=provider?`<button type="button" class="aam-${provider}-disclosure" data-setting-action="toggle-${provider}" aria-label="${web[provider+'Expanded']?'Collapse':'Expand'} ${name} settings" aria-expanded="${web[provider+'Expanded']}" aria-controls="aam-${provider}-details"><span class="aam-icon" aria-hidden="true"><svg viewBox="0 0 24 24"><path d="${web[provider+'Expanded']?'m6 9 6 6 6-6':'m9 6 6 6-6 6'}"/></svg></span></button>`:'';
        return `<div class="aam-web-route"><label title="${available?esc(url):esc(name)}"><input type="radio" name="aam-web-address" ${available?`data-web-address="${esc(url)}"`:'disabled'} ${available&&web.selected===url?'checked':''}><span>${name}</span></label>${disclosure}<small class="${provider?'aam-'+provider+'-status':''}">${status}</small></div>`;
      }
      function tunaPane() {
        const caption=web.tuna==='connected'?'ready':web.tuna==='starting'?'connecting…':web.tuna==='failed'?'connection failed':'not configured';
        const editingToken=!web.tunaSaved||web.tunaChanging;
        const tokenDetails=editingToken?`<form class="aam-tuna-token"><div class="aam-tuna-token-label"><label for="aam-tuna-token">API key:</label>${web.tunaChanging?button('Cancel','cancel-tuna-token'):''}</div><div class="aam-tuna-token-row"><input id="aam-tuna-token" type="password" autocomplete="off" aria-label="Tuna API key" aria-describedby="aam-tuna-storage" placeholder="Paste API key" value="${esc(web.tunaDraft)}"><button type="button" class="aam-action aam-tuna-dashboard" data-setting-action="tuna-dashboard" aria-label="Open tuna dashboard in browser" title="Get your API key from the Tuna dashboard">${tokenDashboardIcon}</button>${button('Connect','connect-tuna',web.tuna==='starting'||!web.tunaDraft.trim()?'disabled':'')}</div><small id="aam-tuna-storage" class="aam-tuna-storage">Will be stored securely in macOS Keychain</small></form>`:`<div class="aam-tuna-saved"><span>API key: <span class="aam-tuna-storage-status">stored securely in macOS Keychain</span></span>${button('Change','change-tuna-token')}${web.tuna==='failed'?button('Reconnect','connect-tuna'):''}</div>`;
        const tuna=`<section class="aam-tuna-section">${accessRow('Tuna','https://tuna-preview.example/',caption,'tuna')}<div id="aam-tuna-details" ${web.tunaExpanded?'':'hidden'}>${tokenDetails}${web.tuna==='failed'?'<small class="aam-settings-help" role="status">Tuna connection failed. Check your account and network, then reconnect.</small>':''}</div></section>`;
        return tuna;
      }
      function webPane() {
        const remote=web.selected.startsWith('https:');
        const selectedProvider=web.selected.includes('tuna-preview')?'Tuna':web.selected.includes('ngrok-preview')?'ngrok':'Cloudflare';
        const header=`<div class="aam-web-title"><h2>Web access</h2><input type="checkbox" role="switch" data-web="running" aria-label="Enable web access" ${web.running?'checked':''}></div><p class="aam-settings-intro">Open your monitor in a browser on your phone.</p>`;
        if(!web.running)return header;
        const caption=web.ngrok==='connected'?'ready':web.ngrok==='starting'?'connecting…':web.ngrok==='failed'?'connection failed':'not configured';
        const editingToken=!web.tokenSaved||web.tokenChanging;
        const tokenDetails=editingToken?`<form class="aam-ngrok-token"><div class="aam-ngrok-token-label"><label for="aam-ngrok-token">Token:</label>${web.tokenChanging?button('Cancel','cancel-ngrok-token'):''}</div><div class="aam-ngrok-token-row"><input id="aam-ngrok-token" type="password" autocomplete="off" aria-label="ngrok authtoken" aria-describedby="aam-ngrok-storage" placeholder="Paste token" value="${esc(web.tokenDraft)}"><button type="button" class="aam-action aam-ngrok-dashboard" data-setting-action="ngrok-dashboard" aria-label="Open ngrok dashboard in browser" title="Get your authtoken from the ngrok dashboard">${tokenDashboardIcon}</button>${button('Connect','connect-ngrok',web.ngrok==='starting'||!web.tokenDraft.trim()?'disabled':'')}</div><small id="aam-ngrok-storage" class="aam-ngrok-storage">Will be stored securely in macOS Keychain</small></form>`:`<div class="aam-ngrok-saved"><span>Token: <span class="aam-ngrok-storage-status">stored securely in macOS Keychain</span></span>${button('Change','change-ngrok-token')}${web.ngrok==='failed'?button('Reconnect','connect-ngrok'):''}</div>`;
        const ngrok=`<section class="aam-ngrok-section">${accessRow('ngrok','https://ngrok-preview.example/',caption,'ngrok')}<div id="aam-ngrok-details" ${web.ngrokExpanded?'':'hidden'}>${tokenDetails}${web.ngrok==='failed'?'<small class="aam-settings-help" role="status">ngrok connection failed. Check your account and network, then reconnect.</small>':''}</div></section>`;
        return `${header}<div class="aam-web-addresses" role="group" aria-label="Connection method">${ngrok}${tunaPane()}${accessRow('Cloudflare','https://monitor-preview.example/',web.remote==='connected'?'ready':'connecting…')}${accessRow('Local network','http://192.0.2.10:8765/','ready')}${accessRow('This computer','http://127.0.0.1:8765/','ready')}</div><div class="aam-web-link-panel"><div class="aam-web-link-row"><a class="aam-web-link-url" href="${esc(web.selected)}#pair=preview-only-${web.generation%2?'b':'a'}" data-setting-action="open-web" title="${esc(web.selected)}">${esc(web.selected)}</a><button type="button" class="aam-ngrok-disclosure aam-web-link-icon" data-setting-action="copy-web" aria-label="Copy link" title="Copy link">${icon('copy')}</button></div><div class="aam-web-countdown"><progress max="120" value="120" aria-label="Pairing link time remaining"></progress><span data-web-remaining>120 sec</span><button type="button" class="aam-ngrok-disclosure aam-web-link-icon" data-setting-action="renew-web" aria-label="Generate new QR code" title="Generate new QR code">${icon('regenerate')}</button></div></div><div class="aam-web-pairing"><div>${qrMarkup()}</div><div class="aam-web-instructions"><p>${remote?'Scan with your phone’s camera.<br><br>Works outside your Wi-Fi. Traffic passes through '+selectedProvider+'.<br><br>The QR code renews automatically.':'Scan with your phone’s camera.<br><br>Same Wi-Fi network required.<br><br>Single-use link. The QR code renews automatically.'}</p></div></div><div class="aam-web-footer"><p class="aam-settings-help">0 paired browsers</p><small>Turning Web access off disconnects all browsers and stops the tunnels.</small></div>`;
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
        if(action==='toggle-ngrok'){web.ngrokExpanded=!web.ngrokExpanded;render();}
        if(action==='ngrok-dashboard')emit({type:'link',url:'https://dashboard.ngrok.com/get-started/your-authtoken'});
        if(action==='change-ngrok-token'){web.tokenChanging=true;web.tokenDraft='';render();screen.querySelector('#aam-ngrok-token')?.focus();}
        if(action==='cancel-ngrok-token'){web.tokenChanging=false;web.tokenDraft='';render();}
        if(action==='connect-ngrok'){
          const value=web.tokenDraft.trim();
          if(!value&&(!web.tokenSaved||web.tokenChanging)){feedback('Enter your ngrok authtoken.');return;}
          web.tokenSaved=true;web.tokenChanging=false;web.tokenDraft='';web.ngrok='starting';render();clearTimeout(ngrokTimer);
          ngrokTimer=setTimeout(()=>{web.ngrok=value==='invalid'?'failed':'connected';if(web.ngrok==='connected')web.selected='https://ngrok-preview.example/';renewWeb();if(pane==='web')render();},800);
        }
        if(action==='toggle-tuna'){web.tunaExpanded=!web.tunaExpanded;render();}
        if(action==='tuna-dashboard')emit({type:'link',url:'https://my.tuna.am/api_keys?lang=en'});
        if(action==='change-tuna-token'){web.tunaChanging=true;web.tunaDraft='';render();screen.querySelector('#aam-tuna-token')?.focus();}
        if(action==='cancel-tuna-token'){web.tunaChanging=false;web.tunaDraft='';render();}
        if(action==='connect-tuna'){
          const value=web.tunaDraft.trim();
          if(!value&&(!web.tunaSaved||web.tunaChanging)){feedback('Enter your Tuna API key.');return;}
          web.tunaSaved=true;web.tunaChanging=false;web.tunaDraft='';web.tuna='starting';render();clearTimeout(tunaTimer);
          tunaTimer=setTimeout(()=>{web.tuna=value==='invalid'?'failed':'connected';if(web.tuna==='connected')web.selected='https://tuna-preview.example/';renewWeb();if(pane==='web')render();},800);
        }
        if(action==='renew-web'){renewWeb();render();}
        if(action==='open-web'){event.preventDefault();emit({type:'link',url:`${web.selected}#pair=preview-only-${web.generation%2?'b':'a'}`});}
        if(action==='copy-web')emit({type:'copy',text:`${web.selected}#pair=preview-only-${web.generation%2?'b':'a'}`});
      });
      screen.addEventListener('change',event=>{
        const input=event.target;
        if(input.dataset.preference)preferences[input.dataset.preference]=input.checked;
        if(input.dataset.application)preferences.applications[input.dataset.application]=input.checked;
        if(input.dataset.preference||input.dataset.application)notify();
        if(input.dataset.web==='running'){
          web.running=input.checked;clearTimeout(remoteTimer);clearTimeout(ngrokTimer);clearTimeout(tunaTimer);web.selected='http://192.0.2.10:8765/';
          web.remote=web.running?'starting':'off';
          if(web.running){
            renewWeb();
            remoteTimer=setTimeout(()=>{web.remote='connected';web.selected='https://monitor-preview.example/';renewWeb();if(pane==='web')render();},800);
          } else {web.deadline=0;web.ngrok='not_configured';web.tuna='not_configured';}
          if(web.running&&web.tokenSaved){web.ngrok='starting';ngrokTimer=setTimeout(()=>{web.ngrok='connected';web.selected='https://ngrok-preview.example/';renewWeb();if(pane==='web')render();},800);}
          if(web.running&&web.tunaSaved){web.tuna='starting';tunaTimer=setTimeout(()=>{web.tuna='connected';web.selected='https://tuna-preview.example/';renewWeb();if(pane==='web')render();},800);}
          render();
        }
        if(input.dataset.webAddress){web.selected=input.dataset.webAddress;render();}
      });
      screen.addEventListener('input',event=>{if(event.target.matches('.aam-bot-token input'))tokenDraft=event.target.value;if(event.target.matches('.aam-ngrok-token input')&&(!web.tokenSaved||web.tokenChanging)){web.tokenDraft=event.target.value;const connect=screen.querySelector('[data-setting-action="connect-ngrok"]');if(connect)connect.disabled=web.ngrok==='starting'||!web.tokenDraft.trim();}if(event.target.matches('.aam-tuna-token input')&&(!web.tunaSaved||web.tunaChanging)){web.tunaDraft=event.target.value;const connect=screen.querySelector('[data-setting-action="connect-tuna"]');if(connect)connect.disabled=web.tuna==='starting'||!web.tunaDraft.trim();}});
      screen.addEventListener('submit',event=>{
        event.preventDefault();event.stopPropagation();if(event.target.matches('.aam-tuna-token')){screen.querySelector('[data-setting-action="connect-tuna"]').click();return;}if(event.target.matches('.aam-ngrok-token')){screen.querySelector('[data-setting-action="connect-ngrok"]').click();return;}if(botStage==='checking')return;
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
        destroy(){destroyed=true;clearTimeout(timer);clearTimeout(agentTimer);clearTimeout(remoteTimer);clearTimeout(ngrokTimer);clearTimeout(tunaTimer);clearInterval(clock);screen.remove();},
      };
    },
  };
})();
