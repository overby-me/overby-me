// Open the deployed wiki's native PDF reader on a document of our choosing and
// report what it made of it, including what the page control does when pressed.
//
// TypeScript on deno rather than nushell, which this repository otherwise
// standardises on, because the consumer requires it: driving a browser is the
// Chrome DevTools Protocol, and that is a WebSocket, which nushell has no
// client for. Run it through `read-a-pdf-in-a-browser.nu`, which is nushell.
//
// Nothing is written to the wiki. A file node the signed-in account can
// already see is made to LOOK like a PDF -- its `data.type` is rewritten in the
// GraphQL answer on the way to the app -- and the storage fetch for its bytes
// is answered with a local fixture. Everything else is the deployed build
// talking to the real backend.
//
//   deno run -A drive.ts <session.json> <fixture.pdf> <url> <out-prefix>
//
// Writes <out-prefix>.json (what it saw), .png (a screenshot) and .log.

const [sessionFile, pdfPath, url, outPrefix] = Deno.args;
if (!outPrefix) {
  console.error("usage: drive.ts <session.json> <fixture.pdf> <url> <out-prefix>");
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
  access_token_expires_at: Date.now() + (signin.accessTokenExpiresIn ?? 900) * 1000,
};
const pdf = await Deno.readFile(pdfPath);
let binary = "";
for (const byte of pdf) binary += String.fromCharCode(byte);
const pdfBase64 = btoa(binary);

const port = 9300 + Math.floor(Math.random() * 600);
const chromium = Deno.env.get("CHROMIUM") ?? "chromium";
const chrome = new Deno.Command(chromium, {
  args: [
    "--headless=new",
    "--no-sandbox",
    "--disable-gpu",
    "--use-gl=swiftshader",
    "--enable-unsafe-swiftshader",
    `--window-size=${Deno.env.get("WINDOW") ?? "1400,1000"}`,
    `--user-data-dir=${outPrefix}-profile`,
    `--remote-debugging-port=${port}`,
    "about:blank",
  ],
  stdout: "null",
  stderr: "null",
}).spawn();

async function targetUrl(): Promise<string> {
  for (let i = 0; i < 60; i++) {
    try {
      const tabs = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      const page = tabs.find((t: { type: string }) => t.type === "page");
      if (page?.webSocketDebuggerUrl) return page.webSocketDebuggerUrl;
    } catch { /* not up yet */ }
    await new Promise((r) => setTimeout(r, 250));
  }
  throw new Error("chromium never opened a debugging port");
}

const ws = new WebSocket(await targetUrl());
await new Promise((res) => ws.onopen = res);

let nextId = 1;
const pending = new Map<number, (v: unknown) => void>();
const logged: string[] = [];
// deno-lint-ignore no-explicit-any
function send(method: string, params: Record<string, unknown> = {}): Promise<any> {
  const id = nextId++;
  return new Promise((res) => {
    pending.set(id, res);
    ws.send(JSON.stringify({ id, method, params }));
  });
}

/// Make anything carrying a `fileId` claim to be a PDF, wherever it sits in the
/// answer: the app reads the mime off the node's own data blob.
// deno-lint-ignore no-explicit-any
function claimPdf(value: any): boolean {
  let touched = false;
  if (Array.isArray(value)) {
    for (const v of value) touched = claimPdf(v) || touched;
  } else if (value && typeof value === "object") {
    if (typeof value.fileId === "string") {
      value.type = "application/pdf";
      touched = true;
    }
    for (const k of Object.keys(value)) touched = claimPdf(value[k]) || touched;
  }
  return touched;
}

let rewrote = 0, served = 0;
ws.onmessage = async (e) => {
  const msg = JSON.parse(e.data);
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)!(msg.result);
    pending.delete(msg.id);
    return;
  }
  if (msg.method === "Runtime.consoleAPICalled") {
    logged.push(
      `${msg.params.type}: ${(msg.params.args ?? []).map((a: { value?: unknown; description?: string }) =>
        a.value !== undefined ? String(a.value) : (a.description ?? "")).join(" ")}`,
    );
    return;
  }
  if (msg.method === "Runtime.exceptionThrown") {
    logged.push(`exception: ${msg.params.exceptionDetails?.exception?.description ?? ""}`);
    return;
  }
  if (msg.method !== "Fetch.requestPaused") return;

  const { requestId, request, responseStatusCode } = msg.params;
  // The file's BYTES. Only the bare `/v1/files/<id>` GET: `<id>/presignedurl`
  // is a different request the page also makes, and swallowing it leaves the
  // card with no url and the reader unmounted behind an empty state.
  if (
    /\/v1\/files\/[0-9a-fA-F-]+$/.test(request.url.split("?")[0]) &&
    request.method === "GET" && responseStatusCode === undefined
  ) {
    served++;
    await send("Fetch.fulfillRequest", {
      requestId,
      responseCode: 200,
      responseHeaders: [
        { name: "content-type", value: "application/pdf" },
        { name: "access-control-allow-origin", value: "*" },
      ],
      body: pdfBase64,
    });
    return;
  }
  if (/\/v1\/graphql/.test(request.url) && responseStatusCode !== undefined) {
    try {
      const got = await send("Fetch.getResponseBody", { requestId });
      const text = got.base64Encoded ? atob(got.body) : got.body;
      const json = JSON.parse(text);
      if (claimPdf(json)) rewrote++;
      await send("Fetch.fulfillRequest", {
        requestId,
        responseCode: 200,
        responseHeaders: [
          { name: "content-type", value: "application/json" },
          { name: "access-control-allow-origin", value: "*" },
        ],
        body: btoa(unescape(encodeURIComponent(JSON.stringify(json)))),
      });
      return;
    } catch { /* fall through */ }
  }
  await send("Fetch.continueRequest", { requestId });
};

