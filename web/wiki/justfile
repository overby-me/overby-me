dx := `which -a dx | grep dioxus | head -1`

# Client-side error/panic shipping to Better Stack (the `remote-logging` feature)
# is auto-enabled when BETTERSTACK_SOURCE_TOKEN is present in the build env. The
# token + host are baked in at compile time via `option_env!` (see logging.rs),
# so export them alongside the build, e.g.:
#   BETTERSTACK_SOURCE_TOKEN=xxx BETTERSTACK_INGEST_HOST=sN.betterstackdata.com \
#     nix develop .#wiki --command bash -c "just build"
# Unset (e.g. the hermetic Nix package build), the flag is empty — console only.
remote_logging := if env_var_or_default("BETTERSTACK_SOURCE_TOKEN", "") != "" { "--features remote-logging" } else { "" }

dev:
    {{dx}} serve

build:
    {{dx}} build --release {{remote_logging}}
    # dx drops files from assets/ it doesn't recognize, so copy them into the
    # served root ourselves. This is the single source of truth for the deploy
    # bundle — the Nix package (default.nix) runs `just build`, so both match.
    #   _redirects — SPA deep-link fallback for statichost.eu (e.g. /a/b resolves)
    #   sw.js      — served from the ROOT so its scope is `/` (not the hashed
    #                /assets/ path, whose scope would only be /assets/)
    cp assets/_redirects target/dx/wiki-dioxus/release/web/public/_redirects
    cp assets/sw.js target/dx/wiki-dioxus/release/web/public/sw.js

# Build the deployable frontend bundle (index.html + assets + sw.js at the root)
# via the Nix package, then print the output dir. The final upload to
# statichost.eu (dev.radikal.wiki) is manual — see README.md#deploy — since the
# statichost credentials are not in the repo.
deploy-build:
    #!/usr/bin/env bash
    set -euo pipefail
    root="$(git rev-parse --show-toplevel)"
    out="$(nix build --no-link --print-out-paths "$root#wiki-frontend")"
    echo "frontend bundle: $out"
    echo "Upload the CONTENTS of that directory to statichost.eu (dev.radikal.wiki),"
    echo "keeping sw.js at the served root so its scope is '/'."

check:
    cargo check --target wasm32-unknown-unknown
    nu scripts/check-css-spacing.nu

# Design-system lint: spacing/font-size must flow through tokens (ratchet gate).
lint-css:
    nu scripts/check-css-spacing.nu

fmt:
    cargo fmt

clippy:
    cargo clippy --target wasm32-unknown-unknown -- -D warnings

# Unit tests (host target — GraphQL serialization, path helpers, etc.)
test:
    cargo test

# Browser smoke tests: build, serve, and drive the app in headless Servo.
# Set WIKI_EMAIL / WIKI_PASSWORD to also run the authenticated tests.
test-browser *ARGS:
    nu test-browser.nu {{ARGS}}
