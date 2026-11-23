// Explicit worker ownership; JSON is the transfer boundary, not a Rust closure.
export function start_worker(module, input, timeoutMs) {
  if (typeof Worker === 'undefined') throw new DOMException('workers unavailable', 'NotSupportedError');
  if (!Number.isFinite(timeoutMs) || timeoutMs <= 0 || timeoutMs > 2_147_483_647)
    throw new Error('worker timeout must be between 1 ms and 24 days');
  const bytes = new TextEncoder();
  if (bytes.encode(input).length > 1_048_576) throw new Error('worker request exceeds 1 MiB');
  const url = new URL(module, document.baseURI);
  if (url.origin !== location.origin || !['https:', 'http:'].includes(url.protocol) || url.username || url.password)
    throw new Error('worker module must be a same-origin HTTP(S) URL');
  const worker = new Worker(url, {type: 'module'});
  let done = false;
  let fail;
  let timer;
  const promise = new Promise((resolve, reject) => {
    const finish = (error, value) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      worker.onmessage = worker.onerror = worker.onmessageerror = null;
      worker.terminate();
      if (error) reject(error); else resolve(value);
    };
    fail = error => finish(error);
    worker.onmessage = event => {
      const raw = event.data;
      if (typeof raw !== 'string' || bytes.encode(raw).length > 1_048_576) {
        finish(new Error('worker reply must be a JSON string of at most 1 MiB'));
        return;
      }
      try { JSON.parse(raw); finish(null, raw); }
      catch (_) { finish(new Error('worker returned invalid JSON')); }
    };
    worker.onerror = event => { event.preventDefault?.(); finish(new Error(event.message || 'worker failed')); };
    worker.onmessageerror = () => finish(new Error('worker reply could not be decoded'));
    timer = setTimeout(() => finish(new DOMException('worker timed out', 'TimeoutError')), timeoutMs);
    try { worker.postMessage(input); } catch (error) { finish(error); }
  });
  return {promise, cancel() { fail(new DOMException('worker cancelled', 'AbortError')); }};
}

export function worker_promise(job) { return job.promise; }
export function cancel_worker(job) { job.cancel(); }

export function deployment_metadata() {
  return document.querySelector('meta[name="rake-deployment"]')?.content ?? null;
}

export async function check_deployment(base) {
  const abort = new AbortController();
  const timer = setTimeout(() => abort.abort(), 15_000);
  try {
    const url = new URL(base + 'rake-version.json', location.origin);
    const response = await fetch(url, {cache: 'no-store', redirect: 'error', signal: abort.signal});
    if (!response.ok || !response.body) throw new Error(`deployment check failed (${response.status})`);
    const reader = response.body.getReader();
    let raw = '';
    let size = 0;
    const decoder = new TextDecoder();
    try {
      while (true) {
        const {value, done} = await reader.read();
        if (done) break;
        size += value.byteLength;
        if (size > 16_384) throw new Error('deployment manifest exceeds 16 KiB');
        raw += decoder.decode(value, {stream: true});
      }
      return raw + decoder.decode();
    } finally { await reader.cancel(); }
  } finally { clearTimeout(timer); }
}

export function reload_page() { location.reload(); }
