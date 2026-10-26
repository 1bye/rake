// Authenticate before the Wasm application reads its connection configuration.
const prefix = '#__rake_control=';
try {
  if (location.hash.startsWith(prefix)) {
    const token = location.hash.slice(prefix.length);
    if (/^[A-Za-z0-9_-]{32,128}$/.test(token)) sessionStorage.setItem('rake-dev-control-token', token);
    const launch = new URL(location.href);
    const instance = launch.searchParams.get('instance');
    if (instance) sessionStorage.setItem('rake-inspector-instance', instance);
    else sessionStorage.removeItem('rake-inspector-instance');
    launch.searchParams.delete('instance');
    history.replaceState(history.state, '', launch.pathname + launch.search);
  }
  const {default: initialize} = await import('./pkg/rake_inspector.js');
  await initialize();
} catch (error) {
  const display = document.getElementById('error');
  display.hidden = false;
  display.textContent = `Rake Inspector could not start: ${error}`;
  console.error(error);
}