await send("Page.enable");
await send("Runtime.enable");
await send("Fetch.enable", {
  patterns: [
    { urlPattern: "*/v1/files/*", requestStage: "Request" },
    { urlPattern: "*/v1/graphql*", requestStage: "Response" },
  ],
});
await send("Page.addScriptToEvaluateOnNewDocument", {
  source: `localStorage.setItem("wiki_session", ${JSON.stringify(JSON.stringify(wikiSession))});
           localStorage.setItem("wiki_pdf_viewer", "native");`,
});

await send("Page.navigate", { url });
await new Promise((r) => setTimeout(r, 25000));

const read = `(() => {
  const control = document.querySelector(".pdf-pages");
  const doc = document.querySelector(".pdf-doc, .docx-doc");
  return JSON.stringify({
    page: control ? (control.innerText.match(/([0-9ivxlcIVXLC]+)\\s*\\/\\s*([0-9ivxlcIVXLC]+)/) || [])[1] || null : null,
    of: control ? (control.innerText.match(/([0-9ivxlcIVXLC]+)\\s*\\/\\s*([0-9ivxlcIVXLC]+)/) || [])[2] || null : null,
    blocks: doc ? doc.children.length : 0,
    headings: [...document.querySelectorAll(".pdf-doc h1, .pdf-doc h2, .pdf-doc h3, .pdf-doc h4")].map(e => e.innerText.trim()).slice(0, 8),
    links: [...document.querySelectorAll(".pdf-doc a")].map(a => a.getAttribute("href")).slice(0, 12),
    images: document.querySelectorAll(".pdf-doc img").length,
    italics: document.querySelectorAll(".pdf-doc i, .pdf-doc em").length,
    marks: document.querySelectorAll(".pdf-page-break").length,
    scrollY: Math.round(window.scrollY),
    landing: (() => { const m = document.querySelector(".pdf-page-break"); return m ? getComputedStyle(m).scrollMarginTop : null; })(),
    // Is a page-ending mark sitting on screen? After turning a page it must
    // not be: it carries the number of the page that ENDED, so seeing it reads
    // as not having moved.
    markOnScreen: [...document.querySelectorAll(".pdf-page-break")].some(m => {
      const t = m.getBoundingClientRect().top;
      return t >= -2 && t < window.innerHeight / 2;
    }),
    sizeClass: (document.querySelector(".app-shell") || {}).dataset?.sizeClass ?? null,
    maxScroll: Math.round(document.documentElement.scrollHeight - window.innerHeight),
    text: (doc ? doc.innerText : document.body.innerText).slice(0, 600),
  });
})()`;

// deno-lint-ignore no-explicit-any
const snap = async (): Promise<any> =>
  JSON.parse((await send("Runtime.evaluate", { expression: read, returnByValue: true })).result.value);

const press = async (selector: string) => {
  await send("Runtime.evaluate", {
    expression: `(document.querySelector(${JSON.stringify(selector)}) || {click(){}}).click()`,
  });
  await new Promise((r) => setTimeout(r, 2500));
};
const forward = ".pdf-pages .pdf-pages-step:last-of-type";
const back = ".pdf-pages .pdf-pages-step";

const seen: Record<string, unknown> = { opened: await snap() };
await press(forward);
seen.afterForward = await snap();
await press(back);
seen.afterBack = await snap();
// The end of the document: its last page must be reportable even though a
// final page shorter than the window can never be scrolled to the top.
await send("Runtime.evaluate", { expression: "window.scrollTo(0, document.body.scrollHeight)" });
await new Promise((r) => setTimeout(r, 2500));
seen.atTheEnd = await snap();
seen.rewroteAnswers = rewrote;
seen.servedBytes = served;
seen.console = logged.slice(-15);

await Deno.writeTextFile(`${outPrefix}.json`, JSON.stringify(seen, null, 1));
const shot = await send("Page.captureScreenshot", { format: "png" });
if (shot?.data) {
  await Deno.writeFile(`${outPrefix}.png`, Uint8Array.from(atob(shot.data), (c) => c.charCodeAt(0)));
}
await Deno.writeTextFile(`${outPrefix}.log`, logged.join("\n"));
ws.close();
try {
  chrome.kill();
} catch { /* already gone */ }
await chrome.status;
console.log(`wrote ${outPrefix}.json`);
