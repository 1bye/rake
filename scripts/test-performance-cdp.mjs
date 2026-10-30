// Covers: frame/callback timings, causal tracks, streams, source lookup, restart and crash.
// Real: native GPUI, Rake command/task/HTTP runtime, IPC, supervisor and CDP.
// Double: loopback HTTP responses. Requires macOS GUI, Node 22+, built rake CLI.
// Stock Chrome UI is verified separately; this script never controls Chrome.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {createServer} from 'node:http';
import {readFileSync, writeFileSync, unlinkSync, rmSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import {createInterface} from 'node:readline';
import {setTimeout as delay} from 'node:timers/promises';

assert.equal(process.platform, 'darwin', 'requires a real macOS GUI; never silently skip');
const root = fileURLToPath(new URL('../', import.meta.url));
const cli = join(root, 'target/debug/rake');
const serve = process.argv.includes('--serve');
const lifecycle = [], events = [], pending = new Map();
const trigger = join(root, `examples/routing/assets/.rake-performance-test-${process.pid}.txt`);
let socket, session, url, sequence = 0, triggerCreated = false;
let stderr = '', restartCount = 0;
const http = createServer((request, response) => {
  const chunks=[];
  request.on('data',chunk=>chunks.push(chunk));
  request.on('end',()=>{
    const body=JSON.stringify({message:'café 你好 🙂',received:Buffer.concat(chunks).toString()});
    response.writeHead(200,{'content-type':'application/json; charset=utf-8','content-length':Buffer.byteLength(body)});
    response.end(body);
  });
});
await new Promise(resolve => http.listen(0, '127.0.0.1', resolve));
const base = `http://127.0.0.1:${http.address().port}`;
const supervisor = spawn(cli, ['dev','--target','desktop','--package','rake-routing','--features','performance-test','--cdp','--no-open','--json'], {cwd:root,env:{...process.env,RAKE_NETWORK_FIXTURE_URL:base},stdio:['ignore','pipe','pipe']});
supervisor.stderr.on('data', data => { stderr = (stderr + data).slice(-16000); });
createInterface({input:supervisor.stdout}).on('line', line => {
  try {
    const event = JSON.parse(line); lifecycle.push(event);
    if (event.event === 'session' && !session && /^\/tmp\/rake-dev-[A-Za-z0-9_-]+$/.test(event.session)) session = event.session;
    if (event.event === 'cdp_listening') url = event.url;
  } catch {}
});
async function eventually(check, timeout=30000) {
  const end=Date.now()+timeout; let last;
  while(Date.now()<end) {
    try { const value=await check(); if(value) return value; } catch(error) { last=error; }
    if(supervisor.exitCode!==null) throw new Error(`supervisor exited: ${stderr}`);
    await delay(50);
  }
  throw new Error(`Performance verification timed out: ${stderr}`, {cause:last});
}
function call(method, params={}) {
  return new Promise((resolve,reject) => {
    const id=++sequence, timer=setTimeout(() => { pending.delete(id);reject(new Error(`${method} timed out`)); },10000);
    pending.set(id,{resolve,reject,timer});socket.send(JSON.stringify({id,method,params}));
  });
}
async function restart() {
  await eventually(()=>JSON.parse(readFileSync(join(session,'status.json'))).phase==='watching');
  restartCount++;
  writeFileSync(trigger,`checkpointed Performance restart ${restartCount}\n`,{flag:triggerCreated?'w':'wx'});triggerCreated=true;
}
async function stop() {
  process.stdin.pause();socket?.close();for(const item of pending.values())clearTimeout(item.timer);
  supervisor.kill('SIGINT');
  await Promise.race([new Promise(resolve=>supervisor.once('exit',resolve)),delay(12000)]);
  if(supervisor.exitCode===null)supervisor.kill('SIGTERM');
  http.closeAllConnections();http.close();
  if(triggerCreated)try{unlinkSync(trigger);}catch(error){if(error.code!=='ENOENT')throw error;}
  if(session&&(supervisor.exitCode!==null||supervisor.signalCode!==null))rmSync(session,{recursive:true,force:true});
}

async function connect(target) {
  socket=new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve,reject)=>{socket.addEventListener('open',resolve,{once:true});socket.addEventListener('error',reject,{once:true});});
  socket.addEventListener('message',event=>{
    const value=JSON.parse(event.data),item=pending.get(value.id);
    if(item){pending.delete(value.id);clearTimeout(item.timer);value.error?item.reject(new Error(JSON.stringify(value.error))):item.resolve(value.result);}
    else if(value.method&&events.length<8192)events.push(value);
  });
}
async function work() {
  const result=await call('Runtime.evaluate',{expression:'test.performance({"slow_draws":3})',returnByValue:true});
  assert.ok(!result.exceptionDetails,JSON.stringify(result));
  assert.equal(result.result.value.status,'committed');
}
async function start(mode='ReturnAsStream') {
  events.length=0;
  await call('Tracing.start',{transferMode:mode});
}
async function complete(stopRecording=true) {
  if(stopRecording)await call('Tracing.end');
  return (await eventually(()=>events.find(e=>e.method==='Tracing.tracingComplete'),60000)).params;
}
async function read(handle) {
  let data='';
  for(;;) {
    const chunk=await call('IO.read',{handle,size:4093});
    assert.equal(chunk.base64Encoded,false);data+=chunk.data;
    if(chunk.eof)break;
    assert.ok(Buffer.byteLength(data)<4*1024*1024);
  }
  assert.ok(Buffer.byteLength(data)<=4*1024*1024);
  return {data,trace:JSON.parse(data)};
}
try {
  await eventually(()=>session&&url&&lifecycle.some(e=>e.event==='watching'&&e.runtime_id),90000);
  const target=await eventually(async()=> (await (await fetch(url+'/json/list')).json())[0]);
  console.log(JSON.stringify({session,base,devtoolsFrontendUrl:target.devtoolsFrontendUrl}));
  await connect(target);
  if(serve) {
    await call('Runtime.enable');
    console.log('Performance UI fixture ready. stdin: work | capture | empty | restart | quit');
    await new Promise(resolve=>{
      const input=createInterface({input:process.stdin});let queue=Promise.resolve();
      input.on('line',line=>{queue=queue.then(async()=>{
        if(line==='work'){await work();console.log('Performance workload complete.');}
        if(line==='empty'){await start();const done=await complete();const output=await read(done.stream);writeFileSync('/tmp/rake-performance-empty.json',output.data);console.log(JSON.stringify({saved:'/tmp/rake-performance-empty.json',frames:output.trace.metadata.frames.windows}));}
        if(line==='capture'){await start();await work();await delay(2200);const done=await complete();const output=await read(done.stream);writeFileSync('/tmp/rake-performance-trace.json',output.data);console.log(JSON.stringify({saved:'/tmp/rake-performance-trace.json',summary:output.trace.metadata}));}
        if(line==='restart')await restart();if(line==='quit')resolve();
      }).catch(console.error);});
      process.once('SIGINT',resolve);process.once('SIGTERM',resolve);supervisor.once('exit',resolve);
    });
  } else {
    // Tracing alone must poll; no Runtime/Log/Network/DOM enable calls.
    await start();await work();await delay(2400);
    const done=await complete(), output=await read(done.stream),trace=output.trace;
    writeFileSync('/tmp/rake-performance-trace.json',output.data);
    assert.equal(done.dataLossOccurred,false,JSON.stringify(trace.metadata));
    assert.equal(trace.metadata.causal.available,true);
    assert.ok(trace.metadata.frames.hotspots.some(h=>h.name==='performance.probe'&&h.phase==='GPUI paint'&&h.maxInclusiveUs>=25000),'actual slow paint attribution');
    assert.ok(trace.traceEvents.filter(e=>e.ph==='b').every(e=>e.pid!==1),'async tracks must not inflate renderer CPU totals');
    const draws=trace.traceEvents.filter(e=>e.name==='RunTask');assert.ok(draws.length>0);
    const custom=trace.traceEvents.filter(e=>e.ph==='b').map(e=>({event:e,detail:JSON.parse(e.args.detail).devtools}));
    for(const lane of ['Commands','Tasks','Tracing spans','HTTP lifecycles'])assert.ok(custom.some(e=>e.detail.track===lane),`missing ${lane}`);
    const callback=trace.traceEvents.find(e=>e.cat==='rake.callbacks'&&e.args.data.url.startsWith('http'));
    assert.ok(callback,'real callback source');
    const source=await fetch(callback.args.data.url);assert.equal(source.status,200);assert.ok((await source.text()).length>0);
    const invalid=callback.args.data.url.replace(/\/frame\/\d+\/\d+\//,'/frame/999999999/99999/');assert.equal((await fetch(invalid)).status,404);
    const resource=await call('Network.loadNetworkResource',{url:callback.args.data.url,options:{disableCache:false,includeCredentials:false}});
    assert.equal(resource.resource.success,true);
    await call('IO.close',{handle:resource.resource.stream});
    const old=done.stream;
    await start();const empty=await complete();assert.notEqual(old,empty.stream);
    await assert.rejects(call('IO.read',{handle:old}),/Unknown|expired/);
    await assert.rejects(call('IO.close',{handle:old}),/Unknown|expired/);
    assert.ok((await read(empty.stream)).trace.traceEvents.length>0);
    await call('IO.close',{handle:empty.stream});
    await start('ReportEvents');await work();await delay(600);const report=await complete();
    assert.equal(report.stream,undefined);assert.ok(events.some(e=>e.method==='Tracing.dataCollected'&&e.params.value.some(v=>v.name==='Rake recording started')));
    await start();await work();await restart();const replaced=await complete(false);
    assert.equal(replaced.dataLossOccurred,true);assert.ok((await read(replaced.stream)).trace.traceEvents.length>0);
    const successor=await eventually(async()=>{const t=(await (await fetch(url+'/json/list')).json())[0];return t&&t.id!==target.id?t:undefined;},60000);
    // Existing socket follows replacement. A fresh recording has a fresh clock.
    await eventually(async()=>{try{await start();return true;}catch{return false;}},60000);
    await work();await delay(700);const next=await complete();assert.equal(next.dataLossOccurred,false);
    await read(next.stream);await call('IO.close',{handle:next.stream});
    await start('ReportEvents');await work();await restart();
    await eventually(()=>events.find(e=>e.method==='Page.frameNavigated'&&e.params.frame.loaderId!==successor.id),60000);
    assert.ok(!events.some(e=>e.method==='Tracing.tracingComplete'),'Chrome needs completion after Stop');
    const replacedReport=await complete();assert.equal(replacedReport.dataLossOccurred,true);
    assert.ok(events.some(e=>e.method==='Tracing.dataCollected'));
    await start();await work();
    await call('Runtime.evaluate',{expression:'test.console({"mode":"crash"})',returnByValue:true});
    const crashed=await complete(false);assert.equal(crashed.dataLossOccurred,true);
    const partial=await read(crashed.stream);assert.ok(partial.trace.traceEvents.length>0);
    await assert.rejects(call('Runtime.evaluate',{expression:'rake.help()'}),/disconnect|detach|unavailable|disappear/i);
    await call('IO.close',{handle:crashed.stream});
    console.log(JSON.stringify({passed:true,draws:draws.length,slowDraws:draws.filter(e=>e.dur>16667).length,tracks:[...new Set(custom.map(e=>e.detail.track))],successor:successor.id,trace:'/tmp/rake-performance-trace.json'}));
  }
} finally {await stop();}
