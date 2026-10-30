// Covers: real HTTP metadata/bodies/timing/origins, failures, recording and replacement.
// Real: GPUI routing fixture, supervisor, Rake HTTP, private IPC, CDP and loopback HTTP.
// Doubles: deterministic HTTP responses. Requires macOS GUI, Node 22+, built rake CLI.
// Stock Chrome UI verification is separate; this script never controls Chrome.
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
const trigger = join(root, `examples/routing/assets/.rake-network-test-${process.pid}.txt`);
let socket, session, url, sequence = 0, triggerCreated = false, closing = false;
let stderr = '';
const timers = new Set();
const http = createServer((request, response) => {
  const path = new URL(request.url, 'http://fixture').pathname;
  const chunks = [];
  request.on('data', chunk => chunks.push(chunk));
  request.on('end', () => {
    if (path === '/fail') { request.socket.destroy(); return; }
    if (path === '/redirect') { response.writeHead(302, {location:'/json'}); response.end(); return; }
    if (path === '/slow') {
      response.writeHead(200, {'content-type':'text/plain', 'content-length':4});
      response.flushHeaders();
      const timer = setTimeout(() => { timers.delete(timer); response.end('slow'); }, 300);
      timers.add(timer); return;
    }
    const body = path === '/binary' ? Buffer.from([0,255,128,42]) :
      path === '/large' ? Buffer.alloc(65537, 120) :
      path === '/error' ? Buffer.from('intentional HTTP error') :
      Buffer.from(JSON.stringify({message:'café 你好 🙂',received:Buffer.concat(chunks).toString(),path:request.url}));
    response.writeHead(path === '/error' ? 503 : 200, {
      'content-type':path === '/binary' ? 'application/octet-stream' : path === '/error' ? 'text/plain' : 'application/json; charset=utf-8',
      'content-length':body.length, 'x-fixture':['one','two'], 'set-cookie':'private=value',
    });
    response.end(body);
  });
});
await new Promise(resolve => http.listen(0, '127.0.0.1', resolve));
const base = `http://127.0.0.1:${http.address().port}`;
const supervisor = spawn(cli, ['dev','--target','desktop','--package','rake-routing','--features','network-test','--cdp','--no-open','--json'], {cwd:root,stdio:['ignore','pipe','pipe']});
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
  throw new Error(`Network verification timed out: ${stderr}`, {cause:last});
}
function call(method, params={}) {
  return new Promise((resolve,reject) => {
    const id=++sequence, timer=setTimeout(() => { pending.delete(id);reject(new Error(`${method} timed out`)); },10000);
    pending.set(id,{resolve,reject,timer});socket.send(JSON.stringify({id,method,params}));
  });
}
async function request(path, mode='get') {
  const expression=`test.network(${JSON.stringify({url:base+path,mode})})`;
  const result=await call('Runtime.evaluate',{expression,returnByValue:true});
  assert.ok(!result.exceptionDetails, JSON.stringify(result));
  return result.result.value;
}
const network = method => events.filter(event => event.method === `Network.${method}`).map(event=>event.params);
async function finished(path, mode='get') {
  const before=new Set(network('requestWillBeSent').map(event=>event.requestId));
  const result=await request(path,mode);
  const start=await eventually(()=>network('requestWillBeSent').find(event=>!before.has(event.requestId)&&new URL(event.request.url).pathname===path.split('?')[0]));
  await eventually(()=>[...network('loadingFinished'),...network('loadingFailed')].some(event=>event.requestId===start.requestId));
  return {start,result};
}
async function seed() {
  await finished('/json?q=hello+world&token=secret','post');
  await finished('/binary');await finished('/error');await finished('/redirect');
  await finished('/slow','cancel');await finished('/slow','timeout');await finished('/slow','abandon');
  await finished('/fail');
  console.log('Seeded success, POST, binary, redirect, HTTP error, cancellation, timeout, abandonment and transport failure.');
}
async function restart() {
  await eventually(()=>JSON.parse(readFileSync(join(session,'status.json'))).phase==='watching');
  if (triggerCreated) throw new Error('This fixture permits one owned restart');
  writeFileSync(trigger,'checkpointed Network restart\n',{flag:'wx'});triggerCreated=true;
}
async function stop() {
  closing=true;process.stdin.pause();socket?.close();for(const item of pending.values())clearTimeout(item.timer);
  supervisor.kill('SIGINT');
  await Promise.race([new Promise(resolve=>supervisor.once('exit',resolve)),delay(12000)]);
  if(supervisor.exitCode===null)supervisor.kill('SIGTERM');
  for(const timer of timers)clearTimeout(timer);http.closeAllConnections();http.close();
  if(triggerCreated)try{unlinkSync(trigger);}catch(error){if(error.code!=='ENOENT')throw error;}
  if(session&&(supervisor.exitCode!==null||supervisor.signalCode!==null))rmSync(session,{recursive:true,force:true});
}
try {
  await eventually(()=>session&&url&&lifecycle.some(event=>event.event==='watching'&&event.runtime_id),90000);
  const target=await eventually(async()=> (await (await fetch(url+'/json/list')).json())[0]);
  console.log(JSON.stringify({session,base,devtoolsFrontendUrl:target.devtoolsFrontendUrl}));
  socket=new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve,reject)=>{socket.addEventListener('open',resolve,{once:true});socket.addEventListener('error',reject,{once:true});});
  socket.addEventListener('message',event=>{
    const value=JSON.parse(event.data),item=pending.get(value.id);
    if(item){pending.delete(value.id);clearTimeout(item.timer);value.error?item.reject(new Error(JSON.stringify(value.error))):item.resolve(value.result);}
    else if(value.method&&events.length<4096)events.push(value);
  });
  socket.addEventListener('error',()=>{if(!closing)for(const item of pending.values())item.reject(new Error('WebSocket failed'));});
  await call('Network.enable');await call('Runtime.enable');
  if(serve) {
    await seed();console.log('UI fixture ready. stdin: seed | restart | quit');
    await new Promise(resolve=>{
      const input=createInterface({input:process.stdin});let work=Promise.resolve();
      input.on('line',line=>{work=work.then(async()=>{if(line==='seed')await seed();if(line==='restart')await restart();if(line==='quit')resolve();}).catch(error=>console.error(error));});
      process.once('SIGINT',resolve);process.once('SIGTERM',resolve);supervisor.once('exit',resolve);
    });
  } else {
    const {start,result}=await finished('/json?q=hello+world&token=secret','post');
    assert.equal(result.status,'committed');
    assert.ok(start.requestId.startsWith(target.id+':'));
    assert.equal(start.loaderId,target.id);
    assert.ok(start.request.url.includes('q=hello+world&token=%5Bredacted%5D'));
    assert.equal(start.request.headers.authorization,'[redacted]');assert.equal(start.request.headers['x-rake-fixture'],'network');
    const origin=start.initiator.stack.callFrames[0];assert.ok(origin.functionName.includes('command_id=test.network'));assert.ok(origin.lineNumber>0);
    const source=await fetch(origin.url);assert.equal(source.status,200);assert.ok(origin.url.endsWith('network_fixture.rs'));assert.ok((await source.text()).includes('x-rake-fixture'));
    const post=await call('Network.getRequestPostData',{requestId:start.requestId});assert.equal(JSON.parse(post.postData).message,'café 你好 🙂');
    const response=network('responseReceived').find(event=>event.requestId===start.requestId).response;
    assert.equal(response.status,200);assert.equal(response.mimeType,'application/json');assert.equal(response.headers['set-cookie'],'[redacted]');
    assert.equal(response.headers['x-fixture'],'one\ntwo');assert.ok(response.timing.receiveHeadersEnd>=0);assert.equal(response.timing.dnsStart,-1);
    const body=await call('Network.getResponseBody',{requestId:start.requestId});assert.equal(body.base64Encoded,false);assert.equal(JSON.parse(body.body).message,'café 你好 🙂');
    const matches=await call('Network.searchInResponseBody',{requestId:start.requestId,query:'CAFÉ'});assert.equal(matches.result.length,1);
    const binary=await finished('/binary');assert.deepEqual(await call('Network.getResponseBody',{requestId:binary.start.requestId}),{body:'AP+AKg==',base64Encoded:true});
    const large=await finished('/large');await assert.rejects(call('Network.getResponseBody',{requestId:large.start.requestId}),/64 KiB/);
    const error=await finished('/error');assert.equal(network('responseReceived').find(e=>e.requestId===error.start.requestId).response.status,503);assert.equal((await call('Network.getResponseBody',{requestId:error.start.requestId})).body,'intentional HTTP error');
    const redirected=await finished('/redirect');assert.ok(network('responseReceived').find(e=>e.requestId===redirected.start.requestId).response.url.endsWith('/json'));
    for(const [mode,phase] of [['cancel','cancelled'],['timeout','timed_out'],['abandon','abandoned'],['get','failed']]) {
      const failed=await finished(mode==='get'?'/fail':'/slow',mode);
      const end=network('loadingFailed').find(e=>e.requestId===failed.start.requestId);assert.equal(end.errorText,phase);assert.equal(end.canceled,['cancelled','abandoned'].includes(phase));
      await assert.rejects(call('Network.getResponseBody',{requestId:failed.start.requestId}),/incomplete/);
    }
    const before=network('requestWillBeSent').length;
    await call('Network.disable');await request('/json?paused=yes');await call('Network.enable');await request('/json?resumed=yes');
    await eventually(()=>network('requestWillBeSent').length>before);
    assert.ok(!network('requestWillBeSent').some(event=>event.request.url.includes('paused=yes')));
    await call('Runtime.discardConsoleEntries');await call('Log.clear');
    assert.equal((await call('Network.getResponseBody',{requestId:start.requestId})).body,body.body,'Console Clear must not erase Network');
    assert.ok((await call('Page.getResourceContent',{frameId:'rake',url:origin.url})).content.includes('x-rake-fixture')); 
    await restart();
    const navigation=await eventually(()=>events.find(event=>event.method==='Page.frameNavigated'&&event.params.frame.loaderId!==target.id),60000);
    const successor=navigation.params.frame.loaderId;
    const after=await finished('/json?successor=yes');assert.ok(after.start.requestId.startsWith(successor+':'));
    assert.equal((await call('Network.getResponseBody',{requestId:start.requestId})).body,body.body,'Preserve log body across replacement');
    assert.ok(after.start.timestamp > start.timestamp, 'Network monotonic timeline must continue across runtimes');
    await assert.rejects(call('Network.getResponseBody',{requestId:target.id+':999999'}),/expired runtime/);
    console.log('Native Network/CDP passed: headers/query/payload, Unicode/binary/limits, origins, timings, redirects, HTTP/transport failures, cancellation/timeout/abandonment, recording and preserved bodies.');
  }
} finally { await stop(); }
