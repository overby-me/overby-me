dx := `which -a dx | grep dioxus | head -1`

# DWARF is not worth its weight in a shipped web bundle: it is ~90 KB gzipped
# and only readable with a browser extension.
release := "--release --debug-symbols false"

# Each screensaver is its own wasm chunk, which needs the cargo feature (so the
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
    # _redirects (SPA fallback + matrix well-knowns) into the served root.
    cp assets/_redirects target/dx/homepage/release/web/public/_redirects

# The same bundle as one module, for comparing against the split one.
build-whole:
    {{dx}} build {{release}}
    cp assets/_redirects target/dx/homepage/release/web/public/_redirects

serve:
    {{dx}} serve {{release}} {{split}}

# Render a screensaver to a PPM to eyeball a port against the original.
# Example: just shot munch 640 480 500
shot slug width="640" height="480" frames="300" query="" seed="20260809":
    cargo run --manifest-path xscreensaver/Cargo.toml --release --example render -- \
        {{slug}} {{width}} {{height}} {{frames}} /tmp/{{slug}}.ppm "{{query}}" {{seed}}

test:
    cargo test --manifest-path xscreensaver/Cargo.toml

# Render a Shadertoy saver in headless chromium, which is the only thing that
# can run one. Needs a build first. Example: just browser starnest
browser slug="--all":
    nu test-browser.nu {{slug}}

# Upload the built bundle to statichost. Needs a fresh `just build` and the
# API key in the environment; the key is never written into this repository:
#   STATICHOST_APIKEY=... just deploy
deploy site="overby-me":
    nu deploy.nu {{site}} target/dx/homepage/release/web/public

clean:
    {{dx}} clean
