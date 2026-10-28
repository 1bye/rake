import {InspectorState, commandArguments, identity, matches, request, requests, safeJSON, stateName, traces} from '/__rake/inspector-model.js';

const $ = id => document.getElementById(id);
const panels = ['Workspace', 'Routes', 'Tasks', 'Operations', 'Decisions', 'Console', 'Requests', 'Traces', 'Elements', 'Reload'];
const model = new InspectorState();
let panel = 'Workspace', instances = [], routes = [], commands = [], info = null, development = null;
let filter = {}, search = '', page = 0, paused = false, polling = false, mutating = false, connected = false;
let receipt = null, operation = null, selectedOperation = null, output = {records: [], dropped: 0}, level = 'ALL', source = 'Structured', taskOwner = 'All';
const expanded = new Set(), drafts = new Map();
let elements = null, selectedElement = null, elementDetail = null, elementError = null;
let rendered = null, workspaceFocus = null, elementTab = 'Styles', resizing = false;
const paneSizes = new Map(), scrollPositions = new Map();
let awaitingSuccessor = null;
let picking = false, pickSequence = 0, hoveredBoundary = null, highlightSent = false, inspectionBusy = false;
const parameters = new URLSearchParams(location.search);
const secret = location.hash.startsWith('#__rake_control=') ? location.hash.slice('#__rake_control='.length) : null;
if (secret && /^[A-Za-z0-9_-]{32,128}$/.test(secret)) sessionStorage.setItem('rake-dev-control-token', secret);
if (secret !== null) history.replaceState(history.state, '', location.pathname + location.search);
const token = sessionStorage.getItem('rake-dev-control-token');

function el(tag, text, className) {
  const element = document.createElement(tag);
  if (text !== undefined && text !== null) element.textContent = String(text);
  if (className) element.className = className;
  return element;
}
function append(parent, ...children) { parent.append(...children.filter(Boolean)); return parent; }
function button(label, action, disabled = false, className = '') {
  const element = el('button', label, className);
  element.type = 'button'; element.disabled = disabled;
  element.addEventListener('click', action);
  return element;
}
function text(value, limit = 1000) {
  const string = typeof value === 'string' ? value : JSON.stringify(value) ?? 'Unavailable';
  return string.length > limit ? string.slice(0, limit) + '…' : string;
}
function details(label, value, key) {
  const element = append(el('details'), el('summary', label), el('pre', text(value, 32000)));
  element.open = expanded.has(key);
  element.addEventListener('toggle', () => element.open ? expanded.add(key) : expanded.delete(key));
  return element;
}
function badge(label) { return el('span', label, 'badge'); }
function card(title, ...children) { return append(el('section', null, 'card'), el('h2', title), ...children); }
function row(...children) { return append(el('div', null, 'row'), ...children); }
function empty(message) { return el('p', message, 'empty'); }
function label(name, input) { return append(el('label', name), input); }
function select(values, value, onChange) {
  const element = el('select');
  for (const [key, name] of values) {
    const option = el('option', name); option.value = key; element.append(option);
  }
  element.value = value;
  if (onChange) element.addEventListener('change', () => onChange(element.value));
  return element;
}
function showError(error) { $('error').hidden = !error; $('error').textContent = error ? String(error.message || error) : ''; }
function message(value) { $('notice').textContent = value; $('notice').title = value; }
function supported(command) { return connected && !mutating && info?.commands.includes(command); }
function action(label, command, className = '') {
  const control = button(label, () => mutate(command).catch(showError), !supported(command.type), className);
  control.dataset.focus = 'action:' + JSON.stringify(command);
  return control;
}
function related(fields, target) {
  const context = {};
  for (const key of ['task_id', 'tab_id', 'entry_id', 'operation_id', 'request_id']) {
    if (identity(fields[key]) !== null) context[key] = fields[key];
  }
  if (target === 'Tasks') {
    delete context.operation_id; delete context.request_id;
  }
  return button(target, () => { filter = context; switchPanel(target); }, !Object.keys(context).length);
}
function table(headers, rows) {
  if (!rows.length) return empty('No matching records.');
  const table = el('table'), head = el('tr');
  for (const heading of headers) head.append(el('th', heading));
  table.append(append(el('thead'), head));
  const body = el('tbody');
  for (const cells of rows) {
    const row = el('tr');
    for (const value of cells) row.append(append(el('td'), value instanceof Node ? value : el('span', value)));
    body.append(row);
  }
  table.append(body);
  return append(el('div', null, 'table-wrap'), table);
}
function paged(items, renderer, size = 100) {
  const matched = items.filter(item => !search || JSON.stringify(item).toLowerCase().includes(search.toLowerCase()));
  page = Math.min(page, Math.max(0, Math.ceil(matched.length / size) - 1));
  const start = page * size;
  const content = renderer(matched.slice(start, start + size));
  const pager = row(button('Previous', () => { page--; render(true); }, !page), el('span', `${matched.length ? start + 1 : 0}–${Math.min(start + size, matched.length)} of ${matched.length}`),
    button('Next', () => { page++; render(true); }, start + size >= matched.length));
  pager.className = 'pager';
  return append(el('div'), content, pager);
}
function switchPanel(name) {
  if (panel === 'Elements') stopInspection();
  panel = name; page = 0; selectedOperation = null; operation = null;
  for (const control of $('panels').children) {
    const active = control.dataset.panel === name;
    control.setAttribute('aria-current', active ? 'page' : 'false');
    control.setAttribute('aria-selected', String(active));
    control.tabIndex = active ? 0 : -1;
  }
  $('title').textContent = name;
  $('content').setAttribute('aria-labelledby', 'panel-' + name.toLowerCase());
  render(true); refresh();
}
function properties(values) {
  return append(el('dl', null, 'property-list'), ...values.map(([name, value]) =>
    append(el('div', null, 'property-row'), el('dt', name), el('dd', value == null ? '—' : String(value)))));
}
function rememberScroll(element, key) {
  element.dataset.scroll = key;
  element.addEventListener('scroll', () => scrollPositions.set(key, element.scrollTop));
  return element;
}
function splitView(master, detail, key) {
  const root = el('div', null, 'split-view');
  const left = rememberScroll(append(el('div', null, 'pane-master'), master), key + ':master');
  const right = rememberScroll(append(el('div', null, 'pane-detail'), detail), key + ':detail');
  left.style.width = paneSizes.get(key) || (key === 'workspace' ? '25%' : key === 'console' ? '68%' : '60%');
  const handle = el('div', null, 'split-handle');
  handle.tabIndex = 0; handle.dataset.focus = 'split:' + key;
  handle.setAttribute('role', 'separator');
  handle.setAttribute('aria-label', 'Resize ' + key + ' panes');
  handle.setAttribute('aria-orientation', 'vertical');
  handle.setAttribute('aria-valuemin', '0'); handle.setAttribute('aria-valuemax', '100');
  handle.setAttribute('aria-valuenow', String(parseFloat(left.style.width)));
  function resize(width) {
    const available = root.getBoundingClientRect().width;
    const value = Math.round(Math.max(180, Math.min(width, available - 225)));
    left.style.width = (value / available * 100).toFixed(1) + '%'; paneSizes.set(key, left.style.width);
    handle.setAttribute('aria-valuenow', String(Math.round(value / available * 100)));
  }
  let drag = null;
  handle.addEventListener('pointerdown', event => {
    drag = {x: event.clientX, width: left.getBoundingClientRect().width}; resizing = true;
    handle.setPointerCapture(event.pointerId); event.preventDefault();
  });
  handle.addEventListener('pointermove', event => { if (drag) resize(drag.width + event.clientX - drag.x); });
  const finish = () => { drag = null; resizing = false; };
  handle.addEventListener('pointerup', finish); handle.addEventListener('lostpointercapture', finish);
  handle.addEventListener('pointercancel', finish);
  handle.addEventListener('keydown', event => {
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    resize(event.key === 'Home' ? 180 : event.key === 'End' ? root.getBoundingClientRect().width - 225 : left.getBoundingClientRect().width + (event.key === 'ArrowLeft' ? -16 : 16));
  });
  return append(root, left, handle, right);
}
function treeRow(name, kind, id, depth, active, onInspect, meta = '') {
  const control = button('', onInspect, false, 'tree-row tree-level-' + depth);
  control.dataset.focus = 'workspace:' + kind + ':' + id;
  control.setAttribute('aria-label', 'Inspect ' + kind + '-' + id);
  control.setAttribute('aria-pressed', String(active));
  control.append(el('span', kind === 'window' ? '▣' : kind === 'tab' ? '▱' : '◇', 'tree-icon'), el('span', name, 'tree-label'), el('span', meta, 'tree-meta'));
  return control;
}
function choose(runtime) {
  stopInspection(); pickSequence = 0;
  awaitingSuccessor = null;
  model.reset(runtime); info = null; routes = []; commands = []; connected = false;
  elements = null; selectedElement = null; elementDetail = null; elementError = null;
  selectedOperation = null; operation = null; filter = {}; drafts.clear(); expanded.clear(); scrollPositions.clear(); workspaceFocus = null; elementTab = 'Styles'; page = 0;
  $('runtime').value = runtime || ''; showError(null);
  message(runtime ? 'Connecting to a fresh runtime. Previous runtime IDs and observations have been cleared.' : 'Start an app with rake dev, or select an available runtime.');
  render(true);
}

