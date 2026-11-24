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
