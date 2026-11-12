(() => {
  const parameters = new URL(document.currentScript.src).searchParams;
  const build = Number(parameters.get('build') || 0);
  const key = 'rake-dev-control-token';
  const prefix = '#__rake_control=';
  function authenticate() {
    if (!location.hash.startsWith(prefix)) return false;
    const token = location.hash.slice(prefix.length);
    if (/^[A-Za-z0-9_-]{32,128}$/.test(token)) sessionStorage.setItem(key, token);
    history.replaceState(history.state, '', location.pathname + location.search);
    return true;
  }
  authenticate();
  const authenticateNavigation = event => {
    if (authenticate()) {
      event.stopImmediatePropagation();
      location.reload();
    }
  };
  window.addEventListener('hashchange', authenticateNavigation, {capture: true});
  window.addEventListener('popstate', authenticateNavigation, {capture: true});
  const token = sessionStorage.getItem(key);
  if (token) {
    const meta = document.createElement('meta');
    meta.name = 'rake-dev-bridge';
    meta.content = JSON.stringify({url: location.origin, token, build_id: build});
    document.head.append(meta);
    let seenRuntime = false;
    async function development() {
      try {
        const response = await fetch('/__rake/development', {method: 'POST', headers: {'Content-Type': 'application/json', 'X-Rake-Token': token}, body: '{}'});
        if (!response.ok) return;
        const state = await response.json();
        const runtime = document.documentElement.getAttribute('data-rake-runtime');
        seenRuntime ||= !!runtime;
        if (state.published > build && !seenRuntime) location.reload();
        const failure = runtime && state.runtimes[runtime] && state.runtimes[runtime].error;
        let status = document.getElementById('__rake_development');
        if (failure || state.error) {
          if (!status && document.body) {
            status = document.createElement('div'); status.id = '__rake_development'; status.setAttribute('role', 'alert');
            status.style.cssText = 'position:fixed;bottom:8px;left:8px;max-width:70vw;max-height:35vh;overflow:auto;white-space:pre-wrap;background:#18232b;color:#f0f4e8;padding:8px;font:12px monospace;z-index:2147483647';
            document.body.append(status);
          }
          if (status) status.textContent = 'Rake: ' + (failure || state.error);
        } else if (status) status.remove();
        if (state.devtools_url && window.parent === window && !document.getElementById('__rake_devtools')) {
          const button = document.createElement('a'); button.id = '__rake_devtools'; button.href = state.devtools_url + '#devtools';
          button.title = 'Open Chrome DevTools'; button.setAttribute('aria-label', button.title); button.textContent = '\u2039\u203a';
          button.style.cssText = 'position:fixed;bottom:12px;right:12px;width:28px;height:28px;border-radius:50%;background:#242424;color:#ddd;display:grid;place-items:center;text-decoration:none;font:14px monospace;z-index:2147483647';
          document.body.append(button);
        }
      } catch (_) { /* The running application remains usable while disconnected. */ }
    }
    // Recursive scheduling bounds concurrent requests if the server is slow.
    async function poll() { await development(); setTimeout(poll, 1000); }
    if (parameters.has('build')) setTimeout(poll, 500);
  }
})();