class TransportError extends Error {}
class RuntimeError extends Error {}
async function rpc(path, body) {
  if (!token) throw new TransportError('Open the private inspector launch URL printed by rake dev or rake inspect.');
  const controller = new AbortController();
  const deadline = setTimeout(() => controller.abort(), 14000);
  try {
    const response = await fetch('/__rake/' + path, {method: 'POST', headers: {'Content-Type': 'application/json', 'X-Rake-Token': token}, body: JSON.stringify(body), signal: controller.signal});
    if (!response.ok) throw new TransportError(`Inspector HTTP ${response.status} ${response.statusText}`);
    return await response.json();
  } catch (error) {
    if (error instanceof TransportError) throw error;
    throw new TransportError(error.message);
  } finally { clearTimeout(deadline); }
}
async function exchange(envelope) {
  const result = await rpc('inspect', {request: envelope});
  if ('Err' in result) throw new RuntimeError(text(result.Err));
  if (!result.Ok) throw new TransportError('Runtime returned an invalid response.');
  return result.Ok;
}
async function query(type, fields = {}, runtime = model.runtime) {
  return await exchange(request(runtime, {type: 'query', value: {type, ...fields}}));
}
async function mutate(command, retry = null) {
  if (mutating) return;
  const envelope = retry || request(model.runtime, {type: 'command', value: command}, crypto.randomUUID());
  receipt = {envelope, pending: true}; mutating = true; showError(null); render(true);
  try {
    const result = await exchange(envelope);
    receipt = {envelope, result};
    if (model.runtime === envelope.runtime_id && result.type === 'submitted') selectedOperation = result.value.operation_id;
    message(`Request ${envelope.key} answered. Submission and decision acknowledgement are distinct from operation commit.`);
  } catch (error) {
    receipt = {envelope, error: String(error.message), uncertain: error instanceof TransportError};
    showError(error);
  } finally { mutating = false; render(true); refresh(); }
}
async function refresh() {
  if (polling) return;
  polling = true;
  const runtime = model.runtime;
  try {
    const found = await rpc('inspect_instances', {});
    if (JSON.stringify(found) !== JSON.stringify(instances)) {
      const previous = instances.find(item => item.info.runtime_id === runtime);
      instances = found;
      $('runtime').replaceChildren(el('option', 'Select a runtime'));
      $('runtime').firstChild.value = '';
      for (const item of instances) {
        const option = el('option', `${item.application_id} · ${item.info.platform} · ${item.info.runtime_id.slice(0, 8)}`);
        option.value = item.info.runtime_id; $('runtime').append(option);
      }
      $('runtime').value = runtime || '';
      const lineage = runtime ? {runtime, application_id: previous?.application_id} : awaitingSuccessor;
      const successors = instances.filter(item => lineage && item.restored_from === lineage.runtime && item.application_id === lineage.application_id);
      if (successors.length === 1 && (!connected || previous?.reload_phase === 'ready')) {
        choose(successors[0].info.runtime_id);
      } else if (runtime && !instances.some(item => item.info.runtime_id === runtime)) {
        choose(null); awaitingSuccessor = lineage;
        message('Runtime disconnected. Waiting for its checkpointed successor, or select another runtime explicitly.');
      } else if (!runtime && !awaitingSuccessor) {
        const selector = parameters.get('instance');
        const matching = instances.filter(item => item.application_id === selector || item.info.runtime_id === selector);
        const next = matching.length === 1 ? matching[0] : !selector && instances.length === 1 ? instances[0] : null;
        choose(next?.info.runtime_id || null);
      }
    }
    const cdpTargets = await rpc('inspect_cdp', {});
    const cdpTarget = cdpTargets.find(target => target.id === model.runtime);
    $('open-cdp').hidden = !cdpTarget;
    if (cdpTarget && /^devtools:\/\/devtools\/bundled\/devtools_app\.html\?ws=127\.0\.0\.1:[0-9]+\/[a-f0-9]+\/[a-f0-9-]+$/.test(cdpTarget.devtoolsFrontendUrl)) {
      $('open-cdp').href = cdpTarget.devtoolsFrontendUrl;
    } else { $('open-cdp').hidden = true; $('open-cdp').removeAttribute('href'); }
    development = await rpc('development', {});
    if (panel === 'Console') {
      const delta = await rpc('inspect_output', {after: output.records.at(-1)?.sequence || 0});
      output = {records: [...output.records, ...delta.records].slice(-128), dropped: delta.dropped};
    }
    if (runtime && runtime === model.runtime) {
      if (!info) {
        const metadata = await Promise.all([query('info'), query('routes'), query('commands')]);
        if (runtime !== model.runtime) return;
        [info, routes, commands] = metadata.map(reply => reply.value);
      }
      const event = await query('events', {cursor: model.cursor}, runtime);
      if (runtime !== model.runtime) return;
      model.event(event.value);
      const diagnostics = await query('diagnostics', {after: model.after, kind: null}, runtime);
      model.diagnostics(diagnostics.value, runtime);
      if (selectedOperation != null && (panel === 'Operations' || receipt?.envelope.runtime_id === runtime)) {
        const selected = selectedOperation;
        try {
          const detail = await query('operation', {operation: selected}, runtime);
          if (runtime === model.runtime) operation = detail.value;
        } catch (error) {
          if (!(error instanceof RuntimeError)) throw error;
          if (runtime === model.runtime && selectedOperation === selected) {
            operation = {operation_id: selected, state: 'Unavailable', error: error.message};
            selectedOperation = null;
          }
        }
      }
      if (panel === 'Elements' && info?.queries?.includes('inspect_tree')) {
        const tree = await query('inspect_tree', {}, runtime);
        if (runtime !== model.runtime) return;
        elements = tree.value;
        if (hoveredBoundary && hoveredBoundary.generation !== elements.capture) hoveredBoundary = null;
      }
      if (!connected) message('Connected to ' + (info?.platform || 'application') + ' runtime.');
      connected = true; showError(null);
    }
  } catch (error) {
    if (runtime === model.runtime) { connected = false; showError(error); }
  } finally {
    polling = false;
    $('connection').textContent = model.runtime ? `${connected ? 'Connected' : 'Disconnected'} · ${model.runtime.slice(0, 8)}` : 'No runtime';
    $('connection').dataset.connected = String(connected);
    render();
  }
}

