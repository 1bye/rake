// Dependency-free Node 22+ client. Native actions are never replayed after failure.
import {setTimeout as delay} from 'node:timers/promises';
import {pathToFileURL} from 'node:url';

const MAX_MESSAGE = 1048576;
function localURL(value, protocol) {
  const url = new URL(value);
  if (!protocol.includes(url.protocol) || !['127.0.0.1', '[::1]'].includes(url.hostname) || url.username || url.password)
    throw new Error('A numeric loopback URL without credentials is required');
  return url;
}
async function fetchBounded(url, limit, timeoutMs = 5000) {
  const response = await fetch(url, {redirect:'error', signal:AbortSignal.timeout(timeoutMs)});
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  const reader = response.body.getReader(), chunks = []; let size = 0;
  try {
    for (;;) {
      const {done, value} = await reader.read(); if (done) break;
      size += value.length;
      if (size > limit) throw new Error('HTTP response exceeds byte budget');
      chunks.push(value);
    }
  } finally { await reader.cancel(); }
  return Buffer.concat(chunks);
}
export class ProtocolError extends Error {
  constructor(method, error) { super(`${method}: ${error.message}`); this.code = error.code; this.method = method; }
}
export class RakeAgent {
  #socket; #pending = new Map(); #sequence = 0; #closed = false; #origin;
  capabilities; runtimeId; windowId;
  constructor(socket, origin) {
    this.#socket = socket; this.#origin = origin;
    const failed = () => this.#fail(new Error('CDP disconnected; action outcome may be uncertain; no replay'));
    socket.addEventListener('close', failed); socket.addEventListener('error', failed);
    socket.addEventListener('message', event => {
      try {
        if (typeof event.data !== 'string' || Buffer.byteLength(event.data) > MAX_MESSAGE) throw new Error('Invalid or oversized CDP reply');
        const value = JSON.parse(event.data), pending = this.#pending.get(value.id);
        if (!pending) return; // Events are intentionally not retained.
        this.#pending.delete(value.id); clearTimeout(pending.timer);
        value.error ? pending.reject(new ProtocolError(pending.method, value.error)) : pending.resolve(value.result);
      } catch (error) { this.#fail(error); socket.close(); }
    });
  }
  #fail(error) {
    this.#closed = true;
    for (const pending of this.#pending.values()) { clearTimeout(pending.timer); pending.reject(error); }
    this.#pending.clear();
  }
  static async connect(endpoint, {targetId, windowId} = {}) {
    const base = localURL(endpoint, ['http:']);
    const targets = JSON.parse((await fetchBounded(`${base.href.replace(/\/$/, '')}/json/list`, MAX_MESSAGE)).toString());
    const matches = targets.filter(t => !targetId || t.id === targetId);
    if (matches.length !== 1) throw new Error(`Expected one runtime target; found ${matches.length}; provide targetId`);
    const target = matches[0], url = localURL(target.webSocketDebuggerUrl, ['ws:']);
    if (url.host !== base.host) throw new Error('CDP discovery returned a different server');
    const socket = new WebSocket(url);
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => { socket.close(); reject(new Error('CDP connection timed out')); }, 5000);
      socket.addEventListener('open', () => { clearTimeout(timer); resolve(); }, {once:true});
      socket.addEventListener('error', () => { clearTimeout(timer); reject(new Error('CDP connection failed')); }, {once:true});
    });
    const client = new RakeAgent(socket, base.origin);
    try {
      client.capabilities = await client.call('Rake.getCapabilities');
      client.runtimeId = client.capabilities.runtimeId;
      const available = client.capabilities.methods["Rake.getWindows"]?.supported ? await client.windows() : [];
      if (windowId !== undefined) await client.selectWindow(windowId);
      else if (available.length === 1) await client.selectWindow(available[0].window);
      return client;
    } catch (error) { client.close(); throw error; }
  }
  call(method, params = {}, timeoutMs = 5000) {
    if (this.#closed) return Promise.reject(new Error('Client is closed'));
    if (!Number.isFinite(timeoutMs) || timeoutMs < 1 || timeoutMs > 35000) return Promise.reject(new Error('Invalid request timeout'));
    if (this.#pending.size >= 32) return Promise.reject(new Error('32 pending request limit reached'));
    const id = ++this.#sequence;
    const text = JSON.stringify({id, method, params:{...params, ...(this.runtimeId ? {rakeRuntimeId:this.runtimeId} : {})}});
    if (Buffer.byteLength(text) > MAX_MESSAGE) return Promise.reject(new Error('CDP request exceeds byte budget'));
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.#pending.delete(id); reject(new Error(`${method} timed out; action outcome may be uncertain; no replay`));
        // Retire the connection so queued late mutations cannot be followed by new actions.
        this.#fail(new Error('Client retired after request timeout')); this.#socket.close();
      }, timeoutMs);
      this.#pending.set(id, {resolve, reject, timer, method});
      try { this.#socket.send(text); } catch (error) { this.#fail(error); this.#socket.close(); }
    });
  }
  #require(method) {
    if (!this.capabilities?.methods[method]?.supported) throw new Error(`${method} is unavailable; inspect capabilities`);
  }
  #window() { if (!this.windowId) throw new Error('Select a native window explicitly'); }
  async windows() { this.#require('Rake.getWindows'); return (await this.call('Rake.getWindows')).windows; }
  async selectWindow(windowId) {
    this.#require('Rake.selectWindow');
    const result = await this.call('Rake.selectWindow', {windowId}); this.windowId = result.window.window; return result.window;
  }
  async snapshot(query = {}, options = {}) {
    this.#window(); this.#require('Rake.getSnapshot');
    return this.call('Rake.getSnapshot', {...options, query});
  }
  async find(query) {
    const result = await this.snapshot(query, {limit:2});
    if (result.totalMatches !== 1) throw new Error(`Expected exactly one element; found ${result.totalMatches}`);
    return {...result.elements[0], runtimeId:result.runtimeId, windowId:result.windowId, revision:result.revision, frameSequence:result.frameSequence};
  }
  async #resolve(reference) {
    if (!reference || reference.runtimeId !== this.runtimeId || reference.windowId !== this.windowId)
      throw new Error('Element belongs to another runtime or window');
    return this.find(reference.stableId ? {stableId:reference.stableId} : {nodeId:reference.nodeId});
  }
  #point(element) {
    if (element.visible !== true || element.enabled === false) throw new Error('Element is not visibly actionable');
    const b = element.visibleBounds ?? element.bounds;
    if (!b || b.width < 1 || b.height < 1) throw new Error('Element has no actionable bounds');
    return {x:b.x + b.width / 2, y:b.y + b.height / 2};
  }
  async #mouse(type, point, extra = {}, capture) {
    this.#require('Input.dispatchMouseEvent'); this.#window();
    return this.call('Input.dispatchMouseEvent', {type, ...point, ...extra, rakeWindowId:this.windowId, ...(capture ? {rakeCapture:capture} : {})});
  }
  async hover(reference) {
    const e = await this.#resolve(reference);
    await this.#mouse('mouseMoved', this.#point(e), {}, e.revision.capture); return {accepted:true, beforeFrame:e.frameSequence};
  }
  async click(reference) {
    await this.hover(reference); const e = await this.#resolve(reference), point = this.#point(e);
    await this.#mouse('mousePressed', point, {button:'left', clickCount:1}, e.revision.capture);
    await this.#mouse('mouseReleased', point, {button:'left', clickCount:1});
    return {accepted:true, beforeFrame:e.frameSequence};
  }
  async scroll(reference, deltaY, deltaX = 0) {
    const e = await this.#resolve(reference);
    await this.#mouse('mouseWheel', this.#point(e), {deltaX, deltaY}, e.revision.capture);
    return {accepted:true, beforeFrame:e.frameSequence};
  }
  async drag(reference, dx, dy) {
    if (![dx,dy].every(Number.isFinite)) throw new Error('Finite drag offsets required');
    await this.hover(reference); const e = await this.#resolve(reference), point = this.#point(e);
    const target = (await this.windows()).find(w=>w.window===this.windowId);
    if (!target || point.x+dx<0 || point.y+dy<0 || point.x+dx>=target.content.width || point.y+dy>=target.content.height)
      throw new Error('Drag endpoint is outside the selected content viewport');
    await this.#mouse('mousePressed', point, {button:'left',clickCount:1}, e.revision.capture);
    for (let step=1; step<=8; step++) await this.#mouse('mouseMoved',{x:point.x+dx*step/8,y:point.y+dy*step/8},{buttons:1});
    await this.#mouse('mouseReleased',{x:point.x+dx,y:point.y+dy},{button:'left',clickCount:1});
    return {accepted:true,beforeFrame:e.frameSequence};
  }
  async key(key, modifiers = 0) {
    this.#window(); this.#require('Input.dispatchKeyEvent');
    await this.call('Input.dispatchKeyEvent',{type:'rawKeyDown',key,modifiers,rakeWindowId:this.windowId});
    await this.call('Input.dispatchKeyEvent',{type:'keyUp',key,modifiers,rakeWindowId:this.windowId});
    return {accepted:true};
  }
  async typeText(text) {
    this.#window(); this.#require('Input.insertText');
    await this.call('Input.insertText',{text,rakeWindowId:this.windowId}); return {accepted:true};
  }
  async waitFor(query, {state='present', timeoutMs=5000, afterFrame, signal} = {}) {
    this.#window(); this.#require('Rake.waitFor');
    signal?.throwIfAborted();
    let wait = await this.call('Rake.waitFor',{query,state,timeoutMs,afterFrame});
    try {
      while (wait.status === 'pending') {
        await delay(Math.min(50, Math.max(1,wait.remainingMs)), undefined, {signal});
        wait = await this.call('Rake.getWait',{waitId:wait.waitId});
      }
      if (wait.status !== 'satisfied') throw new Error(`Wait ${wait.status}: ${JSON.stringify(wait)}`);
      return wait;
    } finally {
      if (wait.status === 'pending' && !this.#closed) await this.call('Rake.cancelWait',{waitId:wait.waitId}).catch(()=>{});
    }
  }
  async screenshot(options = {}) {
    this.#window(); this.#require('Page.captureScreenshot');
    const result = await this.call('Page.captureScreenshot',options);
    return {bytes:Buffer.from(result.data,'base64'), metadata:result.rake};
  }
  async source(reference, radius = 3) {
    if (!this.capabilities.sourceRead.supported) throw new Error('Source reading is unavailable');
    if (!Number.isInteger(radius) || radius<0 || radius>20) throw new Error('Source radius must be 0..20');
    const e = await this.#resolve(reference), source = e.source;
    if (!source?.url) throw new Error('Element has no Rust source metadata');
    const url = localURL(source.url,['http:']);
    if (url.origin !== this.#origin) throw new Error('Source URL belongs to another server');
    const text = (await fetchBounded(url,256*1024)).toString('utf8'), lines = text.split('\n');
    const line = (source.line ?? source.lineNumber+1), start = Math.max(0,line-1-radius);
    return {file:source.file,line,column:source.column,startLine:start+1,text:lines.slice(start,line+radius).join('\n')};
  }
  close() { this.#fail(new Error('Client closed')); this.#socket.close(); }
}

export async function cli(args) {
  let endpoint, targetId, windowId, action;
  while (args.length) {
    const arg = args.shift();
    if (arg==='--url') endpoint=args.shift();
    else if(arg==='--target') targetId=args.shift();
    else if(arg==='--window') windowId=Number(args.shift());
    else if(arg==='--json') continue;
    else if(arg==='--help') { console.log('rake agent --url http://127.0.0.1:PORT/TOKEN [--target ID] [--window ID] JSON_ACTION\nActions: capabilities, windows, snapshot, find, click, hover, scroll, drag, key, typeText, waitFor, screenshot, source. Screenshot requires path.'); return; }
    else if(!action) action=JSON.parse(arg); else throw new Error('Unexpected argument');
  }
  if(!endpoint || !action) throw new Error('--url and one JSON action are required');
  const client=await RakeAgent.connect(endpoint,{targetId,windowId});
  try {
    let result;
    const ref=async()=>client.find(action.query);
    switch(action.action) {
      case 'capabilities': result=client.capabilities; break;
      case 'windows': result=await client.windows(); break;
      case 'snapshot': result=await client.snapshot(action.query,action.options); break;
      case 'find': result=await ref(); break;
      case 'click': case 'hover': case 'source': result=await client[action.action](await ref()); break;
      case 'scroll': result=await client.scroll(await ref(),action.deltaY,action.deltaX); break;
      case 'drag': result=await client.drag(await ref(),action.dx,action.dy); break;
      case 'key': result=await client.key(action.key,action.modifiers); break;
      case 'typeText': result=await client.typeText(action.text); break;
      case 'waitFor': result=await client.waitFor(action.query,action.options); break;
      case 'screenshot': {
        if(typeof action.path!=='string' || !action.path) throw new Error('Screenshot path is required');
        const shot=await client.screenshot(action.options), {writeFile}=await import('node:fs/promises');
        await writeFile(action.path,shot.bytes,{mode:0o600}); result={path:action.path,metadata:shot.metadata}; break;
      }
      default: throw new Error('Unknown agent action');
    }
    console.log(JSON.stringify(result));
  } finally { client.close(); }
}
if (process.argv[1] && import.meta.url===pathToFileURL(process.argv[1]).href) {
  cli(process.argv.slice(2)).catch(error=>{console.error(error.message);process.exitCode=1;});
}
