// Covers: client ownership, admission limits, timeout/disconnect and no replay.
// Real: exported client. Doubles: in-memory WebSocket transport; no native claims.
// Requires: Node 22+; node --test scripts/test-agent-client.mjs.
import assert from 'node:assert/strict';
import {test} from 'node:test';
import {RakeAgent} from './rake-agent.mjs';
class Socket extends EventTarget {
  sent=[]; closed=false;
  send(text) {this.sent.push(JSON.parse(text));}
  close() {this.closed=true;this.dispatchEvent(new Event('close'));}
  reply(result) {this.dispatchEvent(new MessageEvent('message',{data:JSON.stringify({id:this.sent.at(-1).id,result})}));}
}
test('a lost mutation acknowledgement retires the client and never replays',async()=>{
  const socket=new Socket(), client=new RakeAgent(socket,'http://127.0.0.1:1');
  await assert.rejects(client.call('Input.insertText',{text:'once'},5),/uncertain; no replay/);
  await assert.rejects(client.call('Input.insertText',{text:'twice'}),/closed/);
  assert.equal(socket.sent.length,1);assert.ok(socket.closed);
});
test('disconnect rejects every pending request without replay',async()=>{
  const socket=new Socket(),client=new RakeAgent(socket,'http://127.0.0.1:1');
  const results=Promise.allSettled([client.call('A'),client.call('B')]);socket.close();
  assert.ok((await results).every(r=>r.status==='rejected'));assert.equal(socket.sent.length,2);
});
test('the pending request and byte budgets reject before sending',async()=>{
  const socket=new Socket(),client=new RakeAgent(socket,'http://127.0.0.1:1');
  const pending=Array.from({length:32},()=>client.call('A').catch(e=>e));
  await assert.rejects(client.call('A'),/pending request limit/);
  assert.equal(socket.sent.length,32);client.close();await Promise.all(pending);
  const other=new RakeAgent(new Socket(),'http://127.0.0.1:1');
  await assert.rejects(other.call('A',{text:'x'.repeat(1048576)}),/byte budget/);other.close();
});
test('foreign references and unavailable capabilities fail before mutation',async()=>{
  const socket=new Socket(),client=new RakeAgent(socket,'http://127.0.0.1:1');
  client.runtimeId='new';client.windowId=1;client.capabilities={methods:{}};
  await assert.rejects(client.click({runtimeId:'old',windowId:1}),/another runtime/);
  await assert.rejects(client.typeText('hello'),/unavailable/);
  assert.equal(socket.sent.length,0);client.close();
});
test('ambiguous semantic lookup and foreign source URLs cannot trigger input or network reads',async()=>{
  const socket=new Socket(),client=new RakeAgent(socket,'http://127.0.0.1:1');
  client.runtimeId='runtime';client.windowId=1;
  client.capabilities={methods:{'Rake.getSnapshot':{supported:true}},sourceRead:{supported:true}};
  const ambiguous=client.find({role:'button'});
  socket.reply({totalMatches:2,elements:[{},{}]});
  await assert.rejects(ambiguous,/found 2/);
  const source=client.source({runtimeId:'runtime',windowId:1,stableId:'one'});
  socket.reply({totalMatches:1,runtimeId:'runtime',windowId:1,elements:[{source:{url:'http://127.0.0.1:2/private'}}]});
  await assert.rejects(source,/another server/);
  assert.ok(socket.sent.every(request=>request.method==='Rake.getSnapshot'));client.close();
});
test('known actionability blockers stop a click before native mouse dispatch',async()=>{
  const socket=new Socket(),client=new RakeAgent(socket,'http://127.0.0.1:1');
  client.runtimeId='runtime';client.windowId=1;
  client.capabilities={methods:{'Rake.getSnapshot':{supported:true},'Rake.getActionability':{supported:true},'Input.dispatchMouseEvent':{supported:true}}};
  const send=socket.send.bind(socket);
  socket.send=text=>{send(text);const request=JSON.parse(text);queueMicrotask(()=>socket.reply(request.method==='Rake.getSnapshot' ? {runtimeId:'runtime',windowId:1,totalMatches:1,elements:[{nodeId:2}]} : {canDispatch:false,blockers:[{code:'disabled',message:'Disabled control'}]}));};
  await assert.rejects(client.click({runtimeId:'runtime',windowId:1,nodeId:2}),e=>e.code===-32011 && e.report.blockers[0].code==='disabled');
  assert.ok(socket.sent.every(request=>!request.method.startsWith('Input.')));client.close();
});
