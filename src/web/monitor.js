/* Shared monitor UI. Host adapters provide snapshots and receive typed commands. */
(() => {
  'use strict';
  const paths = {
    github:'<path d="M9 19c-4.3 1.4-4.3-2.5-6-3m12 6v-3.87a3.37 3.37 0 0 0-.94-2.61c3.14-.35 6.44-1.54 6.44-7a5.44 5.44 0 0 0-1.5-3.78 5.07 5.07 0 0 0-.09-3.77S17.73.62 15 2.43a13.38 13.38 0 0 0-7 0C5.27.62 4.09.97 4.09.97A5.07 5.07 0 0 0 4 4.74a5.44 5.44 0 0 0-1.5 3.78c0 5.42 3.3 6.61 6.44 7A3.37 3.37 0 0 0 8 18.13V22"/>',
    'arrow-right':'<path d="M5 12h14m-6-6 6 6-6 6"/>',
    'arrow-up':'<path d="M12 19V5m-6 6 6-6 6 6"/>',
    'chevron-up':'<path d="m5 15 7-7 7 7"/>',
    'chevron-right':'<path d="m9 5 7 7-7 7"/>', 'chevron-down':'<path d="m5 9 7 7 7-7"/>',
    check:'<path d="m20 6-11 11-5-5"/>', x:'<path d="m6 6 12 12M6 18 18 6"/>',
    folder:'<path d="M20 20H4a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h4l2 3h10a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2Z"/>',
    branch:'<path d="M6 3v12m12-6a9 9 0 0 1-9 9"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="6" r="3"/>',
    cpu:'<rect x="4" y="4" width="16" height="16" rx="2"/><rect x="8" y="8" width="8" height="8"/><path d="M9 1v3m6-3v3M9 20v3m6-3v3M1 9h3m-3 6h3m16-6h3m-3 6h3"/>',
    terminal:'<path d="m4 6 4 4-4 4m8 4h8"/>', copy:'<rect x="8" y="8" width="13" height="13" rx="2"/><path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3"/>',
    message:'<path d="M21 15a2 2 0 0 1-2 2H7l-5 4V5a2 2 0 0 1 2-2h15a2 2 0 0 1 2 2Z"/>',
    sparkles:'<path d="m12 3 2.5 6.5L21 12l-6.5 2.5L12 21l-2.5-6.5L3 12l6.5-2.5ZM3 2v4M1 4h4"/>',
    power:'<path d="M12 2v10m-6.4-7.4a9 9 0 1 0 12.8 0"/>',
    settings:'<path d="m9 3 1-2h4l1 2 3 2 2 0 2 4-1 2v3l1 2-2 4-2 0-3 2-1 2h-4l-1-2-3-2H4l-2-4 1-2v-3L2 9l2-4h2Z"/><circle cx="12" cy="12" r="3"/>',
    pin:'<path d="M16 9V4l1-1H7l1 1v5l-3 3v2h6v7m2-7h6v-2l-3-3Z"/>',
    more:'<circle cx="5" cy="12" r="1" fill="currentColor"/><circle cx="12" cy="12" r="1" fill="currentColor"/><circle cx="19" cy="12" r="1" fill="currentColor"/>',
    info:'<circle cx="12" cy="12" r="9"/><path d="M12 11v6m0-10h.01"/>',
    'circle-alert':'<circle cx="12" cy="12" r="10"/><path d="M12 7v6m0 4h.01"/>',
    chart:'<rect x="2" y="13" width="3.5" height="8" rx="1"/><rect x="9" y="7" width="3.5" height="14" rx="1"/><path d="M16 11v10h4V11"/><circle cx="18" cy="3.5" r="2.2"/>'
  };
  const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  const icon = name => `<span class="aam-icon" aria-hidden="true"><svg viewBox="0 0 24 24">${paths[name] || paths.message}</svg></span>`;
  const image = url => /^data:image\/(png|jpeg|svg\+xml);base64,[A-Za-z0-9+/=]+$/.test(url || '') ? url : '';
  function safeHTML(html) {
    const template = document.createElement('template'); template.innerHTML = html || '';
    const allowed = new Set(['P','BR','STRONG','B','EM','I','CODE','PRE','UL','OL','LI','BLOCKQUOTE','H1','H2','H3','H4','HR','A','TABLE','THEAD','TBODY','TR','TH','TD','DEL','IMG']);
    for (const element of Array.from(template.content.querySelectorAll('*'))) {
      if (!allowed.has(element.tagName)) { element.replaceWith(document.createTextNode(element.textContent)); continue; }
      for (const attribute of Array.from(element.attributes)) if (!(element.tagName === 'A' && attribute.name === 'href' && /^(https?:|mailto:)/i.test(attribute.value)) && !(element.tagName === 'IMG' && attribute.name === 'src' && /^\/api\/image\/[A-Za-z0-9_-]+\/[A-Za-z0-9_-]+$/.test(attribute.value))) element.removeAttribute(attribute.name);
    }
    return template.innerHTML;
  }
  // Reconcile in place: persistent session/clip/editor nodes must survive snapshots
  // arriving during a disclosure transition or while the user is typing.
  const stableParts = new Set(['aam-row','aam-detail','aam-clip','aam-detail-content','aam-messages','aam-latest','aam-meta','aam-composer','aam-work-status']);
  function nodeKey(node) {
    if(node.nodeType!==Node.ELEMENT_NODE)return null;
    if(node.hasAttribute('data-render-key'))return node.getAttribute('data-render-key');
    if(node.hasAttribute('data-focus'))return 'focus:'+node.getAttribute('data-focus');
    return [...node.classList].find(name=>stableParts.has(name))||null;
  }
  function compatible(a,b) { return a.nodeType===b.nodeType&&a.nodeName===b.nodeName&&nodeKey(a)===nodeKey(b); }
  function patchNode(node,next) {
    if(node.nodeType!==Node.ELEMENT_NODE) {
      if(node.nodeValue!==next.nodeValue)node.nodeValue=next.nodeValue;
      return;
    }
    if(node.matches('.aam-composer')&&node.classList.contains('is-inline'))next.classList.add('is-inline');
    for(const attr of [...node.attributes]) {
      // These attributes belong to live layout/focus, not snapshot markup.
      if(attr.name==='data-initial-keyboard-focus'||(attr.name==='style'&&node.matches('.aam-session,.aam-row,.aam-compose-input,.aam-previous')))continue;
      if(attr.name==='hidden'&&node.matches('.aam-command-expand'))continue;
      if(!next.hasAttribute(attr.name))node.removeAttribute(attr.name);
    }
    for(const attr of next.attributes) {
      if(attr.name==='hidden'&&node.matches('.aam-command-expand'))continue;
      if(node.getAttribute(attr.name)!==attr.value)node.setAttribute(attr.name,attr.value);
    }
    // Input values, selections and IME composition are owned by the editor.
    if(!node.matches('input,textarea')&&!(node.matches('.aam-open-title')&&node.textContent===next.textContent))patchChildren(node,next);
  }
  function placeNode(parent,node,before) {
    if(node===before)return;
    if(typeof parent.moveBefore==='function'&&node.isConnected&&parent.isConnected){parent.moveBefore(node,before);return;}
    const focused=node.contains(document.activeElement)?document.activeElement:null;
    const selection=focused&&'selectionStart' in focused?[focused.selectionStart,focused.selectionEnd]:null;
    parent.insertBefore(node,before);
    if(focused&&document.activeElement!==focused){focused.focus({preventScroll:true});if(selection&&selection[0]!==null)try{focused.setSelectionRange(...selection);}catch{}}
  }
  function patchChildren(parent,next) {
    const remaining=new Set(parent.childNodes);
    let cursor=parent.firstChild;
    for(const child of next.childNodes) {
      const key=nodeKey(child);
      const match=cursor&&remaining.has(cursor)&&compatible(cursor,child)?cursor:
        [...remaining].find(node=>compatible(node,child)&&(key!==null||nodeKey(node)===null));
      const node=match||child.cloneNode(true);
      if(match){remaining.delete(match);patchNode(match,child);}
      if(node!==cursor)parent.insertBefore(node,cursor);
      cursor=node.nextSibling;
    }
    for(const node of remaining)node.remove();
  }
  function text(node,value) { if(node.textContent!==value)node.textContent=value; }
  const renderedHTML = new WeakMap();
  function patchHTML(node,html) {
    if(renderedHTML.get(node)===html)return;
    const template=document.createElement('template');template.innerHTML=html;
    patchChildren(node,template.content);renderedHTML.set(node,html);
  }
  // Rasterize a bounded two-tile period once. Resizing a document-wide filtered
  // SVG on every disclosure frame can invalidate a huge mobile GPU surface.
  function prepareBackdrop(product,ready) {
    const source=product.querySelector('.aam-mesh');
    const svg=source.cloneNode(true),ns='http://www.w3.org/2000/svg';
    svg.setAttribute('xmlns',ns);svg.setAttribute('width','460');svg.setAttribute('height','2000');svg.setAttribute('viewBox','0 0 460 2000');
    const tiles=svg.querySelector('.aam-mesh-tiles');
    tiles.replaceChildren(...[-1,0,1,2].map(index=>{
      const use=document.createElementNS(ns,'use');use.setAttribute('href','#aam-mesh-art-0');
      use.setAttribute('transform',Math.abs(index)%2?`translate(460 ${index*1000}) scale(-1 1)`:`translate(0 ${index*1000})`);return use;
    }));
    svg.querySelector(':scope > rect').setAttribute('height','2000');
    const filter=document.createElementNS(ns,'filter');filter.id='aam-raster-soft';
    filter.setAttribute('x','-20%');filter.setAttribute('y','-10%');filter.setAttribute('width','140%');filter.setAttribute('height','120%');
    const blur=document.createElementNS(ns,'feGaussianBlur');blur.setAttribute('stdDeviation','14');filter.append(blur);svg.querySelector('defs').append(filter);
    const layer=document.createElementNS(ns,'g');layer.setAttribute('filter','url(#aam-raster-soft)');
    for(const child of [...svg.children])if(child.tagName!=='defs')layer.append(child);
    svg.append(layer);
    const image=new Image();
    image.onload=()=>{
      if(!source.isConnected)return;
      try {
        const canvas=document.createElement('canvas');canvas.width=460;canvas.height=2000;
        const context=canvas.getContext('2d');if(!context)return;
        context.drawImage(image,0,0);
        const raster=document.createElement('div');raster.className='aam-mesh-raster';raster.setAttribute('aria-hidden','true');
        raster.style.backgroundImage=`url("${canvas.toDataURL('image/png')}")`;
        source.replaceWith(raster);ready();
      } catch { /* Keep the original artwork if this browser cannot rasterize SVG. */ }
    };
    image.src='data:image/svg+xml;charset=utf-8,'+encodeURIComponent(new XMLSerializer().serializeToString(svg));
  }
  function mount(root, send, options = {}) {
    root.classList.add('aam-inset-composer');
    root.classList.toggle('aam-preview-lighter-quota',options.live!==true&&new URLSearchParams(location.search).get('quota')==='light');
    root.classList.toggle('aam-preview-compact-hovers',options.live!==true&&new URLSearchParams(location.search).get('hovers')==='compact');
    root.innerHTML = '<div class="aam-stage"><div class="aam-product"></div></div>';
    const product = root.querySelector('.aam-product');
    product.append(document.getElementById('aam-shell').content.cloneNode(true)); product.tabIndex = -1;
    if(options.live!==true&&new URLSearchParams(location.search).get('pin')==='true'){
      const pin=document.createElement('button');pin.type='button';pin.className='aam-preview-unpin';pin.dataset.command='preview-pin';pin.setAttribute('aria-label','Detach window from menu bar');pin.title='Detach window from menu bar';pin.innerHTML=icon('pin');product.querySelector('.aam-brand').prepend(pin);
    }
    product.querySelector('[data-command="github"]').innerHTML = icon('github');
    product.querySelector('[data-command="settings"]').innerHTML = icon('settings');
    product.querySelector('[data-command="quit"]').innerHTML = icon('power');
    const list = product.querySelector('.aam-list'), groups = product.querySelector('.aam-session-groups');
    const expanded = new Set(), contexts = new Set(), histories = new Set(), signatures = new Map();
    const titleHighlights = new WeakMap();
    const historySignatures = new Map(), drafts = new Map(), composerSizes = new WeakMap(), threadMasks = new WeakMap();
    let snapshot = {sessions:[],usage:[],version:'',loading:true}, query = '', counter = 0, composing = false, resetsExpanded = false, resetsPendingGeneration = null;
    let layoutPending = false, destroyed = false, pointerY = null;
    const emit = message => send(message);
    const settings = options.settings === false ? {active:()=>false,update:()=>{},open:()=>{},close:()=>{},destroy:()=>{}} : window.AAMSettings.mount(product, emit, scheduleLayout);
    if(options.settings === false) product.querySelector('[data-command="settings"]').hidden = true;
    const sessionFor = node => snapshot.sessions.find(session => session.id === node.closest('.aam-session')?.dataset.session);
    const words = () => query.toLowerCase().split(/\s+/u).filter(Boolean);
    const matches = session => words().every(word => session.title.toLowerCase().includes(word));
    function status(session) {
      if (session.status === 'idle' || session.status === 'working') return '';
      const label = {working:'Working',finished:'Finished (unseen)',request:'Needs response',unknown:'Unknown status'}[session.status] || 'Unknown status';
      let art = '<circle cx="12" cy="12" r="10"/><path class="aam-cutout" d="M11 5h2v9h-2Zm0 11h2v3h-2Z"/>';
      if (session.status === 'finished') art = '<circle cx="12" cy="12" r="10"/><path class="aam-cutout" d="m6 12 2-2 3 3 6-6 2 2-8 8Z"/>';
      return `<${session.status === 'finished' ? 'button type="button" data-command="seen"' : 'span'} class="aam-state" data-status="${esc(session.status)}" aria-label="${label}" title="${session.status === 'finished' ? 'Mark as seen' : label}"><span class="aam-icon"><svg viewBox="0 0 24 24">${art}</svg></span></${session.status === 'finished' ? 'button' : 'span'}>`;
    }
    function commandBlock(text, key, caption) {
      if (!text) return '';
      const id = `aam-code-${++counter}`, isExpanded = contexts.has(key);
      const label = isExpanded ? 'Hide command' : 'Show command';
      return `<div class="aam-command-heading"><button type="button" class="aam-action aam-command-expand aam-command-disclosure" data-context="${esc(key)}" tabindex="-1" aria-controls="${id}" aria-expanded="${isExpanded}" aria-label="${label}"><span class="aam-command-label">${esc(caption)}</span><span class="aam-command-chevron">${icon('chevron-down')}</span></button></div><div class="aam-command-wrap" ${isExpanded ? '' : 'hidden'}><pre id="${id}" class="aam-command ${isExpanded ? 'is-expanded' : ''}"><code>${esc(text)}</code></pre></div>`;
    }
    function requestMarkup(session) {
      const request = session.request; if (!request) return '';
      const disabled = !request.enabled, busy = request.phase === 'sending' || request.phase === 'reconciling';
      const off = disabled || busy ? 'disabled' : '';
      let controls = '';
      if (request.questions?.some(question => !question.freeform && !question.choices.length)) {
        return `<div class="aam-question"><p class="aam-question-title">${esc(request.body)}</p><button type="button" class="aam-action" data-command="open">Answer in editor…</button></div>`;
      }
      if (request.questions?.length) {
        const single = request.questions.length === 1;
        controls += `<form class="aam-question-form ${single ? 'aam-single' : ''}" data-context="${esc(request.context)}" data-enabled="${!disabled && !busy}">${request.questions.map((q,index) => {
          const own = `<input type="${q.secret ? 'password' : 'text'}" class="aam-custom" aria-label="Your own answer" placeholder="Your own answer" data-question="${esc(q.id)}" data-focus="custom:${esc(q.id)}" autocomplete="off" ${off}>`;
          const choices = q.choices.map((choice,i) => single
            ? `<button type="button" class="aam-action" data-preset="${esc(choice.label)}" data-question="${esc(q.id)}" data-focus="choice:${index}:${i}" ${off}>${esc(choice.label)}</button>${choice.description ? `<p class="aam-answer-description">${esc(choice.description)}</p>` : ''}`
            : `<label class="aam-answer"><input type="radio" name="q-${index}-${esc(session.id)}" value="${esc(choice.label)}" data-focus="choice:${index}:${i}" ${i === 0 ? 'checked' : ''} ${off}><span class="aam-answer-text">${esc(choice.label)}</span></label>${choice.description ? `<p class="aam-answer-description">${esc(choice.description)}</p>` : ''}`).join('');
          return `<fieldset data-question="${esc(q.id)}"><legend class="aam-question-title">${esc(q.title)}</legend>${choices}${q.freeform ? `<div class="${single ? 'aam-single-own' : ''}"><label class="aam-answer"><input type="radio" name="q-${index}-${esc(session.id)}" value="custom" data-own="true" tabindex="-1" data-focus="own:${index}" ${single ? 'hidden' : ''} ${off}>${own}</label>${single ? `<button type="submit" class="aam-action aam-primary" data-focus="submit" ${off}>Send answer</button>` : ''}</div>` : ''}</fieldset>`;
        }).join('')}${single ? '' : `<button type="submit" class="aam-action aam-primary aam-send" data-focus="submit" ${off}>Send answer</button>`}</form>`;
      } else controls += `<p class="aam-question-title">${esc(request.body)}</p>`;
      const commandAction = request.command ? (request.actions || []).find(action => !action.detail && (action.primary || /^allow once$/i.test(action.label))) : null;
      if(!commandAction) controls += commandBlock(request.command, `${session.id}:${request.context}:command`, 'Command');
      const immediate = (request.actions || []).filter(action => !action.detail), scoped = (request.actions || []).filter(action => action.detail);
      const button = action => `<button type="button" class="aam-action ${action.primary ? 'aam-primary' : ''}" data-response="${esc(action.id)}" data-focus="action:${esc(action.id)}" ${off}>${action.label === 'Allow similar' ? '' : icon(action.id === 'deny' || /deny|decline/i.test(action.label) ? 'x' : 'check')}${esc(action.label)}</button>`;
      const scopedButton = action => {
        const key=`${session.id}:${request.context}:${action.id}:scope`, open=contexts.has(key), id=`aam-scope-${++counter}`;
        return `<div class="aam-action-detail"><div class="aam-scope-row aam-action ${action.primary?'aam-primary':''} aam-command-split">${button({...action,primary:false})}<button type="button" class="aam-scope-toggle" data-scope="${esc(key)}" tabindex="-1" aria-controls="${id}" aria-expanded="${open}" aria-label="${open?'Hide':'Show'} command scope">${icon('chevron-down')}</button></div><pre id="${id}" class="aam-command is-expanded" ${open?'':'hidden'}><code>${esc(action.detail)}</code></pre></div>`;
      };
      const commandButton = action => {
        const key=`${session.id}:${request.context}:command`, open=contexts.has(key), id=`aam-code-${++counter}`;
        return `<div class="aam-action-detail"><div class="aam-scope-row aam-action ${action.primary?'aam-primary':''} aam-command-split">${button({...action,primary:false})}<button type="button" class="aam-command-expand" data-context="${esc(key)}" tabindex="-1" aria-controls="${id}" aria-expanded="${open}" aria-label="${open?'Hide':'Show'} command">${icon('chevron-down')}</button></div><div class="aam-command-wrap" ${open?'':'hidden'}><pre id="${id}" class="aam-command ${open?'is-expanded':''}"><code>${esc(request.command)}</code></pre></div></div>`;
      };
      const ordered=root.classList.contains('aam-preview-compact-hovers')?(request.actions||[]):[...immediate,...scoped];
      controls += `<div class="aam-actions">${ordered.map(action=>action===commandAction?commandButton(action):action.detail?scopedButton(action):button(action)).join('')}</div>`;
      if (!request.actions?.length && !request.questions?.length) controls += '<button type="button" class="aam-action" data-command="open">Answer in editor…</button>';
      const note = busy ? (request.phase === 'sending' ? 'Sending…' : 'Checking delivery…') : request.error || (!request.enabled ? 'Waiting for a live connection to the agent.' : '');
      return `<div class="aam-question ${request.questions?.length ? '' : 'aam-approval'}" data-render-key="request:${esc(request.context)}"><span class="aam-request-mark" aria-hidden="true">${icon('circle-alert')}</span>${controls}${note ? `<p class="aam-status-note" role="status">${esc(note)}</p>` : ''}</div>`;
    }
    function progressMarkup(session) {
      const text=Array.from(typeof session.progress==='string'&&session.progress.trim()?session.progress:'Working…').slice(0,512).join('');
      return session.status==='working'&&!session.request&&text ? `<p class="aam-work-status" title="${esc(text)}"><span class="aam-work-glow" aria-hidden="true">${esc(text)}</span><span class="aam-work-text">${esc(text)}</span></p>` : '';
    }
    function composerMarkup(session) {
      if (session.request || !['idle','finished'].includes(session.status)) return '';
      const text=drafts.get(session.id)||'', enabled=session.authoritative!==false;
      return `<form class="aam-composer${text.trim()?' has-draft':''}"><textarea class="aam-compose-input" rows="1" aria-label="Message agent" placeholder="Message agent…" data-focus="compose" autocomplete="off" ${enabled?'':'disabled'}>${esc(text)}</textarea><button type="submit" class="aam-action" data-focus="compose-send" aria-label="Send message" title="Send message" ${enabled&&text.trim()&&Array.from(text).length<=16000?'':'disabled'}>${icon('arrow-up')}</button></form>`;
    }
    function sessionMarkup(session) {
      const history = session.history || {}, messages = histories.has(session.id) ? history.messages || [] : [];
      const metadata = [[icon('folder'),session.directory,session.fullDirectory || session.directory],[icon('branch'),session.branch,session.branch],[icon('cpu'),session.model,session.model]];
      return `<article class="aam-session ${expanded.has(session.id) ? 'is-expanded' : ''}" data-session="${esc(session.id)}" data-status="${esc(session.status)}"><div class="aam-row"><span class="aam-group-marker"><button type="button" class="aam-robot-open" data-command="toggle" aria-label="Collapse / expand session" aria-expanded="${expanded.has(session.id)}"><span class="aam-session-mark" aria-hidden="true"></span></button></span><button type="button" class="aam-open" data-command="open" data-focus="title" title="Switch to ${esc(session.app)}"><span class="aam-open-title">${esc(session.title)}</span><span class="aam-open-destination">${icon('arrow-right')}<img src="${image(session.icon)}" width="16" height="16" alt=""></span></button><span class="aam-state-slot">${status(session)}</span><button type="button" class="aam-toggle" data-command="toggle" aria-label="Collapse / expand session" aria-expanded="${expanded.has(session.id)}">${icon('chevron-right')}</button></div><div class="aam-detail"><div class="aam-clip"><div class="aam-detail-content"><div class="aam-messages ${messages.length ? 'has-history' : ''}">${messages.map(message => `<div class="aam-message"><span class="aam-message-dot" aria-hidden="true"></span>${safeHTML(message.html)}</div>`).join('')}<div class="aam-latest aam-message"><span class="aam-message-dot" aria-hidden="true"></span>${session.html ? safeHTML(session.html) : '<p>No messages yet</p>'}</div><button type="button" class="aam-previous" data-command="history" aria-label="Previous message" title="Previous message" ${history.loading || (history.exhausted && (histories.has(session.id) || !history.messages?.length)) ? 'disabled' : ''}>${icon('chevron-up')}</button>${history.loading ? '<p class="aam-status-note">Loading previous message…</p>' : ''}${history.error ? `<p class="aam-status-note" role="status">${esc(history.error)}</p>` : ''}</div>${progressMarkup(session)}${requestMarkup(session)}${composerMarkup(session)}<div class="aam-meta">${metadata.filter(([,value]) => value).map(([art,value,hint]) => `<div class="aam-meta-line">${art}<span title="${esc(hint)}">${esc(value)}</span></div>`).join('')}</div></div></div></div></article>`;
    }
    function updateFormValidity(form) {
      const button=form.querySelector('button[type="submit"]');if(!button)return;
      const valid=Array.from(form.querySelectorAll('fieldset')).every(fieldset=>{
        const selected=fieldset.querySelector('input[type="radio"]:checked');
        return selected && (!selected.dataset.own || !!fieldset.querySelector('.aam-custom')?.value.trim());
      });
      button.disabled=form.dataset.enabled!=='true'||!valid;
    }
    function preserve(node) {
      if (!node) return null;
      return {context:node.querySelector('.aam-question-form')?.dataset.context, focus:node.contains(document.activeElement) ? document.activeElement.dataset.focus : null,
        inputs:Array.from(node.querySelectorAll('.aam-question-form input, .aam-compose-input')).map(input => ({key:input.dataset.focus,value:input.value,checked:input.checked,start:input.selectionStart,end:input.selectionEnd}))};
    }
    function restore(node, saved) {
      if (!saved || saved.context !== node.querySelector('.aam-question-form')?.dataset.context) return;
      for (const input of node.querySelectorAll('.aam-question-form input, .aam-compose-input')) {
        const old = saved.inputs.find(item => item.key === input.dataset.focus); if (!old) continue;
        if(input.value!==old.value)input.value=old.value; if(input.checked!==old.checked)input.checked=old.checked;
        if (old.start !== null && (input.selectionStart!==old.start||input.selectionEnd!==old.end)) try { input.setSelectionRange(old.start,old.end); } catch {}
      }
      const focus = Array.from(node.querySelectorAll('[data-focus]')).find(item => item.dataset.focus === saved.focus);
      if (focus && !focus.disabled && document.activeElement!==focus) focus.focus({preventScroll:true});
    }
    function renderUsageResets(){
      const quota=product.querySelector('.aam-usage');let summary=snapshot.usageResets;
      if(!quota||!summary)return;
      if(resetsPendingGeneration!==null){
        if((summary.generation||0)>=resetsPendingGeneration||summary.phase==='error')resetsPendingGeneration=null;
        else summary={...summary,phase:'loading',credits:null,availableCount:null};
      }
      quota.classList.add('aam-usage-resets');
      let panel=quota.querySelector('.aam-reset-list');
      if(!panel){panel=document.createElement('section');panel.id='aam-reset-list';panel.className='aam-reset-list';panel.setAttribute('aria-label','Usage limit resets');quota.prepend(panel);}
      let toggle=quota.querySelector('[data-command="reset-credits"]');
      if(!toggle){toggle=document.createElement('button');toggle.type='button';toggle.className='aam-resets-toggle';toggle.dataset.command='reset-credits';toggle.setAttribute('aria-controls',panel.id);quota.querySelector('.aam-usage-date').append(toggle);}
      toggle.setAttribute('aria-expanded',String(resetsExpanded));toggle.setAttribute('aria-label',resetsExpanded?'Hide usage limit resets':'Show usage limit resets');toggle.title=resetsExpanded?'Hide usage limit resets':'Show usage limit resets';toggle.disabled=snapshot.usageResetsEnabled===false;
      patchHTML(toggle,icon('more'));
      panel.hidden=!resetsExpanded;
      const loading=summary.phase==='idle'||summary.phase==='loading';panel.setAttribute('aria-busy',String(loading));
      const rows=Array.isArray(summary.credits)?summary.credits:[];
      const date=value=>new Date(value*1000).toLocaleString('en-GB',{day:'numeric',month:'short',year:'numeric',hour:'2-digit',minute:'2-digit'});
      patchHTML(panel,summary.phase==='error'?`<p role="alert">${esc(summary.error||'Could not load usage limit resets. Close and reopen to retry.')}</p>`:loading?'<div class="aam-resets-loading" role="status"><span class="aam-resets-spinner" aria-hidden="true"></span><span>Loading usage limit resets…</span></div>':`<h3><span class="aam-reset-heading">Usage limit resets${Number.isInteger(summary.availableCount)?` <span class="aam-reset-count">${summary.availableCount} available</span>`:''}</span><span>Expires</span></h3>${rows.length?rows.map(credit=>`<div class="aam-reset-row"><span>${esc(credit.title||'Usage reset')}</span><span class="aam-reset-expiry">${credit.expiresAt==null?'No expiration':esc(date(credit.expiresAt))}</span></div>`).join(''):`<p>${summary.availableCount===0?'No resets available.':'Expiration details unavailable.'}</p>`}`);
    }
    function quotaStyle(remaining){
      if(remaining===null||!Number.isFinite(remaining))return '';
      remaining=Math.max(0,Math.min(100,remaining));
      const stops=[[0,[255,48,61]],[30,[255,126,52]],[40,[246,198,58]],[50,[146,180,221]],[60,[130,177,230]],[100,[68,143,247]]];
      const upper=stops.findIndex(([percent])=>remaining<=percent);
      const [start,from]=stops[Math.max(0,upper-1)],[end,to]=stops[Math.max(1,upper)];
      const fraction=(remaining-start)/(end-start);
      const color=from.map((channel,index)=>Math.round(channel+(to[index]-channel)*fraction)).join(' ');
      return `--aam-quota-color:${color};--aam-quota-text-color:${remaining<=50?`rgb(${color})`:'light-dark(#776385,#c0afd8)'};--aam-quota-text-weight:${remaining<=40?'700':'400'}`;
    }
    function setSnapshot(value) {
      if (!value || !Array.isArray(value.sessions)) return;
      const project = session => {
        const activities = [...(session.activities || [])].sort((a,b) => a.startedAt.localeCompare(b.startedAt) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
        const message = activities.findLast(a => a.type === 'message' && a.role === 'agent');
        const request = activities.find(a => a.type === 'request' && a.status === 'pending');
        const action = activities.findLast(a => a.type === 'action' && a.current);
        return {...session, activities, html:message?.html || (message?.text ? `<p>${esc(message.text)}</p>` : ''), request:request?.request || null, progress:action?.text || null};
      };
      snapshot = {...value,sessions:value.sessions.map(project)};
      const update=product.querySelector('.aam-update-link');
      const release=value.versions?.available;
      const releaseUrl=typeof release?.url==='string'&&/^https:\/\/github\.com\/melsomino\/ai-agent-monitor\/releases\/tag\/[^/?#]+$/.test(release.url);
      const currentVersion=product.querySelector('.aam-version');
      currentVersion.hidden=!!releaseUrl;
      update.hidden=!releaseUrl;
      if(releaseUrl){text(update,`${release.version} available`);update.href=release.url;update.setAttribute('aria-label',`Update available: ${release.version}. Open release on GitHub`);}
      else{const checked=!release&&value.versions?.checked_at!=null;text(currentVersion,`${snapshot.versions?.current||snapshot.version||''}${checked?' ✓':''}`);currentVersion.title=checked?'Up to date':'Not checked yet';text(update,'');update.removeAttribute('href');update.removeAttribute('aria-label');}
      settings.update(value);
      root.style.colorScheme = snapshot.theme || 'dark'; root.dataset.theme = snapshot.theme || 'dark';
      product.classList.toggle('reduce-motion',snapshot.animate === false);
      const ids = new Set(snapshot.sessions.map(session => session.id));
      for (const node of groups.querySelectorAll('.aam-session')) if (!ids.has(node.dataset.session)) { signatures.delete(node.dataset.session); expanded.delete(node.dataset.session); drafts.delete(node.dataset.session); node.remove(); }
      const nodes=new Map([...groups.querySelectorAll('.aam-session')].map(node=>[node.dataset.session,node]));
      let cursor=groups.firstElementChild;
      for (const session of snapshot.sessions) {
        if (session.expanded && !signatures.has(session.id)) expanded.add(session.id);
        if (session.request && !historySignatures.has(session.id + ':' + session.request.context)) { expanded.add(session.id); historySignatures.set(session.id + ':' + session.request.context,true); }
        let node=nodes.get(session.id);
        const signature=JSON.stringify({...session,activities:undefined,expanded:undefined,completion:undefined});
        if(signatures.get(session.id)===signature&&node){placeNode(groups,node,cursor);cursor=node.nextElementSibling;continue;}
        const saved=preserve(node),template=document.createElement('template');template.innerHTML=sessionMarkup(session);
        const fresh=template.content.firstElementChild;
        if(node)patchNode(node,fresh);else node=fresh;
        placeNode(groups,node,cursor);cursor=node.nextElementSibling;
        restore(node,saved);signatures.set(session.id,signature);
      }
      const usage=product.querySelector('.aam-usage-windows');
      const resetInventory=snapshot.usageResets;
      const quotaWindows=snapshot.usage?.length?snapshot.usage:resetInventory?[{label:'usage',remaining:null,reset:'—'}]:[];
      const resetPanel=resetInventory?usage.querySelector('.aam-reset-list'):null;
      const resetToggle=resetInventory?usage.querySelector('[data-command="reset-credits"]'):null;
      patchHTML(usage,quotaWindows.map((window,index) => `<div class="aam-usage" style="${quotaStyle(window.remaining)}">${index===0&&resetInventory?resetPanel?.outerHTML||'':''}<div class="aam-usage-main"><span class="aam-usage-name">Codex ${esc(window.label)}</span><div class="aam-usage-track" role="progressbar" aria-label="${esc(window.label)} remaining" ${window.remaining === null ? '' : `aria-valuenow="${window.remaining}"`} aria-valuemin="0" aria-valuemax="100"><span style="width:${Math.max(0,Math.min(100,window.remaining || 0))}%"></span></div><span class="aam-usage-percent">${window.remaining === null ? '—' : esc(window.remaining)+'%'}</span></div><span class="aam-usage-date">${esc(window.reset || '')}${index===0&&resetInventory?resetToggle?.outerHTML||'':''}</span></div>`).join('') || '<p class="aam-status-note">Codex usage unavailable</p>');
      renderUsageResets();
      const note=product.querySelector('.aam-list > .aam-status-note');text(note,snapshot.error || '');note.hidden=!snapshot.error;
      product.querySelectorAll('.aam-question-form').forEach(updateFormValidity);
      applyFilter(false); scheduleLayout();
      if (snapshot.focusSession && !settings.active()) reveal(snapshot.focusSession,true);
    }
    function highlight(element,title) {
      const key=JSON.stringify([title,query]);if(titleHighlights.get(element)===key)return;titleHighlights.set(element,key);
      let folded='', position=0; const offsets=[], ranges=[];
      for (const character of title) { const lower=character.toLowerCase(); for(let i=0;i<lower.length;i++) offsets.push([position,position+character.length]);folded+=lower;position+=character.length; }
      for(const word of words()) for(let at=folded.indexOf(word);at!==-1;at=folded.indexOf(word,at+1)) ranges.push([offsets[at][0],offsets[at+word.length-1][1]]);
      ranges.sort((a,b)=>a[0]-b[0]||a[1]-b[1]);const merged=[];
      for(const range of ranges) {const last=merged.at(-1);if(last&&range[0]<=last[1])last[1]=Math.max(last[1],range[1]);else merged.push([...range]);}
      const fragment=document.createDocumentFragment();position=0;
      for(const [start,end] of merged) {fragment.append(document.createTextNode(title.slice(position,start)));const mark=document.createElement('mark');mark.textContent=title.slice(start,end);fragment.append(mark);position=end;}
      fragment.append(document.createTextNode(title.slice(position)));element.replaceChildren(fragment);
    }
    function keyboardFocus(element) {
      if(!element)return; element.setAttribute('data-initial-keyboard-focus','');
      element.addEventListener('blur',()=>element.removeAttribute('data-initial-keyboard-focus'),{once:true});
      element.focus({preventScroll:true,focusVisible:true});
    }
    function applyFilter(moveFocus=true) {
      let count=0;const counts={waiting:0,working:0,finished:0,idle:0,unknown:0};
      for(const article of groups.querySelectorAll('.aam-session')) {
        const session=snapshot.sessions.find(session=>session.id===article.dataset.session);
        article.hidden=!matches(session);article.classList.remove('aam-session-first');
        highlight(article.querySelector('.aam-open-title'),session.title);
        if(!article.hidden) {count++;if(count===1)article.classList.add('aam-session-first');const state=session.request||session.status==='request'?'waiting':session.status;counts[Object.hasOwn(counts,state)?state:'unknown']++;}
      }
      const status=product.querySelector('.aam-filter-status');text(status,query?`Filter: ${query}`:'');status.hidden=!query;
      const empty=product.querySelector('.aam-filter-empty');empty.hidden=!!count;text(empty,snapshot.loading?'Loading sessions…':query.trim()?'No matching sessions':'No open sessions');
      const overview=product.querySelector('.aam-overview');
      patchHTML(overview,[['waiting','waiting for you'],['working','working'],['finished','finished'],['idle','idle']].filter(([state])=>counts[state]>0).map(([state,label])=>`<span class="aam-overview-count" data-state="${state}">${counts[state]} ${label}</span>`).join(' · '));
      overview.title=overview.textContent+(counts.unknown?`\nStatus unavailable: ${counts.unknown}`:'');
      if(document.activeElement.closest('.aam-session[hidden]'))document.activeElement.blur();
      if(moveFocus && (!product.contains(document.activeElement)||document.activeElement===product))keyboardFocus(product.querySelector('.aam-session:not([hidden]) .aam-open')||product);
      scheduleLayout();
    }
    function reveal(id,answer=false) {
      const node=Array.from(groups.querySelectorAll('.aam-session')).find(node=>node.dataset.session===id);if(!node)return;
      query='';expanded.add(id);node.classList.add('is-expanded');node.querySelectorAll('[data-command="toggle"]').forEach(button=>button.setAttribute('aria-expanded','true'));applyFilter(false);
      requestAnimationFrame(()=>{const target=answer?node.querySelector('.aam-action:not(:disabled), .aam-answer input:not([data-own]):not(:disabled)'):null;keyboardFocus(target||node.querySelector('.aam-open'));(target||node).scrollIntoView({block:'nearest'});});
    }
    function scheduleLayout() {
      if(layoutPending)return;layoutPending=true;
      let ran=false,timer;
      const run=()=>{if(ran)return;ran=true;clearTimeout(timer);layoutPending=false;if(!destroyed)layout();};
      // Batch visible work once per frame, with a fallback for occluded WebKit views.
      if(document.hidden)queueMicrotask(run);else {requestAnimationFrame(run);timer=setTimeout(run,50);}
    }
    function layout() {
      if (settings.active()) {
        const screen=product.querySelector('.aam-settings-screen'),backdrop=product.querySelector('.aam-backdrop');
        backdrop.style.height=Math.max(screen.scrollHeight,screen.clientHeight)+'px';
        backdrop.style.transform='translate3d(0,0,0)';
        emit({type:'size',height:screen.scrollHeight,ready:true});return;
      }
      for(const article of groups.querySelectorAll('.aam-session:not([hidden])')) {
        const row=article.querySelector('.aam-row'), title=row.querySelector('.aam-open'), box=row.getBoundingClientRect();
        row.style.setProperty('--aam-title-highlight-width',Math.max(0,title.getBoundingClientRect().right-box.left-29+12)+'px');
        article.style.setProperty('--aam-header-height',box.height+'px');
        const articleBox=article.getBoundingClientRect(),lineTop=articleBox.top+box.height/2+5.8;
        const lineX=articleBox.left+parseFloat(getComputedStyle(product.closest('.aam-stage')).getPropertyValue('--aam-inset'))+5.5;
        const marks=Array.from(article.querySelectorAll('.aam-message-dot,.aam-request-mark'));
        const rounded=value=>Math.round(value*100)/100;
        const maskKey=JSON.stringify([box.height,lineX-articleBox.left,...marks.map(mark=>{const b=mark.getBoundingClientRect();return [rounded(b.left-articleBox.left),rounded(b.top-articleBox.top),b.width,b.height,getComputedStyle(mark).display];})]);
        if(threadMasks.get(article)===maskKey)continue;
        threadMasks.set(article,maskKey);
        const cuts=marks
          .filter(mark=>mark.getClientRects().length&&getComputedStyle(mark).display!=='none')
          .map(mark=>{
            if(mark.classList.contains('aam-message-dot')){const bounds=mark.getBoundingClientRect();return {top:bounds.top+.1,bottom:bounds.bottom-.1};}
            const svg=mark.querySelector('svg');if(!svg)return null;
            const bounds=svg.getBoundingClientRect();
            const shapes=Array.from(svg.querySelectorAll('path,circle,rect,line,polyline,polygon,ellipse')).map(shape=>({shape,matrix:shape.getScreenCTM()?.inverse(),filled:getComputedStyle(shape).fill!=='none'})).filter(item=>item.matrix&&typeof item.shape.isPointInStroke==='function');
            let top=null,bottom=null;
            for(let y=bounds.top;y<=bounds.bottom;y+=.05){const point=new DOMPoint(lineX,y);if(shapes.some(({shape,matrix,filled})=>{const local=point.matrixTransform(matrix);return shape.isPointInStroke(local)||(filled&&shape.isPointInFill(local));})){if(top===null)top=y;bottom=y;}}
            return top===null?null:{top:top+.1,bottom:bottom-.1};
          }).filter(cut=>cut&&cut.bottom>lineTop).sort((a,b)=>a.top-b.top);
        const stops=['#000 0px'];for(const cut of cuts){const start=Math.max(0,cut.top-lineTop),end=Math.max(start,cut.bottom-lineTop);stops.push(`#000 ${start}px`,`transparent ${start}px`,`transparent ${end}px`,`#000 ${end}px`);}stops.push('#000 100%');article.style.setProperty('--aam-thread-mask',`linear-gradient(to bottom,${stops.join(',')})`);

      }
      for(const input of product.querySelectorAll('.aam-compose-input')) {if(input.getClientRects().length)resizeComposer(input);}
      for(const button of product.querySelectorAll('.aam-command-expand')) button.hidden=false;
      const backdrop=product.querySelector('.aam-backdrop'),footer=product.querySelector('.aam-footer');
      product.style.setProperty('--aam-footer-height',footer.offsetHeight+'px');
      const height=list.scrollHeight;backdrop.style.height=height+'px';
      const mesh=backdrop.querySelector('.aam-mesh');
      if(mesh){
        const logical=height*460/Math.max(backdrop.clientWidth,1);mesh.setAttribute('viewBox',`0 0 460 ${logical}`);mesh.querySelector(':scope > rect').setAttribute('height',logical);
        const tiles=mesh.querySelector('.aam-mesh-tiles'), count=Math.max(1,Math.ceil(logical/1000));
        if(tiles.children.length!==count)tiles.replaceChildren(...Array.from({length:count},(_,index)=>{const use=document.createElementNS('http://www.w3.org/2000/svg','use');use.setAttribute('href','#aam-mesh-art-0');use.setAttribute('transform',index%2?`translate(460 ${index*1000}) scale(-1 1)`:`translate(0 ${index*1000})`);return use;}));
      }
      backdrop.style.transform=`translate3d(0,${-list.scrollTop}px,0)`;
      const natural=Math.min(1200,Math.ceil(list.scrollHeight));
      emit({type:'size',height:natural,ready:snapshot.loading!==true});
      previousHover();
    }
    function resizeComposer(input) {
      const width=input.getBoundingClientRect().width;
      let style=getComputedStyle(input);
      const sizeKey=()=>JSON.stringify([width,input.value,style.font,style.lineHeight,style.padding,style.borderWidth,style.boxSizing,style.letterSpacing,style.wordSpacing]);
      let key=sizeKey();const cached=composerSizes.get(input);if(cached?.key===key)return false;
      {
        const form=input.closest('.aam-composer');
        let inline=false;
        if(input.value.trim()&&!/[\r\n]/.test(input.value)){
          const text=document.createElement('span');text.textContent=input.value;
          Object.assign(text.style,{position:'absolute',visibility:'hidden',pointerEvents:'none',whiteSpace:'pre',font:style.font,letterSpacing:style.letterSpacing,wordSpacing:style.wordSpacing});
          form.append(text);inline=text.getBoundingClientRect().width<=width-58;text.remove();
        }
        form.classList.toggle('is-inline',inline);style=getComputedStyle(input);key=sizeKey();
      }
      // Measure off-flow: collapsing the live field even momentarily can move the
      // scroll anchor or clamp scrollTop during the disclosure's ResizeObserver ticks.
      const measure=input.cloneNode(false);measure.removeAttribute('id');measure.removeAttribute('data-focus');
      measure.setAttribute('aria-hidden','true');measure.tabIndex=-1;measure.value=input.value;
      Object.assign(measure.style,{position:'absolute',visibility:'hidden',pointerEvents:'none',height:'0px',minHeight:'0px',width:width+'px',flex:'none'});
      input.parentElement.append(measure);
      const height=Math.max(32,measure.scrollHeight);measure.remove();
      const changed=input.style.height!==height+'px';if(changed)input.style.height=height+'px';
      composerSizes.set(input,{key});return changed;
    }
    function previousHover(){
      for(const messages of product.querySelectorAll('.aam-messages')){
        const first=messages.querySelector('.aam-message'),box=first.getBoundingClientRect(),parent=messages.getBoundingClientRect(),button=messages.querySelector('.aam-previous');
        button.style.top=(box.top-parent.top)+'px';button.style.height=box.height+'px';
        messages.classList.toggle('is-first-message-hovered',pointerY!==null&&pointerY>=box.top&&pointerY<=box.bottom);
      }
    }

    const resize=new ResizeObserver(scheduleLayout);resize.observe(list);resize.observe(product.querySelector('.aam-footer'));resize.observe(groups);
    list.addEventListener('scroll',()=>{product.querySelector('.aam-backdrop').style.transform=`translate3d(0,${-list.scrollTop}px,0)`;previousHover();},{passive:true});
    product.addEventListener('pointermove',event=>{pointerY=event.clientY;previousHover();},{passive:true});product.addEventListener('pointerleave',()=>{pointerY=null;previousHover();});
    product.addEventListener('pointerdown',event=>{product.querySelectorAll('[data-initial-keyboard-focus]').forEach(node=>node.removeAttribute('data-initial-keyboard-focus'));if(event.target.closest('.aam-command-expand,.aam-scope-toggle'))event.preventDefault();});
    function seen(session){if(session?.status==='finished')emit({type:'seen',session:session.id,generation:session.completion});}
    product.addEventListener('click',event=>{
      if(event.target.closest('.aam-settings-screen'))return;
      if(event.target.closest('[data-command="preview-pin"]')){emit({type:'preview-pin'});return;}
      if(event.target.closest('[data-command="reset-credits"]:enabled')){
        resetsExpanded=!resetsExpanded;
        const requestReset=resetsExpanded&&snapshot.usageResets?.phase!=='loading';
        if(resetsExpanded){const current=snapshot.usageResets||{};resetsPendingGeneration=(current.generation||0)+(current.phase==='loading'?0:1);}
        else resetsPendingGeneration=null;
        renderUsageResets();scheduleLayout();
        if(requestReset)emit({type:'reset-credits'});
        return;
      }

      if(event.target.closest('[data-command="settings"]')){settings.open();return;}
      const scope=event.target.closest('.aam-scope-toggle');if(scope){const key=scope.dataset.scope,open=!contexts.has(key);if(open)contexts.add(key);else contexts.delete(key);document.getElementById(scope.getAttribute('aria-controls')).hidden=!open;scope.setAttribute('aria-expanded',open);scope.setAttribute('aria-label',open?'Hide command scope':'Show command scope');scheduleLayout();return;}
      const expander=event.target.closest('.aam-command-expand');if(expander){const key=expander.dataset.context,expand=!contexts.has(key);if(expand)contexts.add(key);else contexts.delete(key);const command=document.getElementById(expander.getAttribute('aria-controls'));command.classList.toggle('is-expanded',expand);command.closest('.aam-command-wrap').hidden=!expand;expander.setAttribute('aria-expanded',expand);expander.setAttribute('aria-label',expand?'Hide command':'Show command');scheduleLayout();return;}
      const link=event.target.closest('a');if(link){event.preventDefault();if(/^(https?:|mailto:)/i.test(link.href))emit({type:'link',url:link.href});return;}
      const button=event.target.closest('button');if(!button||button.disabled)return;
      const session=sessionFor(button), command=button.dataset.command;
      if(command==='toggle'){seen(session);if(expanded.has(session.id))expanded.delete(session.id);else expanded.add(session.id);const article=button.closest('.aam-session');article.classList.toggle('is-expanded',expanded.has(session.id));article.querySelectorAll('[data-command="toggle"]').forEach(control=>control.setAttribute('aria-expanded',expanded.has(session.id)));scheduleLayout();emit({type:'expansion',sessions:[...expanded]});}
      else if(command==='open'){seen(session);emit({type:'open',session:session.id});}
      else if(command==='seen')seen(session);
      else if(command==='history'){seen(session);histories.add(session.id);signatures.delete(session.id);setSnapshot(snapshot);emit({type:'history',session:session.id});}
      else if(command==='github')emit({type:'link',url:'https://github.com/melsomino/ai-agent-monitor'});
      else if(command)emit({type:command});
      if(button.dataset.response)emit({type:'submit',session:session.id,context:session.request.context,action:button.dataset.response});
      if(button.dataset.preset!==undefined)emit({type:'submit',session:session.id,context:session.request.context,answers:{[button.dataset.question]:[button.dataset.preset]}});
    });
    product.addEventListener('input',event=>{if(event.target.matches('.aam-custom')){event.target.setCustomValidity('');event.target.closest('.aam-answer').querySelector('input[type="radio"]').checked=true;updateFormValidity(event.target.closest('form'));}});
    product.addEventListener('input',event=>{
      if(!event.target.matches('.aam-compose-input'))return;
      const session=sessionFor(event.target),text=event.target.value;drafts.set(session.id,text);event.target.closest('.aam-composer').classList.toggle('has-draft',!!text.trim());resizeComposer(event.target);scheduleLayout();
      event.target.closest('form').querySelector('button').disabled=!text.trim()||Array.from(text).length>16000;
    });
    product.addEventListener('change',event=>{if(event.target.matches('input[type="radio"]')){event.target.closest('fieldset').querySelector('.aam-custom')?.setCustomValidity('');updateFormValidity(event.target.closest('form'));}});
    product.addEventListener('submit',event=>{
      if(event.target.matches('.aam-composer')) {
        event.preventDefault();const form=event.target, session=sessionFor(form), text=form.querySelector('.aam-compose-input').value;
        if(!session.request&&['idle','finished'].includes(session.status)&&session.authoritative!==false&&text.trim()&&Array.from(text).length<=16000) {
          if(!options.live)drafts.delete(session.id);emit({type:'message',session:session.id,text});
        }
        return;
      }
      event.preventDefault();const form=event.target,session=sessionFor(form),answers={};let valid=true;
      for(const fieldset of form.querySelectorAll('fieldset')){const chosen=fieldset.querySelector('input[type="radio"]:checked'),custom=fieldset.querySelector('.aam-custom');if(!chosen||(chosen.dataset.own&&!custom.value.trim())){valid=false;if(custom){custom.setCustomValidity('Enter your own answer or select an option.');custom.reportValidity();}else keyboardFocus(fieldset.querySelector('input'));}else answers[fieldset.dataset.question]=[chosen.dataset.own?custom.value:chosen.value];}
      if(valid)emit({type:'submit',session:session.id,context:session.request.context,answers});
    });
    product.addEventListener('compositionstart',()=>composing=true);product.addEventListener('compositionend',()=>composing=false);
    product.addEventListener('keydown',event=>{
      if(settings.active())return;
      if(event.target.matches('[data-command="preview-pin"]')&&['Enter',' '].includes(event.key)){event.preventDefault();event.target.click();return;}
      if(resetsExpanded&&event.key==='Escape'){event.preventDefault();resetsExpanded=false;resetsPendingGeneration=null;renderUsageResets();product.querySelector('[data-command="reset-credits"]')?.focus({preventScroll:true});return;}
      if(event.target.matches('[data-command="reset-credits"]')&&['Enter',' '].includes(event.key)){event.preventDefault();event.target.click();return;}
      const editing=event.target.matches('input[type="text"],input[type="password"],textarea,[contenteditable="true"]');
      if(event.isComposing||composing)return;
      if(editing&&event.key==='Enter'){if(event.shiftKey&&event.target.matches('.aam-compose-input'))return;event.preventDefault();if(event.target.matches('.aam-compose-input'))event.target.closest('form').requestSubmit();else event.target.closest('.aam-answer')?.querySelector('input[type="radio"]').click();return;}
      if(editing&&!['ArrowUp','ArrowDown'].includes(event.key))return;
      if(event.metaKey||event.ctrlKey||event.altKey)return;
      if(event.key==='Escape'){event.preventDefault();if(query){query='';applyFilter();}else emit({type:'close'});return;}
      if(event.key==='Backspace'&&!editing){event.preventDefault();const chars=typeof Intl.Segmenter==='function'?Array.from(new Intl.Segmenter(undefined,{granularity:'grapheme'}).segment(query),item=>item.segment):Array.from(query);chars.pop();query=chars.join('');applyFilter();return;}
      if(event.target.matches('input[type="radio"]')&&(event.key==='Enter'||event.key===' ')){event.preventDefault();event.target.click();return;}
      if(event.key===' '&&event.target.matches('[data-preset]')){event.preventDefault();event.target.click();return;}
      if(!editing&&Array.from(event.key).length===1&&!/[\u0000-\u001f\u007f-\u009f\uf700-\uf8ff]/u.test(event.key)){event.preventDefault();query+=event.key;applyFilter();return;}
      if(['ArrowLeft','ArrowRight'].includes(event.key)&&event.target.matches('.aam-open')){event.preventDefault();const session=sessionFor(event.target),should=event.key==='ArrowRight';if(expanded.has(session.id)!==should)event.target.closest('.aam-row').querySelector('[data-command="toggle"]').click();return;}
      if(['ArrowLeft','ArrowRight'].includes(event.key)&&event.target.matches('.aam-action')){const controls=Array.from(event.target.parentElement.querySelectorAll('.aam-action:not(:disabled)'));if(controls.length>1&&controls.every(control=>Math.abs(control.getBoundingClientRect().top-event.target.getBoundingClientRect().top)<1)){event.preventDefault();keyboardFocus(controls[(controls.indexOf(event.target)+(event.key==='ArrowRight'?1:controls.length-1))%controls.length]);}return;}
      if(!['ArrowUp','ArrowDown'].includes(event.key))return;
      const targets=Array.from(product.querySelectorAll('.aam-open,.aam-action,.aam-compose-input,.aam-answer input:not([data-own])')).filter(node=>!node.disabled&&node.getClientRects().length&&getComputedStyle(node).visibility!=='hidden'&&!node.closest('[hidden]'));
      const at=targets.indexOf(event.target),next=Math.max(0,Math.min(targets.length-1,at+(event.key==='ArrowDown'?1:-1)));if(targets[next]){event.preventDefault();keyboardFocus(targets[next]);targets[next].scrollIntoView({block:'nearest',inline:'nearest'});}
    });
    function visibility(visible,focusSession){product.classList.toggle('is-inactive',!visible);if(!visible){settings.close();document.activeElement.blur();query='';applyFilter(false);}else{if(focusSession)reveal(focusSession,true);else requestAnimationFrame(()=>keyboardFocus(product.querySelector('.aam-session:not([hidden]) .aam-open')));}}
    prepareBackdrop(product,scheduleLayout);
    applyFilter(false);
    function messageResult(id,text,sent) {
      if(sent && drafts.get(id)===text) drafts.delete(id);
      if(!sent) drafts.set(id,text);
      const node=Array.from(groups.querySelectorAll('.aam-session')).find(node=>node.dataset.session===id);
      const input=node?.querySelector('.aam-compose-input');if(input){input.value=drafts.get(id)||'';resizeComposer(input);scheduleLayout();}
      signatures.delete(id);
    }
    return {setSnapshot,reveal,visibility,messageResult,expanded:()=>[...expanded],destroy:()=>{destroyed=true;resize.disconnect();settings.destroy();}};
  }
  window.AAMMonitor = {mount};
})();
