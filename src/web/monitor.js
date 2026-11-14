/* Shared monitor UI. Host adapters provide snapshots and receive typed commands. */
(() => {
  'use strict';
  const paths = {
    'arrow-right':'<path d="M5 12h14m-6-6 6 6-6 6"/>',
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
  function mount(root, send, options = {}) {
    root.innerHTML = '<div class="aam-stage"><div class="aam-product"></div></div>';
    const product = root.querySelector('.aam-product');
    product.append(document.getElementById('aam-shell').content.cloneNode(true)); product.tabIndex = -1;
    product.querySelector('[data-command="settings"]').innerHTML = icon('settings');
    product.querySelector('[data-command="quit"]').innerHTML = icon('power');
    const list = product.querySelector('.aam-list'), groups = product.querySelector('.aam-session-groups');
    const expanded = new Set(), contexts = new Set(), histories = new Set(), signatures = new Map();
    const historySignatures = new Map(), drafts = new Map();
    let snapshot = {sessions:[],usage:[],version:'',loading:true}, query = '', counter = 0, composing = false;
    let layoutPending = false, pointerY = null;
    const emit = message => send(message);
    const settings = options.settings === false ? {active:()=>false,update:()=>{},open:()=>{},close:()=>{},destroy:()=>{}} : window.AAMSettings.mount(product, emit, scheduleLayout);
    if(options.settings === false) product.querySelector('[data-command="settings"]').hidden = true;
    const sessionFor = node => snapshot.sessions.find(session => session.id === node.closest('.aam-session')?.dataset.session);
    const words = () => query.toLowerCase().split(/\s+/u).filter(Boolean);
    const matches = session => words().every(word => session.title.toLowerCase().includes(word));
    function status(session) {
      if (session.status === 'idle') return '';
      const label = {working:'Working',finished:'Finished (unseen)',request:'Needs response',unknown:'Unknown status'}[session.status] || 'Unknown status';
      let art = '<circle cx="12" cy="12" r="10"/><path class="aam-cutout" d="M11 5h2v9h-2Zm0 11h2v3h-2Z"/>';
      if (session.status === 'working') art = '<circle cx="12" cy="12" r="10"/><path class="aam-cutout" d="m9 6 9 6-9 6Z"/>';
      if (session.status === 'finished') art = '<circle cx="12" cy="12" r="10"/><path class="aam-cutout" d="m6 12 2-2 3 3 6-6 2 2-8 8Z"/>';
      return `<${session.status === 'finished' ? 'button type="button" data-command="seen"' : 'span'} class="aam-state" data-status="${esc(session.status)}" aria-label="${label}" title="${session.status === 'finished' ? 'Mark as seen' : label}"><span class="aam-icon"><svg viewBox="0 0 24 24">${art}</svg></span></${session.status === 'finished' ? 'button' : 'span'}>`;
    }
    function commandBlock(text, key, caption, copy) {
      if (!text) return '';
      const id = `aam-code-${++counter}`, isExpanded = contexts.has(key);
      return `<p class="aam-command-caption">${icon(copy ? 'terminal' : 'sparkles')}${esc(caption)}<button type="button" class="aam-command-expand" data-context="${esc(key)}" tabindex="-1" aria-controls="${id}" aria-expanded="${isExpanded}" aria-label="${isExpanded ? 'Collapse' : 'Show full'} command" hidden>${icon('chevron-down')}</button></p><div class="aam-command-wrap"><pre id="${id}" class="aam-command ${isExpanded ? 'is-expanded' : ''}"><code>${esc(text)}</code></pre>${copy ? `<button type="button" class="aam-copy-command" aria-label="Copy command" title="Copy command">${icon('copy')}</button>` : ''}</div>`;
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
      controls += commandBlock(request.command, `${session.id}:${request.context}:command`, 'Command', true);
      const immediate = (request.actions || []).filter(action => !action.detail), scoped = (request.actions || []).filter(action => action.detail);
      const button = action => `<button type="button" class="aam-action ${action.primary ? 'aam-primary' : ''}" data-response="${esc(action.id)}" data-focus="action:${esc(action.id)}" ${off}>${icon(action.id === 'deny' || /deny|decline/i.test(action.label) ? 'x' : 'check')}${esc(action.label)}</button>`;
      controls += `<div class="aam-actions">${immediate.map(button).join('')}</div>`;
      controls += scoped.map(action => {
        const key=`${session.id}:${request.context}:${action.id}:scope`, open=contexts.has(key), id=`aam-scope-${++counter}`;
        return `<div class="aam-action-detail"><div class="aam-scope-row">${button(action)}<button type="button" class="aam-scope-toggle" data-scope="${esc(key)}" tabindex="-1" aria-controls="${id}" aria-expanded="${open}" aria-label="${open?'Hide':'Show'} command scope">${icon('chevron-down')}</button></div><pre id="${id}" class="aam-command is-expanded" ${open?'':'hidden'}><code>${esc(action.detail)}</code></pre></div>`;
      }).join('');
      if (!request.actions?.length && !request.questions?.length) controls += '<button type="button" class="aam-action" data-command="open">Answer in editor…</button>';
      const note = busy ? (request.phase === 'sending' ? 'Sending…' : 'Checking delivery…') : request.error || (!request.enabled ? 'Waiting for a live connection to the agent.' : '');
      return `<div class="aam-question ${request.questions?.length ? '' : 'aam-approval'}"><span class="aam-request-mark" aria-hidden="true">${icon('circle-alert')}</span>${controls}${note ? `<p class="aam-status-note" role="status">${esc(note)}</p>` : ''}</div>`;
    }
    function progressMarkup(session) {
      const text=Array.from(typeof session.progress==='string'&&session.progress.trim()?session.progress:'Working…').slice(0,512).join('');
      return session.status==='working'&&!session.request&&text ? `<p class="aam-work-status" title="${esc(text)}"><span class="aam-work-glow" aria-hidden="true">${esc(text)}</span><span class="aam-work-text">${esc(text)}</span></p>` : '';
    }
    function composerMarkup(session) {
      if (session.request || !['idle','finished'].includes(session.status)) return '';
      const text=drafts.get(session.id)||'', enabled=session.authoritative!==false;
      return `<form class="aam-composer"><textarea class="aam-compose-input" rows="1" aria-label="Message agent" placeholder="Message agent…" data-focus="compose" autocomplete="off" ${enabled?'':'disabled'}>${esc(text)}</textarea><button type="submit" class="aam-action" data-focus="compose-send" ${enabled&&text.trim()&&Array.from(text).length<=16000?'':'disabled'}>Send</button></form>`;
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
        input.value = old.value; input.checked = old.checked;
        if (old.start !== null) try { input.setSelectionRange(old.start,old.end); } catch {}
      }
      const focus = Array.from(node.querySelectorAll('[data-focus]')).find(item => item.dataset.focus === saved.focus);
      if (focus && !focus.disabled) focus.focus({preventScroll:true});
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
      snapshot = {...value,sessions:value.sessions.map(project)}; product.querySelector('.aam-version').textContent = snapshot.version || '';
      settings.update(value);
      root.style.colorScheme = snapshot.theme || 'dark'; root.dataset.theme = snapshot.theme || 'dark';
      product.classList.toggle('reduce-motion',snapshot.animate === false);
      const ids = new Set(snapshot.sessions.map(session => session.id));
      for (const node of groups.querySelectorAll('.aam-session')) if (!ids.has(node.dataset.session)) { signatures.delete(node.dataset.session); expanded.delete(node.dataset.session); drafts.delete(node.dataset.session); node.remove(); }
      const grouped = new Map();
      for (const session of snapshot.sessions) {
        if (session.expanded && !signatures.has(session.id)) expanded.add(session.id);
        if (session.request && !historySignatures.has(session.id + ':' + session.request.context)) { expanded.add(session.id); historySignatures.set(session.id + ':' + session.request.context,true); }
        if (!grouped.has(session.appId)) grouped.set(session.appId,[]); grouped.get(session.appId).push(session);
      }
      for (const group of groups.querySelectorAll('.aam-group')) if (!grouped.has(group.dataset.app)) group.remove();
      for (const [id,sessions] of grouped) {
        let group = Array.from(groups.children).find(node => node.dataset.app === id);
        if (!group) { group = document.createElement('section'); group.className='aam-group';group.dataset.app=id;group.innerHTML='<div class="aam-sticky-app"></div><div class="aam-group-sessions"></div>';groups.append(group); }
        const heading=group.querySelector('.aam-sticky-app');
        const headingHTML=`<span class="aam-app-icon"><img src="${image(sessions[0].icon)}" width="24" height="24" alt=""></span><span class="aam-app-name">${esc(sessions[0].app)}</span>`;
        if (heading.innerHTML !== headingHTML) heading.innerHTML=headingHTML;
        for (const session of sessions) {
          let node = Array.from(group.querySelector('.aam-group-sessions').children).find(node => node.dataset.session === session.id);
          const signature = JSON.stringify(session);
          if (signatures.get(session.id) === signature && node) continue;
          const saved = preserve(node), template = document.createElement('template'); template.innerHTML=sessionMarkup(session);
          const fresh=template.content.firstElementChild;
          if (node) node.replaceWith(fresh); else group.querySelector('.aam-group-sessions').append(fresh);
          restore(fresh,saved); signatures.set(session.id,signature);
        }
      }
      const usage=product.querySelector('.aam-usage-windows');
      usage.innerHTML=(snapshot.usage || []).map(window => `<div class="aam-usage"><div class="aam-usage-main"><span class="aam-usage-name">${icon('chart')}Codex ${esc(window.label)}</span><div class="aam-usage-track" role="progressbar" aria-label="${esc(window.label)} remaining" ${window.remaining === null ? '' : `aria-valuenow="${window.remaining}"`} aria-valuemin="0" aria-valuemax="100"><span style="width:${Math.max(0,Math.min(100,window.remaining || 0))}%"></span></div><span class="aam-usage-percent">${window.remaining === null ? '—' : esc(window.remaining)+'%'}</span></div><span class="aam-usage-date">${esc(window.reset || '')}</span></div>`).join('') || '<p class="aam-status-note">Codex usage unavailable</p>';
      const note=product.querySelector('.aam-status-note');note.textContent=snapshot.error || '';note.hidden=!snapshot.error;
      product.querySelectorAll('.aam-question-form').forEach(updateFormValidity);
      applyFilter(false); scheduleLayout();
      if (snapshot.focusSession && !settings.active()) reveal(snapshot.focusSession,true);
    }
    function highlight(element,title) {
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
        if(!article.hidden) {count++;const state=session.request||session.status==='request'?'waiting':session.status;counts[Object.hasOwn(counts,state)?state:'unknown']++;}
      }
      for(const group of groups.children) {const visible=Array.from(group.querySelectorAll('.aam-session')).filter(node=>!node.hidden);group.hidden=!visible.length;visible[0]?.classList.add('aam-session-first');}
      const status=product.querySelector('.aam-filter-status');status.textContent=query?`Filter: ${query}`:'';status.hidden=!query;
      const empty=product.querySelector('.aam-filter-empty');empty.hidden=!!count;empty.textContent=snapshot.loading?'Loading sessions…':query.trim()?'No matching sessions':'No open sessions';
      const overview=product.querySelector('.aam-overview');
      overview.innerHTML=[['waiting','waiting for you'],['working','working'],['finished','finished'],['idle','idle']].filter(([state])=>counts[state]>0).map(([state,label])=>`<span class="aam-overview-count" data-state="${state}">${counts[state]} ${label}</span>`).join(' · ');
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
      // Keep prototype geometry current even when a hidden browser document
      // suspends animation frames.
      queueMicrotask(()=>{layoutPending=false;layout();});
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
        const cuts=Array.from(article.querySelectorAll('.aam-message-dot,.aam-request-mark'))
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
      for(const button of product.querySelectorAll('.aam-command-expand')) {
        const command=document.getElementById(button.getAttribute('aria-controls'));if(!command.getClientRects().length)continue;
        const code=command.querySelector('code'),style=getComputedStyle(command);
        button.hidden=code.scrollHeight<=parseFloat(style.lineHeight)*2+1;
      }
      const backdrop=product.querySelector('.aam-backdrop'),footer=product.querySelector('.aam-footer');
      product.style.setProperty('--aam-footer-height',footer.offsetHeight+'px');
      const height=list.scrollHeight;backdrop.style.height=height+'px';
      const mesh=backdrop.querySelector('.aam-mesh'), logical=height*460/Math.max(backdrop.clientWidth,1);mesh.setAttribute('viewBox',`0 0 460 ${logical}`);mesh.querySelector(':scope > rect').setAttribute('height',logical);
      const tiles=mesh.querySelector('.aam-mesh-tiles'), count=Math.max(1,Math.ceil(logical/1000));
      if(tiles.children.length!==count)tiles.replaceChildren(...Array.from({length:count},(_,index)=>{const use=document.createElementNS('http://www.w3.org/2000/svg','use');use.setAttribute('href','#aam-mesh-art-0');use.setAttribute('transform',index%2?`translate(460 ${index*1000}) scale(-1 1)`:`translate(0 ${index*1000})`);return use;}));
      backdrop.style.transform=`translate3d(0,${-list.scrollTop}px,0)`;
      const natural=Math.min(1200,Math.ceil(list.scrollHeight));
      emit({type:'size',height:natural,ready:snapshot.loading!==true});
      previousHover();
    }
    function resizeComposer(input) {
      const before=input.style.height;
      input.style.height='0px';
      const height=Math.max(32,input.scrollHeight);
      input.style.height=height+'px';
      return before!==input.style.height;
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
      if(event.target.closest('[data-command="settings"]')){settings.open();return;}
      const copy=event.target.closest('.aam-copy-command');if(copy){emit({type:'copy',text:copy.closest('.aam-command-wrap').querySelector('code').textContent});return;}
      const scope=event.target.closest('.aam-scope-toggle');if(scope){const key=scope.dataset.scope,open=!contexts.has(key);if(open)contexts.add(key);else contexts.delete(key);document.getElementById(scope.getAttribute('aria-controls')).hidden=!open;scope.setAttribute('aria-expanded',open);scope.setAttribute('aria-label',open?'Hide command scope':'Show command scope');scheduleLayout();return;}
      const expander=event.target.closest('.aam-command-expand');if(expander){const key=expander.dataset.context,expand=!contexts.has(key);if(expand)contexts.add(key);else contexts.delete(key);document.getElementById(expander.getAttribute('aria-controls')).classList.toggle('is-expanded',expand);expander.setAttribute('aria-expanded',expand);expander.setAttribute('aria-label',expand?'Collapse command':'Show full command');scheduleLayout();return;}
      const link=event.target.closest('a');if(link){event.preventDefault();if(/^(https?:|mailto:)/i.test(link.href))emit({type:'link',url:link.href});return;}
      const button=event.target.closest('button');if(!button||button.disabled)return;
      const session=sessionFor(button), command=button.dataset.command;
      if(command==='toggle'){seen(session);if(expanded.has(session.id))expanded.delete(session.id);else expanded.add(session.id);const article=button.closest('.aam-session');article.classList.toggle('is-expanded',expanded.has(session.id));article.querySelectorAll('[data-command="toggle"]').forEach(control=>control.setAttribute('aria-expanded',expanded.has(session.id)));scheduleLayout();emit({type:'expansion',sessions:[...expanded]});}
      else if(command==='open'){seen(session);emit({type:'open',session:session.id});}
      else if(command==='seen')seen(session);
      else if(command==='history'){seen(session);histories.add(session.id);signatures.delete(session.id);setSnapshot(snapshot);emit({type:'history',session:session.id});}
      else if(command)emit({type:command});
      if(button.dataset.response)emit({type:'submit',session:session.id,context:session.request.context,action:button.dataset.response});
      if(button.dataset.preset!==undefined)emit({type:'submit',session:session.id,context:session.request.context,answers:{[button.dataset.question]:[button.dataset.preset]}});
    });
    product.addEventListener('input',event=>{if(event.target.matches('.aam-custom')){event.target.setCustomValidity('');event.target.closest('.aam-answer').querySelector('input[type="radio"]').checked=true;updateFormValidity(event.target.closest('form'));}});
    product.addEventListener('input',event=>{
      if(!event.target.matches('.aam-compose-input'))return;
      const session=sessionFor(event.target),text=event.target.value;drafts.set(session.id,text);resizeComposer(event.target);scheduleLayout();
      event.target.closest('form').querySelector('button').disabled=!text.trim()||Array.from(text).length>16000;
    });
    product.addEventListener('change',event=>{if(event.target.matches('input[type="radio"]')){event.target.closest('fieldset').querySelector('.aam-custom')?.setCustomValidity('');updateFormValidity(event.target.closest('form'));}});
    product.addEventListener('submit',event=>{
      if(event.target.matches('.aam-composer')) {
        event.preventDefault();const form=event.target, session=sessionFor(form), text=form.querySelector('input').value;
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
    applyFilter(false);
    function messageResult(id,text,sent) {
      if(sent && drafts.get(id)===text) drafts.delete(id);
      if(!sent) drafts.set(id,text);
      const node=Array.from(groups.querySelectorAll('.aam-session')).find(node=>node.dataset.session===id);
      const input=node?.querySelector('.aam-compose-input');if(input){input.value=drafts.get(id)||'';resizeComposer(input);scheduleLayout();}
      signatures.delete(id);
    }
    return {setSnapshot,reveal,visibility,messageResult,expanded:()=>[...expanded],destroy:()=>{resize.disconnect();settings.destroy();}};
  }
  window.AAMMonitor = {mount};
})();
