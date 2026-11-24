// Browser adapter shared by Wasm bindings and host-boundary tests.
function unsupported(operation) {
  throw new DOMException(`${operation} is unavailable`, 'NotSupportedError');
}

function activation(operation) {
  if (globalThis.navigator?.userActivation?.isActive === false)
    throw new DOMException(`${operation} requires a user gesture`, 'NotAllowedError');
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
    if (!(globalThis.isSecureContext && globalThis.navigator?.clipboard?.readText)) unsupported('clipboard read');
    activation('clipboard read');
    return navigator.clipboard.readText();
  } catch (error) { return Promise.reject(error); }
}

export function write_clipboard(text) {
  try {
    if (!(globalThis.isSecureContext && globalThis.navigator?.clipboard?.writeText)) unsupported('clipboard write');
    activation('clipboard write');
    return navigator.clipboard.writeText(text);
  } catch (error) { return Promise.reject(error); }
}

export function open_url(url) {
  activation('open URL');
  const opened = window.open('about:blank', '_blank');
  if (!opened) throw new DOMException('popup blocked', 'NotAllowedError');
  try { opened.opener = null; opened.location.href = url; }
  catch (error) { opened.close(); throw error; }
}

// Canvas-wide listener; disposal removes every listener and releases the callback.
export function on_canvas_file_drop(callback) {
  const canvasEvent = event => event.target?.tagName === 'CANVAS';
  const fileEvent = event => canvasEvent(event) && Array.from(event.dataTransfer?.types ?? []).includes('Files');
  const over = event => {
    if (!fileEvent(event)) return;
    event.preventDefault();
    event.target.style.outline = '2px solid currentColor';
  };
  const leave = event => {
    if (canvasEvent(event)) event.target.style.outline = '';
  };
  const drop = event => {
    if (!fileEvent(event)) return;
    event.preventDefault();
    leave(event);
    const files = event.dataTransfer?.files;
    callback(files?.length === 1 ? files[0] : null);
  };
  document.addEventListener('dragover', over);
  document.addEventListener('dragleave', leave);
  document.addEventListener('drop', drop);
  return () => {
    document.removeEventListener('dragover', over);
    document.removeEventListener('dragleave', leave);
    document.removeEventListener('drop', drop);
  };
}
