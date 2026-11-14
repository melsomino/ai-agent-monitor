// Run with: node --test tests/web_stream.js
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vm=require('node:vm');
const context={window:{}};
vm.runInNewContext(fs.readFileSync('src/web/stream.js','utf8'),context);
const Stream=context.window.AAMStream;
const plain=value=>JSON.parse(JSON.stringify(value));
const initial=()=>({type:'snapshot',protocol:1,revision:1,state:{sessionOrder:['codex:a/b'],sessions:{'codex:a/b':{id:'codex:a/b',title:'Original',activityOrder:['old','request'],activities:{old:{id:'old',type:'message',html:'Keep this'},request:{id:'request',request:{phase:'ready',error:null}}}}},usage:[],error:null}});
const patch=(changes,revision=2,baseRevision=1)=>({type:'patch',protocol:1,baseRevision,revision,changes});

test('keyed changes preserve messages, append/reorder activities and reconstruct renderer arrays',()=>{
 const stream=new Stream();const first=stream.receive(initial());
 first.sessions[0].activities[0].html='Mutation by renderer';
 const view=stream.receive(patch([
  {op:'set',path:['sessions','codex:a/b','activities','request','request','phase'],value:'sending'},
  {op:'set',path:['sessions','codex:a/b','activities','new'],value:{id:'new',type:'message',html:'New'}},
  {op:'set',path:['sessions','codex:a/b','activityOrder'],value:['new','old','request']}
 ]));
 assert.equal(view.sessions[0].activities[1].html,'Keep this');
 assert.equal(view.sessions[0].activities[2].request.phase,'sending');
 assert.deepEqual(plain(view.sessions[0].activities.map(a=>a.id)),['new','old','request']);
 assert.equal(view.revision,2);
});

test('missing/duplicate/out of order patches require resync and never advance the baseline',()=>{
 const stream=new Stream();stream.receive(initial());
 for(const packet of [patch([],3,2),patch([],3,1),{...patch([]),protocol:2}]) assert.throws(()=>stream.receive(packet));
 assert.equal(stream.revision,1);
 const full=initial();full.revision=8;full.state.sessions['codex:a/b'].title='Resynced';
 assert.equal(stream.receive(full).sessions[0].title,'Resynced');
 assert.throws(()=>stream.receive(initial()));
 assert.throws(()=>stream.receive(patch([],8,7)));
 assert.equal(stream.revision,8);
});

test('removal and null values remain distinct and an invalid multi-operation update is atomic',()=>{
 const stream=new Stream();stream.receive(initial());
 const good=patch([{op:'set',path:['error'],value:'Error'}]);stream.receive(good);
 const view=stream.receive(patch([{op:'set',path:['error'],value:null},{op:'remove',path:['sessions','codex:a/b','activities','request']},{op:'set',path:['sessions','codex:a/b','activityOrder'],value:['old']}],3,2));
 assert.equal(view.error,null);assert.equal(view.sessions[0].activities.length,1);
 assert.throws(()=>stream.receive(patch([{op:'set',path:['error'],value:'Partial'},{op:'remove',path:['missing']}],4,3)));
 assert.equal(stream.revision,3);assert.equal(stream.state.error,null);
 assert.throws(()=>stream.receive(patch([{op:'set',path:['sessionOrder'],value:['not-present']}],4,3)));
});

test('opaque entity keys cannot pollute object prototypes',()=>{
 const stream=new Stream();stream.receive(initial());
 const view=stream.receive(patch([{op:'set',path:['sessions','__proto__'],value:{id:'__proto__',title:'Safe',activities:{},activityOrder:[]}},{op:'set',path:['sessionOrder'],value:['__proto__','codex:a/b']} ]));
 assert.equal(view.sessions[0].title,'Safe');assert.equal({}.title,undefined);
 assert.throws(()=>stream.receive(patch([{op:'set',path:['constructor','prototype','polluted'],value:true}],3,2)));
 assert.equal({}.polluted,undefined);
});

