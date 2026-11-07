// Synchronous panic persistence; delivery/retention serialize across tabs via Web Locks.
const encoder = new TextEncoder();
const lockName = prefix => prefix + 'spool';
const hasLocks = () => typeof navigator !== 'undefined' && !!navigator.locks?.request;

function keys(prefix) {
  const result = [];
  for (let i = 0; i < localStorage.length; i++) {
    const key = localStorage.key(i);
    if (key?.startsWith(prefix)) result.push(key);
  }
  return result.sort();
}

function prune(prefix, count, waiting = new Set()) {
  const entries = keys(prefix);
  let excess = entries.length - count;
  // A sink panic must not evict the queue it was trying to deliver.
  const candidates = [...entries.filter(key => !waiting.has(key)), ...entries.filter(key => waiting.has(key))];
  for (const key of candidates) {
    if (excess <= 0) break;
    localStorage.removeItem(key);
    excess--;
  }
}

function storedRecord(stored) {
  const marker = /^([PC])([0-3]):/.exec(stored);
  if (marker) return {attempts: Number(marker[2]), raw: stored.slice(3)};
  // Read records written before retry counts were introduced, including old claims.
  if (stored[0] === 'P' || stored[0] === 'C') {
    return {attempts: stored[0] === 'C' ? 1 : 0, raw: stored.slice(1)};
  }
  return null;
}

export function crash_open() {
  void localStorage.length;
}

export function crash_prune(prefix, count) {
  try {
    if (hasLocks()) {
      void navigator.locks.request(lockName(prefix), () => prune(prefix, count)).catch(() => {});
    } else {
      prune(prefix, count);
    }
  } catch (_) {
    // Recording is best-effort even when storage policy changes after startup.
  }
}

export function crash_write(prefix, id, raw, bytes, count) {
  try {
    if (encoder.encode(raw).length > bytes) return;
    localStorage.setItem(prefix + id, 'P0:' + raw);
    // Panic persistence cannot await. Retention settles under the delivery lock.
    crash_prune(prefix, count);
  } catch (_) {
    // Quota/denied storage must not turn a panic into a second panic.
  }
}

export async function crash_deliver(prefix, bytes, count, report, abandoned = () => {}) {
  // localStorage has no atomic compare-and-swap; a storage lease would race.
  if (!hasLocks()) throw new Error('crash delivery requires Web Locks (secure context)');
  return navigator.locks.request(lockName(prefix), {ifAvailable: true}, lock => {
    if (!lock) return 0;
    // Snapshot once: records written by a sink's panic wait for another pass.
    const waiting = keys(prefix);
    let delivered = 0;
    try {
      for (const key of waiting) {
        const stored = localStorage.getItem(key);
        if (stored === null) continue;
        const record = storedRecord(stored);
        if (!record || encoder.encode(record.raw).length > bytes) {
          localStorage.removeItem(key);
          continue;
        }
        const {attempts, raw} = record;
        if (attempts >= 3) {
          localStorage.removeItem(key);
          abandoned(key.slice(prefix.length));
          continue;
        }
        localStorage.setItem(key, 'C' + (attempts + 1) + ':' + raw);
        let outcome;
        try {
          outcome = report(raw);
        } catch (_) {
          // A trap leaves the incremented claim durable for the next page load.
          return -2;
        }
        if (outcome === 0 || outcome === 3) {
          // Future schemas and ordinary Err results do not consume an attempt.
          localStorage.setItem(key, 'P' + attempts + ':' + raw);
          if (outcome === 3) return -1;
          continue;
        }
        localStorage.removeItem(key);
        if (outcome === 1) delivered++;
      }
      return delivered;
    } finally {
      prune(prefix, count, new Set(waiting));
    }
  });
}
