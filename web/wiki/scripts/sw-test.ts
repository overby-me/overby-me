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

Object.defineProperty(globalThis, "self", { configurable: true, writable: true, value: {
  addEventListener: (t: string, fn: (e: unknown) => void) => (listeners[t] = fn),
  location: { origin: "https://radikal.wiki" },
  clients: { claim: () => Promise.resolve() },
  registration: {},
  skipWaiting: () => {},
} });
Object.defineProperty(globalThis, "caches", { configurable: true, writable: true, value: {
  open: () => Promise.resolve({ put: () => Promise.resolve(), addAll: () => Promise.resolve() }),
  keys: () => Promise.resolve([]),
  delete: () => Promise.resolve(true),
  match: (req: { url?: string } | string) => {
    const url = typeof req === "string" ? req : (req.url ?? "");
    return Promise.resolve(cacheHas.some((u) => url.endsWith(u)) ? new Response("cached") : undefined);
  },
} });
Object.defineProperty(globalThis, "fetch", { configurable: true, writable: true, value: () =>
  networkFails ? Promise.reject(new TypeError("Failed to fetch")) : Promise.resolve(new Response("net")) });

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
  ["/user/login", { cached: [], offline: true }],       // the failing case
  ["/user/login", { cached: [], offline: false }],
  ["/user/login", { cached: ["/"], offline: true }],
  ["/assets/app-abc.wasm", { cached: [], offline: true }],
  ["/assets/app-abc.wasm", { cached: ["/assets/app-abc.wasm"], offline: true }],
  ["/", { cached: [], offline: true }],
];
let bad = 0;
for (const [path, opts] of cases) {
  const res = await run(path, opts);
  const ok = res instanceof Response;
  if (!ok) bad++;
  console.log(`  ${ok ? "ok  " : "FAIL"} ${path.padEnd(24)} cached=${JSON.stringify(opts.cached).padEnd(28)} offline=${opts.offline}  -> ${ok ? `Response ${(res as Response).status}` : typeof res}`);
}
console.log(bad === 0 ? "\nevery path returns a Response" : `\n${bad} path(s) did not`);
Deno.exit(bad === 0 ? 0 : 1);
