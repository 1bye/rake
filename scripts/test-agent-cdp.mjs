// Covers: complete public agent workflow against a real native GPUI window.
// Real: app/GPU, Unix control, Rust CLI/CDP, Node client, ScreenCaptureKit image/source.
// Doubles: none. Requires: unlocked macOS 14+, Screen Recording, Node22+, Xcode tools.
// Build: cargo build -p rake-cli -p rake-agent-example --features rake-agent-example/devtools
// Run: node scripts/test-agent-cdp.mjs
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {spawn,spawnSync,execFileSync} from 'node:child_process';
import {mkdtempSync,mkdirSync,openSync,closeSync,writeFileSync,readFileSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import {createInterface} from 'node:readline';
import {setTimeout as delay} from 'node:timers/promises';
import {RakeAgent} from './rake-agent.mjs';
assert.equal(process.platform,'darwin','requires a native macOS GUI; never silently skip');
const root=fileURLToPath(new URL('../',import.meta.url)), directory=mkdtempSync('/tmp/rake-agent-cdp-');
const control=join(directory,'control');mkdirSync(control,{mode:0o700});
const http=createServer((_,res)=>{res.writeHead(503,{'content-type':'text/plain'});res.end('native diagnostic fixture');});
await new Promise(resolve=>http.listen(0,'127.0.0.1',resolve));
const env={...process.env,RAKE_DEV_CONTROL:'1',RAKE_CONTROL_DIR:control,RAKE_DEV_PREVIEW:'1',RAKE_AGENT_HTTP_URL:`http://127.0.0.1:${http.address().port}/failure`};
const log=openSync(join(directory,'runtime.log'),'w',0o600),children=[];
const rake=join(root,'target/debug/rake');let client, native;
async function eventually(check,timeoutMs=20000) {
  const until=Date.now()+timeoutMs;let last;
  while(Date.now()<until) {try {const result=await check();if(result)return result;}catch(error){last=error;}await delay(50);}
  throw new Error('Native readiness deadline exceeded',{cause:last});
}
try {
  children.push(spawn('caffeinate',['-d','-i','-t','180'],{stdio:'ignore'}));
  native=spawn(join(root,'target/debug/rake-agent-example'),[],{cwd:root,env,stdio:['ignore',log,log]});children.push(native);
  await eventually(()=>JSON.parse(execFileSync(rake,['ctl','--json','inspect','tree'],{env,timeout:4000,stdio:['ignore','pipe','ignore']})).value?.elements.some(n=>n.semantic_id==='agent-increment'));
  const adapter=spawn(rake,['cdp','--session',directory,'--capture','--no-open','--json'],{cwd:root,env,stdio:['ignore','pipe',log]});children.push(adapter);
  const endpoint=await new Promise((resolve,reject)=>{
    const lines=createInterface({input:adapter.stdout}),timer=setTimeout(()=>reject(new Error('Capture helper preparation/discovery timed out')),30000);
    lines.on('line',line=>{try{const value=JSON.parse(line);if(value.event==='cdp_listening'){clearTimeout(timer);resolve(value.url);}}catch{}});
    adapter.once('error',reject);adapter.once('exit',code=>{if(code)reject(new Error(`Adapter exited ${code}`));});
  });
  client=await RakeAgent.connect(endpoint);
  assert.equal(client.capabilities.methods['Input.insertText'].supported,true);
  assert.equal(client.capabilities.methods['Page.captureScreenshot'].supported,true,JSON.stringify(await client.call('Rake.getCaptureStatus')));
  assert.equal((await client.windows()).length,1);
  console.log('Connected native runtime, selected window, discovered capabilities');
  const increment=await client.find({role:'button',name:'Increment counter'});
  assert.ok(increment.stableId);assert.ok(increment.source.file.endsWith('examples/agent/src/main.rs'));
  const accepted=await client.click(increment);
  await client.waitFor({text:'Count: 1'},{afterFrame:accepted.beforeFrame});
  assert.equal((await client.find({id:'agent-increment'})).stableId,increment.stableId);
  console.log('Semantic lookup → native click → later rendered count; stable identity retained');
  const input=await client.find({role:'TextInput',name:'Message'});
  await client.click(input);await client.typeText('café 你好 🙂');
  await client.waitFor({text:'Text: café 你好 🙂'});
  await client.key('a',4);await client.typeText('replaced 🙂');
  await client.waitFor({text:'Text: replaced 🙂'});
  await client.key('F2');await client.waitFor({text:'Keys: 1; ticks: 0'});
  console.log('Focused Unicode text input, select-all replacement, native keyboard shortcut');
  await client.drag(await client.find({id:'agent-drag'}),60,0);
  const dragged=await client.waitFor({textContains:'Drags:'});
  assert.notEqual(dragged.observation.elements[0].text,'Drags: 0');
  const row=await client.find({id:'agent-row-1'}), scroll=await client.find({id:'agent-scroll'});
  const scrolled=await client.scroll(scroll,140);
  await client.waitFor({id:'agent-row-1',visible:false},{afterFrame:scrolled.beforeFrame});
  assert.ok((await client.find({id:'agent-row-1'})).bounds.y < row.bounds.y);
  console.log('Native drag and scrolling changed captured geometry/clipping');
  const source=await client.source(increment);
  assert.ok(source.text.includes('agent-increment'),JSON.stringify(source));
  const image=await client.screenshot({format:'png'});
  assert.deepEqual([...image.bytes.subarray(0,8)],[137,80,78,71,13,10,26,10]);
  assert.ok(image.metadata.image.width>0 && image.metadata.image.height>0);
  const jpeg=await client.screenshot({format:'jpeg',quality:70});
  assert.deepEqual([...jpeg.bytes.subarray(0,3)],[255,216,255]);
  assert.equal(jpeg.metadata.image.kind,'jpeg');
  const imagePath=join(directory,'native-agent.png');writeFileSync(imagePath,image.bytes,{mode:0o600});
  const cliResult=JSON.parse(execFileSync(rake,['agent','--url',endpoint,JSON.stringify({action:'find',query:{text:'Count: 1'}})],{env,timeout:10000,encoding:'utf8'}));
  assert.equal(cliResult.text,'Count: 1');
  const invalid=spawnSync(rake,['agent','--url',endpoint,'{bad'],{env,timeout:5000,encoding:'utf8'});
  assert.equal(invalid.status,1);assert.equal(invalid.stdout,'');assert.match(invalid.stderr,/JSON|property name/i);
  console.log('Rust source snippet, real native PNG, bundled rake agent CLI');
  await client.key('F5');
  await eventually(async()=>{await client.call('Input.dispatchMouseEvent',{type:'mouseMoved',x:1,y:1});return (await client.snapshot({text:'HTTP: 503'})).totalMatches===1;});
  const evidence=await client.evidence({query:{id:'agent-increment'},limit:64});
  for(const name of ['tree','selected','source','diagnostics','screenshot']) assert.ok(['ok','partial'].includes(evidence.sections[name].status),JSON.stringify(evidence.sections[name]));
  assert.equal(evidence.sections.selected.data.id,'agent-increment');
  assert.ok(evidence.sections.source.data.text.includes('agent-increment'));
  assert.ok(evidence.sections.diagnostics.data.logs.some(r=>r.fields.message==='Agent HTTP diagnostic'));
  assert.ok(evidence.sections.diagnostics.data.networkErrors.some(r=>r.fields.status===503));
  assert.ok(evidence.sections.screenshot.data.data.length>0);
  assert.equal(evidence.sections.tree.data.revision.capture,evidence.revision.capture);
  assert.ok(Buffer.byteLength(JSON.stringify(evidence))<900*1024);
  const partial=await client.evidence({query:{id:'missing'},screenshot:false,limit:1});
  assert.equal(partial.complete,false);assert.equal(partial.sections.selected.status,'error');assert.equal(partial.sections.screenshot.status,'omitted');
  assert.equal(partial.sections.tree.status,'partial');
  console.log('Evidence bundle contains correlated tree/selection, source, logs, HTTP failure and independent image');

  await assert.rejects(client.find({}),/exactly one/);
  await assert.rejects(client.find({id:'missing'}),/found 0/);
  await assert.rejects(client.waitFor({id:'missing'},{timeoutMs:20}),/timed_out/);
  const wait=await client.call('Rake.waitFor',{query:{id:'missing'},timeoutMs:5000});
  assert.equal((await client.call('Rake.cancelWait',{waitId:wait.waitId})).status,'cancelled');
  const point={type:'mousePressed',x:20,y:20,button:'left',rakeWindowId:client.windowId};
  await assert.rejects(client.call('Input.dispatchMouseEvent',{...point,rakeCapture:'retired'}),e=>e.code===-32010);
  await assert.rejects(client.call('Input.dispatchMouseEvent',{...point,rakeWindowId:client.windowId+999}),e=>e.code===-32010);
  const runtime=client.runtimeId;client.runtimeId='00000000-0000-0000-0000-000000000000';
  await assert.rejects(client.call('Input.insertText',{text:'wrong runtime'}),e=>e.code===-32010);client.runtimeId=runtime;
  await assert.rejects(client.click({...increment,runtimeId:'retired'}),/another runtime/);
  assert.equal((await client.find({text:'Count: 1'})).text,'Count: 1');
  assert.equal((await client.find({text:'Text: replaced 🙂'})).text,'Text: replaced 🙂');
  console.log('Ambiguity, missing node, timeout/cancel and stale runtime/window/capture reject safely');
  const beforeResize=(await client.windows())[0];
  await client.key('F3');
  await eventually(async()=>{const w=(await client.windows())[0];return w.content.width>beforeResize.content.width && w.content.height>beforeResize.content.height;});
  await client.waitFor({id:'agent-increment',bounds:{width:668,tolerance:1}});
  assert.equal((await client.find({id:'agent-increment'})).stableId,increment.stableId);
  await client.key('F4');
  const frames=new Set();
  // Occluded macOS windows need an input-driven draw; do not treat cached pixels as live frames.
  await eventually(async()=>{await client.call('Input.dispatchMouseEvent',{type:'mouseMoved',x:1,y:1});const s=await client.snapshot({textContains:'ticks:'});frames.add(s.frameSequence);return s.elements.some(e=>e.text==='Keys: 1; ticks: 24');});
  assert.ok(frames.size>1,'continuous updates must produce several observable presentations');
  const reconnect=await RakeAgent.connect(endpoint);
  assert.equal(reconnect.runtimeId,client.runtimeId);await reconnect.waitFor({text:'Count: 1'});reconnect.close();
  const old=client;const oldRuntime=old.runtimeId;
  native.kill('SIGTERM');await new Promise(resolve=>native.once('exit',resolve));
  await assert.rejects(old.typeText('must not reach the replacement'));
  native=spawn(join(root,'target/debug/rake-agent-example'),[],{cwd:root,env:{...env,RAKE_AGENT_ROUTER:'1'},stdio:['ignore',log,log]});children.push(native);
  await eventually(()=>JSON.parse(execFileSync(rake,['ctl','--json','inspect','tree'],{env,timeout:4000,stdio:['ignore','pipe','ignore']})).value?.elements.some(n=>n.semantic_id==='agent-increment'));
  client=await eventually(async()=>{const c=await RakeAgent.connect(endpoint);if(c.runtimeId!==oldRuntime)return c;c.close();});
  old.close();await assert.rejects(client.click(increment),/another runtime/);
  await client.waitFor({text:'Count: 0'});
  const first=client.windowId, firstRef=await client.find({id:'agent-increment'});
  execFileSync(rake,['ctl','--json','router','open','demo','--target','new-window'],{env,timeout:10000});
  const windows=await eventually(async()=>{const w=await client.windows();return w.length===2 && w;});
  const second=windows.find(w=>w.window!==first).window;
  const waitWindow=await client.call('Rake.waitFor',{query:{id:'missing'},timeoutMs:5000});
  await client.selectWindow(second);
  assert.equal((await client.call('Rake.getWait',{waitId:waitWindow.waitId})).status,'invalidated');
  await assert.rejects(client.click(firstRef),/another runtime or window/);
  await client.click(await client.find({id:'agent-increment'}));await client.waitFor({text:'Count: 1'});
  await client.selectWindow(first);await client.waitFor({text:'Count: 0'});
  execFileSync(rake,['ctl','--json','windows','close',`window-${second}`],{env,timeout:10000});
  await eventually(async()=> (await client.windows()).length===1);
  console.log('Native resize, rapid draws, reconnect/restart and isolated multi-window input passed');

  console.log(JSON.stringify({passed:true,directory,imagePath,runtimeId:runtime}));
} catch(error) {
  if(client) { try { console.error('Text at failure',JSON.stringify(await client.snapshot({textContains:':'}))); } catch {} }
  console.error(`Native agent test failed; logs: ${directory}`);console.error(readFileSync(join(directory,'runtime.log'),'utf8').slice(-6000));throw error;
} finally {
  client?.close();
  for(const child of children.reverse()) {
    if(child.exitCode!==null)continue;
    child.kill('SIGTERM');
    await Promise.race([new Promise(resolve=>child.once('exit',resolve)),delay(1000)]);
    if(child.exitCode===null)child.kill('SIGKILL');
  }
  http.closeAllConnections();await new Promise(resolve=>http.close(resolve));
  closeSync(log);
}
