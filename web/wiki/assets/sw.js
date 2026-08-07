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

// Which build this worker IS. Substituted when the bundle is assembled (see the
// justfile), like version.json; unsubstituted in `dx serve`, where it says so.
//
// A worker outlives the page that installed it: a visitor can be running one
// build in the tab while a worker from an older deploy serves the files. That
// gap is invisible from the page -- it explains "I still see the old version"
// and a stack that does not match the code -- so the worker answers when asked.
const BUILD = "__WIKI_BUILD__";

self.addEventListener("message", (event) => {
	if (event.data !== "which build?") return;
	const reply = { build: BUILD, cache: CACHE };
	// Back down the port the asker sent, when they sent one; otherwise to
	// everybody, which is what a page without a MessageChannel would hear.
	if (event.ports && event.ports[0]) {
		event.ports[0].postMessage(reply);
		return;
	}
	event.source && event.source.postMessage(reply);
});

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

	// Every path below must resolve to a Response. `respondWith` given anything
	// else — including the `undefined` that `caches.match` returns on a miss —
	// throws "Failed to convert value to 'Response'", which is what the deployed
	// worker did whenever a fetch failed before anything had been cached: a bad
	// connection on a first visit, precisely when the fallback is the point.
	const offline = () =>
		new Response("", { status: 503, statusText: "Offline" });

	// Content-hashed assets never change for a given URL: cache-first.
	if (url.pathname.startsWith("/assets/")) {
		event.respondWith(
			caches
				.match(req)
				.then((hit) => hit || fetch(req).then((res) => cachePut(req, res)))
				.catch(offline),
		);
		return;
	}

	// App shell + wasm: stale-while-revalidate.
	event.respondWith(
		caches
			.match(req)
			.then((hit) => {
				const network = fetch(req)
					.then((res) => cachePut(req, res))
					// Nothing from the network: the cached copy of this URL if there
					// is one, else the app shell, else a plain offline response. The
					// last step is what was missing.
					.catch(async () => hit || (await caches.match("/")) || offline());
				return hit || network;
			})
			.catch(offline),
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