function workspace() {
  const snapshot = model.snapshot;
  if (!snapshot) return empty('No workspace snapshot yet. The inspector stays available while the application starts or restarts.');
  const hosts = snapshot.windows || [{id: snapshot.window_id, selected: snapshot.selected}];
  const selectedTabs = new Set(hosts.map(host => host.selected));
  const currentTab = snapshot.tabs.find(tab => selectedTabs.has(tab.id)) || snapshot.tabs[0];
  const findTab = id => snapshot.tabs.find(tab => tab.id === id);
  const focusExists = workspaceFocus && (workspaceFocus.kind === 'window' ? hosts.some(host => host.id === workspaceFocus.id) : workspaceFocus.kind === 'tab' ? !!findTab(workspaceFocus.id) : snapshot.tabs.some(tab => tab.entries.some(entry => entry.id === workspaceFocus.id)));
  if (!focusExists) workspaceFocus = currentTab ? {kind: 'tab', id: currentTab.id} : hosts[0] ? {kind: 'window', id: hosts[0].id} : null;
  const inspect = (kind, id) => { workspaceFocus = {kind, id}; render(true); };
  const isFocused = (kind, id) => workspaceFocus?.kind === kind && workspaceFocus.id === id;
  const treeRows = [];
  for (const host of hosts) {
    const tabs = snapshot.tabs.filter(tab => (tab.window_id ?? snapshot.window_id) === host.id);
    treeRows.push(['window-' + host.id, 'window', host.id, 0, host.visibility || 'Unknown']);
    for (const tab of tabs) {
      const current = tab.entries[tab.cursor];
      treeRows.push([current.destination.route + ' · tab-' + tab.id, 'tab', tab.id, 1, selectedTabs.has(tab.id) ? 'selected' : '']);
      for (const [index, entry] of tab.entries.entries()) {
        treeRows.push([entry.destination.route + ' · entry-' + entry.id, 'entry', entry.id, 2, (entry.dirty ? '● ' : '') + (index === tab.cursor ? 'current' : index < tab.cursor ? 'back' : 'forward'), entry]);
      }
    }
  }
  const tree = paged(treeRows, items => append(el('div', null, 'workspace-tree'), ...items.map(([name, kind, id, depth, meta]) => treeRow(name, kind, id, depth, isFocused(kind, id), () => inspect(kind, id), meta))));
  const master = append(el('div'), el('h2', 'Workspace', 'pane-heading'),
    append(el('div', null, 'workspace-summary'), el('span', `${hosts.length} windows · ${snapshot.tabs.length} tabs`, 'muted')), hosts.length ? tree : empty('No open windows. Open a destination in Routes to create one.'));
  const detail = append(el('div'), el('h2', 'Properties', 'pane-heading'));
  if (workspaceFocus?.kind === 'window') {
    const host = hosts.find(host => host.id === workspaceFocus.id);
    detail.append(card('window-' + host.id, properties([
      ['Window', 'window-' + host.id], ['Visibility', host.visibility || 'Unknown'], ['Selected tab', host.selected == null ? 'None' : 'tab-' + host.selected], ['Close operation', host.closing == null ? 'None' : 'op-' + host.closing],
    ]), row(...(supported('close_window') && info?.supported_targets?.includes('NewWindow') ? [action('Close window', {type: 'close_window', window: host.id})] : []))));
  } else if (workspaceFocus?.kind === 'tab') {
    const tab = findTab(workspaceFocus.id), current = tab.entries[tab.cursor];
    detail.append(card('tab-' + tab.id + ' / ' + current.destination.route,
      properties([['Window', 'window-' + (tab.window_id ?? snapshot.window_id)], ['Tab', 'tab-' + tab.id], ['Destination', current.destination.route], ['Parameters', text(current.destination.params)], ['Work policy', tab.mode], ['Presentation', selectedTabs.has(tab.id) ? 'Selected' : 'Hidden tab'], ['History cursor', `${tab.cursor + 1} of ${tab.entries.length}`]]),
      row(action('Select', {type: 'select', tab: tab.id}), action('Back', {type: 'back', tab: tab.id}), action('Forward', {type: 'forward', tab: tab.id}), action(tab.mode === 'Live' ? 'Use Foreground' : 'Use Live', {type: 'mode', tab: tab.id, mode: tab.mode === 'Live' ? 'Foreground' : 'Live'}), action('Close tab', {type: 'close', tab: tab.id}), related({tab_id: tab.id}, 'Tasks'), related({tab_id: tab.id}, 'Requests'))),
      el('h2', 'Retained history', 'pane-heading'), table(['Entry', 'Destination', 'Position', 'State'], tab.entries.map((entry, index) => [
        button('entry-' + entry.id, () => inspect('entry', entry.id)), append(el('div'), el('span', entry.destination.route), el('div', text(entry.destination.params), 'fields')),
        index === tab.cursor ? 'Current' : index < tab.cursor ? 'Back history' : 'Forward history', `${entry.dirty ? 'Dirty' : 'Clean'} · revision ${entry.revision}`])));
  } else if (workspaceFocus?.kind === 'entry') {
    const tab = snapshot.tabs.find(tab => tab.entries.some(entry => entry.id === workspaceFocus.id));
    const entry = tab.entries.find(entry => entry.id === workspaceFocus.id);
    detail.append(card('entry-' + entry.id + ' / ' + entry.destination.route, properties([
      ['Window', 'window-' + (tab.window_id ?? snapshot.window_id)], ['Tab', 'tab-' + tab.id], ['Entry', 'entry-' + entry.id], ['Destination', entry.destination.route], ['Parameters', text(entry.destination.params)], ['State', entry.dirty ? 'Dirty' : 'Clean'], ['Revision', entry.revision], ['Position', tab.entries[tab.cursor].id === entry.id ? 'Current' : 'Retained history'],
    ]), row(related({entry_id: entry.id}, 'Tasks'), related({entry_id: entry.id}, 'Requests'), related({entry_id: entry.id}, 'Console'))));
  } else detail.append(empty('No retained entries.'));
  const quit = snapshot.quit || {phase: 'idle', pending_tasks: [], pending_operations: []};
  const pendingQuit = ['waiting_for_jobs', 'guarding', 'stopping'].includes(quit.phase);
  const quitControls = row(badge(`Quit: ${quit.phase}`), el('span', `${quit.pending_tasks.length} finite jobs · ${quit.pending_operations.length} operations`));
  if (!pendingQuit && quit.phase !== 'ready') quitControls.append(action('Quit (wait for jobs)', {type: 'quit', options: {jobs: 'wait'}}), action('Quit and cancel jobs', {type: 'quit', options: {jobs: 'cancel'}}));
  else if (pendingQuit && quit.phase !== 'stopping') quitControls.append(action('Cancel quit', {type: 'cancel_operation', operation: quit.operation_id}));
  detail.append(card('Application', properties([['Runtime', snapshot.runtime_id], ['Generation', snapshot.reload.generation], ['Revision', snapshot.revision]]), quitControls,
    el('p', 'Quit checks every retained entry. Save, discard and cancel choices appear in Decisions.', 'hint')));
  if (snapshot.history_error) detail.append(el('p', snapshot.history_error, 'danger'));
  return splitView(master, detail, 'workspace');
}

