// Covers: captured Rust source documents, cross-tab links, search, source streams and replacement.
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
const trigger = join(root, `examples/routing/assets/.rake-sources-test-${process.pid}.txt`);
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
  throw new Error(`Sources verification timed out: ${stderr}`, {cause:last});
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
  writeFileSync(trigger,`checkpointed Sources restart ${restartCount}\n`,{flag:triggerCreated?'w':'wx'});triggerCreated=true;
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

function documents(tree) {return tree.frameTree.childFrames.map(child=>child.frame);}
async function sourceTree() {return documents(await call('Page.getResourceTree'));}
async function command(expression) {
  const result=await call('Runtime.evaluate',{expression,returnByValue:true});
  assert.ok(!result.exceptionDetails,JSON.stringify(result));return result.result.value;
}
try {
  await eventually(()=>session&&url&&lifecycle.some(e=>e.event==='watching'&&e.runtime_id),90000);
  const target=await eventually(async()=> (await (await fetch(url+'/json/list')).json())[0]);
  console.log(JSON.stringify({session,base,devtoolsFrontendUrl:target.devtoolsFrontendUrl}));
  await connect(target);
  await call('Page.enable');await call('Runtime.enable');await call('Log.enable');await call('Network.enable');
  if(serve) {
    console.log('Sources UI fixture ready. stdin: work | log | restart | quit');
    await new Promise(resolve=>{
      const input=createInterface({input:process.stdin});let queue=Promise.resolve();
      input.on('line',line=>{queue=queue.then(async()=>{
        if(line==='work'){await work();console.log('Performance/HTTP workload complete.');}
        if(line==='log'){await command('test.console({"mode":"log"})');console.log('Console fixture log complete.');}
        if(line==='restart')await restart();if(line==='quit')resolve();
      }).catch(console.error);});
      process.once('SIGINT',resolve);process.once('SIGTERM',resolve);supervisor.once('exit',resolve);
    });
  } else {
    const docs=await sourceTree();assert.ok(docs.length>=3,JSON.stringify(docs));
    const status=await call('Rake.getSources');assert.equal(status.omitted,false);assert.ok(docs.every(d=>status.files.some(file=>file.url===d.url))); 
    const frameTree=await call('Page.getFrameTree');assert.ok(docs.every(d=>frameTree.frameTree.childFrames.some(child=>child.frame.url===d.url))); 
    assert.equal(new Set(docs.map(d=>d.url)).size,docs.length,'one catalog entry per file');
    const lib=docs.find(d=>d.url.endsWith('/examples/routing/src/lib.rs'));assert.ok(lib);
    const content=await call('Page.getResourceContent',{frameId:lib.id,url:lib.url});
    assert.ok(content.content.includes('routing.screen'));assert.equal(content.base64Encoded,false);
    assert.equal(await (await fetch(lib.url)).text(),content.content);
    const matches=await call('Page.searchInResource',{frameId:lib.id,url:lib.url,query:'routing.screen'});
    assert.ok(matches.result.length>0);const line=matches.result[0].lineNumber;
    assert.ok(content.content.split('\n')[line].includes('routing.screen'));
    const location=await call('Rake.getSourceLocation',{url:lib.url,lineNumber:line,columnNumber:0});
    assert.equal(location.line,line+1);assert.equal(location.column,1);assert.equal(location.url,lib.url);
    await assert.rejects(call('Rake.getSourceLocation',{url:lib.url,lineNumber:9999999}),/outside/);
    await assert.rejects(call('Page.searchInResource',{url:lib.url,query:'[',isRegex:true}),/regex|unclosed|parse/i);
    const uncaptured=lib.url.replace('examples/routing/src/lib.rs','crates/rake-cli/src/main.rs');
    assert.equal((await fetch(uncaptured)).status,404);
    await assert.rejects(call('Page.getResourceContent',{frameId:'rake',url:uncaptured}),/captured/);
    await assert.rejects(call('Debugger.getScriptSource',{scriptId:lib.url}),/Unsupported/);
    const resource=await call('Network.loadNetworkResource',{url:lib.url,options:{disableCache:false,includeCredentials:false}});
    const handle=resource.resource.stream;let streamed='';
    for(;;){const c=await call('IO.read',{handle,size:4093});streamed+=c.data;if(c.eof)break;}
    assert.equal(streamed,content.content);await call('IO.close',{handle});
    await call('DOM.enable');await call('DOM.getDocument',{depth:2});
    const search=await call('DOM.performSearch',{query:'routing.screen'});
    const nodes=await call('DOM.getSearchResults',{searchId:search.searchId,fromIndex:0,toIndex:1});
    const site=await call('Rake.getSourceLocation',{nodeId:nodes.nodeIds[0]});assert.equal(site.url,lib.url);
    const helper=await command(`rake.source(${nodes.nodeIds[0]})`);assert.equal(helper.url,lib.url);
    await command('test.console({"mode":"log"})');await work();
    const log=await eventually(()=>events.find(e=>e.method==='Log.entryAdded'&&e.params.entry.url?.endsWith('/console_fixture.rs')));
    assert.ok(log.params.entry.url.includes('/source/workspace/'));
    const request=await eventually(()=>events.find(e=>e.method==='Network.requestWillBeSent'&&e.params.request.url.includes('from=performance')));
    assert.ok(request.params.initiator.stack.callFrames[0].url.endsWith('/source/workspace/examples/routing/src/performance_fixture.rs'));
    await start();await work();await delay(1100);const done=await complete();const trace=(await read(done.stream)).trace;
    const callback=trace.traceEvents.find(e=>e.cat==='rake.callbacks'&&e.args.data.url===lib.url);assert.ok(callback,'Performance uses the same source URL');
    await call('IO.close',{handle:done.stream});
    const open=await call('Network.loadNetworkResource',{url:lib.url,options:{disableCache:false,includeCredentials:false}});
    await restart();
    await eventually(()=>events.find(e=>e.method==='Page.frameNavigated'&&e.params.frame.id==='rake'&&e.params.frame.loaderId!==target.id),180000);
    const next=await sourceTree();const successor=next.find(d=>d.url.endsWith('/examples/routing/src/lib.rs'));assert.ok(successor);assert.notEqual(successor.url,lib.url);
    await assert.rejects(call('IO.read',{handle:open.resource.stream}),/expired/);
    await assert.rejects(call('Page.getResourceContent',{frameId:lib.id,url:lib.url}),/captured|expired/);
    assert.equal((await fetch(lib.url)).status,404);
    assert.ok((await call('Page.getResourceContent',{frameId:successor.id,url:successor.url})).content.includes('routing.screen'));
    const replacement=(await call('Rake.getSources')).runtimeId;
    socket.close();await new Promise(resolve=>socket.addEventListener('close',resolve,{once:true}));
    await eventually(async()=> (await fetch(successor.url)).status===404);
    console.log(JSON.stringify({passed:true,files:docs.length,source:lib.name,searchLine:line+1,replacement}));
  }
} finally {await stop();}