test('remote host resyncs safely and sends history through WebSocket without HTTP or replay',async()=>{
 const renders=[],sockets=[],timers=new Map(),httpCalls=[];let nextTimer=0,dispatch;
 const element=()=>({hidden:true,append(){},setAttribute(){}});
 const browser={URLSearchParams,crypto:require('node:crypto').webcrypto,window:{},document:{body:{append(){}},createElement:element,querySelector:element,addEventListener(){}},
  location:{protocol:'http:',host:'localhost:8765',hash:'',pathname:'/',search:''},history:{replaceState(){}},
  matchMedia:()=>({matches:false,addEventListener(){}}),
  fetch:async path=>{httpCalls.push(path);return {ok:true,status:200,json:async()=>new Stream().receive(initial())};},
  setTimeout:fn=>{const id=++nextTimer;timers.set(id,fn);return id;},clearTimeout:id=>timers.delete(id)};
 browser.document.getElementById=element;
 browser.WebSocket=class {
  static OPEN=1;
  constructor(){this.readyState=1;this.sent=[];sockets.push(this);}
  send(text){this.sent.push(JSON.parse(text));}
  close(){this.readyState=3;this.onclose?.();}
 };
 browser.window.AAMMonitor={mount:(_root,send)=>{dispatch=send;return {setSnapshot:value=>renders.push(plain(value)),messageResult(){}};}};
 vm.runInNewContext(fs.readFileSync('src/web/stream.js','utf8'),browser);
 vm.runInNewContext(fs.readFileSync('src/web/remote.js','utf8'),browser);
 await new Promise(setImmediate);
 const socket=sockets[0];assert.ok(socket);
 const emit=packet=>socket.onmessage({data:JSON.stringify(packet)});
 emit(initial());
 emit(patch([{op:'set',path:['sessions','codex:a/b','title'],value:'Live patch'}]));
 assert.equal(renders.at(-1).sessions[0].title,'Live patch');
 emit(patch([],9,8));
 assert.deepEqual(socket.sent,[{type:'resync',protocol:1}]);
 assert.equal(renders.at(-1).sessions[0].authoritative,false);
 emit(patch([],10,9));assert.equal(socket.sent.length,1);
 const reset=initial();reset.revision=10;reset.state.sessions['codex:a/b'].title='Restored';
 emit(reset);assert.equal(renders.at(-1).sessions[0].title,'Restored');assert.equal(timers.size,0);
 emit(patch([{op:'set',path:['sessions','codex:a/b','title'],value:'After recovery'}],11,10));
 assert.equal(renders.at(-1).sessions[0].title,'After recovery');
 const previous=socket.sent.length;
 const a=dispatch({type:'history',session:'codex:a/b'}),b=dispatch({type:'history',session:'codex:a/b'});
 assert.equal(socket.sent.length,previous+1,'duplicate clicks must share the outstanding request');
 const history=socket.sent.at(-1);assert.equal(history.type,'history');assert.match(history.id,/^[a-f0-9]{32}$/);
 assert.equal(renders.at(-1).sessions[0].history.loading,true);
 emit({type:'command_result',protocol:1,id:history.id,accepted:true});await Promise.all([a,b]);
 assert.equal(timers.size,0);assert.deepEqual(httpCalls,['/api/snapshot']);
 const failed=dispatch({type:'history',session:'codex:a/b'});
 emit({type:'command_result',protocol:1,id:socket.sent.at(-1).id,error:'No history page is available right now.'});await failed;
 const disconnected=dispatch({type:'history',session:'codex:a/b'});const count=socket.sent.length;
 socket.close();await disconnected;
 await dispatch({type:'history',session:'codex:a/b'});
 assert.equal(socket.sent.length,count,'disconnect must not replay an admitted or uncertain request');
 assert.deepEqual(httpCalls,['/api/snapshot']);
});
