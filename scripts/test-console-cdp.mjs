// Covers: native Console logs/sources, command outcomes, clear, restart and crash retention.
// Real: GPUI application, development supervisor, private IPC, CDP/WS and source files.
// Doubles: none. Requires macOS GUI, Node 22+, built rake CLI and console-test routing fixture.
// This exercises the adapter; stock Chrome UI verification is documented separately.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {readFileSync, writeFileSync, unlinkSync, rmSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import {createInterface} from 'node:readline';
import {setTimeout as delay} from 'node:timers/promises';

assert.equal(process.platform,'darwin','requires a real macOS GUI; never silently skip');
const root=fileURLToPath(new URL('../',import.meta.url)), cli=join(root,'target/debug/rake');
const serve=process.argv.includes('--serve'), lifecycle=[], events=[], pending=new Map();
const trigger=join(root,`examples/routing/assets/.rake-console-test-${process.pid}.txt`);
let socket, selectedAdapter, session, url, sequence=0, intentionalClose=false, triggerCreated=false;
const supervisor=spawn(cli,['dev','--target','desktop','--package','rake-routing','--features','console-test','--cdp','--no-open','--json'],{cwd:root,stdio:['ignore','pipe','pipe']});
let stderr=''; supervisor.stderr.on('data',data=>{stderr=(stderr+data).slice(-16000);});
createInterface({input:supervisor.stdout}).on('line',line=>{
  try {const event=JSON.parse(line);lifecycle.push(event); if(event.event==='session'&&!session&&/^\/tmp\/rake-dev-[A-Za-z0-9_-]+$/.test(event.session))session=event.session;if(event.event==='cdp_listening')url=event.url;} catch {}
});
async function eventually(check,timeout=30000){const end=Date.now()+timeout;let last;while(Date.now()<end){try{const value=await check();if(value)return value;}catch(error){last=error;}if(supervisor.exitCode!==null)throw new Error(`supervisor exited: ${stderr}`);await delay(50);}throw new Error(`Console verification timed out: ${stderr}`,{cause:last});}
function call(method,params={}){return new Promise((resolve,reject)=>{const id=++sequence,timer=setTimeout(()=>{pending.delete(id);reject(new Error(`${method} timed out`));},8000);pending.set(id,{resolve,reject,timer});socket.send(JSON.stringify({id,method,params}));});}
async function evaluate(expression,params={}){return call('Runtime.evaluate',{expression,returnByValue:true,...params});}
function entries(){return events.filter(event=>event.method==='Log.entryAdded').map(event=>event.params.entry);}
async function stop(){intentionalClose=true;socket?.close();selectedAdapter?.kill('SIGTERM');for(const item of pending.values())clearTimeout(item.timer);supervisor.kill('SIGINT');await Promise.race([new Promise(resolve=>supervisor.once('exit',resolve)),delay(12000)]);if(supervisor.exitCode===null)supervisor.kill('SIGTERM');if(triggerCreated)try{unlinkSync(trigger);}catch(error){if(error.code!=='ENOENT')throw error;}if(session&&(supervisor.exitCode!==null||supervisor.signalCode!==null))rmSync(session,{recursive:true,force:true});}
try{
  await eventually(()=>session&&url&&lifecycle.some(event=>event.event==='watching'&&event.runtime_id),90000);
  let target=await eventually(async()=> (await (await fetch(url+'/json/list')).json())[0]);
  console.log(JSON.stringify({session,devtoolsFrontendUrl:target.devtoolsFrontendUrl}));
  if(serve){await new Promise(resolve=>{process.once('SIGINT',resolve);process.once('SIGTERM',resolve);supervisor.once('exit',resolve);});}
  else{
    selectedAdapter=spawn(cli,['cdp','--session',session,'--instance',target.id,'--no-open','--json'],{cwd:root,stdio:['ignore','pipe','pipe']});
    selectedAdapter.stderr.on('data',data=>{stderr=(stderr+data).slice(-16000);});
    target=await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(new Error('UUID-selected adapter timed out')),10000);createInterface({input:selectedAdapter.stdout}).on('line',line=>{try{const value=JSON.parse(line);if(value.event==='cdp_listening'){clearTimeout(timer);resolve(value.targets[0]);}}catch{}});});
    socket=new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((resolve,reject)=>{socket.addEventListener('open',resolve,{once:true});socket.addEventListener('error',reject,{once:true});});
    socket.addEventListener('message',event=>{const value=JSON.parse(event.data),item=pending.get(value.id);if(item){pending.delete(value.id);clearTimeout(item.timer);value.error?item.reject(new Error(JSON.stringify(value.error))):item.resolve(value.result);}else if(value.method&&events.length<1024)events.push(value);});
    socket.addEventListener('error',()=>{if(!intentionalClose)for(const item of pending.values())item.reject(new Error('WebSocket failed'));});
    await call('Network.enable'); // Advancing Network must not consume the Console backlog.
    await call('Runtime.enable');await call('Log.enable');
    assert.ok((await evaluate('rake.help()')).result.value.undoText);
    const registry=(await evaluate('rake.commands()')).result.value;
    assert.ok(registry.some(command=>command.id==='test.console'&&command.arguments));
    const names=(await call('Runtime.globalLexicalScopeNames')).names;
    assert.ok(names.includes('demo')&&names.includes('rake'));
    const namespace=await evaluate('demo',{returnByValue:false,throwOnSideEffect:true,objectGroup:'completion'});
    assert.ok((await call('Runtime.getProperties',{objectId:namespace.result.objectId})).result.some(p=>p.name==='echo'));
    const log=await evaluate('test.console({"mode":"log"})');
    assert.equal(log.result.value.status,'committed');
    await eventually(()=>['info','warning','error'].every(level=>entries().some(entry=>entry.text.startsWith('Console fixture '+level))));
    await eventually(()=>entries().some(entry=>entry.text.includes('[stdout] Console fixture stdout')));
    assert.equal(entries().filter(entry=>entry.text.startsWith('Console fixture info')).length,1);
    assert.ok(!entries().some(entry=>entry.text.includes('[stderr] INFO rake_routing::console_fixture')),'structured mirrors must not duplicate logs');
    const info=entries().find(entry=>entry.text.startsWith('Console fixture info'));
    const warning=entries().find(entry=>entry.text.startsWith('Console fixture warning'));
    const error=entries().find(entry=>entry.text.startsWith('Console fixture error'));
    assert.equal(warning.level,'warning');assert.equal(error.level,'error');
    assert.ok(info.url.endsWith('console_fixture.rs')&&info.lineNumber>0);
    const linked=await fetch(info.url);
    assert.equal(linked.status,200); assert.ok(linked.headers.get('content-type').startsWith('text/plain'));
    assert.equal(linked.headers.get('x-content-type-options'),'nosniff');
    assert.ok((await linked.text()).includes('Console fixture info'));
    const uncapturedSource=new URL(info.url);uncapturedSource.pathname+='.not-captured';
    assert.equal((await fetch(uncapturedSource)).status,404,'source access requires the exact captured file URL');
    const resource=(await call('Network.loadNetworkResource',{frameId:'rake',url:info.url,options:{disableCache:true,includeCredentials:false}})).resource;
    let source='';for(;;){const part=await call('IO.read',{handle:resource.stream});source+=part.data;if(part.eof)break;}await call('IO.close',{handle:resource.stream});
    assert.ok(source.includes('Console fixture info'));
    await assert.rejects(call('Page.getResourceContent',{frameId:'rake',url:'rake://source/../../private.rs'}),/not captured/);
    const object=await evaluate('demo.echo({"message":"group one"})',{returnByValue:false,objectGroup:'one'});
    const other=await evaluate('demo.echo({"message":"group two"})',{returnByValue:false,objectGroup:'two'});
    const nested=(await call('Runtime.getProperties',{objectId:object.result.objectId})).result.find(p=>p.name==='operation').value.objectId;
    await call('Runtime.releaseObjectGroup',{objectGroup:'one'});
    await assert.rejects(call('Runtime.getProperties',{objectId:nested}),/expired/);
    assert.ok((await call('Runtime.getProperties',{objectId:other.result.objectId})).result.length);
    assert.equal((await evaluate('test.console({"mode":"fail"})')).result.value.status,'failed');
    const slow=(await evaluate('test.console({"mode":"slow"})')).result.value;
    assert.ok(['queued','running'].includes(slow.status));
    await eventually(async()=> (await evaluate(`rake.operation(${JSON.stringify(slow.observe)})`)).result.value?.status==='committed');
    const invalid=await evaluate('1+1');assert.equal(invalid.result.subtype,'error');assert.ok(invalid.exceptionDetails);
    await call('Runtime.discardConsoleEntries');await call('Log.clear');events.length=0;
    await evaluate('demo.echo({"message":"after clear"})');
    await eventually(async()=>JSON.parse(readFileSync(join(session,'status.json'))).phase==='watching');
    assert.ok(!entries().some(entry=>entry.text.startsWith('Console fixture info')),'cleared records must not replay');
    writeFileSync(trigger,'checkpointed Console restart\n',{flag:'wx'});triggerCreated=true;
    await eventually(()=>events.some(event=>event.method==='Runtime.executionContextCreated'&&event.params.context.uniqueId!==target.id),60000);
    assert.ok(events.some(event=>event.method==='Page.frameNavigated'));
    await assert.rejects(call('Runtime.getProperties',{objectId:other.result.objectId}),/expired/);
    assert.ok((await evaluate(`rake.operation(${JSON.stringify(slow.observe)})`)).exceptionDetails,'old operation tokens must not retarget');
    const successor=events.find(event=>event.method==='Runtime.executionContextCreated'&&event.params.context.uniqueId!==target.id).params.context.uniqueId;
    await evaluate('test.console({"mode":"log"})');
    const successorSource=await eventually(()=>entries().find(entry=>entry.text==='Console fixture info'&&entry.url.includes(successor)));
    assert.equal((await fetch(successorSource.url)).status,200,'source links must follow a successor even when --instance named the old UUID');
    await evaluate('test.console({"mode":"crash"})');
    await eventually(()=>entries().some(entry=>entry.text.includes('Intentional Console fixture panic')));
    await eventually(()=>entries().some(entry=>entry.text.includes('stack backtrace')));
    await eventually(()=>entries().some(entry=>entry.text.includes('Application exited with code 73')));
    console.log('Native Console/CDP passed: sources, one delivery, severities, nested groups, commands, pending/failed results, clear, checkpointed restart, expired tokens and retained panic/backtrace/exit output.');
  }
}finally{await stop();}
