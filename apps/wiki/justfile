dx := `which -a dx | grep dioxus | head -1`

# Client-side error/panic shipping to Better Stack (the `remote-logging` feature)
# is auto-enabled when BETTERSTACK_SOURCE_TOKEN is present in the build env. The
# token + host are baked in at compile time via `option_env!` (see logging.rs),
# so export them alongside the build, e.g.:
#   BETTERSTACK_SOURCE_TOKEN=xxx BETTERSTACK_INGEST_HOST=sN.betterstackdata.com \
#     nix develop .#wiki --command bash -c "just build"
# Unset (e.g. the hermetic Nix package build), the flag is empty — console only.
remote_logging := if env_var_or_default("BETTERSTACK_SOURCE_TOKEN", "") != "" { "--features remote-logging" } else { "" }

# The commit this bundle is built from, baked in via `option_env!("GIT_COMMIT")`
# (src/build_info.rs). It is what ties a crash report or a piece of feedback to
# the code that produced it, and what the running app compares against to notice
# it is outdated.
#
# GIT_COMMIT from the environment wins, so the Nix build — which has no .git —
# can pass the flake's rev in. Falls back to `unknown` rather than to a wrong
# answer: a report naming a commit that was never deployed is worse than one
# naming none.
# A `-dirty` suffix marks a bundle built over uncommitted changes, so a report
# from one is not read as coming from the commit it merely sat on top of.
export GIT_COMMIT := env_var_or_default("GIT_COMMIT", `printf '%s%s' "$(git rev-parse --short=8 HEAD 2>/dev/null || echo unknown)" "$(test -z "$(git status --porcelain --untracked-files=no 2>/dev/null)" || echo -dirty)"`)

dev:
    {{dx}} serve

build:
    # --debug-symbols keeps the DWARF line tables through wasm-bindgen;
    # split-symbols.nu then moves them out of the shipped binary into a sidecar,
    # so a crash can be traced to a source line without every visitor
    # downloading 20 MB of debug info. Nothing may rewrite the wasm after that
    # split; see the header of split-symbols.nu for what that cost us.
    {{dx}} build --release --debug-symbols true {{remote_logging}}
    nu scripts/split-symbols.nu
    # The boot screen's download progress needs a denominator, and the network
    # cannot supply one (gzipped + chunked = no Content-Length). Written in here,
    # after split-symbols, so it is the size a reader actually downloads.
    nu scripts/inject-wasm-size.nu
    # dx drops files from assets/ it doesn't recognize, so copy them into the
    # served root ourselves. This is the single source of truth for the deploy
    # bundle — the Nix package (default.nix) runs `just build`, so both match.
    #   _redirects — SPA deep-link fallback for statichost.eu (e.g. /a/b resolves)
    #   _headers   — cache policy: /assets/* is content-hashed, so immutable
    #   sw.js      — served from the ROOT so its scope is `/` (not the hashed
    #                /assets/ path, whose scope would only be /assets/)
    cp assets/_redirects target/dx/wiki/release/web/public/_redirects
    cp assets/_headers target/dx/wiki/release/web/public/_headers
    # The HEIC decoder, as its own wasm module for the Web Worker that runs it
    # (heic-worker/src/lib.rs). Built with the default release profile rather
    # than the app's size-tuned `wasm-release`: this one is fetched only by a
    # visitor who actually meets a HEIC, and opt-level=3 decodes an iPhone photo
    # in about two thirds the time of opt-level="z".
    #
    # At the ROOT and unhashed, because the worker script names both files by
    # absolute path -- it has no bundler to rewrite them. `_headers` therefore
    # keeps them revalidated rather than immutable like /assets/*.
    cargo build --release --target wasm32-unknown-unknown -p heic-worker
    wasm-bindgen --target no-modules --no-typescript --out-name heic-decode \
        --out-dir target/dx/wiki/release/web/public \
        target/wasm32-unknown-unknown/release/heic_worker.wasm
    cp assets/heic-worker.js target/dx/wiki/release/web/public/heic-worker.js
    # sw.js carries the build it is, so a page can ask the worker serving it
    # which deploy it came from (src/logging.rs). Substituted here rather than
    # baked by `dx`, which does not process this file.
    sed 's/__WIKI_BUILD__/'"$GIT_COMMIT"'/' assets/sw.js \
        > target/dx/wiki/release/web/public/sw.js
    # What this deploy is, for a running tab to compare itself against
    # (src/update.rs). At the root, so `_headers` keeps it revalidated rather
    # than cached like the hashed assets.
    printf '{"commit":"%s","version":"0.1.0"}\n' "$GIT_COMMIT" \
        > target/dx/wiki/release/web/public/version.json

# Build and deploy the frontend to a statichost.eu site (default: production).
#
# Needs two credentials in the environment, neither of which is in the repo:
#
#   BETTERSTACK_SOURCE_TOKEN  enables --features remote-logging in the build.
#                             REQUIRED: a bundle without it looks identical once
#                             deployed and silently reports no crashes, so the
#                             script refuses rather than ship one by accident,
#                             and checks the built wasm as well as the env var.
#   STATICHOST_APIKEY         the drop credential.
#
#   BETTERSTACK_SOURCE_TOKEN=... STATICHOST_APIKEY=... just deploy-frontend
#   ... just deploy-frontend radikal-wiki        # the dev site instead
#
# A drop REPLACES the site, so the script carries symbols/ across the build
# (see its header) and verifies the deployed commit is the one being served.
deploy-frontend site="wiki-prod":
    nu scripts/deploy-frontend.nu {{site}}

# Build the deployable frontend bundle (index.html + assets + sw.js at the root)
# via the Nix package, then print the output dir. Unlike `deploy-frontend` this
# is the hermetic Nix build, which has no Better Stack token and so ships
# without remote logging; it uploads nothing.
deploy-build:
    #!/usr/bin/env bash
    set -euo pipefail
    root="$(git rev-parse --show-toplevel)"
    out="$(nix build --no-link --print-out-paths "$root#wiki-frontend")"
    echo "frontend bundle: $out"
    echo "Upload the CONTENTS of that directory to statichost.eu (dev.radikal.wiki),"
    echo "keeping sw.js at the served root so its scope is '/'."
    echo "Include symbols/ — the backend fetches it to turn crash reports into"
    echo "source lines. No reader ever downloads it."

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
    # `logging.rs` exists only under `remote-logging`, and that is the build every
    # deploy ships — so its tests never ran in the default run.
    cargo test --features remote-logging

# Browser smoke tests: build, serve, and drive the app in headless Servo.
# Set WIKI_EMAIL / WIKI_PASSWORD to also run the authenticated tests.
test-browser *ARGS:
    nu test-browser.nu {{ARGS}}

# Drive assets/sw.js against stubbed Cache/fetch and assert every path answers
# with a Response. The service worker only matters when the network is failing,
# which is exactly when nobody is watching it.
test-sw:
    deno run --allow-read scripts/sw-test.ts

# Post-deploy check: does the LIVE site still serve content to a signed-out
# visitor? Points at a URL rather than building anything, so it is what to run
# after `just build` + upload. `just test-browser` is the pre-merge suite.
smoke *ARGS:
    nu scripts/smoke.nu {{ARGS}}
