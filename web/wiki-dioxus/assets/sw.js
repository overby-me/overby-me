// RadikalWiki service worker (#33). Runtime caching so the app keeps working
// offline after a first online visit: network-first, falling back to the cache.
//
// NOTE: to control the whole app (offline of /, /wasm/*) the worker must be
// served from the site ROOT (/sw.js) or with a `Service-Worker-Allowed: /`
// header. At /assets/sw.js its scope is /assets/ only, so this is a deploy
// concern; installability (the web manifest) works regardless.
const CACHE = 'radikalwiki-v1';

self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (event) => event.waitUntil(self.clients.claim()));

self.addEventListener('fetch', (event) => {
  const req = event.request;
  if (req.method !== 'GET') return;
  if (new URL(req.url).origin !== self.location.origin) return;
  event.respondWith(
    fetch(req)
      .then((res) => {
        const copy = res.clone();
        caches.open(CACHE).then((c) => c.put(req, copy)).catch(() => {});
        return res;
      })
      .catch(() => caches.match(req).then((hit) => hit || caches.match('/')))
  );
});
