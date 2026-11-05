// Rake supplies CACHE, PREFIX, ROOT, INDEX and ASSETS above this source.
// Default waiting/activation is deliberate: no skipWaiting or clients.claim.
self.addEventListener('install', event => {
  event.waitUntil((async () => {
    try {
      const cache = await caches.open(CACHE);
      await cache.addAll(ASSETS.map(path => new Request(path, {cache: 'reload'})));
    } catch (error) {
      await caches.delete(CACHE); // A partial install must never become an offline app.
      throw error;
    }
  })());
});

self.addEventListener('activate', event => {
  event.waitUntil((async () => {
    for (const key of await caches.keys()) {
      if (key.startsWith(PREFIX) && key !== CACHE) await caches.delete(key);
    }
  })());
});

self.addEventListener('fetch', event => {
  const request = event.request;
  const url = new URL(request.url);
  const root = new URL(ROOT, self.location.origin);
  if (request.method !== 'GET' || url.origin !== root.origin || !url.pathname.startsWith(root.pathname)) return;
  if (url.pathname.startsWith(root.pathname + 'rake-builds/')) {
    // Content-addressed paths cannot replace a live tab's modules with a new build.
    event.respondWith((async () => (await caches.match(request)) ?? fetch(request))());
  } else if (request.mode === 'navigate') {
    event.respondWith((async () => {
      try {
        const response = await fetch(request);
        if (!response.ok) throw new Error(`navigation failed (${response.status})`);
        return response;
      } catch (error) {
        const cache = await caches.open(CACHE);
        const fallback = await cache.match(new URL(INDEX, root).href);
        if (!fallback) throw error;
        return fallback;
      }
    })());
  }
  // APIs, credentials, version checks and arbitrary downloads are never cached.
});
