import {Packets, position, Wheel} from './preview-stream.js';
import {Annotations, targetKey, visibleBounds, elementName, formatReference, formatAnnotation, formatAnnotations} from './preview-annotations.js';
const $ = id => document.getElementById(id);
const prefix = '#__rake_control=';
if (location.hash.startsWith(prefix)) {
  const credential = location.hash.slice(prefix.length);
  if (/^[A-Za-z0-9_-]{32,128}$/.test(credential)) sessionStorage.setItem('rake-dev-control-token', credential);
  history.replaceState(null, '', location.pathname);
}
const token = sessionStorage.getItem('rake-dev-control-token');
const screen = $('screen'), context = screen.getContext('2d', {alpha:false});
const notes = new Annotations(sessionStorage), geometries = new Map(), pins = new Map();
const copyKey = /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘C' : 'Ctrl+C';
$('picker-copy-key').textContent = copyKey;
let current, selected, selectedGeneration, editingID, hoveredID, anchor, selecting = false, channel, nextID = 0, paintedEpoch;
let video, codec = typeof VideoDecoder === 'function' ? 'h264' : 'jpeg';
let latest, paintScheduled = false, draws = 0, fps = 0, lastRate = performance.now(), lastPaint = 0;
let noticeTimer, menuTimer, menuOpen = false, menuPinned = false, resolveTimer, resolving = false, nextResolve = 0;
const pending = new Map();
let pointer, picked, pickedPoint, pickedGeneration, pickTimer, pickController, lastPick = 0, pickVersion = 0, copyBusy = false, overlayScheduled = false, copyVersion = 0, commentVersion = 0, copyController;
let wheel = new Wheel(), wheelPoint, wheelEnd = false, wheelTimer, inputScheduled = false, hover;
function status(message, kind = 'info') {
  clearTimeout(noticeTimer);
  $('status').textContent = message; $('status').dataset.kind = kind; $('status').hidden = false;
  noticeTimer = setTimeout(() => { $('status').hidden = true; }, kind === 'error' ? 6000 : 2600);
}
function error(message) { status(message, 'error'); }
function connection(message, state = 'connecting') {
  $('connection').dataset.state = state; $('connection-label').textContent = message;
  if (state !== 'live') {
    $('fps').firstChild.textContent = '— ';
    $('empty-state').hidden = !screen.hidden;
    $('empty-state').lastChild.textContent = message;
  }
}
function nameOf(element) {
  return elementName(element);
}
function labelOf(element) { return nameOf(element) + (element.semantic_id ? ' #' + element.semantic_id : ''); }
function sourceOf(element) { return element.source ? element.source.file + ':' + element.source.line : 'Source unavailable'; }
function geometryFor(element, generation) {
  if (!current) return null;
  const resolved = geometries.get(targetKey(element, generation));
  if (resolved?.generation === current.window.generation) return resolved.element;
  return generation === current.window.generation ? element : null;
}
function stageBounds(element, frameBox) {
  const bounds = visibleBounds(element, current?.window);
  if (!bounds || screen.hidden) return null;
  const box = frameBox || screen.getBoundingClientRect(), window = current.window;
  const sx = box.width / window.frame.width, sy = box.height / window.frame.height;
  return {left:(window.content.x + bounds.x)*sx, top:(window.content.y + bounds.y)*sy,
    width:bounds.width*sx, height:bounds.height*sy};
}
function drawOverlays() {
  const frameBox = screen.getBoundingClientRect();
  for (const note of notes.items) {
    const bounds = stageBounds(geometryFor(note.element, note.generation), frameBox), pin = pins.get(note.id);
    if (!pin) continue;
    pin.hidden = !bounds;
    if (bounds) Object.assign(pin.style, {left:bounds.left+'px', top:bounds.top+'px'});
    pin.classList.toggle('active', hoveredID === note.id);
  }
  const note = notes.items.find(note => note.id === hoveredID);
  const element = note?.element || selected || picked, generation = note?.generation || (selected ? selectedGeneration : pickedGeneration);
  const bounds = element && stageBounds(geometryFor(element, generation), frameBox);
  $('outline').hidden = !bounds;
  $('outline').classList.toggle('grabbing', !!picked && !selected && !note);
  if (bounds) {
    for (const [key, value] of Object.entries(bounds)) $('outline').style[key] = value+'px';
    $('outline-label').textContent = labelOf(element);
  }
  $('picker-label').hidden = !picked || !bounds || !!selected || !!note || !selecting || menuOpen;
  if (!$('picker-label').hidden && pointer) {
    $('picker-name').textContent = labelOf(picked);
    $('picker-source').textContent = sourceOf(picked); $('picker-source').title = sourceOf(picked);
    const label = $('picker-label').getBoundingClientRect();
    $('picker-label').style.left = Math.max(12, Math.min(innerWidth - label.width - 12, pointer.clientX + 18))+'px';
    $('picker-label').style.top = Math.max(70, Math.min(innerHeight - label.height - 95, pointer.clientY + 20))+'px';
  }
  for (const item of $('annotation-list').children) {
    const annotation = notes.items.find(note => note.id === item.dataset.id);
    const live = annotation && geometryFor(annotation.element, annotation.generation);
    item.querySelector('.annotation-location').textContent = !current ? 'App disconnected'
      : !live ? 'Position unavailable in this frame' : visibleBounds(live, current.window) ? hoveredID === annotation.id ? copyKey + ' to copy' : 'Hover to highlight' : 'Outside the visible window';
  }
}
function scheduleOverlays() {
  if (overlayScheduled) return;
  overlayScheduled = true;
  requestAnimationFrame(() => { overlayScheduled = false; drawOverlays(); });
}

