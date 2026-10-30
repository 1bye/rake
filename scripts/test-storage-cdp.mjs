// Covers: real typed/file storage over native IPC and the Chrome storage protocol.
// Real: example process, disk, cache, runtime commands, local server and CDP adapter.
// Doubles: none. Requires Unix, Node 22+, built rake CLI and storage example.
// This script never automates Chrome. --serve supports manual Chrome verification.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {copyFileSync,mkdtempSync,mkdirSync,rmSync,writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createInterface} from 'node:readline';
import {setTimeout as delay} from 'node:timers/promises';

assert.notEqual(process.platform,'win32','example transport requires Unix');
const root=fileURLToPath(new URL('../',import.meta.url));
// macOS's per-user temp path exceeds Unix socket pathname limits with a UUID.
const session=mkdtempSync(join(process.platform==='darwin'?'/tmp':tmpdir(),'rake-storage-'));
mkdirSync(join(session,'control'),{mode:0o700});
const serve=process.argv.includes('--serve'),children=[],events=[],pending=new Map();
let executable=join(root,'target/debug/examples/storage');
if(serve&&process.platform==='darwin'){
  const contents=join(session,'Rake Storage.app','Contents');
  mkdirSync(join(contents,'MacOS'),{recursive:true,mode:0o700});
  copyFileSync(executable,join(contents,'MacOS','storage'));
  writeFileSync(join(contents,'Info.plist'),`<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>storage</string>
<key>CFBundleIdentifier</key><string>com.example.rake.storage</string>
<key>CFBundleName</key><string>Rake Storage</string>
<key>CFBundleDisplayName</key><string>Rake Storage</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>1</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>`);
  executable=join(contents,'MacOS','storage');
}
let socket,sequence=0,frontend,discovery,stderr='';
function launch(binary,args){
  const child=spawn(binary,args,{cwd:root,stdio:['ignore','pipe','pipe']});children.push(child);
  child.stderr.on('data',data=>stderr=(stderr+data).slice(-8000));
  child.once('error',error=>stderr+=String(error));return child;
}
async function eventually(check){
  const end=Date.now()+30000;
  while(Date.now()<end){const value=await check();if(value)return value;
    if(children.some(child=>child.exitCode!==null))throw new Error(`fixture exited: ${stderr}`);
    await delay(30);
  }
  throw new Error(`storage fixture timed out: ${stderr}`);
}
async function start(){
  let ready=false;events.length=0;frontend=undefined;discovery=undefined;
  const app=launch(executable,['--data',join(session,'data'),'--control',join(session,'control'),...(serve?[]:['--headless'])]);
  createInterface({input:app.stdout}).on('line',line=>{try{if(JSON.parse(line).event==='storage_ready')ready=true;}catch{}});
  await eventually(()=>ready);
  const cdp=launch(join(root,'target/debug/rake'),['cdp','--session',session,'--no-open','--json']);
  createInterface({input:cdp.stdout}).on('line',line=>{try{const event=JSON.parse(line);if(event.event==='cdp_listening'){discovery=event.url;frontend=event.targets[0].devtoolsFrontendUrl;}}catch{}});
  await eventually(()=>frontend);
  const target=(await (await fetch(discovery+'/json/list')).json())[0];
  socket=new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve,reject)=>{socket.addEventListener('open',resolve,{once:true});socket.addEventListener('error',reject,{once:true});});
  socket.addEventListener('message',event=>{const message=JSON.parse(event.data),item=pending.get(message.id);
    if(item){pending.delete(message.id);clearTimeout(item.timer);message.error?item.reject(new Error(JSON.stringify(message.error))):item.resolve(message.result);}
    else if(message.method&&events.length<4096)events.push(message);
  });
  await call('Runtime.enable');await call('DOMStorage.enable');
}
function call(method,params={}){return new Promise((resolve,reject)=>{
  const id=++sequence,timer=setTimeout(()=>{pending.delete(id);reject(new Error(`${method} timed out`));},15000);
  pending.set(id,{resolve,reject,timer});socket.send(JSON.stringify({id,method,params}));
});}
async function stopProcesses(){
  socket?.close();for(const item of pending.values())clearTimeout(item.timer);pending.clear();
  for(const child of children)child.kill('SIGTERM');
  for(const child of children)if(child.exitCode===null&&child.signalCode===null){await Promise.race([new Promise(resolve=>child.once('exit',resolve)),delay(3000)]);if(child.exitCode===null&&child.signalCode===null)child.kill('SIGKILL');}
  children.length=0;
  // A forced process exit cannot run LocalServer's Drop; remove only this fixture's discovery.
  rmSync(join(session,'control'),{recursive:true,force:true});mkdirSync(join(session,'control'),{mode:0o700});
}
const storageId={storageKey:'rake://runtime/',isLocalStorage:true};
const sessionId={...storageId,isLocalStorage:false};
async function preferences(){const {entries}=await call('DOMStorage.getDOMStorageItems',{storageId});return JSON.parse(entries.find(([key])=>key==='preferences/editor')[1]);}
async function app(theme){
  const args=theme?{theme}:{};
  const result=await call('Runtime.evaluate',{expression:`test.storage(${JSON.stringify(args)})`,returnByValue:true});
  assert.ok(!result.exceptionDetails,JSON.stringify(result));assert.equal(result.result.value.status,'committed',JSON.stringify(result));return result.result.value;
}
try{
  await start();
  console.log(JSON.stringify({session,devtoolsFrontendUrl:frontend}));
  if(serve){
    console.log('Storage example ready. stdin: light | dark | read | quit');
    await new Promise(resolve=>{const input=createInterface({input:process.stdin});let work=Promise.resolve();
      for(const child of children)child.once('exit',resolve);
      input.on('line',line=>{work=work.then(async()=>{if(line==='quit'){input.close();resolve();}else if(['light','dark','read'].includes(line))console.log(JSON.stringify(await app(line==='read'?undefined:line)));}).catch(console.error);});
      process.once('SIGINT',resolve);process.once('SIGTERM',resolve);
    });
  }else{
    for(const method of ['Storage.getStorageKey','Storage.getStorageKeyForFrame'])assert.equal((await call(method,{frameId:'rake'})).storageKey,storageId.storageKey);
    assert.deepEqual((await call('Rake.getStorage')).stores.map(store=>store.name),['preferences','session']);
    assert.equal((await preferences()).theme,'system');
    assert.deepEqual((await call('DOMStorage.getDOMStorageItems',{storageId:sessionId})).entries,[['session/example','"Typed storage example"']]);
    await call('DOMStorage.setDOMStorageItem',{storageId,key:'preferences/editor',value:'{"theme":"dark","tab_width":8}'});
    assert.deepEqual(await preferences(),{theme:'dark',tab_width:8});
    const before=events.length;
    await app('light');
    await eventually(()=>events.slice(before).some(event=>event.method==='DOMStorage.domStorageItemUpdated'&&event.params.newValue.includes('light')));
    assert.deepEqual(await preferences(),{theme:'light',tab_width:8},'app writes invalidate the same cache and update Chrome');
    const rejected=events.length;
    await assert.rejects(call('DOMStorage.setDOMStorageItem',{storageId,key:'preferences/editor',value:'{"theme":"invalid","tab_width":8}'}),/unknown variant/);
    assert.ok(events.slice(rejected).some(event=>event.method==='DOMStorage.domStorageItemAdded'&&JSON.parse(event.params.newValue).theme==='light'),'rejected optimistic edit recreates actual Chrome row');
    await assert.rejects(call('DOMStorage.setDOMStorageItem',{storageId,key:'preferences/editor',value:'bad json'}),/storage/);
    assert.equal((await preferences()).theme,'light','invalid edits preserve previous data');
    await assert.rejects(call('DOMStorage.removeDOMStorageItem',{storageId:sessionId,key:'session/example'}),/read-only/);
    await assert.rejects(call('DOMStorage.setDOMStorageItem',{storageId,key:'unregistered/secret',value:'"no"'}),/Unknown/);
    await assert.rejects(call('DOMStorage.getDOMStorageItems',{storageId:{...storageId,storageKey:'https://other/'}}),/rake/);
    const cleared=events.length;
    await assert.rejects(call('DOMStorage.clear',{storageId}),/Bulk/);
    assert.ok(events.slice(cleared).some(event=>event.method==='DOMStorage.domStorageItemAdded'&&event.params.key==='preferences/editor'),'unsupported Clear All restores actual rows');
    await call('DOMStorage.setDOMStorageItem',{storageId,key:'preferences/unicode',value:'{"theme":"system","tab_width":2}'});
    await call('DOMStorage.removeDOMStorageItem',{storageId,key:'preferences/unicode'});
    assert.equal((await call('DOMStorage.getDOMStorageItems',{storageId})).entries.length,1);
    await call('DOMStorage.disable');
    await stopProcesses();await start();
    assert.deepEqual(await preferences(),{theme:'light',tab_width:8},'real data survives process restart');
    await app();
    console.log(JSON.stringify({passed:true,persistence:true,typedValidation:true,liveEvents:true,permissions:true}));
  }
}finally{await stopProcesses();rmSync(session,{recursive:true,force:true});process.stdin.pause();}
