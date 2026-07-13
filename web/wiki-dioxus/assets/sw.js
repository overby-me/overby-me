// RadikalWiki service worker (#33). Served from the site ROOT (/sw.js — copied
// there by the frontend Nix package's install phase) so its scope is `/` and it
// controls the whole app (/, /wasm/*, /assets/*) for offline + instant repeat
// loads on the constrained venue wifi/cellular typical of a landsmøde.
//
// Strategy:
//   /assets/*  (content-hashed, immutable) → cache-first (no re-download).
//   everything else same-origin (/, /wasm/*) → stale-while-revalidate: serve the
//     cached copy instantly, refresh it in the background; fall back to the
//     network, then to the cached app shell (/) when offline.
const CACHE = 'radikalwiki-v2';

self.addEventListener('install', () => self.skipWaiting());

self.addEventListener('activate', (event) =>
  event.waitUntil(
    (async () => {
      // Drop old cache versions so a new deploy fully rolls out.
      const keys = await caches.keys();
      await Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k)));
      await self.clients.claim();
    })(),
  ),
);

function cachePut(req, res) {
  const copy = res.clone();
  caches
    .open(CACHE)
    .then((c) => c.put(req, copy))
    .catch(() => {});
  return res;
}

self.addEventListener('fetch', (event) => {
  const req = event.request;
  if (req.method !== 'GET') return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;

  // Content-hashed assets never change for a given URL: cache-first.
  if (url.pathname.startsWith('/assets/')) {
    event.respondWith(
      caches.match(req).then((hit) => hit || fetch(req).then((res) => cachePut(req, res))),
    );
    return;
  }

  // App shell + wasm: stale-while-revalidate.
  event.respondWith(
    caches.match(req).then((hit) => {
      const network = fetch(req)
        .then((res) => cachePut(req, res))
        .catch(() => hit || caches.match('/'));
      return hit || network;
    }),
  );
});
