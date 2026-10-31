// Covers: native Rust allocation sampling, snapshot comparisons and recorder lifecycle.
// Real: GPUI example, TrackingAllocator<System>, native stacks, IPC, supervisor and CDP.
// Double: none. Requires macOS GUI, Node 22+, built rake CLI. Does not automate Chrome.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {readFileSync,writeFileSync,unlinkSync,rmSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import {createInterface} from 'node:readline';
import {setTimeout as delay} from 'node:timers/promises';

assert.equal(process.platform,'darwin','requires real macOS GUI; never silently skip');
const root=fileURLToPath(new URL('../',import.meta.url));
const serve=process.argv.includes('--serve');
const events=[],lifecycle=[],pending=new Map();
const trigger=join(root,`examples/routing/assets/.rake-memory-test-${process.pid}.txt`);
let socket,session,url,sequence=0,stderr='',triggerCreated=false,restarts=0;
const supervisor=spawn(join(root,'target/debug/rake'),['dev','--target','desktop','--package','rake-routing','--features','memory-test','--cdp','--no-open','--json'],{cwd:root,stdio:['ignore','pipe','pipe']});
supervisor.stderr.on('data',data=>{stderr=(stderr+data).slice(-12000);});
createInterface({input:supervisor.stdout}).on('line',line=>{
  try{const event=JSON.parse(line);lifecycle.push(event);
    if(event.event==='session'&&!session&&/^\/tmp\/rake-dev-[A-Za-z0-9_-]+$/.test(event.session))session=event.session;
    if(event.event==='cdp_listening')url=event.url;
  }catch{}
});
async function eventually(check,timeout=180000){
  const end=Date.now()+timeout;let last;
  while(Date.now()<end){try{const value=await check();if(value)return value;}catch(error){last=error;}
    if(supervisor.exitCode!==null)throw new Error(`supervisor exited: ${stderr}`);
    await delay(50);
  }
  throw new Error(`Memory verification timed out: ${stderr}`,{cause:last});
}
function call(method,params={}){
  return new Promise((resolve,reject)=>{
    const id=++sequence,timer=setTimeout(()=>{pending.delete(id);reject(new Error(`${method} timed out`));},60000);
    pending.set(id,{resolve,reject,timer});socket.send(JSON.stringify({id,method,params}));
  });
}
async function connect(target){
  socket=new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve,reject)=>{socket.addEventListener('open',resolve,{once:true});socket.addEventListener('error',reject,{once:true});});
  socket.addEventListener('message',event=>{
    const value=JSON.parse(event.data),item=pending.get(value.id);
    if(item){pending.delete(value.id);clearTimeout(item.timer);value.error?item.reject(new Error(JSON.stringify(value.error))):item.resolve(value.result);}
    else if(value.method&&events.length<8192)events.push(value);
  });
}
async function command(action){
  if(socket.readyState!==WebSocket.OPEN){const target=(await (await fetch(url+'/json/list')).json())[0];await connect(target);}
  const result=await call('Runtime.evaluate',{expression:`test.memory(${JSON.stringify({action})})`,returnByValue:true});
  assert.ok(!result.exceptionDetails,JSON.stringify(result));
  assert.equal(result.result.value.status,'committed',JSON.stringify(result));return result;
}
async function restart(){
  await eventually(()=>JSON.parse(readFileSync(join(session,'status.json'))).phase==='watching');
  writeFileSync(trigger,`checkpointed Memory restart ${++restarts}\n`,{flag:triggerCreated?'w':'wx'});triggerCreated=true;
}
async function stop(){
  process.stdin.pause();socket?.close();for(const item of pending.values())clearTimeout(item.timer);
  supervisor.kill('SIGINT');await Promise.race([new Promise(resolve=>supervisor.once('exit',resolve)),delay(12000)]);
  if(supervisor.exitCode===null)supervisor.kill('SIGTERM');
  if(triggerCreated)try{unlinkSync(trigger);}catch(error){if(error.code!=='ENOENT')throw error;}
  if(session&&(supervisor.exitCode!==null||supervisor.signalCode!==null))rmSync(session,{recursive:true,force:true});
}
function allNodes(node){return [node,...node.children.flatMap(allNodes)];}
async function openPeer(target){
  const peer=new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve,reject)=>{peer.addEventListener('open',resolve,{once:true});peer.addEventListener('error',reject,{once:true});});
  return {socket:peer,call:(method,params={})=>new Promise((resolve,reject)=>{
    const id=++sequence,timer=setTimeout(()=>{peer.removeEventListener('message',receive);reject(new Error(`${method} timed out`));},60000);
    function receive(event){const value=JSON.parse(event.data);if(value.id!==id)return;clearTimeout(timer);peer.removeEventListener('message',receive);value.error?reject(new Error(JSON.stringify(value.error))):resolve(value.result);}
    peer.addEventListener('message',receive);peer.send(JSON.stringify({id,method,params}));
  })};
}
async function snapshot(){
  // Stock Chrome suspends Overlay while taking a heap snapshot.
  await call('Overlay.disable');
  const start=events.length;
  await call('HeapProfiler.takeHeapSnapshot',{reportProgress:true});
  await call('Overlay.enable');
  const captured=events.slice(start),chunks=captured.filter(e=>e.method==='HeapProfiler.addHeapSnapshotChunk');
  assert.ok(chunks.length>0,'snapshot chunks delivered before reply');
  assert.equal(captured.find(e=>e.method==='HeapProfiler.reportHeapSnapshotProgress').params.finished,true);
  const text=chunks.map(e=>e.params.chunk).join('');let cursor=0;
  for(const field of ['snapshot','nodes','edges','samples','locations','strings']){const next=text.indexOf(`"${field}":`,cursor);assert.ok(next>=cursor,'Chrome streaming field order');cursor=next+field.length+3;}
  assert.ok(text.endsWith(']}'),'strings must be the final array');return JSON.parse(text);
}
function probeIds(snapshot){
  const width=snapshot.snapshot.meta.node_fields.length,nodes=snapshot.nodes,ids=[];
  for(let index=width;index<nodes.length;index+=width){if(snapshot.strings[nodes[index+1]].includes('memory_fixture::allocate_probe'))ids.push(nodes[index+2]);}
  return ids.sort((a,b)=>a-b);
}
try{
  await eventually(()=>session&&url&&lifecycle.some(e=>e.event==='watching'&&e.runtime_id));
  const target=await eventually(async()=> (await (await fetch(url+'/json/list')).json())[0]);
  console.log(JSON.stringify({session,devtoolsFrontendUrl:target.devtoolsFrontendUrl}));
  await connect(target);await call('Page.enable');await call('Page.getResourceTree');await call('Runtime.enable');
  if(serve){
    console.log('Memory UI fixture ready. stdin: allocate | release | churn | restart | quit');
    await new Promise(resolve=>{
      const input=createInterface({input:process.stdin});let queue=Promise.resolve();
      input.on('line',line=>{queue=queue.then(async()=>{
        if(['allocate','release','churn'].includes(line)){await command(line);console.log(`${line} complete.`);}
        if(line==='restart')await restart();if(line==='quit')resolve();
      }).catch(console.error);});
      process.once('SIGINT',resolve);process.once('SIGTERM',resolve);supervisor.once('exit',resolve);
    });
  }else{
    await call('HeapProfiler.enable');
    await assert.rejects(call('HeapProfiler.startSampling',{samplingInterval:1}),/samplingInterval/);
    await call('HeapProfiler.startSampling',{samplingInterval:32768,includeObjectsCollectedByMajorGC:true});
    const baseline=await snapshot();assert.equal(probeIds(baseline).length,0);
    await command('allocate');
    const first=(await call('HeapProfiler.getSamplingProfile')).profile;
    const probes=allNodes(first.head).filter(n=>n.callFrame.functionName.includes('memory_fixture::allocate_probe'));
    assert.ok(probes.length>0,JSON.stringify({reason:'real allocation caller resolves',metadata:first.rake,functions:allNodes(first.head).slice(-20).map(n=>n.callFrame.functionName)}));
    assert.ok(probes.some(node=>node.callFrame.url.endsWith('/memory_fixture.rs')),'Rust allocation source URL');
    assert.equal(first.rake.liveSamples>=8,true);
    const held=await snapshot(),again=await snapshot();
    assert.equal(probeIds(held).length,8,JSON.stringify({classes:held.strings.slice(0,24),metadata:held.rake}));assert.deepEqual(probeIds(held),probeIds(again),'stable live allocation IDs');
    await command('churn');await command('release');
    const released=await snapshot();assert.equal(probeIds(released).length,0,'release removes actual sampled blocks');
    const final=(await call('HeapProfiler.stopSampling')).profile;
    assert.ok(final.rake.freedSamples>=8,'actual released blocks appear in churn capture');
    assert.equal(final.rake.recording,false);
    await assert.rejects(call('HeapProfiler.collectGarbage'),/Rust memory/);
    await call('Rake.stopMemory');await call('HeapProfiler.startSampling');
    await call('Runtime.compileScript',{expression:'rake.stopMemory()'});
    await assert.rejects(call('Runtime.evaluate',{expression:'rake.stopMemory()',throwOnSideEffect:true}),/side-effect/);
    assert.equal((await call('Rake.getMemorySummary')).recording,true);
    const peer=await openPeer(target);
    try{
      await assert.rejects(peer.call('HeapProfiler.startSampling'),/another client/);
      await peer.call('HeapProfiler.disable');
      assert.equal((await call('Rake.getMemorySummary')).recording,true);
      await call('HeapProfiler.disable');
      await peer.call('HeapProfiler.startSampling');await peer.call('HeapProfiler.disable');
    }finally{peer.socket.close();}
    await call('HeapProfiler.startSampling');await restart();
    await eventually(()=>events.find(e=>e.method==='Page.frameNavigated'&&e.params.frame.id==='rake'&&e.params.frame.loaderId!==target.id));
    assert.ok(events.some(e=>e.method==='HeapProfiler.resetProfiles'));
    assert.equal((await call('Rake.getMemorySummary')).recording,false);
    await assert.rejects(call('HeapProfiler.getSamplingProfile'),/Start a native memory recording/);
    await call('HeapProfiler.startSampling');
    const successor=(await (await fetch(url+'/json/list')).json())[0];
    socket.close();await new Promise(resolve=>socket.addEventListener('close',resolve,{once:true}));
    const next=await openPeer(successor);
    try{await eventually(async()=>{try{await next.call('HeapProfiler.startSampling');return true;}catch{return false;}},15000);await next.call('HeapProfiler.disable');}finally{next.socket.close();}
    console.log(JSON.stringify({passed:true,liveSamples:first.rake.liveSamples,freedSamples:final.rake.freedSamples,replacement:successor.id,sites:probes.map(n=>n.callFrame)}));
  }
}finally{await stop();}