function fitWindow() {
  if (!current || screen.hidden) return;
  const margin = innerWidth <= 600 ? 32 : 64;
  const scale = Math.min(1, Math.max(1, innerWidth - margin) / current.window.frame.width,
    Math.max(1, innerHeight - 192) / current.window.frame.height);
  screen.style.width = current.window.frame.width * scale + 'px';
  drawOverlays(); positionComposer();
}
function closeComposer() {
  commentVersion++;
  selected = null; editingID = null; $('composer').hidden = true; $('comment').value = '';
  $('mode-hint').hidden = !selecting; drawOverlays();
}
function positionComposer() {
  if ($('composer').hidden) return;
  const box = $('composer').getBoundingClientRect(), stage = screen.getBoundingClientRect();
  const bounds = selected && stageBounds(geometryFor(selected, selectedGeneration));
  const point = anchor || {x:stage.left + (bounds?.left || 0) + (bounds?.width || stage.width)/2,
    y:stage.top + (bounds?.top || 0) + (bounds?.height || 0)/2};
  const x = point.x > innerWidth/2 ? point.x - box.width - 18 : point.x + 18;
  const y = point.y - 36;
  $('composer').style.left = Math.max(16, Math.min(innerWidth - box.width - 16, x))+'px';
  $('composer').style.top = Math.max(70, Math.min(innerHeight - box.height - 110, y))+'px';
}
function openComposer(element, generation, note, location) {
  cancelPick(); selected = element; selectedGeneration = generation; editingID = note?.id; anchor = location;
  hoveredID = null; setMenu(false); $('composer').hidden = false; $('mode-hint').hidden = true;
  $('draft-number').textContent = note?.number || '+';
  $('draft-title').textContent = note ? 'Edit annotation' : 'New annotation';
  $('element-name').textContent = labelOf(element);
  $('element-source').textContent = sourceOf(element); $('element-source').title = sourceOf(element);
  $('comment').value = note?.comment || ''; $('save').firstChild.textContent = note ? 'Save note ' : 'Add note ';
  positionComposer(); drawOverlays(); queueResolution(); $('comment').focus();
}
function previewNote(id) { hoveredID = id; drawOverlays(); queueResolution(); }
function cancelPick() {
  pickVersion++; clearTimeout(pickTimer); pickTimer = null; pickController?.abort(); pickController = null;
  picked = pickedPoint = null; $('picker-label').hidden = true;
}
async function readPreview(action, body, signal = AbortSignal.timeout(3000)) {
  const response = await fetch('/__rake/' + action, {method:'POST',
    headers:{'Content-Type':'application/json','X-Rake-Token':token || ''}, body:JSON.stringify(body), signal});
  if (!response.ok) throw new Error('Preview context unavailable (' + response.status + ')');
  return response.json();
}
function queuePick() {
  if (!selecting || !pointer || selected || menuOpen || !current || pickTimer || pickController) return;
  if (picked && pickedGeneration === current.window.generation && pointer.x === pickedPoint?.x && pointer.y === pickedPoint?.y) return;
  const version = pickVersion;
  pickTimer = setTimeout(async () => {
    pickTimer = null;
    if (version !== pickVersion || !pointer || !selecting || selected || menuOpen || !current) return;
    const position = {...pointer, reference:current.reference}, runtime = current.runtime_id;
    const controller = new AbortController(); pickController = controller; lastPick = performance.now();
    const timeout = setTimeout(() => controller.abort(), 1500);
    try {
      const result = await readPreview('preview_hover', position, controller.signal);
      const samePoint = pointer && pointer.x === position.x && pointer.y === position.y;
      const bounds = result.element?.layout, mask = result.element?.content_mask;
      const stillInside = pointer && bounds && mask && [bounds, mask].every(box => pointer.x >= box.x && pointer.y >= box.y && pointer.x < box.x + box.width && pointer.y < box.y + box.height);
      if (version === pickVersion && current?.runtime_id === runtime && current.window.generation === result.generation && (samePoint || stillInside)) {
        picked = result.element; pickedPoint = position; pickedGeneration = result.generation; drawOverlays();
      }
    } catch { /* Stale frames and superseded pointer reads are retried without a notification. */ }
    finally {
      clearTimeout(timeout);
      if (pickController === controller) {
        pickController = null;
        if (pointer && (pointer.x !== position.x || pointer.y !== position.y || !picked)) queuePick();
      }
    }
  }, Math.max(0, lastPick + 100 - performance.now()));
}
async function grabAtPointer(signal) {
  if (!pointer || !current || paintedEpoch !== current.epoch) return null;
  const position = {...pointer, reference:current.reference}, runtime = current.runtime_id, window = current.window.window;
  const result = await readPreview('preview_grab', position, signal);
  if (current?.runtime_id !== runtime || current.window.window !== window) throw new Error('The native window changed. Select the element again.');
  return {...result, position};
}
function cancelCopy() {
  copyVersion++; copyController?.abort(); copyController = null; copyBusy = false;
}
async function copyContext(note) {
  if (copyBusy) return;
  copyBusy = true;
  const version = ++copyVersion, controller = new AbortController(); copyController = controller;
  const timeout = setTimeout(() => controller.abort(), 3000);
  try {
    let content;
    if (note) content = formatAnnotation(note);
    else if (selected) content = formatReference(selected);
    else {
      const result = await grabAtPointer(controller.signal); if (!result) return;
      content = formatReference(result.element);
    }
    if (version !== copyVersion || controller.signal.aborted) return;
    await navigator.clipboard.writeText(content);
    status('Context copied · paste it into your agent');
  } catch (failure) { if (version === copyVersion) error(failure.message); }
  finally {
    clearTimeout(timeout);
    if (version === copyVersion) { copyBusy = false; copyController = null; }
  }
}
async function commentAtPointer() {
  const version = ++commentVersion;
  try {
    const result = await grabAtPointer();
    if (result && !selected && selecting && version === commentVersion) openComposer(result.element, result.generation, null, {x:result.position.clientX, y:result.position.clientY});
  } catch (failure) { error(failure.message); }
}
function setMenu(open) {
  clearTimeout(menuTimer); menuOpen = open;
  if (open) { cancelPick(); $('picker-label').hidden = true; }
  if (!open) { menuPinned = false; hoveredID = null; }
  $('annotations-panel').hidden = !open; $('annotations-menu').classList.toggle('open', open);
  $('annotations-toggle').setAttribute('aria-expanded', String(open));
  drawOverlays(); if (open) queueResolution();
}
function renderNotes() {
  $('annotation-count').textContent = $('list-count').textContent = notes.items.length;
  $('annotations-toggle').classList.toggle('has-notes', notes.items.length > 0);
  $('annotation-empty').hidden = notes.items.length > 0; $('copy').disabled = !notes.items.length;
  const retained = new Set(notes.items.map(note => targetKey(note.element, note.generation)));
  if (selected) retained.add(targetKey(selected, selectedGeneration));
  for (const key of geometries.keys()) if (!retained.has(key)) geometries.delete(key);
  $('annotation-list').replaceChildren(); $('pins').replaceChildren(); pins.clear();
  for (const note of notes.items) {
    const row = document.createElement('li'); row.className = 'annotation-row'; row.dataset.id = note.id;
    const button = document.createElement('button'); button.className = 'annotation-item'; button.type = 'button';
    const number = document.createElement('span'); number.className = 'note-number'; number.textContent = note.number;
    const content = document.createElement('span'); content.className = 'annotation-copy';
    const comment = document.createElement('span'); comment.className = 'annotation-comment'; comment.textContent = note.comment;
    const target = document.createElement('span'); target.className = 'annotation-target'; target.textContent = labelOf(note.element); target.title = sourceOf(note.element);
    const location = document.createElement('span'); location.className = 'annotation-location';
    content.append(comment, target, location); button.append(number, content);
    button.onmouseenter = button.onfocus = () => previewNote(note.id);
    button.onmouseleave = button.onblur = () => previewNote(null);
    button.onclick = () => openComposer(note.element, note.generation, note);
    const remove = document.createElement('button'); remove.type = 'button'; remove.className = 'icon-button delete-note';
    remove.setAttribute('aria-label', 'Delete annotation '+note.number); remove.textContent = '×';
    remove.onclick = () => { notes.remove(note.id); hoveredID = null; renderNotes(); $('annotations-toggle').focus(); if (notes.warning) error(notes.warning); };
    row.append(button, remove); $('annotation-list').append(row);
    const pin = document.createElement('button'); pin.type = 'button'; pin.className = 'pin'; pin.textContent = note.number;
    pin.setAttribute('aria-label', 'Edit annotation '+note.number); pin.onmouseenter = pin.onfocus = () => previewNote(note.id);
    pin.onmouseleave = pin.onblur = () => previewNote(null); pin.onclick = () => openComposer(note.element, note.generation, note);
    pins.set(note.id, pin); $('pins').append(pin);
  }
  drawOverlays();
}
function queueResolution() {
  if (resolving || resolveTimer || !current || !(menuOpen || hoveredID || selected)) return;
  const candidates = menuOpen ? [...notes.items] : notes.items.filter(note => note.id === hoveredID);
  if (selected) candidates.push({element:selected, generation:selectedGeneration});
  const targets = [...new Map(candidates.map(note => [targetKey(note.element, note.generation), note])).values()]
    .filter(note => !geometryFor(note.element, note.generation)
      && geometries.get(targetKey(note.element, note.generation))?.generation !== current.window.generation);
  if (!targets.length) return;
  resolveTimer = setTimeout(async () => {
    resolveTimer = null; if (!current || !(menuOpen || hoveredID || selected)) return;
    resolving = true; nextResolve = performance.now() + 500;
    const runtime = current.runtime_id, window = current.window.window;
    try {
      // Geometry lookups use HTTP so paged reads never queue ahead of native input.
      const result = await readPreview('preview_resolve', {reference:current.reference, targets:targets.map(note =>
        note.element.stable_id ? {stable_id:note.element.stable_id} : {id:note.element.id, generation:note.generation})});
      if (current?.runtime_id === runtime && current.window.window === window) {
        result.elements.forEach((element, index) => {
          const key = targetKey(targets[index].element, targets[index].generation);
          if (notes.items.some(note => targetKey(note.element, note.generation) === key)
            || selected && targetKey(selected, selectedGeneration) === key) geometries.set(key, {generation:result.generation, element});
        });
        drawOverlays();
      }
    } catch { /* A changing capture is retried while the annotation UI is active. */ }
    finally { resolving = false; }
  }, Math.max(0, nextResolve - performance.now()));
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
      const resized = screen.width !== w || screen.height !== h || screen.hidden;
      if (resized) { screen.width = w; screen.height = h; }
      context.drawImage(next.frame, 0, 0, w, h); screen.hidden = false; $('empty-state').hidden = true;
      paintedEpoch = next.epoch; draws++;
      const now = performance.now(); lastPaint = now;
      if (now - lastRate > 1000) { fps = Math.round(draws * 1000 / (now - lastRate)); draws = 0; lastRate = now; }
      if (resized) fitWindow();
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
  if (next.error) { cancelCopy(); cancelPick(); current = null; paintedEpoch = null; video?.abort(); connection('Waiting for app'); $('connection').title = next.error; drawOverlays(); return; }
  const changed = current?.epoch !== next.epoch;
  if (changed || current?.window.generation !== next.window.generation) cancelPick();
  current = next;
  if (notes.activate(next.runtime_id, next.window.window)) {
    closeComposer(); geometries.clear(); renderNotes();
  }
  if (changed) {
    cancelCopy(); commentVersion++; pointer = null; paintedEpoch = null; wheel = new Wheel(); wheelPoint = hover = null;
    latest?.frame.close(); latest = null;
    video?.abort(); video = null; fitWindow();
  }
  if (!video) void stream(next.epoch);
  if (next.capture_errors?.h264 && codec === 'h264') {
    codec = 'jpeg'; video?.abort(); video = null; void stream(next.epoch);
    error(next.capture_errors.h264 + ' · Using JPEG fallback.');
  }
  const live = paintedEpoch === next.epoch && performance.now() - lastPaint < 2000;
  connection(live ? 'Live' : 'Connecting', live ? 'live' : 'connecting');
  $('fps').firstChild.textContent = (live ? fps : '—')+' ';
  $('connection').title = `${codec === 'h264' ? 'H.264 hardware video' : 'JPEG fallback'} · ${Math.round(next.window.content.width)} × ${Math.round(next.window.content.height)} pt · ${next.window.translucent ? 'translucent' : 'opaque'}`;
  drawOverlays(); queueResolution(); queuePick();
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
  if (!token) { connection('Authentication required', 'error'); error('Open the authenticated preview URL printed by rake dev.'); return; }
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
    pending.clear(); cancelCopy(); commentVersion++; cancelPick(); current = null; paintedEpoch = null; video?.abort(); video = null;
    drawOverlays(); connection('Reconnecting'); setTimeout(connect, 1000);
  };
}
function point(event, box = screen.getBoundingClientRect()) {
  if (!current || current.epoch !== paintedEpoch) return null;
  const result = position(event, box, current.window);
  return result ? {...result, reference:current.reference} : null;
}
async function send(action, position, input) {
  if (!position) return;
  try { return await request(action, {...position, ...(input ? {input} : {})}); }
  catch (failure) { error(failure.message); }
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
  selecting = value; hover = null; cancelPick(); cancelCopy(); commentVersion++;
  document.body.classList.toggle('selecting', value); $('mode-hint').hidden = !value || !$('composer').hidden;
  $('select').setAttribute('aria-pressed', String(value)); $('interact').setAttribute('aria-pressed', String(!value));
  if (value) queuePick(); else drawOverlays();
}
$('select').onclick = () => mode(true);
$('interact').onclick = () => { mode(false); closeComposer(); };
$('reconnect').onclick = () => { void request('preview_restart').catch(failure => error(failure.message)); };
$('cancel').onclick = () => { closeComposer(); $('select').focus(); };
$('copy-context').onclick = () => { void copyContext(); };
$('composer').onsubmit = event => {
  event.preventDefault(); if (!selected) return;
  try {
    notes.save($('comment').value, selected, selectedGeneration, editingID);
    closeComposer(); renderNotes(); $('select').focus();
    status(notes.warning || 'Annotation saved', notes.warning ? 'error' : 'info');
  } catch (failure) { error(failure.message); }
};
$('comment').onkeydown = event => { if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) { event.preventDefault(); $('composer').requestSubmit(); } };
$('annotations-menu').onmouseenter = () => setMenu(true);
$('annotations-menu').onmouseleave = () => { if (!menuPinned) menuTimer = setTimeout(() => setMenu(false), 160); };
$('annotations-menu').onfocusin = () => setMenu(true);
$('annotations-menu').onfocusout = event => { if (!event.currentTarget.contains(event.relatedTarget) && !menuPinned) setMenu(false); };
$('annotations-toggle').onclick = () => { if (menuPinned) setMenu(false); else { menuPinned = true; setMenu(true); } };
document.addEventListener('pointerdown', event => { if (!$('annotations-menu').contains(event.target)) setMenu(false); });
// Embedded browsers may route their native Copy command straight to this event.
// Leave selected DOM text and editable fields to the browser's ordinary copy flow.
document.addEventListener('copy', event => {
  if (event.target.closest?.('input,textarea,[contenteditable=true]') || window.getSelection()?.toString()) { cancelCopy(); return; }
  const note = notes.items.find(note => note.id === hoveredID);
  if (!(note || selected || pointer)) return;
  event.preventDefault();
  const element = selected || picked;
  if (note || element) event.clipboardData?.setData('text/plain', note ? formatAnnotation(note) : formatReference(element));
  void copyContext(note);
});
document.addEventListener('focusin', event => { if (event.target.closest?.('input,textarea,[contenteditable=true]')) cancelCopy(); });
document.addEventListener('keydown', event => {
  if (event.key === 'Escape') { closeComposer(); setMenu(false); mode(false); $('interact').focus(); return; }
  if (event.target.closest('input,textarea,[contenteditable=true]') || event.isComposing) return;
  if ((event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey && event.key.toLowerCase() === 'c') {
    if (window.getSelection()?.toString()) return;
    const note = notes.items.find(note => note.id === hoveredID);
    if (note || selected || pointer) { event.preventDefault(); if (!event.repeat) void copyContext(note); }
    return;
  }
  if (event.metaKey || event.ctrlKey || event.altKey || event.repeat) return;
  if (event.key.toLowerCase() === 'c') { event.preventDefault(); if (selecting && pointer) void copyContext(); else mode(true); }
  if (event.key === 'Enter' && selecting && pointer && !selected && !menuOpen) { event.preventDefault(); void commentAtPointer(); }
  if (event.key.toLowerCase() === 'v') { event.preventDefault(); mode(false); closeComposer(); }
});
screen.addEventListener('click', async event => {
  event.preventDefault(); hover = null;
  const position = point(event); if (!position) return;
  pointer = {...position, clientX:event.clientX, clientY:event.clientY};
  if (!selecting) { void send('preview_input', position, {type:'click'}); return; }
  if (!$('composer').hidden) { $('comment').focus(); return; }
  void commentAtPointer();
});
screen.addEventListener('wheel', event => {
  event.preventDefault(); const next = point(event); if (!next) return;
  const multiplier = event.deltaMode === 1 ? 20 : event.deltaMode === 2 ? current.window.content.height : 1;
  cancelPick(); drawOverlays();
  wheel.add(event.deltaX * multiplier, event.deltaY * multiplier); wheelPoint = next; wheelEnd = false;
  clearTimeout(wheelTimer); wheelTimer = setTimeout(() => { wheelEnd = true; scheduleInput(); }, 100);
  scheduleInput();
}, {passive:false});
screen.addEventListener('mousemove', event => {
  const preview = screen.getBoundingClientRect(), position = point(event, preview);
  pointer = position ? {...position, clientX:event.clientX, clientY:event.clientY} : null;
  if (selecting && !selected && !menuOpen) {
    const bounds = picked && stageBounds(geometryFor(picked, pickedGeneration), preview);
    if (bounds && (event.clientX < preview.left + bounds.left || event.clientY < preview.top + bounds.top || event.clientX >= preview.left + bounds.left + bounds.width || event.clientY >= preview.top + bounds.top + bounds.height)) picked = null;
    scheduleOverlays(); queuePick();
  } else if (!selecting) { hover = position; scheduleInput(); }
});
screen.addEventListener('mouseleave', () => { hover = pointer = null; cancelPick(); drawOverlays(); });
$('copy').onclick = async () => {
  try { await navigator.clipboard.writeText(formatAnnotations(notes.items)); status('Annotations copied · paste them into your agent'); }
  catch (failure) { error(failure.message); }
};
new ResizeObserver(() => { drawOverlays(); positionComposer(); }).observe(screen);
new ResizeObserver(positionComposer).observe($('composer'));
window.addEventListener('resize', () => { pointer = null; cancelPick(); fitWindow(); });
setInterval(() => { if (channel?.readyState === WebSocket.OPEN && pending.size < 4) void request('preview_state').catch(() => {}); }, 5000);
renderNotes(); connect();
