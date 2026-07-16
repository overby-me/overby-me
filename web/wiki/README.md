# RadikalWiki (Dioxus / WASM)

A Rust + [Dioxus](https://dioxuslabs.com) 0.7 frontend compiled to WebAssembly:
the port that replaced the former React RadikalWiki app in this directory. It
talks to the same NHost / Hasura backend (auth over REST, data over GraphQL via
[cynic](https://cynic-rs.dev)). The crate and Dioxus app are still named
`wiki-dioxus` internally.

See [`PLAN.md`](./PLAN.md) for the roadmap and the list of known issues.

## Develop

Everything runs inside the `wiki` dev shell (`nix develop .#wiki`,
or automatically via direnv). Commands are in the `justfile`:

```bash
just dev        # dx serve — hot-reloading dev server on http://127.0.0.1:8080
just build      # dx build --release → target/dx/wiki-dioxus/release/web/public
just check      # cargo check (wasm32 target)
just clippy     # cargo clippy -D warnings (wasm32 target)
just test       # cargo unit tests (host target)
just fmt        # cargo fmt
```

## Testing in the browser (Servo)

We test the real app in a real browser engine using **headless
[Servo](https://servo.org)** driven over the W3C **WebDriver** protocol. Servo
ships with a built-in WebDriver server, so no external driver binary is needed —
`test-browser.nu` orchestrates everything from nushell + `curl` + `jq`.

```bash
just test-browser              # unauthenticated smoke tests
just test-browser -- --verbose # + print Servo's stderr at the end
just test-browser -- --keep    # leave dx serve + Servo running afterwards

# Authenticated tests are opt-in and need real credentials.
# NEVER commit these — pass them via the environment:
WIKI_EMAIL=you@example.com WIKI_PASSWORD=secret just test-browser
```

What it does:

1. Starts `dx serve` on port `8134` (the **debug** build — see caveats below).
2. Starts `servoshell --headless --webdriver=7134`.
3. Opens a WebDriver session, navigates the app, and asserts on the DOM
   (the app mounts into `#main`): shell renders, welcome card, login/register
   links, client-side routing into `/user/login`, and — when credentials are
   given — that logging in populates the drawer's groups/events list.

### Driving Servo by hand

Useful when debugging a specific interaction. Start Servo's WebDriver server:

```bash
servoshell --headless --webdriver=7134 about:blank &
```

Then talk to it with plain HTTP (WebDriver is just JSON over HTTP):

```bash
WD=http://127.0.0.1:7134
SID=$(curl -s -H 'content-type: application/json' -d '{"capabilities":{}}' \
        $WD/session | jq -r '.value.sessionId')

# Load the app
curl -s -H 'content-type: application/json' \
     -d '{"url":"http://127.0.0.1:8080/"}' $WD/session/$SID/url

# Read the DOM / run JS in the page
curl -s -H 'content-type: application/json' \
     -d '{"script":"return document.getElementById(\"main\").innerText","args":[]}' \
     $WD/session/$SID/execute/sync | jq -r '.value'
```

Rust `log::info!` / panics surface on Servo's **stderr**, so run `servoshell`
with its output captured to see `RadikalWiki starting...` and any wasm traps.

### Servo caveats

- **Use the debug build (`dx serve`) for Servo.** The size-optimised release
  bundle currently fails to load in Servo 0.3 (`TypeError: Module fetching
  failed`); a normal browser (Chrome/Firefox) loads either build fine.
- **Flaky wasm panic on authenticated reload.** Reloading a page that already
  has a stored session sometimes traps with `unreachable executed` during wasm
  instantiation. The harness sidesteps it by always signing in *fresh*
  (mount logged-out → submit the form). Tracked in `PLAN.md`.

## Deploy

Two independent pieces against the shared NHost/Hasura backend.

**Backend** (Scaleway Serverless Container `wiki-backend`, fr-par): `just deploy`
in `backend/`. It provisions `skopeo` + `scaleway-cli` from nixpkgs, builds the
OCI image with Nix, pushes it to `rg.fr-par.scw.cloud/wiki-dioxus`, redeploys the
container, and polls until it reports `ready`. Env vars/secrets live on the
container (managed in the Scaleway console).

**Frontend** (`dev.radikal.wiki`, served by statichost.eu): `just deploy-build`
builds the Nix bundle (`#wiki-frontend` → `index.html` + `assets/` + a
root `sw.js`) and prints the output dir. The upload to statichost.eu is **manual**
(credentials are not in the repo) — upload the *contents* of that dir, keeping
`sw.js` at the served root so its service-worker scope is `/`.

## Layout

```text
src/
├── main.rs          # entry point, router, stylesheet
├── route.rs         # Route enum
├── graphql.rs       # cynic queries/mutations + HTTP execution (unit tested)
├── nhost.rs         # NHost auth (REST)
├── session.rs       # session signal + localStorage
├── i18n.rs theme.rs snackbar.rs
└── components/       # layout (drawer/home list), auth, folder, content, …
graphql/schema.graphql   # Hasura schema (source for cynic)
test-browser.nu          # headless-Servo WebDriver smoke tests
```
