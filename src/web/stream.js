/* Versioned WebSocket state projection. No transport or UI side effects. */
(() => {
 'use strict';
 const object=value=>value!==null&&typeof value==='object'&&!Array.isArray(value);
 const own=(value,key)=>Object.prototype.hasOwnProperty.call(value,key);
 const clone=value=>JSON.parse(JSON.stringify(value));
 const assert=(condition)=>{if(!condition)throw Error('Invalid monitor stream state.');};
 function ordered(values,order){
  assert(object(values)&&Array.isArray(order)&&order.length===Object.keys(values).length);
  const seen=new Set();
  return order.map(id=>{
   assert(typeof id==='string'&&!seen.has(id)&&own(values,id)&&object(values[id])&&values[id].id===id);
   seen.add(id);return values[id];
  });
 }
 function snapshot(state,revision){
  const result=clone(state);
  result.sessions=ordered(result.sessions,result.sessionOrder).map(session=>{
   session.activities=ordered(session.activities,session.activityOrder);
   delete session.activityOrder;return session;
  });
  delete result.sessionOrder;result.revision=revision;return result;
 }
 class Stream {
  constructor(){this.state=null;this.revision=0;}
  receive(message){
   assert(object(message)&&message.protocol===1&&Number.isSafeInteger(message.revision)&&message.revision>0);
   let next;
   if(message.type==='snapshot'){
    assert(object(message.state)&&message.revision>this.revision);next=clone(message.state);
   }else{
    assert(message.type==='patch'&&this.state!==null&&message.baseRevision===this.revision&&message.revision===this.revision+1&&Array.isArray(message.changes));
    next=clone(this.state);
    for(const change of message.changes){
     assert(object(change)&&Array.isArray(change.path)&&change.path.length>0&&change.path.every(key=>typeof key==='string'));
     let parent=next;
     for(const key of change.path.slice(0,-1)){
      assert(object(parent)&&own(parent,key));parent=parent[key];
     }
     assert(object(parent));const key=change.path[change.path.length-1];
     if(change.op==='remove'){
      assert(own(parent,key));delete parent[key];
     }else{
      assert(change.op==='set'&&own(change,'value'));
      // Entity IDs are opaque data, including keys such as __proto__.
      Object.defineProperty(parent,key,{value:clone(change.value),writable:true,enumerable:true,configurable:true});
     }
    }
   }
   // Validate and reconstruct before committing; a bad patch leaves the base intact.
   const result=snapshot(next,message.revision);
   this.state=next;this.revision=message.revision;return result;
  }
 }
 window.AAMStream=Stream;
})();
