// Covers: native view! text/styles/accessibility through the actual CDP adapter.
// Real: GPUI window/GPU, private Unix control, CLI, TCP and WebSocket. Doubles: none.
// Requires: macOS GUI, Node 22+, cargo build -p rake-cli -p rake-view --features rake/devtools.
// This verifies live protocol behavior; it does not automate Chrome's UI.
import assert from 'node:assert/strict';
import {spawn, execFileSync} from 'node:child_process';
import {mkdtempSync, mkdirSync, openSync, closeSync, rmSync, readFileSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import {createInterface} from 'node:readline';
import {setTimeout as delay} from 'node:timers/promises';

assert.equal(process.platform, 'darwin', 'requires a real macOS GUI; never silently skip');
const root = fileURLToPath(new URL('../', import.meta.url));
const serve = process.argv.includes('--serve');
const directory = mkdtempSync('/tmp/rake-elements-cdp-'), control = join(directory, 'control');
mkdirSync(control, {mode:0o700});
const env = {...process.env, RAKE_DEV_CONTROL:'1', RAKE_CONTROL_DIR:control};
const cli = join(root, 'target/debug/rake'), logPath = join(directory, 'runtime.log');
const log = openSync(logPath, 'w', 0o600), children = [];
let socket, intentionalClose = false;
const pending = new Map(), events = [];
let next = 0;
async function eventually(check) {
  const deadline = Date.now() + 20000; let last;
  while (Date.now() < deadline) {
    try { const result = await check(); if (result) return result; } catch (error) { last = error; }
    await delay(50);
  }
  throw new Error('Live Elements check timed out', {cause:last});
}
function call(method, params = {}) {
  return new Promise((resolve, reject) => {
    const id = ++next, timer = setTimeout(() => { pending.delete(id); reject(new Error(method + ' timed out')); }, 5000);
    pending.set(id, {resolve, reject, timer, method}); socket.send(JSON.stringify({id, method, params}));
  });
}
async function evaluate(expression) {
  const result = await call('Runtime.evaluate',{expression});
  assert.ok(!result.exceptionDetails, JSON.stringify(result.exceptionDetails));
  return result;
}
function flatten(node) { return [node, ...(node.children || []).flatMap(flatten)]; }
function semantic(node, id) { return node.attributes?.some((value, i, values) => i % 2 === 0 && value === 'id' && values[i+1] === id); }

try {
  children.push(spawn(join(root, 'target/debug/rake-view'), [], {cwd:root, env, stdio:['ignore', log, log]}));
  const capture = await eventually(() => {
    const reply = JSON.parse(execFileSync(cli, ['ctl','--json','inspect','tree'], {env, timeout:4000, stdio:['ignore','pipe','ignore']}));
    return reply.value?.elements.some(node => node.semantic_id === 'increment') && reply.value;
  });
  const adapter = spawn(cli, ['cdp','--session',directory,'--no-open','--json'], {cwd:root, env, stdio:['ignore','pipe',log]}); children.push(adapter);
  const discovery = await new Promise((resolve, reject) => {
    const lines = createInterface({input:adapter.stdout}), timer = setTimeout(() => reject(new Error('CDP discovery timed out')), 5000);
    lines.on('line', line => { try { const value = JSON.parse(line); if (value.event === 'cdp_listening') { clearTimeout(timer); resolve(value.url); } } catch {} });
    adapter.once('error', reject);
  });
  const target = await eventually(async () => (await (await fetch(discovery + '/json/list')).json())[0]);
  socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.addEventListener('open',resolve,{once:true}); socket.addEventListener('error',reject,{once:true}); });
  socket.addEventListener('error', () => { if (!intentionalClose) for (const request of pending.values()) request.reject(new Error('WebSocket failed')); });
  socket.addEventListener('message', event => {
    const reply = JSON.parse(event.data), request = pending.get(reply.id);
    if (request) { pending.delete(reply.id); clearTimeout(request.timer); reply.error ? request.reject(new Error(`${request.method}: ${JSON.stringify(reply.error)}`)) : request.resolve(reply.result); }
    else if (reply.method && events.length < 256) events.push(reply);
  });
  await call('DOM.enable'); await call('CSS.enable'); await call('Accessibility.enable');
  if (serve) {
    console.log(JSON.stringify({session:directory,devtoolsFrontendUrl:target.devtoolsFrontendUrl}));
    await new Promise(resolve => { process.once('SIGINT',resolve); process.once('SIGTERM',resolve); });
  } else {
  const document = await call('DOM.getDocument', {depth:-1}), nodes = flatten(document.root);
  const paragraph = nodes.find(node => node.nodeName === 'p' && node.children?.some(child => child.nodeValue === 'Count: 0'));
  assert.ok(paragraph, 'paragraph text must reach CDP');
  const preview = (await call('DOM.describeNode',{nodeId:paragraph.nodeId,depth:0})).node;
  assert.equal(preview.children[0].nodeType, 3, 'lazy preview includes native text node');
  assert.equal((await call('DOM.getOuterHTML',{nodeId:paragraph.nodeId})).outerHTML, '<p>Count: 0</p>');
  const textNode = preview.children[0], temporary = '<Temporary> café 你好 🙂 '.repeat(12);
  const renderedText = async () => (await call('DOM.describeNode',{nodeId:textNode.nodeId})).node.nodeValue;
  const measuredHeight = async () => parseFloat((await call('CSS.getComputedStyleForNode',{nodeId:paragraph.nodeId})).computedStyle.find(p=>p.name==='height').value);
  const originalHeight = await measuredHeight();
  events.length = 0;
  await call('DOM.setNodeValue',{nodeId:textNode.nodeId,value:temporary});
  await eventually(async () => await renderedText() === temporary);
  assert.ok(await measuredHeight() > originalHeight, 'text edits must relayout the real paragraph');
  assert.ok(events.some(event=>event.method==='DOM.characterDataModified' && event.params.nodeId===textNode.nodeId));
  assert.ok(!events.some(event=>event.method==='DOM.documentUpdated'), 'editing preserves tree identities');
  assert.ok((await call('DOM.getOuterHTML',{nodeId:paragraph.nodeId})).outerHTML.includes('&lt;Temporary&gt; café 你好 🙂'));
  await assert.rejects(call('DOM.setNodeValue',{nodeId:paragraph.nodeId,value:'no element replacement'}), /editable text node/i);
  await assert.rejects(call('DOM.setNodeValue',{nodeId:textNode.nodeId,value:'🙂'.repeat(4097)}), /16 KiB/);
  await call('DOM.setInspectedNode',{nodeId:paragraph.nodeId});
  await evaluate('rake.resetText($0)');
  await eventually(async () => await renderedText() === 'Count: 0');
  await evaluate('rake.undoText($0)');
  await eventually(async () => await renderedText() === temporary);
  await call('DOM.setNodeValue',{nodeId:textNode.nodeId,value:''});
  await eventually(async () => await renderedText() === '');
  await evaluate('rake.undoText($0)');
  await eventually(async () => await renderedText() === temporary);
  await evaluate('rake.resetText($0)');
  await eventually(async () => await renderedText() === 'Count: 0');
  await call('CSS.setStyleSheetText',{styleSheetId:`rake-edit-${paragraph.nodeId}`,text:'display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); align-items: center; column-gap: 4px;'});
  await eventually(async () => (await call('CSS.getComputedStyleForNode',{nodeId:paragraph.nodeId})).computedStyle
    .some(property=>property.name==='grid-template-columns' && property.value==='repeat(2, minmax(0, 1fr))'));
  await call('Rake.resetStyles',{nodeId:paragraph.nodeId});
  const button = nodes.find(node => node.nodeName === 'button' && semantic(node,'increment'));
  assert.ok(button);
  const ax = await call('Accessibility.getPartialAXTree',{nodeId:button.nodeId});
  assert.equal(ax.nodes.find(node => node.backendDOMNodeId === button.nodeId).name.value, 'Increment counter');
  const fullAX = await call('Accessibility.getFullAXTree');
  assert.ok(fullAX.nodes.some(node => node.role.value === 'StaticText' && node.name?.value === 'Count: 0'));
  const object = await call('DOM.resolveNode',{nodeId:button.nodeId});
  const properties = (await call('Runtime.getProperties',{objectId:object.object.objectId})).result;
  assert.ok(JSON.parse(properties.find(p=>p.name==='rakeStyleSources').value.value).length);
  assert.ok(JSON.parse(properties.find(p=>p.name==='rakeHandlers').value.value).some(site=>site.name==='click'));
  await assert.rejects(call('Rake.openSource',{nodeId:button.nodeId,target:{kind:'style',index:999}}), /source location unavailable/i);
  const computed = async () => (await call('CSS.getComputedStyleForNode',{nodeId:button.nodeId})).computedStyle;
  const before = await computed(), original = before.find(p=>p.name==='width').value;
  await call('DOM.setInspectedNode',{nodeId:button.nodeId});
  const selected = await call('Runtime.evaluate',{expression:'$0',contextId:1,objectGroup:'console',throwOnSideEffect:true});
  assert.equal((await call('DOM.requestNode',{objectId:selected.result.objectId})).nodeId,button.nodeId);
  await assert.rejects(call('Runtime.evaluate',{expression:'$0',contextId:99}), /context is stale/i);
  await call('CSS.trackComputedStyleUpdatesForNode',{nodeId:button.nodeId});
  await call('DOM.getOuterHTML',{nodeId:button.nodeId});
  events.length = 0;
  for (let poll = 0; poll < 3; poll++) await call('DOM.getOuterHTML',{nodeId:button.nodeId});
  assert.ok(!events.some(event=>event.method==='CSS.styleSheetChanged'), 'unchanged paged polls must preserve Chrome style editors');
  // Chrome previews and commits edits using returned UTF-16 ranges, before a redraw.
  let edited = await call('CSS.setStyleTexts',{edits:[{styleSheetId:`rake-edit-${button.nodeId}`,range:{startLine:0,startColumn:0,endLine:0,endColumn:0},text:'\n width: 240px;\n'}]});
  assert.equal(edited.styles[0].cssProperties[0].value,'240px', 'acknowledged edit must be returned immediately');
  edited = await call('CSS.setStyleTexts',{edits:[{styleSheetId:`rake-edit-${button.nodeId}`,range:edited.styles[0].range,text:'/* width: 240px; */'}]});
  assert.equal(edited.styles[0].cssProperties[0].disabled,true);
  edited = await call('CSS.setStyleTexts',{edits:[{styleSheetId:`rake-edit-${button.nodeId}`,range:edited.styles[0].range,text:'width: 240px;'}]});
  assert.equal(edited.styles[0].cssProperties[0].disabled,false);
  await evaluate('rake.resetStyles($0)');
  events.length = 0;
  await call('CSS.setStyleSheetText',{styleSheetId:`rake-edit-${button.nodeId}`,text:'width: 240px; border-width: 2px; border-radius: 8px; overflow: hidden;'});
  const changed = await eventually(async () => {
    const properties = await computed();
    // Width can already match an earlier frame; wait for the complete acknowledged edit.
    return [['width','240px'],['border-left-width','2px'],['overflow-x','hidden']]
      .every(([name,value])=>properties.some(p=>p.name===name && p.value===value)) && properties;
  });
  assert.equal(changed.find(p=>p.name==='border-left-width').value,'2px');
  assert.equal(changed.find(p=>p.name==='overflow-x').value,'hidden');
  await call('Rake.resetStyles',{nodeId:button.nodeId});
  await eventually(async () => (await computed()).some(p=>p.name==='width' && p.value===original));
  await evaluate('rake.undoStyles($0)');
  await eventually(async () => (await computed()).some(p=>p.name==='width' && p.value==='240px'));
  const current = flatten((await call('DOM.getDocument',{depth:-1})).root);
  assert.equal(current.find(node=>node.nodeName==='button' && semantic(node,'increment')).nodeId,button.nodeId);
  assert.ok(!events.some(event=>event.method==='DOM.documentUpdated'), 'style edits preserve document identity');
  await call('DOM.focus',{nodeId:button.nodeId});
  await call('Overlay.highlightNode',{nodeId:button.nodeId}); await call('Overlay.hideHighlight');
  console.log(`Native Elements/CDP passed: ${capture.elements.length} boundaries; temporary text/Unicode/empty edits and reset/undo, real relayout, stable IDs, lazy text, copy, sources, AX, style edits/toggles, stable polling, $0, focus and highlight.`);
  }
} catch (error) {
  console.error(readFileSync(logPath,'utf8')); throw error;
} finally {
  intentionalClose = true; socket?.close();
  for (const request of pending.values()) clearTimeout(request.timer);
  for (const child of children.reverse()) {
    if (child.exitCode !== null || child.signalCode !== null) continue;
    const ended = new Promise(resolve=>child.once('exit',resolve)); child.kill('SIGTERM');
    await Promise.race([ended,delay(3000)]); if (child.exitCode === null && child.signalCode === null) { child.kill('SIGKILL'); await ended; }
  }
  closeSync(log); rmSync(directory,{recursive:true,force:true});
}