function routeForm(route) {
  const draft = drafts.get('route:' + route.id) || {params: {}, enabled: {}, target: 'NewTab', mode: '', preserve: false, tab: model.snapshot?.selected};
  drafts.set('route:' + route.id, draft);
  const form = el('form');
  form.append(el('p', route.path, 'mono small'));
  const fields = el('div', null, 'grid');
  for (const parameter of route.params) {
    const input = el('input'); input.value = draft.params[parameter.name] || ''; input.autocomplete = 'off';
    input.setAttribute('aria-label', `${route.id} ${parameter.name}`);
    input.addEventListener('input', () => draft.params[parameter.name] = input.value);
    const field = label(`${parameter.name} · ${parameter.value_type}${parameter.required ? '' : ' (optional)'}`, input);
    if (!parameter.required) {
      const include = el('input'); include.type = 'checkbox'; include.checked = draft.enabled[parameter.name] || false; input.disabled = !include.checked;
      include.addEventListener('change', () => { draft.enabled[parameter.name] = include.checked; input.disabled = !include.checked; });
      field.append(label('Include optional parameter', include));
    }
    fields.append(field);
  }
  form.append(fields);
  const target = select((info?.supported_targets || []).map(value => [value, value]), draft.target, value => draft.target = value);
  const hosts = model.snapshot?.windows || [{id: model.snapshot?.window_id ?? 1}];
  const preferredHost = draft.window ?? model.snapshot?.window_id;
  const hostId = hosts.some(item => item.id === preferredHost) ? preferredHost : hosts[0]?.id;
  const host = select(hosts.map(item => [String(item.id), `window-${item.id}`]), hostId == null ? '' : String(hostId), value => draft.window = Number(value));
  const scope = select([['Application', 'All windows'], ['Window', 'Selected window']], draft.scope || 'Application', value => draft.scope = value);
  form.append(row(label('Window for Open / Reveal', host), label('Reveal scope', scope)));
  const mode = select([['', 'Default policy'], ['Foreground', 'Foreground'], ['Live', 'Live']], draft.mode, value => draft.mode = value);
  const preserve = el('input'); preserve.type = 'checkbox'; preserve.checked = draft.preserve;
  preserve.addEventListener('change', () => draft.preserve = preserve.checked);
  form.append(row(label('Open target', target), label('Work policy', mode), label('Preserve selection', preserve)));
  const tab = select((model.snapshot?.tabs || []).map(item => [String(item.id), `tab-${item.id}`]), String(draft.tab), value => draft.tab = Number(value));
  const buttons = row();
  for (const [name, type] of [['Open', 'open'], ['Reveal', 'reveal'], ['Navigate in tab', 'navigate'], ['Replace in tab', 'replace']]) {
    const submit = el('button', name); submit.type = 'submit'; submit.dataset.action = type;
    submit.disabled = !supported(type === 'replace' ? 'navigate' : type) || (['navigate', 'replace'].includes(type) && !(model.snapshot?.tabs.length));
    buttons.append(submit);
  }
  form.append(label('Exact tab for Navigate / Replace', tab), buttons);
  form.addEventListener('submit', event => {
    event.preventDefault();
    const params = {};
    for (const parameter of route.params) if (parameter.required || draft.enabled[parameter.name]) params[parameter.name] = draft.params[parameter.name] || '';
    const destination = {route: route.id, params}, type = event.submitter.dataset.action;
    const command = type === 'open' ? {type, destination, window: host.value ? Number(host.value) : null, options: {target: draft.target, mode: draft.mode || null, activation: draft.preserve ? 'PreserveSelection' : 'Activate'}} :
      type === 'reveal' ? {type, destination, window: host.value ? Number(host.value) : null, scope: scope.value} : {type: 'navigate', destination, tab: Number(tab.value), replace: type === 'replace'};
    mutate(command).catch(showError);
  });
  return card(route.id, form);
}
function routing() {
  return append(el('div'), el('p', 'Forms use the runtime’s destination registry. Parameter parsing, capabilities and navigation guards are validated by the shared runtime dispatcher.', 'hint'),
    paged(routes, items => append(el('div'), ...items.map(routeForm)), 20));
}

