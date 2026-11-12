// Shared by Wasm bindings and deterministic host tests. No permission prompt at startup.
export function notification_permission() {
  if (!globalThis.isSecureContext || typeof globalThis.Notification !== 'function')
    throw new DOMException('notifications are unavailable', 'NotSupportedError');
  return Notification.permission;
}

export function request_notification_permission() {
  try {
    const permission = notification_permission();
    if (permission !== 'default') return Promise.resolve(permission);
    if (globalThis.navigator?.userActivation?.isActive === false)
      throw new DOMException('notification permission requires a user gesture', 'NotAllowedError');
    return Notification.requestPermission();
  } catch (error) { return Promise.reject(error); }
}

export function create_host(activate) {
  return {activate, posted: new Map()};
}

export function show_notification(host, tag, title, body) {
  if (notification_permission() !== 'granted') return false;
  const notification = new Notification(title, {body, tag});
  dismiss_notification(host, tag);
  host.posted.set(tag, notification);
  notification.onclick = () => {
    try { globalThis.window?.focus(); } catch (_) { /* Best effort. */ }
    dismiss_notification(host, tag);
    host.activate(tag);
  };
  return true;
}

export function dismiss_notification(host, tag) {
  const notification = host.posted.get(tag);
  if (!notification) return;
  notification.onclick = null;
  notification.close();
  host.posted.delete(tag);
}

export function dispose_host(host) {
  for (const tag of host.posted.keys()) dismiss_notification(host, tag);
  host.activate = null;
}
