import {Packets, position, Wheel} from './preview-stream.js';
const $ = id => document.getElementById(id);
const prefix = '#__rake_control=';
if (location.hash.startsWith(prefix)) {
  const credential = location.hash.slice(prefix.length);
  if (/^[A-Za-z0-9_-]{32,128}$/.test(credential)) sessionStorage.setItem('rake-dev-control-token', credential);
  history.replaceState(null, '', location.pathname);
}
const token = sessionStorage.getItem('rake-dev-control-token');
const screen = $('screen'), context = screen.getContext('2d', {alpha:false});
let current, selected, selecting = false, channel, nextID = 0, paintedEpoch;
let video, codec = typeof VideoDecoder === 'function' ? 'h264' : 'jpeg';
let latest, paintScheduled = false, draws = 0, fps = 0, lastRate = performance.now(), lastPaint = 0, errorUntil = 0;
const pending = new Map();
let wheel = new Wheel(), wheelPoint, wheelEnd = false, wheelTimer, inputScheduled = false, hover;
function status(message) { $('status').textContent = message; }
function error(message) { errorUntil = performance.now() + 3000; status(message); }
function clearSelection() { selected = null; $('details').textContent = ''; $('copy').disabled = true; $('outline').hidden = true; }
function drawOutline() {
  const bounds = selected?.layout, window = current?.window;
  if (!bounds || !window) { $('outline').hidden = true; return; }
  const box = screen.getBoundingClientRect(), sx = box.width / window.frame.width, sy = box.height / window.frame.height;
  $('outline').hidden = false;
  Object.assign($('outline').style, {left:(window.content.x + bounds.x)*sx+'px',top:(window.content.y + bounds.y)*sy+'px',width:bounds.width*sx+'px',height:bounds.height*sy+'px'});
}
function present(frame, epoch) {
  latest?.frame.close(); latest = {frame, epoch};
  if (paintScheduled) return;
  paintScheduled = true;
  requestAnimationFrame(() => {
    paintScheduled = false;
    const next = latest; latest = null;
    if (!next) return;
    try {
      if (current?.epoch !== next.epoch) return;
      const w = next.frame.displayWidth || next.frame.width, h = next.frame.displayHeight || next.frame.height;
      if (screen.width !== w || screen.height !== h) { screen.width = w; screen.height = h; }
      context.drawImage(next.frame, 0, 0, w, h);
      screen.hidden = false; screen.style.width = current.window.frame.width + 'px';
      paintedEpoch = next.epoch; draws++;
      const now = performance.now(); lastPaint = now;
      if (now - lastRate > 1000) { fps = Math.round(draws * 1000 / (now - lastRate)); draws = 0; lastRate = now; }
      drawOutline();
    } finally { next.frame.close(); }
  });
}
async function stream(epoch) {
  const controller = new AbortController(); video = controller;
  let decoder, lastConfig, frameEpoch, decodeFailure;
  try {
    const response = await fetch('/__rake/preview_video', {method:'POST', headers:{'Content-Type':'application/json','X-Rake-Token':token || ''}, body:JSON.stringify({codec}), signal:controller.signal});
    if (!response.ok) throw new Error('Native video connection failed (' + response.status + ')');
    const parser = new Packets(), reader = response.body.getReader();
    if (codec === 'h264') decoder = new VideoDecoder({output:frame => present(frame, frameEpoch),error:failure => { decodeFailure = failure; controller.abort(); }});
    while (true) {
      const {value, done} = await reader.read();
      if (done) throw new Error('Native capture disconnected');
      for (const {metadata:m, payload} of parser.push(value)) {
        if (m.kind === 'ping') continue;
        if (m.epoch !== epoch) throw new Error('Native window changed');
        frameEpoch = m.epoch;
        if (m.kind === 'config') {
          const key = m.codec + ':' + Array.from(payload).join(',');
          if (lastConfig !== key) {
            const config = {codec:m.codec, description:payload, optimizeForLatency:true, hardwareAcceleration:'prefer-hardware'};
            if (!(await VideoDecoder.isConfigSupported(config)).supported) throw new Error('H.264 browser decoder unavailable');
            decoder.configure(config); lastConfig = key;
          }
        } else if (m.kind === 'frame') {
          if (!lastConfig) throw new Error('Video configuration missing');
          if (decoder.decodeQueueSize > 3) throw new Error('Decoder fell behind; requesting a fresh frame');
          decoder.decode(new EncodedVideoChunk({type:m.key ? 'key' : 'delta',timestamp:m.timestamp,data:payload}));
        } else if (m.kind === 'jpeg') {
          present(await createImageBitmap(new Blob([payload], {type:'image/jpeg'})), epoch);
        }
      }
    }
  } catch (failure) {
    if (video !== controller || current?.epoch !== epoch) return;
    if (decodeFailure || /decoder unavailable/.test(failure.message)) { codec = 'jpeg'; error('Using JPEG fallback: H.264 decoding unavailable.'); }
    else if (!controller.signal.aborted) error(failure.message);
  } finally {
    controller.abort();
    if (decoder && decoder.state !== 'closed') decoder.close();
    if (video === controller) {
      video = null;
      setTimeout(() => { if (!video && current?.epoch === epoch) void stream(epoch); }, 400);
    }
  }
}
function receiveState(next) {
  if (next.error) { current = null; paintedEpoch = null; video?.abort(); status(next.error); return; }
  const changed = current?.epoch !== next.epoch;
  if (changed || current?.window.generation !== next.window.generation) clearSelection();
  current = next;
  if (changed) {
    paintedEpoch = null; wheel = new Wheel(); wheelPoint = hover = null;
    latest?.frame.close(); latest = null;
    video?.abort(); video = null;
  }
  if (!video) void stream(next.epoch);
  if (next.capture_errors?.h264 && codec === 'h264') {
    codec = 'jpeg'; video?.abort(); video = null; void stream(next.epoch);
    error(next.capture_errors.h264 + ' · Using JPEG fallback.');
  }
  if (performance.now() > errorUntil) status(`Live · ${codec === 'h264' ? 'H.264 hardware video' : 'JPEG fallback'} · ${performance.now() - lastPaint > 1000 ? 0 : fps} displayed FPS · ${Math.round(next.window.content.width)} × ${Math.round(next.window.content.height)} pt · ${next.window.translucent ? 'translucent' : 'opaque'}`);
}
function request(action, body = {}) {
  if (channel?.readyState !== WebSocket.OPEN || pending.size >= 8 || channel.bufferedAmount > 32768) return Promise.reject(new Error('Input connection busy or unavailable'));
  const id = ++nextID;
  return new Promise((resolve, reject) => {
    pending.set(id, {resolve,reject});
    channel.send(JSON.stringify({id, action, body}));
  });
}
function connect() {
  if (!token) { status('Open the authenticated preview URL printed by rake dev.'); return; }
  channel = new WebSocket(`ws://${location.host}/__rake/preview_control`, ['rake-preview-v1', 'rake-token.' + token]);
  channel.onmessage = event => {
    const message = JSON.parse(event.data);
    if (message.type === 'state') receiveState(message.value);
    else if (message.type === 'reply') {
      const item = pending.get(message.id); pending.delete(message.id);
      if (item) message.error ? item.reject(new Error(message.error)) : item.resolve(message.value);
      scheduleInput();
    }
  };
  channel.onclose = () => {
    for (const item of pending.values()) item.reject(new Error('Preview input disconnected'));
    pending.clear(); current = null; paintedEpoch = null; video?.abort(); video = null;
    status('Reconnecting to the native app…'); setTimeout(connect, 1000);
  };
}
function point(event) {
  if (!current || current.epoch !== paintedEpoch) return null;
  const result = position(event, screen.getBoundingClientRect(), current.window);
  return result ? {...result, reference:current.reference} : null;
}
async function send(action, position, input) {
  if (!position) return;
  try {
    const result = await request(action, {...position, ...(input ? {input} : {})});
    if (action === 'preview_select') {
      selected = result;
      $('details').textContent = JSON.stringify({name:result.view?.tag || result.kind,kind:result.kind,id:result.semantic_id,source:result.source,style_sources:result.view?.style_sources,layout:result.layout,text:result.text}, null, 2);
      $('copy').disabled = false; drawOutline();
    }
  } catch (failure) { error(failure.message); }
}
function scheduleInput() {
  if (inputScheduled) return;
  inputScheduled = true;
  requestAnimationFrame(() => {
    inputScheduled = false;
    if (!current || pending.size >= 4 || channel?.bufferedAmount > 32768) return;
    if (wheelPoint) {
      const input = wheel.take(wheelEnd);
      if (input) {
        void send('preview_input', {...wheelPoint, reference:current.reference}, input);
        if (input.phase === 'ended') { wheelPoint = null; wheelEnd = false; }
        else if (wheelEnd) scheduleInput();
      }
    }
    if (hover && pending.size < 3) { const next = hover; hover = null; void send('preview_input', {...next, reference:current.reference}, {type:'move'}); }
  });
}
function mode(value) {
  selecting = value; hover = null;
  document.body.classList.toggle('selecting', value);
  $('select').setAttribute('aria-pressed', String(value)); $('interact').setAttribute('aria-pressed', String(!value));
}
$('select').onclick = () => mode(true);
$('interact').onclick = () => mode(false);
$('reconnect').onclick = () => { void request('preview_restart').catch(failure => error(failure.message)); };
screen.addEventListener('click', event => { event.preventDefault(); hover = null; void send(selecting ? 'preview_select' : 'preview_input', point(event), selecting ? null : {type:'click'}); });
screen.addEventListener('wheel', event => {
  event.preventDefault(); const next = point(event); if (!next) return;
  const multiplier = event.deltaMode === 1 ? 20 : event.deltaMode === 2 ? current.window.content.height : 1;
  wheel.add(event.deltaX * multiplier, event.deltaY * multiplier); wheelPoint = next; wheelEnd = false;
  clearTimeout(wheelTimer); wheelTimer = setTimeout(() => { wheelEnd = true; scheduleInput(); }, 100);
  scheduleInput();
}, {passive:false});
screen.addEventListener('mousemove', event => { if (!selecting) { hover = point(event); scheduleInput(); } });
screen.addEventListener('mouseleave', () => { hover = null; });
$('copy').onclick = async () => {
  try { await navigator.clipboard.writeText(JSON.stringify({runtime_id:current?.runtime_id,element:selected},null,2)); status('Selection context copied'); }
  catch (failure) { error(failure.message); }
};
new ResizeObserver(drawOutline).observe(screen);
setInterval(() => { if (channel?.readyState === WebSocket.OPEN && pending.size < 4) void request('preview_state').catch(() => {}); }, 5000);
connect();