function owner(value) { return typeof value === 'string' ? value : Object.entries(value).map(([kind, id]) => `${kind.toLowerCase()}-${id}`).join(' '); }
function tasksPanel() {
  const tasks = (model.snapshot?.work.tasks || []).filter(task =>
    (!filter.task_id || identity(task.id) === identity(filter.task_id)) &&
    (!filter.tab_id || identity(task.origin.tab_id) === identity(filter.tab_id)) &&
    (!filter.entry_id || identity(task.origin.entry_id) === identity(filter.entry_id)) &&
    (taskOwner === 'All' || (typeof task.owner === 'string' ? task.owner : Object.keys(task.owner)[0]) === taskOwner));
  return append(el('div'), label('Lifetime owner', select(['All', 'Application', 'Tab', 'Entry'].map(value => [value, value]), taskOwner, value => { taskOwner = value; page = 0; render(true); })), el('p', 'Owner controls lifetime; origin records the initiating tab/entry even after it closes. Eligibility and state are authoritative. Application progress is unavailable unless emitted as diagnostics.', 'hint'),
    paged(tasks, items => table(['Task / resource', 'Lifetime owner', 'Causal origin', 'Run / generation', 'State / eligibility', 'Actions'], items.map(task => [
      append(el('div'), el('strong', `task-${task.id} / ${task.key}`), el('div', task.kind, 'muted')),
      owner(task.owner), `${task.origin.tab_id == null ? 'Tab unavailable' : `tab-${task.origin.tab_id}`} / ${task.origin.entry_id == null ? 'Entry unavailable' : `entry-${task.origin.entry_id}`}`,
      `${task.run_id == null ? 'No run' : `run-${task.run_id}`} / ${task.generation_id}`, `${stateName(task.state)} · ${task.eligible ? 'eligible' : 'ineligible'}`,
      row(action('Cancel task', {type: 'cancel_task', task: task.id}), related({task_id: task.id}, 'Requests'), related({task_id: task.id}, 'Console'), related({task_id: task.id}, 'Traces'),
        details('Task details', task, `task:${task.id}`))]))));
}
function operationsPanel() {
  const operations = (model.snapshot?.operations || []).filter(([id]) => !filter.operation_id || identity(id) === identity(filter.operation_id));
  return append(el('div'), paged(operations, items => table(['Operation', 'Status', 'Actions'], items.slice().reverse().map(([id, state]) => [
    `op-${id}`, stateName(state), row(button('Inspect result', async () => { selectedOperation = id; await refresh(); render(true); }, !connected),
      action('Cancel operation', {type: 'cancel_operation', operation: id}), related({operation_id: id}, 'Traces'), related({operation_id: id}, 'Console'),
      typeof state === 'object' ? details('Status details', state, `op:${id}`) : null)]))),
    operation ? card(`op-${operation.operation_id} / ${stateName(operation.state)}`, el('pre', text(operation, 32000))) : null,
    el('p', 'A committed navigation means the entry was attached. Data may still be loading in managed tasks. Missing completed operations have expired under runtime retention.', 'hint'));
}
function decisionsPanel() {
  const decisions = model.snapshot?.decisions || [];
  if (!decisions.length) return empty('No pending guard decisions.');
  return append(el('div'), ...decisions.map(decision => {
    const actions = row(...decision.choices.map(choice => action(choice.label, {type: 'respond', decision: decision.id, choice: choice.id})));
    const result = card(`decision-${decision.id} / op-${decision.operation_id}`, el('p', decision.message),
      el('p', `Retained entry-${decision.entry_id}. Choices are revalidated by the runtime; disconnecting approves nothing.`, 'hint'), actions);
    result.classList.add('decision'); return result;
  }));
}
function diagnosticRecords() { return model.diagnosticsRecords.filter(record => matches(record, filter)); }
function consolePanel() {
  const controls = row(label('Severity', select(['ALL', 'TRACE', 'DEBUG', 'INFO', 'WARN', 'ERROR'].map(value => [value, value]), level, value => { level = value; page = 0; render(true); })),
    label('Source', select(['Structured', 'Process', 'Both'].map(value => [value, value]), source, value => { source = value; page = 0; render(true); })));
  let rows = [];
  if (source !== 'Process') rows = diagnosticRecords().filter(record => record.kind !== 'request' && !record.kind.startsWith('span_') && (level === 'ALL' || record.level === level)).map(record => ({...record, source: 'runtime'}));
  if (source !== 'Structured' && !Object.keys(filter).length) rows.push(...output.records.map(record => ({...record, source: 'process'})));
  const logs = paged(rows.slice().reverse(), items => table(['Time', 'Source / level', 'Message / fields', 'Context'], items.map(record => record.source === 'process' ? [
    `${(record.elapsed_us / 1000).toFixed(1)} ms`, `PID ${record.pid} / ${record.stream}`,
    append(el('div'), el('span', record.text), record.truncated ? el('span', ' (truncated)', 'warning') : null), 'Unassociated process output'] : [
    `${(record.elapsed_us / 1000).toFixed(1)} ms`, `${record.level} / ${record.target}`, append(el('div'), el('span', text(record.fields.message || record.fields, 1000)), details('Structured fields', record, `record:${record.sequence}`)),
    row(related(record.fields, 'Tasks'), related(record.fields, 'Requests'), related(record.fields, 'Traces'))])));
  controls.className = 'console-toolbar';
  return splitView(append(el('div'), controls, el('p', 'Runtime tracing and bounded supervisor stdout/stderr have separate elapsed clocks. Process text is never assigned to the selected tab. Legacy log forwarding needs a host tracing bridge; arbitrary browser console output is outside capture.', 'hint'), logs),
    append(el('div'), el('h2', 'Application commands', 'pane-heading'), commands.length ? append(el('div', null, 'card'), ...commands.map(commandForm)) : empty('No application commands registered.')), 'console');
}
function commandForm(command) {
  const form = el('form');
  const initial = commandArguments(command.arguments);
  const input = el('textarea'); input.value = drafts.get('command:' + command.id) || JSON.stringify(initial, null, 2);
  input.setAttribute('aria-label', `${command.id} arguments`);
  input.addEventListener('input', () => drafts.set('command:' + command.id, input.value));
  const submit = el('button', 'Run command'); submit.type = 'submit'; submit.disabled = !supported('run');
  form.append(el('h3', `${command.id} · ${command.capability}`), details('Argument / result schema', command, `command:${command.id}`), el('p', 'Edit the JSON template to satisfy the advertised argument schema. Runtime decoding and application validation remain authoritative.', 'hint'), label('JSON arguments', input), submit);
  form.addEventListener('submit', event => { event.preventDefault(); try { mutate({type: 'run', id: command.id, arguments: safeJSON(input.value)}).catch(showError); } catch (error) { showError(error); } });
  return form;
}
function requestsPanel() {
  const data = requests(diagnosticRecords());
  return append(el('div'), el('p', 'Coverage: Rake HTTP and the native GPUI adapter. Browser GPUI-internal Fetch and third-party clients need their own adapters. Credentials, queries/fragments, headers and bodies are not captured. DNS/TLS and browser-hidden timings are unavailable.', 'hint'),
    paged(data, items => table(['Request', 'Method / URL', 'Status / lifecycle', 'Elapsed / body', 'Context'], items.map(item => [
      `request-${item.id}${item.started ? '' : ' · partial'}`, `${item.fields.method || 'Unknown method'} ${item.fields.url || 'URL not retained'}`,
      `${item.fields.status ?? 'No headers observed'} · ${item.fields.phase}`, `${item.fields.duration_us == null ? 'Timing unavailable' : (item.fields.duration_us / 1000).toFixed(2) + ' ms'} / ${item.fields.received_bytes ?? 'Unknown'} bytes`,
      row(related(item.fields, 'Tasks'), related({...item.fields, request_id: item.id}, 'Console'), related({...item.fields, request_id: item.id}, 'Traces'), details('Lifecycle events', item.events, `request:${item.id}`))]))));
}
function tracePanel() {
  const data = traces(diagnosticRecords()).filter(node => !search || JSON.stringify(node).toLowerCase().includes(search.toLowerCase()));
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', `0 0 1000 ${Math.max(50, Math.min(Math.max(0, data.length - page * 100), 100) * 26)}`);
  svg.setAttribute('role', 'img'); svg.setAttribute('aria-label', 'Instrumented span timeline in collector elapsed time'); svg.classList.add('trace-chart');
  const start = Math.min(...data.map(node => node.start ?? 0), 0), end = Math.max(1, ...data.map(node => node.end ?? node.start ?? 0));
  for (const [index, node] of data.slice(page * 100, page * 100 + 100).entries()) {
    const rect = document.createElementNS(svg.namespaceURI, 'rect');
    rect.setAttribute('x', 350 + ((node.start ?? 0) - start) / (end - start) * 640);
    rect.setAttribute('y', index * 26 + 7); rect.setAttribute('width', Math.max(2, (node.duration || 0) / (end - start) * 640));
    rect.setAttribute('height', 10); rect.setAttribute('fill', node.closed ? '#8ab57b' : '#7a888c');
    const title = document.createElementNS(svg.namespaceURI, 'title'); title.textContent = `${node.name}: ${node.duration == null ? 'duration unavailable' : node.duration + ' µs'}`; rect.append(title);
    const caption = document.createElementNS(svg.namespaceURI, 'text'); caption.setAttribute('x', Math.min(node.depth, 12) * 10 + 8); caption.setAttribute('y', index * 26 + 17); caption.textContent = `${node.id} ${node.name}`.slice(0, 48);
    svg.append(rect, caption);
  }
  return append(el('div'), el('p', 'Span identities preserve nesting even when names repeat. Timing is elapsed wall time, not CPU usage. Missing opens, parents or closes stay explicit after diagnostic loss.', 'hint'), data.length ? svg : null,
    paged(data, items => table(['Span / parent', 'Instrumented work', 'Elapsed', 'Capture / context'], items.map(node => [
      `${node.id} / ${node.parent ?? 'root'}`, `${'↳ '.repeat(Math.min(node.depth, 10))}${node.name}`,
      node.duration == null ? 'No duration observed' : `${(node.duration / 1000).toFixed(3)} ms`, row(badge(!node.opened || node.missingParent ? 'Partial ancestry' : node.closed ? 'Closed' : 'No close observed'),
        related(node.fields, 'Tasks'), related(node.fields, 'Requests'), related(node.fields, 'Console'), details('Span fields', node, `span:${node.id}`))]))));
}
function inspectionControl(value, runtime = model.runtime) {
  return exchange(request(runtime, {type: 'command', value}));
}
function stopInspection() {
  const runtime = model.runtime;
  const used = picking || highlightSent || hoveredBoundary;
  picking = false; hoveredBoundary = null; highlightSent = false;
  if (runtime && used) {
    inspectionControl({type: 'inspect_mode', enabled: false, lease_ms: 0}, runtime).catch(() => {});
    inspectionControl({type: 'highlight_element', id: null, generation: ''}, runtime).catch(() => {});
  }
}
async function inspectionLoop() {
  if (!inspectionBusy && connected && info?.element_interaction) {
    const runtime = model.runtime;
    inspectionBusy = true;
    try {
      if (picking) {
        const pick = (await query('inspect_pick', {}, runtime)).value;
        if (runtime === model.runtime) {
          picking = pick.mode;
          if (pick.picked && pick.pick_sequence > pickSequence) {
            pickSequence = pick.pick_sequence;
            if (elements?.capture !== pick.generation) elements = (await query('inspect_tree', {}, runtime)).value;
            await inspectElement(pick.picked);
          }
          if (picking) await inspectionControl({type: 'inspect_mode', enabled: true, lease_ms: 1000}, runtime);
          render(true);
        }
      } else if (hoveredBoundary) {
        const hover = hoveredBoundary;
        await inspectionControl({type: 'highlight_element', ...hover}, runtime);
        highlightSent = true;
      } else if (highlightSent) {
        await inspectionControl({type: 'highlight_element', id: null, generation: ''}, runtime);
        highlightSent = false;
      }
    } catch (error) { stopInspection(); showError(error); }
    finally { inspectionBusy = false; }
  }
  setTimeout(inspectionLoop, 100);
}
async function inspectElement(id) {
  const runtime = model.runtime;
  selectedElement = id; elementDetail = null; elementError = null;
  try {
    const detail = await query('inspect_element', {id}, runtime);
    if (runtime === model.runtime && selectedElement === id) elementDetail = detail.value;
  } catch (error) {
    if (runtime === model.runtime && selectedElement === id) elementError = error.message;
  }
  render(true);
}
function styleValue(value, key) {
  const color = value?.Color?.tag === 'Solid' ? value.Color.solid : null;
  if (color && ['h', 's', 'l', 'a'].every(component => Number.isFinite(color[component]) && color[component] >= 0 && color[component] <= 1)) {
    const css = `hsl(${(color.h * 360).toFixed(1)} ${(color.s * 100).toFixed(1)}% ${(color.l * 100).toFixed(1)}% / ${color.a})`;
    const swatch = el('span', null, 'color-swatch'); swatch.style.backgroundColor = css;
    return append(el('span', null, 'value'), swatch, document.createTextNode(css + ';'));
  }
  if (value && typeof value === 'object') {
    const fields = Object.entries(value);
    const summary = fields.slice(0, 3).map(([name, item]) => name + ': ' + (item && typeof item === 'object' ? '…' : String(item))).join(', ');
    return details('{ ' + summary + (fields.length > 3 ? ', … }' : ' }'), JSON.stringify(value, null, 2), key);
  }
  return el('span', text(value) + ';', 'value');
}
function elementsPanel() {
  if (!info?.queries?.includes('inspect_tree')) return empty('This runtime does not expose element inspection.');
  if (!elements) return empty('Waiting for the inspected tree.');
  const depths = new Map();
  for (const element of elements.elements) depths.set(element.id, element.parent ? (depths.get(element.parent) ?? 0) + 1 : 0);
  const selected = elementDetail;
  const replaced = selectedElement && !elements.elements.some(element => element.id === selectedElement);
  const tree = paged(elements.elements, items => append(el('div', null, 'element-tree'), ...items.map(element => {
    const control = button('', () => inspectElement(element.id), !connected, 'tree-row');
    control.addEventListener('pointerenter', () => { if (info?.element_interaction) hoveredBoundary = {id: element.id, generation: elements.capture}; });
    control.addEventListener('pointerleave', () => { if (hoveredBoundary?.id === element.id) hoveredBoundary = null; });
    control.dataset.focus = 'element:' + element.id;
    control.setAttribute('aria-label', 'Inspect ' + element.id);
    control.setAttribute('aria-pressed', String(selectedElement === element.id));
    control.title = `${element.kind} · ${element.id} · ${element.entry_id == null ? 'Workspace shell' : `tab-${element.tab_id} / entry-${element.entry_id}`}`;
    control.style.setProperty('--depth', Math.min(depths.get(element.id), 16));
    const kind = element.kind.replace(/\b(?:[a-z_]\w*::)+/g, '');
    control.append(el('span', '<', 'muted'), el('span', kind, 'syntax-tag'));
    if (element.semantic_id != null) control.append(el('span', 'id=', 'syntax-attribute'), el('span', '"' + element.semantic_id + '"', 'syntax-value'));
    control.append(el('span', '>', 'muted'));
    return control;
  })));
  const master = append(el('div'), append(el('div', null, 'capture-strip'), badge(elements.status), button(picking ? 'Cancel picking' : 'Pick element in app', async () => {
      if (picking) stopInspection();
      else {
        try { await inspectionControl({type: 'inspect_mode', enabled: true, lease_ms: 1000}); picking = true; }
        catch (error) { showError(error); }
      }
      render(true);
    }, !info?.element_interaction), el('span', `${elements.elements.length} boundaries · ${elements.omitted} omitted · capture ${elements.capture}`)), tree,
    details('Inspection coverage', elements.coverage, 'inspection-coverage'));
  const detail = el('div');
  const tabs = el('div', null, 'inspector-tabs');
  for (const name of ['Styles', 'Layout', 'Source']) {
    const control = button(name, () => { elementTab = name; render(true); });
    control.dataset.focus = 'element-tab:' + name;
    control.setAttribute('aria-pressed', String(elementTab === name)); tabs.append(control);
  }
  detail.append(tabs);
  if (replaced) detail.append(el('p', 'The selected capture has been replaced. Details are frozen; select a current row to inspect again.', 'hint warning'));
  if (elementError) detail.append(el('p', elementError, 'hint warning'));
  if (!selected) detail.append(empty('Select a view to inspect its styles, layout and source.'));
  else {
    const layout = selected.layout, source = selected.source;
    detail.append(append(el('div', null, 'element-identity'), el('span', selected.kind, 'syntax-tag'), el('p', selected.semantic_id ?? 'Semantic ID not supplied', 'syntax-value'),
      el('p', selected.id, 'muted'), el('p', source ? `${source.file}:${source.line}:${source.column}` : 'Source location unavailable.', 'muted')));
    if (elementTab === 'Styles') {
      if (selected.styles) {
        const styles = append(el('div', null, 'style-block'), el('span', 'Authored base styles {', 'muted'));
        for (const [property, value] of Object.entries(selected.styles)) styles.append(append(el('div', null, 'style-rule'), el('span', property + ':', 'property'), styleValue(value, `style:${selected.id}:${property}`)));
        styles.append(el('span', '}', 'muted')); detail.append(styles);
      } else detail.append(empty('Style refinements unavailable for this boundary.'));
    }
    if (elementTab !== 'Source') {
      detail.append(el('h3', 'Layout', 'pane-heading'));
      if (layout) detail.append(append(el('div', null, 'box-model'), el('span', `x: ${layout.x} · y: ${layout.y}`), el('div', `${layout.width} × ${layout.height}`, 'box-size')),
        el('p', `Layout in window logical pixels: x ${layout.x}, y ${layout.y}, width ${layout.width}, height ${layout.height}. Before clipping.`, 'hint'));
      else detail.append(el('p', 'Layout unavailable.', 'hint'));
    } else detail.append(properties([['Source file', source?.file], ['Line', source?.line], ['Column', source?.column], ['Inspected parent', selected.parent || 'Root inspection boundary'], ['Entry', selected.entry_id == null ? 'Workspace shell' : 'entry-' + selected.entry_id]]));
    detail.append(el('p', selected.metadata_truncated ? 'Some metadata exceeded capture limits.' : 'Metadata within capture limits.', 'hint'),
      append(el('div', null, 'pane-actions'), related(selected, 'Tasks'), related(selected, 'Requests')));
  }
  return splitView(master, detail, 'elements');
}

