// Open a page of the deployed wiki as a signed-in reader and run a probe in it.
//
// TypeScript on deno rather than nushell, which this repository otherwise
// standardises on, because the consumer requires it: driving a browser is the
// Chrome DevTools Protocol, and that is a WebSocket, which nushell has no
// client for. Run it through `check-word-pagination.nu`, which is nushell.
//
//   deno run -A drive.ts <session.json> <url> <out-prefix> <probe.js>
//
// Nothing is written to the wiki: the probe reads the page and nothing else.
// session.json is the nhost sign-in answer; it is rewritten into the app's own
// `wiki_session` shape and seeded before the first script runs, so the app boots
// already signed in.

const [sessionFile, url, outPrefix, probeFile] = Deno.args;
if (!url || !probeFile) {
	console.error("usage: drive.ts <session.json> <url> <out-prefix> <probe.js>");
	Deno.exit(2);
}

const signin = JSON.parse(await Deno.readTextFile(sessionFile)).session;
const wikiSession = {
	user: {
		id: signin.user.id,
		email: signin.user.email ?? "",
		display_name: signin.user.displayName ?? "",
		avatar_url: signin.user.avatarUrl ?? "",
	},
	access_token: signin.accessToken,
	refresh_token: signin.refreshToken,
	node_id: null,
	access_token_expires_at:
		Date.now() + (signin.accessTokenExpiresIn ?? 900) * 1000,
};

// A port of its own per run, so two checks can run side by side.
const port = 9222 + Math.floor(Math.random() * 700);
const profile = `${outPrefix}-profile`;
const chromium = Deno.env.get("CHROMIUM") ?? "chromium";
const chrome = new Deno.Command(chromium, {
	args: [
		"--headless=new",
		"--no-sandbox",
		"--disable-gpu",
		// The app draws with WebGL, which headless has no GPU for.
		"--use-gl=swiftshader",
		"--enable-unsafe-swiftshader",
		"--window-size=1400,1000",
		`--user-data-dir=${profile}`,
		`--remote-debugging-port=${port}`,
		"about:blank",
	],
	stdout: "null",
	stderr: "null",
}).spawn();

async function targetUrl(): Promise<string> {
	for (let i = 0; i < 60; i++) {
		try {
			const r = await fetch(`http://127.0.0.1:${port}/json/list`);
			const tabs = await r.json();
			const page = tabs.find((t: { type: string }) => t.type === "page");
			if (page?.webSocketDebuggerUrl) return page.webSocketDebuggerUrl;
		} catch {
			/* not up yet */
		}
		await new Promise((r) => setTimeout(r, 250));
	}
	throw new Error("chromium never opened a debugging port");
}

const ws = new WebSocket(await targetUrl());
await new Promise((res) => (ws.onopen = res));

let nextId = 1;
const pending = new Map<number, (v: unknown) => void>();
const consoleLines: string[] = [];
ws.onmessage = (e) => {
	const msg = JSON.parse(e.data);
	if (msg.id && pending.has(msg.id)) {
		pending.get(msg.id)!(msg.result);
		pending.delete(msg.id);
	} else if (msg.method === "Runtime.consoleAPICalled") {
		const text = (msg.params.args ?? [])
			.map((a: { value?: unknown; description?: string }) =>
				a.value !== undefined ? String(a.value) : (a.description ?? ""),
			)
			.join(" ");
		consoleLines.push(`${msg.params.type}: ${text}`);
	} else if (msg.method === "Runtime.exceptionThrown") {
		consoleLines.push(
			`exception: ${msg.params.exceptionDetails?.exception?.description ?? ""}`,
		);
	}
};

// deno-lint-ignore no-explicit-any
function send(
	method: string,
	params: Record<string, unknown> = {},
): Promise<any> {
	const id = nextId++;
	return new Promise((res) => {
		pending.set(id, res);
		ws.send(JSON.stringify({ id, method, params }));
	});
}

await send("Page.enable");
await send("Runtime.enable");
await send("Page.addScriptToEvaluateOnNewDocument", {
	source: `localStorage.setItem(${JSON.stringify("wiki_session")}, ${JSON.stringify(
		JSON.stringify(wikiSession),
	)});`,
});

await send("Page.navigate", { url });
// The app is WASM: give it time to boot, sign in, fetch the document and
// measure it. The measurement waits for its own fonts, so this waits for both.
await new Promise((r) => setTimeout(r, 22000));

const probed = await send("Runtime.evaluate", {
	expression: await Deno.readTextFile(probeFile),
	awaitPromise: true,
	returnByValue: true,
});
await Deno.writeTextFile(
	`${outPrefix}.json`,
	String(probed.result?.value ?? JSON.stringify(probed)),
);
await Deno.writeTextFile(`${outPrefix}.log`, consoleLines.join("\n"));

ws.close();
try {
	chrome.kill();
} catch {
	/* already gone */
}
await chrome.status;
