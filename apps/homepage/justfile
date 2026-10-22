dx := `which -a dx | grep dioxus | head -1`

# DWARF is not worth its weight in a shipped web bundle: it is ~90 KB gzipped
# and only readable with a browser extension.
release := "--release --debug-symbols false"

# `/cardioid` is its own wasm chunk, which needs the cargo feature (so the
# `lazy_loader!` calls compile) and the dx flag (so the module is actually
# split) together. With the feature on and the flag off the generated
# `./__wasm_split.js` imports are never resolved and the app will not start.
split := "--features split --wasm-split"

# Development builds skip the split: it costs a full extra pass over the wasm
# on every rebuild, and everything still works, just as one module.
dev:
    {{dx}} serve

build:
    {{dx}} build {{release}} {{split}}
    # dx drops files from assets/ it doesn't recognize, so copy the host's
    # _redirects (SPA fallback + matrix well-knowns) and the page that forwards
    # /screensaver links into the served root.
    cp assets/_redirects assets/screensaver.html target/dx/homepage/release/web/public/

# The same bundle as one module, for comparing against the split one.
build-whole:
    {{dx}} build {{release}}
    cp assets/_redirects assets/screensaver.html target/dx/homepage/release/web/public/

serve:
    {{dx}} serve {{release}} {{split}}

# Upload the built bundle to statichost. Needs a fresh `just build` and the
# API key in the environment; the key is never written into this repository:
#   STATICHOST_APIKEY=... just deploy
deploy site="overby-me":
    nu deploy.nu {{site}} target/dx/homepage/release/web/public

clean:
    {{dx}} clean
