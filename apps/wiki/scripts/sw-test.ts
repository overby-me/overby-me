// Does the service worker always answer with a Response?
//
// `respondWith` accepts a Response or a promise of one. Given anything else —
// including the `undefined` that `caches.match` returns on a miss — the browser
// throws "Failed to convert value to 'Response'" and the request fails outright.
// The deployed worker did that whenever a fetch failed before anything had been
// cached: a bad connection on a first visit, which is the one moment a service
// worker exists for.
//
// It went unnoticed because a service worker is invisible when it works and
// silent when it does not, and no test drove it. This one does, with stubbed
// Cache and fetch, so the failure cases can be asked for directly.
//
//   just test-sw

const listeners: Record<string, (e: unknown) => void> = {};
let cacheHas: string[] = [];
let networkFails = false;

Object.defineProperty(globalThis, "self", {
	configurable: true,
	writable: true,
	value: {
		addEventListener: (t: string, fn: (e: unknown) => void) =>
			(listeners[t] = fn),
		location: { origin: "https://radikal.wiki" },
		clients: { claim: () => Promise.resolve() },
		registration: {},
		skipWaiting: () => {},
	},
});
Object.defineProperty(globalThis, "caches", {
	configurable: true,
	writable: true,
	value: {
		open: () =>
			Promise.resolve({
				put: () => Promise.resolve(),
				addAll: () => Promise.resolve(),
			}),
		keys: () => Promise.resolve([]),
		delete: () => Promise.resolve(true),
		match: (req: { url?: string } | string) => {
			const url = typeof req === "string" ? req : (req.url ?? "");
			return Promise.resolve(
				cacheHas.some((u) => url.endsWith(u))
					? new Response("cached")
					: undefined,
			);
		},
	},
});
Object.defineProperty(globalThis, "fetch", {
	configurable: true,
	writable: true,
	value: () =>
		networkFails
			? Promise.reject(new TypeError("Failed to fetch"))
			: Promise.resolve(new Response("net")),
});

await import("../assets/sw.js" as string);

async function run(path: string, opts: { cached: string[]; offline: boolean }) {
	cacheHas = opts.cached;
	networkFails = opts.offline;
	let answered: unknown;
	listeners.fetch({
		request: { method: "GET", url: `https://radikal.wiki${path}` },
		respondWith: (v: unknown) => (answered = v),
	});
	return await answered;
}

const cases: [string, { cached: string[]; offline: boolean }][] = [
	["/user/login", { cached: [], offline: true }], // the failing case
	["/user/login", { cached: [], offline: false }],
	["/user/login", { cached: ["/"], offline: true }],
	["/assets/app-abc.wasm", { cached: [], offline: true }],
	["/assets/app-abc.wasm", { cached: ["/assets/app-abc.wasm"], offline: true }],
	["/", { cached: [], offline: true }],
];
let bad = 0;
console.log("every path answers with a Response");
for (const [path, opts] of cases) {
	const res = await run(path, opts);
	const ok = res instanceof Response;
	if (!ok) bad++;
	console.log(
		`  ${ok ? "ok  " : "FAIL"} ${path.padEnd(24)} cached=${JSON.stringify(opts.cached).padEnd(28)} offline=${opts.offline}  -> ${ok ? `Response ${(res as Response).status}` : typeof res}`,
	);
}

// Does ONE press of the update banner's Reload land on the new build?
//
// The worker serves the shell stale-while-revalidate, so any URL takes two loads
// to turn over on its own: the first is answered from the cache and replaces the
// entry behind the reader's back, the second gets what the first stored. The
// banner exists to skip that, and it does so by emptying the cache first
// (`drop_cached_shell`, src/update.rs).
//
// It used to empty only `/` and `/index.html`. But the host answers every deep
// link with index.html (assets/_redirects), so the shell is cached under every
// path the reader has opened, and a reader standing on an agenda item got the
// old build back and had to press again. The stub below stores what it is given,
// which the one above cannot, so the two-loads-per-URL behaviour is visible.
const store = new Map<string, string>();
const cache = {
	put: (req: { url?: string } | string, res: Response) =>
		res
			.text()
			.then(
				(body) =>
					void store.set(typeof req === "string" ? req : (req.url ?? ""), body),
			),
	delete: (key: string) => Promise.resolve(store.delete(key)),
	keys: () => Promise.resolve([...store.keys()].map((url) => ({ url }))),
};
Object.defineProperty(globalThis, "caches", {
	configurable: true,
	writable: true,
	value: {
		open: () => Promise.resolve(cache),
		keys: () => Promise.resolve(["radikalwiki-v5"]),
		delete: () => Promise.resolve(true),
		match: (req: { url?: string } | string) => {
			const url =
				typeof req === "string"
					? new URL(req, "https://radikal.wiki").href
					: (req.url ?? "");
			const hit = store.get(url);
			return Promise.resolve(hit === undefined ? undefined : new Response(hit));
		},
	},
});
const NEW = "new build";
Object.defineProperty(globalThis, "fetch", {
	configurable: true,
	writable: true,
	value: () => Promise.resolve(new Response(NEW)),
});

/** One navigation through the worker, waiting for its background revalidate. */
async function load(path: string): Promise<string> {
	let answered: unknown;
	listeners.fetch({
		request: { method: "GET", url: `https://radikal.wiki${path}` },
		respondWith: (v: unknown) => (answered = v),
	});
	const body = await ((await answered) as Response).text();
	await new Promise((r) => setTimeout(r, 0));
	return body;
}

/** What `drop_cached_shell` does. Mirrors `names_its_own_content` in update.rs. */
async function purge() {
	for (const { url } of await cache.keys()) {
		const { pathname } = new URL(url);
		if (pathname.startsWith("/assets/") || pathname.startsWith("/symbols/"))
			continue;
		await cache.delete(url);
	}
}

const ASSET = "/assets/material-icons-dxhd3c.woff2";
const reloads: [string, string][] = [
	["/", NEW],
	["/user/login", NEW],
	["/radikal_ungdom/hb5/dagsorden_1.0", NEW],
	["/radikal_ungdom/hb5/dagsorden_1.0?tab=2", NEW],
	// A page whose own path contains /assets/ is a page, and must not be kept.
	["/radikal_ungdom/assets/plan", NEW],
];
console.log("\none press of Reload is enough, whatever page the reader is on");
for (const [path, want] of reloads) {
	store.clear();
	store.set(`https://radikal.wiki${path}`, "old build");
	store.set(`https://radikal.wiki${ASSET}`, "the icon font");
	await purge();
	const got = await load(path);
	const kept = store.get(`https://radikal.wiki${ASSET}`) === "the icon font";
	const ok = got === want && kept;
	if (!ok) bad++;
	console.log(
		`  ${ok ? "ok  " : "FAIL"} ${path.padEnd(38)} -> ${got}${kept ? "" : "  (re-downloaded the hashed asset)"}`,
	);
}

// And the same page WITHOUT the purge, so the reason for it stays on the record.
store.clear();
store.set("https://radikal.wiki/radikal_ungdom/hb5/dagsorden_1.0", "old build");
const first = await load("/radikal_ungdom/hb5/dagsorden_1.0");
const second = await load("/radikal_ungdom/hb5/dagsorden_1.0");
const doubles = first === "old build" && second === NEW;
if (!doubles) bad++;
console.log(
	`  ${doubles ? "ok  " : "FAIL"} ${"without the purge, it takes two".padEnd(38)} -> ${first}, then ${second}`,
);

console.log(bad === 0 ? "\nall good" : `\n${bad} case(s) failed`);
Deno.exit(bad === 0 ? 0 : 1);
