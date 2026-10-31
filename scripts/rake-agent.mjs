// Dependency-free Node 22+ client. Native actions are never replayed after failure.
import {setTimeout as delay} from 'node:timers/promises';
import {pathToFileURL} from 'node:url';

const MAX_MESSAGE = 1048576;
function localURL(value, protocol) {
  const url = new URL(value);
  if (!protocol.includes(url.protocol) || !['127.0.0.1', '[::1]'].includes(url.hostname) || url.username || url.password)
    throw new AgentError('invalid_endpoint','A numeric loopback URL without credentials is required',{code:-32602,recovery:'fix_endpoint',outcome:'rejected'});
  return url;
}
async function fetchBounded(url, limit, timeoutMs = 5000) {
  const response = await fetch(url, {redirect:'error', signal:AbortSignal.timeout(timeoutMs)});
  if (!response.ok) throw new AgentError('http_error',`HTTP ${response.status}`,{recovery:'check_endpoint',outcome:'read_failed',data:{status:response.status}});
  const reader = response.body.getReader(), chunks = []; let size = 0;
  try {
    for (;;) {
      const {done, value} = await reader.read(); if (done) break;
      size += value.length;
      if (size > limit) throw new AgentError('capacity_exceeded','HTTP response exceeds byte budget',{recovery:'reduce_payload',outcome:'read_failed'});
      chunks.push(value);
    }
  } finally { await reader.cancel(); }
  return Buffer.concat(chunks);
}
export class AgentError extends Error {
  constructor(category,message,{code=-32000,recovery='inspect_evidence',outcome='unknown',data={},cause}={}) {
    super(message,{cause});this.name=this.constructor.name;this.code=code;
    this.category=category;this.recovery=recovery;this.outcome=outcome;
    this.data={...data,category,recovery,outcome,automaticReplay:false};
  }
}
export class ProtocolError extends AgentError {
  constructor(method,error) {
    const fallback=error.code===-32602 ? ['invalid_arguments','fix_request'] : error.code===-32601 ? ['unsupported_operation','inspect_capabilities'] : error.code===-32010 ? ['precondition_failed','resolve_again'] : ['adapter_error','inspect_evidence'];
    const data=error.data ?? {};
    super(data.category ?? fallback[0],`${method}: ${error.message}`,{code:error.code,recovery:data.recovery ?? fallback[1],outcome:data.outcome ?? 'unknown',data});this.method=method;
  }
}
export class ActionabilityError extends AgentError {
  constructor(report) {
    super('not_actionable',report.blockers.map(b=>`${b.code}: ${b.message}`).join('; '),{code:-32011,recovery:report.blockers.some(b=>b.code==='fully_clipped') ? 'scroll_into_view' : 'check_application_state',outcome:'rejected',data:{report}});this.report=report;
  }
}
export class RakeAgent {
  #socket; #pending = new Map(); #sequence = 0; #closed = false; #origin;
  capabilities; runtimeId; windowId;
  constructor(socket, origin) {
    this.#socket = socket; this.#origin = origin;
    const failed = () => this.#fail(new AgentError('disconnected','CDP disconnected; action outcome may be uncertain; no replay',{code:-32018,recovery:'reconnect'}));
    socket.addEventListener('close', failed); socket.addEventListener('error', failed);
    socket.addEventListener('message', event => {
      try {
        if (typeof event.data !== 'string' || Buffer.byteLength(event.data) > MAX_MESSAGE) throw new AgentError('invalid_response','Invalid or oversized CDP reply',{recovery:'reconnect'});
        const value = JSON.parse(event.data), pending = this.#pending.get(value.id);
        if (!pending) return; // Events are intentionally not retained.
        this.#pending.delete(value.id); clearTimeout(pending.timer);
        value.error ? pending.reject(new ProtocolError(pending.method, value.error)) : pending.resolve(value.result);
      } catch (error) { this.#fail(error); socket.close(); }
    });
  }
  #fail(error) {
    if(!(error instanceof AgentError)) error=new AgentError('transport_error',error.message,{cause:error});
    this.#closed = true;
    for (const pending of this.#pending.values()) { clearTimeout(pending.timer); pending.reject(error); }
    this.#pending.clear();
  }
  static async connect(endpoint, {targetId, windowId} = {}) {
    const base = localURL(endpoint, ['http:']);
    const targets = JSON.parse((await fetchBounded(`${base.href.replace(/\/$/, '')}/json/list`, MAX_MESSAGE)).toString());
    const matches = targets.filter(t => !targetId || t.id === targetId);
    if (matches.length !== 1) throw new AgentError('runtime_selection_required',`Expected one runtime target; found ${matches.length}; provide targetId`,{code:-32014,recovery:'select_target',outcome:'read_failed'});
    const target = matches[0], url = localURL(target.webSocketDebuggerUrl, ['ws:']);
    if (url.host !== base.host) throw new AgentError('server_mismatch','CDP discovery returned a different server',{code:-32010,recovery:'check_endpoint',outcome:'read_failed'});
    const socket = new WebSocket(url);
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => { socket.close(); reject(new AgentError('connection_timed_out','CDP connection timed out',{code:-32017,recovery:'check_endpoint',outcome:'read_failed'})); }, 5000);
      socket.addEventListener('open', () => { clearTimeout(timer); resolve(); }, {once:true});
      socket.addEventListener('error', () => { clearTimeout(timer); reject(new AgentError('connection_failed','CDP connection failed',{recovery:'check_endpoint',outcome:'read_failed'})); }, {once:true});
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
    if (this.#closed) return Promise.reject(new AgentError('client_closed','Client is closed',{code:-32018,recovery:'reconnect'}));
    if (!Number.isFinite(timeoutMs) || timeoutMs < 1 || timeoutMs > 35000) return Promise.reject(new AgentError('invalid_arguments','Invalid request timeout',{code:-32602,recovery:'fix_request',outcome:'rejected'}));
    if (this.#pending.size >= 32) return Promise.reject(new AgentError('capacity_exceeded','32 pending request limit reached',{recovery:'reduce_concurrency',outcome:'rejected'}));
    const id = ++this.#sequence;
    const text = JSON.stringify({id, method, params:{...params, ...(this.runtimeId ? {rakeRuntimeId:this.runtimeId} : {})}});
    if (Buffer.byteLength(text) > MAX_MESSAGE) return Promise.reject(new AgentError('capacity_exceeded','CDP request exceeds byte budget',{recovery:'reduce_payload',outcome:'rejected'}));
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.#pending.delete(id); reject(new AgentError('request_timed_out',`${method} timed out; action outcome may be uncertain; no replay`,{code:-32017,recovery:'check_application_state'}));
        // Retire the connection so queued late mutations cannot be followed by new actions.
        this.#fail(new AgentError('client_closed','Client retired after request timeout',{code:-32018,recovery:'reconnect'})); this.#socket.close();
      }, timeoutMs);
      this.#pending.set(id, {resolve, reject, timer, method});
      try { this.#socket.send(text); } catch (error) { this.#fail(error); this.#socket.close(); }
    });
  }
  #require(method) {
    if (!this.capabilities?.methods[method]?.supported) throw new AgentError('unsupported_operation',`${method} is unavailable; inspect capabilities`,{code:-32601,recovery:'inspect_capabilities',outcome:'rejected'});
  }
  #window() { if (!this.windowId) throw new AgentError('window_selection_required','Select a native window explicitly',{code:-32014,recovery:'select_window',outcome:'rejected'}); }
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
    if (result.totalMatches !== 1) throw new AgentError(result.totalMatches===0?'element_not_found':'ambiguous_element',`Expected exactly one element; found ${result.totalMatches}`,{code:result.totalMatches===0?-32012:-32013,recovery:result.totalMatches===0?'change_query':'narrow_query',outcome:'read_failed',data:{totalMatches:result.totalMatches}});
    return {...result.elements[0], runtimeId:result.runtimeId, windowId:result.windowId, revision:result.revision, frameSequence:result.frameSequence};
  }
  async subtree(reference,query={},options={}) {
    const root=await this.#resolve(reference);
    return this.snapshot(query,{...options,rootNodeId:root.nodeId});
  }
  async #resolve(reference) {
    if (!reference || reference.runtimeId !== this.runtimeId || reference.windowId !== this.windowId)
      throw new AgentError('reference_scope_mismatch','Element belongs to another runtime or window',{code:-32010,recovery:'resolve_again',outcome:'rejected'});
    return this.find(reference.stableId ? {stableId:reference.stableId} : {nodeId:reference.nodeId});
  }
  async actionability(reference) {
    const e=await this.#resolve(reference); this.#require('Rake.getActionability');
    return this.call('Rake.getActionability',{query:{nodeId:e.nodeId}});
  }
  async #target(reference) {
    const report=await this.actionability(reference);
    if(!report.canDispatch) throw new ActionabilityError(report);
    return report;
  }
  async #mouse(type, point, extra = {}, capture) {
    this.#require('Input.dispatchMouseEvent'); this.#window();
    return this.call('Input.dispatchMouseEvent', {type, ...point, ...extra, rakeWindowId:this.windowId, ...(capture ? {rakeCapture:capture} : {})});
  }
  async hover(reference) {
    const e = await this.#target(reference);
    await this.#mouse('mouseMoved', e.point, {}, e.revision.capture); return {accepted:true, beforeFrame:e.frameSequence};
  }
  async click(reference) {
    await this.hover(reference); const e = await this.#target(reference), point = e.point;
    await this.#mouse('mousePressed', point, {button:'left', clickCount:1}, e.revision.capture);
    await this.#mouse('mouseReleased', point, {button:'left', clickCount:1});
    return {accepted:true, beforeFrame:e.frameSequence};
  }
  async scroll(reference, deltaY, deltaX = 0) {
    const e = await this.#target(reference);
    await this.#mouse('mouseWheel', e.point, {deltaX, deltaY}, e.revision.capture);
    return {accepted:true, beforeFrame:e.frameSequence};
  }
  async drag(reference, dx, dy) {
    if (![dx,dy].every(Number.isFinite)) throw new AgentError('invalid_arguments','Finite drag offsets required',{code:-32602,recovery:'fix_request',outcome:'rejected'});
    await this.hover(reference); const e = await this.#target(reference), point = e.point;
    const target = (await this.windows()).find(w=>w.window===this.windowId);
    if (!target || point.x+dx<0 || point.y+dy<0 || point.x+dx>=target.content.width || point.y+dy>=target.content.height)
      throw new AgentError('invalid_arguments','Drag endpoint is outside the selected content viewport',{code:-32602,recovery:'fix_request',outcome:'rejected'});
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
  async waitFor(query, {state='present', timeoutMs=5000, afterFrame, rootNodeId, signal} = {}) {
    this.#window(); this.#require('Rake.waitFor');
    signal?.throwIfAborted();
    let wait = await this.call('Rake.waitFor',{query,state,timeoutMs,afterFrame,rootNodeId});
    try {
      while (wait.status === 'pending') {
        await delay(Math.min(50, Math.max(1,wait.remainingMs)), undefined, {signal});
        wait = await this.call('Rake.getWait',{waitId:wait.waitId});
      }
      if (wait.status !== 'satisfied') throw new AgentError(`wait_${wait.status}`,`Wait ${wait.status}: ${JSON.stringify(wait)}`,{code:-32017,recovery:'inspect_evidence',outcome:'observation_only',data:{wait}});
      return wait;
    } finally {
      if (wait.status === 'pending' && !this.#closed) await this.call('Rake.cancelWait',{waitId:wait.waitId}).catch(()=>{});
    }
  }
  async evidence(options = {}) {
    this.#window(); this.#require("Rake.getEvidence");
    return this.call("Rake.getEvidence", options);
  }
  async screenshot(options = {}) {
    this.#window(); this.#require('Page.captureScreenshot');
    const result = await this.call('Page.captureScreenshot',options);
    return {bytes:Buffer.from(result.data,'base64'), metadata:result.rake};
  }
  async source(reference, radius = 3) {
    if (!this.capabilities.sourceRead.supported) throw new AgentError('unsupported_operation','Source reading is unavailable',{code:-32601,recovery:'inspect_capabilities',outcome:'read_failed'});
    if (!Number.isInteger(radius) || radius<0 || radius>20) throw new AgentError('invalid_arguments','Source radius must be 0..20',{code:-32602,recovery:'fix_request',outcome:'rejected'});
    const e = await this.#resolve(reference), source = e.source;
    if (!source?.url) throw new AgentError('source_unavailable','Element has no Rust source metadata',{code:-32012,recovery:'inspect_evidence',outcome:'read_failed'});
    const url = localURL(source.url,['http:']);
    if (url.origin !== this.#origin) throw new AgentError('source_access_denied','Source URL belongs to another server',{code:-32602,recovery:'use_snapshot_source_url',outcome:'rejected'});
    const text = (await fetchBounded(url,256*1024)).toString('utf8'), lines = text.split('\n');
    const line = (source.line ?? source.lineNumber+1), start = Math.max(0,line-1-radius);
    return {file:source.file,line,column:source.column,startLine:start+1,text:lines.slice(start,line+radius).join('\n')};
  }
  close() { this.#fail(new AgentError('client_closed','Client closed',{code:-32018,recovery:'reconnect'})); this.#socket.close(); }
}

export async function cli(args) {
  let endpoint, targetId, windowId, action;
  while (args.length) {
    const arg = args.shift();
    if (arg==='--url') endpoint=args.shift();
    else if(arg==='--target') targetId=args.shift();
    else if(arg==='--window') windowId=Number(args.shift());
    else if(arg==='--json') continue;
    else if(arg==='--help') { console.log('rake agent --url http://127.0.0.1:PORT/TOKEN [--target ID] [--window ID] JSON_ACTION\nActions: capabilities, windows, snapshot, subtree, find, click, hover, scroll, drag, key, typeText, waitFor, evidence, actionability, screenshot, source. Screenshot requires path.'); return; }
    else if(!action) action=JSON.parse(arg); else throw new AgentError('invalid_arguments','Unexpected argument',{code:-32602,recovery:'fix_request',outcome:'rejected'});
  }
  if(!endpoint || !action) throw new AgentError('invalid_arguments','--url and one JSON action are required',{code:-32602,recovery:'fix_request',outcome:'rejected'});
  const client=await RakeAgent.connect(endpoint,{targetId,windowId});
  try {
    let result;
    const ref=async()=>client.find(action.query);
    switch(action.action) {
      case 'capabilities': result=client.capabilities; break;
      case 'windows': result=await client.windows(); break;
      case 'evidence': result=await client.evidence({...action.options,query:action.query}); break;
      case 'subtree': result=await client.subtree(await ref(),action.filter,action.options); break;
      case 'snapshot': result=await client.snapshot(action.query,action.options); break;
      case 'find': result=await ref(); break;
      case 'click': case 'hover': case 'source': case 'actionability': result=await client[action.action](await ref()); break;
      case 'scroll': result=await client.scroll(await ref(),action.deltaY,action.deltaX); break;
      case 'drag': result=await client.drag(await ref(),action.dx,action.dy); break;
      case 'key': result=await client.key(action.key,action.modifiers); break;
      case 'typeText': result=await client.typeText(action.text); break;
      case 'waitFor': result=await client.waitFor(action.query,action.options); break;
      case 'screenshot': {
        if(typeof action.path!=='string' || !action.path) throw new AgentError('invalid_arguments','Screenshot path is required',{code:-32602,recovery:'fix_request',outcome:'rejected'});
        const shot=await client.screenshot(action.options), {writeFile}=await import('node:fs/promises');
        await writeFile(action.path,shot.bytes,{mode:0o600}); result={path:action.path,metadata:shot.metadata}; break;
      }
      default: throw new AgentError('invalid_arguments','Unknown agent action',{code:-32602,recovery:'fix_request',outcome:'rejected'});
    }
    console.log(JSON.stringify(result));
  } finally { client.close(); }
}
export async function cliMain(args) {
  try { await cli(args); }
  catch(error) {
    const normalized=error instanceof AgentError ? error : error instanceof SyntaxError
      ? new AgentError('invalid_arguments',error.message,{code:-32602,recovery:'fix_request',outcome:'rejected'})
      : new AgentError('client_error',error.message,{cause:error});
    console.error(JSON.stringify({error:{code:normalized.code,message:normalized.message,...normalized.data}}));process.exitCode=1;
  }
}
if (process.argv[1] && import.meta.url===pathToFileURL(process.argv[1]).href) {
  await cliMain(process.argv.slice(2));
}
