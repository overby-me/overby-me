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
const CACHE = "radikalwiki-v5";

self.addEventListener("install", () => self.skipWaiting());

self.addEventListener("activate", (event) =>
	event.waitUntil(
		(async () => {
			// Drop old cache versions so a new deploy fully rolls out.
			const keys = await caches.keys();
			await Promise.all(
				keys.filter((k) => k !== CACHE).map((k) => caches.delete(k)),
			);
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

self.addEventListener("fetch", (event) => {
	const req = event.request;
	if (req.method !== "GET") return;
	const url = new URL(req.url);
	if (url.origin !== self.location.origin) return;

	// Which build is deployed (src/update.rs). Never cached, and never served
	// from cache: answering it from here would compare the running build against
	// itself and could never report an update. Each check also carries a unique
	// cache-buster, so caching them would only grow the cache forever.
	if (url.pathname === "/version.json") return;

	// Content-hashed assets never change for a given URL: cache-first.
	if (url.pathname.startsWith("/assets/")) {
		event.respondWith(
			caches
				.match(req)
				.then((hit) => hit || fetch(req).then((res) => cachePut(req, res))),
		);
		return;
	}

	// App shell + wasm: stale-while-revalidate.
	event.respondWith(
		caches.match(req).then((hit) => {
			const network = fetch(req)
				.then((res) => cachePut(req, res))
				.catch(() => hit || caches.match("/"));
			return hit || network;
		}),
	);
});

// Background Web Push (#128). The backend encrypts a JSON payload
// {title, body, url}; show it as a notification even when the app is closed.
self.addEventListener("push", (event) => {
	let data = { title: "RadikalWiki", body: "", url: "/" };
	try {
		if (event.data) data = { ...data, ...event.data.json() };
	} catch {
		if (event.data) data.body = event.data.text();
	}
	event.waitUntil(
		self.registration.showNotification(data.title, {
			body: data.body,
			icon: "/assets/icon.svg",
			badge: "/assets/icon.svg",
			tag: "radikalwiki",
			data: { url: data.url || "/" },
		}),
	);
});

// Tapping a notification focuses an open tab (navigating it) or opens a new one.
self.addEventListener("notificationclick", (event) => {
	event.notification.close();
	const target =
		(event.notification.data && event.notification.data.url) || "/";
	event.waitUntil(
		(async () => {
			const all = await self.clients.matchAll({
				type: "window",
				includeUncontrolled: true,
			});
			for (const client of all) {
				if ("focus" in client) {
					if ("navigate" in client && target !== "/") {
						try {
							await client.navigate(target);
						} catch {
							/* cross-origin or detached: just focus */
						}
					}
					return client.focus();
				}
			}
			if (self.clients.openWindow) return self.clients.openWindow(target);
		})(),
	);
});
