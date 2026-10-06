const CACHE_NAME = 'file-server-v2';
const urlsToCache = [
  '/webui/',
  '/webui/styles.css',
  '/webui/style.css',
  '/webui/script.js',
  '/webui/manifest.json',
  '/webui/icon.svg'
];

// Install service worker. Takes over right away so an older, cache-first worker
// doesn't keep serving a stale UI until every tab is closed.
self.addEventListener('install', event => {
  event.waitUntil(
    caches.open(CACHE_NAME)
      .then(cache => cache.addAll(urlsToCache))
      .then(() => self.skipWaiting())
  );
});

// Only the UI's own files are handled; API calls and served files always go to the server.
// Network first, so a new server build's UI is picked up; the cache is only an offline fallback.
self.addEventListener('fetch', event => {
  const url = new URL(event.request.url);
  if (event.request.method !== 'GET' || url.origin !== self.location.origin || !url.pathname.startsWith('/webui/')) {
    return;
  }

  event.respondWith(
    fetch(event.request)
      .then(response => {
        if (response.ok) {
          const copy = response.clone();
          caches.open(CACHE_NAME).then(cache => cache.put(event.request, copy));
        }
        return response;
      })
      .catch(() => caches.match(event.request))
  );
});

// Clean up old caches
self.addEventListener('activate', event => {
  event.waitUntil(
    caches.keys().then(cacheNames => {
      return Promise.all(
        cacheNames.map(cacheName => {
          if (cacheName !== CACHE_NAME) {
            return caches.delete(cacheName);
          }
        })
      );
    }).then(() => self.clients.claim())
  );
});