function reloadPanel() {
  return append(el('div'), card('Supervisor', el('pre', text(development, 12000))),
    model.snapshot ? card('Runtime reload boundary', el('pre', text(model.snapshot.reload, 12000))) : null,
    el('p', 'Native code replacement is delegated to the configured backend. Checkpointed process restart and full Web page reload receive fresh RuntimeIds. This panel observes the shared reload status; it does not unload code.', 'hint'));
}
function receiptPanel() {
  if (!receipt) return null;
  const current = receipt.envelope.runtime_id === model.runtime;
  const body = card('Last control request', el('p', `${receipt.envelope.key} · ${receipt.envelope.runtime_id}`, 'mono small'),
    receipt.pending ? el('p', 'Waiting for submission response…') : receipt.uncertain ? el('p', 'Delivery is uncertain. Disconnecting or timing out does not cancel or approve an operation.', 'warning') :
      el('p', receipt.error || text(receipt.result), receipt.error ? 'danger' : 'small'),
    !current ? el('p', 'This request belongs to a previous runtime. Its key and IDs cannot target the replacement.', 'warning') : null,
    receipt.uncertain ? button('Retry original request', () => mutate(null, receipt.envelope).catch(showError), !current || mutating || !connected) : null,
    details('Exact request envelope', receipt.envelope, 'receipt'));
  const drawer = append(el('details', null, 'receipt'), el('summary', receipt.pending ? 'Control request · waiting for response' : receipt.uncertain ? 'Control request · delivery uncertain' : receipt.error ? 'Control request · failed' : 'Last control request'), body);
  drawer.open = !!(receipt.pending || receipt.uncertain || receipt.error || expanded.has('receipt-drawer'));
  drawer.addEventListener('toggle', () => drawer.open ? expanded.add('receipt-drawer') : expanded.delete('receipt-drawer'));
  return drawer;
}
function exportRecords() {
  const records = diagnosticRecords().filter(record => panel === 'Requests' ? record.kind === 'request' : panel === 'Traces' ? record.kind.startsWith('span_') : true)
    .filter(record => !search || JSON.stringify(record).toLowerCase().includes(search.toLowerCase()));
  const blob = new Blob([JSON.stringify({protocol: 1, runtime_id: model.runtime, records,
    dropped: model.dropped, inspector_evicted: model.evicted, diagnostic_gaps: model.diagnosticGaps}, null, 2)], {type: 'application/json'});
  const url = URL.createObjectURL(blob), anchor = el('a');
  anchor.href = url; anchor.download = `rake-${panel.toLowerCase()}-${model.runtime || 'disconnected'}.json`; anchor.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function render(force = false) {
  const snapshot = model.snapshot;
  $('summary').textContent = snapshot ? `${snapshot.tabs.length} tabs · ${snapshot.work.tasks.length} tasks · revision ${snapshot.revision}` : development?.phase || 'Waiting for runtime';
  $('context').textContent = Object.entries(filter).map(([key, value]) => `${key}: ${value}`).join(' / ');
  $('clear-context').hidden = !Object.keys(filter).length;
  $('loss').textContent = `Diagnostics: ${model.diagnosticsRecords.length} retained · ${model.dropped} collector drops · ${model.outputDropped} runtime output drops · ${model.evicted} inspector evictions · ${model.diagnosticGaps} missing diagnostic sequences · ${model.gaps} state resyncs. Process buffer drops: ${output.dropped}.`;
  $('loss-summary').textContent = `${model.diagnosticsRecords.length} records · ${model.dropped + model.outputDropped + model.evicted + output.dropped} drops`;
  $('export').hidden = !['Console', 'Requests', 'Traces'].includes(panel);
  $('export').disabled = !model.runtime;
  if (resizing || (!force && document.activeElement?.closest('form'))) return;
  const signature = JSON.stringify([panel, model.runtime, model.cursor, model.after, connected, mutating,
    receipt, operation, development, elements, selectedElement, elementDetail, elementError, output.records.at(-1)?.sequence, output.dropped]);
  if (!force && signature === rendered) return;
  rendered = signature;
  const content = ({Workspace: workspace, Routes: routing, Tasks: tasksPanel, Operations: operationsPanel, Decisions: decisionsPanel,
    Console: consolePanel, Requests: requestsPanel, Traces: tracePanel, Elements: elementsPanel, Reload: reloadPanel})[panel]();
  content.classList.add('panel-body');
  if (!content.classList.contains('split-view')) rememberScroll(content, 'panel:' + panel);
  const focused = document.activeElement?.dataset.focus;
  $('content').replaceChildren(...[content, receiptPanel()].filter(Boolean));
  if (focused) [...$('content').querySelectorAll('[data-focus]')].find(control => control.dataset.focus === focused)?.focus({preventScroll: true});
  for (const scroller of $('content').querySelectorAll('[data-scroll]')) scroller.scrollTop = scrollPositions.get(scroller.dataset.scroll) || 0;
}
for (const [index, name] of panels.entries()) {
  const control = button(name, () => switchPanel(name)); control.dataset.panel = name;
  control.id = 'panel-' + name.toLowerCase(); control.tabIndex = index ? -1 : 0;
  control.setAttribute('role', 'tab'); control.setAttribute('aria-controls', 'content');
  control.setAttribute('aria-selected', index ? 'false' : 'true');
  control.setAttribute('aria-current', index ? 'false' : 'page'); $('panels').append(control);
}
$('panels').addEventListener('keydown', event => {
  if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
  event.preventDefault();
  const index = panels.indexOf(panel);
  const next = event.key === 'Home' ? 0 : event.key === 'End' ? panels.length - 1 : (index + (event.key === 'ArrowLeft' ? -1 : 1) + panels.length) % panels.length;
  switchPanel(panels[next]); $('panels').children[next].focus();
});
function setTheme(theme) {
  $('app').dataset.theme = theme;
  const label = 'Switch to ' + (theme === 'dark' ? 'light' : 'dark') + ' theme';
  $('theme').setAttribute('aria-label', label); $('theme').title = label;
}
setTheme(sessionStorage.getItem('rake-inspector-theme') === 'light' ? 'light' : 'dark');
$('theme').addEventListener('click', () => {
  const theme = $('app').dataset.theme === 'dark' ? 'light' : 'dark';
  sessionStorage.setItem('rake-inspector-theme', theme); setTheme(theme);
});
$('export').addEventListener('click', exportRecords);
$('runtime').addEventListener('change', () => { choose($('runtime').value || null); refresh(); });
$('search').addEventListener('input', () => { search = $('search').value; page = 0; render(true); });
$('clear-context').addEventListener('click', () => { filter = {}; render(true); });
$('pause').addEventListener('click', () => { paused = !paused; $('pause').textContent = paused ? 'Resume updates' : 'Pause updates'; $('pause').setAttribute('aria-pressed', String(paused)); $('pause').title = $('pause').textContent; message(paused ? 'Automatic observation paused. The runtime and its operations continue.' : 'Automatic observation resumed.'); if (!paused) refresh(); });
async function loop() { if (!paused) await refresh(); setTimeout(loop, 750); }
if (typeof window !== 'undefined') window.addEventListener('pagehide', () => {
  if (!model.runtime || !info?.element_interaction) return;
  for (const value of [{type: 'inspect_mode', enabled: false, lease_ms: 0}, {type: 'highlight_element', id: null, generation: ''}]) {
    fetch('/__rake/inspect', {method: 'POST', keepalive: true, headers: {'Content-Type': 'application/json', 'X-Rake-Token': token}, body: JSON.stringify({request: request(model.runtime, {type: 'command', value})})}).catch(() => {});
  }
});
render(true); loop(); inspectionLoop();
