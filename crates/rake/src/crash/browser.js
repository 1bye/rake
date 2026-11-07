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

function prune(prefix, count) {
  const entries = keys(prefix);
  let excess = entries.length - count;
  for (const key of entries) {
    if (excess <= 0) break;
    localStorage.removeItem(key);
    excess--;
  }
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
    localStorage.setItem(prefix + id, 'P' + raw);
    // Panic persistence cannot await. Retention settles under the delivery lock.
    crash_prune(prefix, count);
  } catch (_) {
    // Quota/denied storage must not turn a panic into a second panic.
  }
}

export async function crash_deliver(prefix, bytes, count, report) {
  // localStorage has no atomic compare-and-swap; a storage lease would race.
  if (!hasLocks()) throw new Error('crash delivery requires Web Locks (secure context)');
  return navigator.locks.request(lockName(prefix), {ifAvailable: true}, lock => {
    if (!lock) return 0;
    let delivered = 0;
    try {
      prune(prefix, count);
      for (const key of keys(prefix)) {
        const stored = localStorage.getItem(key);
        if (stored === null) continue;
        const raw = stored.slice(1);
        if (!['P', 'C'].includes(stored[0]) || encoder.encode(raw).length > bytes) {
          localStorage.removeItem(key);
          continue;
        }
        localStorage.setItem(key, 'C' + raw);
        try {
          const outcome = report(raw);
          if (outcome === 0) continue; // Preserve unknown schemas byte-for-byte.
          localStorage.removeItem(key);
          if (outcome === 1) delivered++;
        } finally {
          // Includes a throwing JS callback or a trapping/unwinding wasm sink.
          if (localStorage.getItem(key) === 'C' + raw) localStorage.setItem(key, 'P' + raw);
        }
      }
      return delivered;
    } finally {
      prune(prefix, count);
    }
  });
}
