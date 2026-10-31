// Browser adapter shared by Wasm bindings and host-boundary tests.
function unsupported(operation) {
  throw new DOMException(`${operation} is unavailable`, 'NotSupportedError');
}

function activation(operation) {
  if (globalThis.navigator?.userActivation?.isActive === false)
    throw new DOMException(`${operation} requires a user gesture`, 'NotAllowedError');
}

export function capabilities() {
  let bits = 0;
  if (globalThis.document?.body) bits |= 1 | 2 | 32;
  if (globalThis.isSecureContext && globalThis.navigator?.clipboard?.readText) bits |= 4;
  if (globalThis.isSecureContext && globalThis.navigator?.clipboard?.writeText) bits |= 8;
  try { if (globalThis.localStorage) bits |= 16; } catch (_) { /* Access denied. */ }
  return bits;
}

export function pick_file() {
  try {
    if (!globalThis.document?.body) unsupported('file import');
    activation('file import');
    return new Promise((resolve, reject) => {
      const input = document.createElement('input');
      input.type = 'file';
      input.hidden = true;
      const cleanup = () => {
        input.removeEventListener('change', changed);
        input.removeEventListener('cancel', cancelled);
        input.remove();
      };
      const changed = () => { const file = input.files?.[0] ?? null; cleanup(); resolve(file); };
      const cancelled = () => { cleanup(); resolve(null); };
      input.addEventListener('change', changed);
      input.addEventListener('cancel', cancelled);
      document.body.append(input);
      try { input.click(); } catch (error) { cleanup(); reject(error); }
    });
  } catch (error) { return Promise.reject(error); }
}

export async function read_file(file, maxBytes) {
  if (file.size > maxBytes) throw new DOMException('file exceeds byte limit', 'RakeSizeLimit');
  const bytes = await file.arrayBuffer();
  if (bytes.byteLength > maxBytes) throw new DOMException('file exceeds byte limit', 'RakeSizeLimit');
  return new Uint8Array(bytes);
}

export function export_file(name, bytes) {
  if (!globalThis.document?.body) unsupported('file export');
  activation('file export');
  const url = URL.createObjectURL(new Blob([bytes], {type: 'application/octet-stream'}));
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = name;
  try { document.body.append(anchor); anchor.click(); }
  finally { anchor.remove(); setTimeout(() => URL.revokeObjectURL(url), 60_000); }
}

export function read_clipboard() {
  try {
    if (!(capabilities() & 4)) unsupported('clipboard read');
    activation('clipboard read');
    return navigator.clipboard.readText();
  } catch (error) { return Promise.reject(error); }
}

export function write_clipboard(text) {
  try {
    if (!(capabilities() & 8)) unsupported('clipboard write');
    activation('clipboard write');
    return navigator.clipboard.writeText(text);
  } catch (error) { return Promise.reject(error); }
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

export function open_url(url) {
  activation('open URL');
  const opened = window.open('about:blank', '_blank');
  if (!opened) throw new DOMException('popup blocked', 'NotAllowedError');
  try { opened.opener = null; opened.location.href = url; }
  catch (error) { opened.close(); throw error; }
}
