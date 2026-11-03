// Request tracking for the real-browser host test; no request is replayed.
export function createBrowserCdp(socket, chrome, {onEvent = () => {}, stderr = () => ''} = {}) {
  const pending = new Map();
  let sequence = 0;
  let failure;

  function fail(error) {
    failure ??= error;
    for (const request of pending.values()) {
      clearTimeout(request.timer);
      request.reject(failure);
    }
    pending.clear();
  }

  function message(event) {
    const reply = JSON.parse(event.data);
    const request = pending.get(reply.id);
    if (request) {
      pending.delete(reply.id);
      clearTimeout(request.timer);
      reply.error ? request.reject(new Error(JSON.stringify(reply.error))) : request.resolve(reply.result);
    } else if (reply.method) {
      onEvent(reply);
    }
  }
  function exited(code, signal) {
    const diagnostic = stderr();
    fail(new Error(`Chrome exited (${signal ?? code})${diagnostic ? `: ${diagnostic}` : ''}`));
  }
  function closed(event) {
    fail(new Error(`CDP WebSocket closed${event ? ` (${event.code}${event.reason ? `: ${event.reason}` : ''})` : ''}`));
  }
  function socketError() {
    fail(new Error('CDP WebSocket error'));
  }

  chrome.once('exit', exited);
  socket.addEventListener('close', closed);
  socket.addEventListener('error', socketError);
  socket.addEventListener('message', message);
  if (chrome.exitCode !== null || chrome.signalCode !== null) exited(chrome.exitCode, chrome.signalCode);
  if (socket.readyState !== WebSocket.OPEN) closed();

  return {
    call(method, params = {}, sessionId) {
      if (failure) return Promise.reject(failure);
      return new Promise((resolve, reject) => {
        const id = ++sequence;
        const timer = setTimeout(() => { pending.delete(id); reject(new Error(`${method} timed out`)); }, 25_000);
        pending.set(id, {resolve, reject, timer});
        try {
          socket.send(JSON.stringify({id, method, params, sessionId}));
        } catch (error) {
          fail(error);
        }
      });
    },
    close() {
      fail(new Error('browser check ended'));
      chrome.removeListener('exit', exited);
      socket.removeEventListener('close', closed);
      socket.removeEventListener('error', socketError);
      socket.removeEventListener('message', message);
    },
  };
}
