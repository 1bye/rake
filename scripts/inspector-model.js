// Pure projections of shared runtime replies. Authoritative state only comes from snapshots.
export function safeJSON(text) {
  return JSON.parse(text, (_key, value) => {
    if (typeof value === 'number' && (!Number.isFinite(value) || (Number.isInteger(value) && !Number.isSafeInteger(value)))) {
      throw new Error('This form requires numbers within JavaScript’s safe range. Use the typed Rust client or CLI for larger integers.');
    }
    return value;
  });
}

export function request(runtime, message, key = null) {
  if (!runtime) throw new Error('Select a runtime first.');
  if (message.type === 'command') {
    for (const name of ['tab', 'task', 'operation', 'decision']) {
      const value = message.value[name];
      if (value !== undefined && (!Number.isSafeInteger(value) || value <= 0)) throw new Error(`Invalid ${name} identity.`);
    }
    if (!key) throw new Error('Commands require a stable request key.');
  }
  return {protocol: 1, runtime_id: runtime, key, message};
}

export function identity(value) {
  if (value === null || value === undefined || value === '') return null;
  return String(value).replace(/^(?:tab|entry|task|run|op|generation|gen|window)-/, '');
}

export function matches(record, filter) {
  return Object.entries(filter).every(([key, value]) => identity(record.fields[key]) === identity(value));
}

export function stateName(state) {
  return typeof state === 'string' ? state : Object.keys(state || {})[0] || 'Unavailable';
}

export class InspectorState {
  constructor(capacity = 1000, byteLimit = 2 * 1024 * 1024) {
    this.capacity = capacity;
    this.byteLimit = byteLimit;
    this.reset(null);
  }
  reset(runtime) {
    this.runtime = runtime;
    this.cursor = null;
    this.snapshot = null;
    this.records = [];
    this.after = 0;
    this.dropped = 0;
    this.outputDropped = 0;
    this.evicted = 0;
    this.gaps = 0;
    this.diagnosticGaps = 0;
    this.bytes = 0;
  }
  event(event) {
    if (event.cursor.runtime_id !== this.runtime) return false;
    if (this.cursor && this.cursor.epoch === event.cursor.epoch && event.cursor.revision < this.cursor.revision) return false;
    if (event.kind !== 'unchanged' && !event.snapshot) throw new Error('State event is missing its authoritative snapshot.');
    if (event.snapshot && event.snapshot.runtime_id !== this.runtime) throw new Error('Snapshot/runtime mismatch.');
    if (event.kind === 'gap') this.gaps++;
    this.cursor = event.cursor;
    if (event.snapshot) this.snapshot = event.snapshot;
    return true;
  }
  diagnostics(snapshot, runtime) {
    if (runtime !== this.runtime) return;
    const fresh = snapshot.records.filter(record => record.sequence > this.after).sort((a, b) => a.sequence - b.sequence);
    if (fresh.length && this.after && fresh[0].sequence > this.after + 1) this.diagnosticGaps += fresh[0].sequence - this.after - 1;
    for (const record of fresh) {
      if (record.sequence <= this.after) continue;
      const bytes = new TextEncoder().encode(JSON.stringify(record)).length;
      this.records.push({record, bytes});
      this.bytes += bytes;
      this.after = record.sequence;
      while (this.records.length > this.capacity || this.bytes > this.byteLimit) {
        this.bytes -= this.records.shift().bytes;
        this.evicted++;
      }
    }
    this.dropped = Math.max(this.dropped, snapshot.dropped);
    this.outputDropped = Math.max(this.outputDropped, snapshot.output_dropped);
  }
  get diagnosticsRecords() { return this.records.map(item => item.record); }
}

export function requests(records) {
  const grouped = new Map();
  for (const record of records) {
    if (record.kind !== 'request' || record.fields.request_id === undefined) continue;
    const key = String(record.fields.request_id);
    const previous = grouped.get(key);
    const item = previous || {id: key, fields: {}, events: [], started: false};
    item.fields = {...item.fields, ...record.fields};
    item.events.push(record);
    item.started ||= record.fields.phase === 'started';
    item.span_id = record.span_id ?? item.span_id;
    item.elapsed_us = record.elapsed_us;
    grouped.set(key, item);
  }
  return [...grouped.values()].sort((a, b) => b.elapsed_us - a.elapsed_us);
}

export function traces(records) {
  const nodes = new Map();
  for (const record of records) {
    if (!['span_open', 'span_close'].includes(record.kind) || record.span_id == null) continue;
    const item = nodes.get(record.span_id) || {id: record.span_id, opened: false, closed: false, fields: {}};
    item.fields = {...item.fields, ...record.fields};
    item.parent = record.parent_span_id;
    item.name = record.spans.at(-1) || 'Unnamed span';
    if (record.kind === 'span_open') { item.opened = true; item.start = record.elapsed_us; }
    if (record.kind === 'span_close') {
      item.closed = true;
      item.duration = record.duration_us;
      item.end = record.elapsed_us;
      item.start ??= record.duration_us == null ? record.elapsed_us : Math.max(0, record.elapsed_us - record.duration_us);
    }
    nodes.set(item.id, item);
  }
  const ordered = [];
  const visited = new Set();
  const children = new Map();
  for (const item of nodes.values()) {
    const parent = nodes.has(item.parent) ? item.parent : null;
    if (!children.has(parent)) children.set(parent, []);
    children.get(parent).push(item);
  }
  function visit(item, depth) {
    if (visited.has(item.id)) return;
    visited.add(item.id);
    ordered.push({...item, depth, missingParent: item.parent != null && !nodes.has(item.parent)});
    for (const child of children.get(item.id) || []) visit(child, depth + 1);
  }
  for (const root of children.get(null) || []) visit(root, 0);
  // Corrupt/cyclic parent references cannot hang the inspector.
  for (const item of nodes.values()) visit(item, 0);
  return ordered;
}
