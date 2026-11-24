// Browser adapter shared by Wasm bindings and host-boundary tests.
export function capabilities() {
  let bits = 0;
  if (globalThis.document?.body) bits |= 1 | 2 | 32;
  if (globalThis.isSecureContext && globalThis.navigator?.clipboard?.readText) bits |= 4;
  if (globalThis.isSecureContext && globalThis.navigator?.clipboard?.writeText) bits |= 8;
  try { if (globalThis.localStorage) bits |= 16; } catch (_) { /* Access denied. */ }
  return bits;
}

export async function clipboard_permission(write) {
  try {
    return (await navigator.permissions.query({name: write ? 'clipboard-write' : 'clipboard-read'})).state;
  } catch (_) { return 'unknown'; } // Firefox/Safari need not implement this permission query.
}

export function preference_get(namespace, key) {
  if (!(capabilities() & 16)) throw new DOMException('preferences access denied', 'SecurityError');
  return localStorage.getItem(`rake:prefs:${namespace}:${key}`);
}

export function preference_set(namespace, key, value) {
  if (!(capabilities() & 16)) throw new DOMException('preferences access denied', 'SecurityError');
  const id = `rake:prefs:${namespace}:${key}`;
  if (value === undefined || value === null) localStorage.removeItem(id);
  else localStorage.setItem(id, value);
}
