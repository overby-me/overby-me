dx := `which -a dx | grep dioxus | head -1`

# DWARF is not worth its weight in a shipped web bundle: it is ~90 KB gzipped
# and only readable with a browser extension.
release := "--release --debug-symbols false"

# Each screensaver is its own wasm chunk, which needs the cargo feature (so the
# `lazy_loader!` calls compile) and the dx flag (so the module is actually
# split) together. With the feature on and the flag off the generated
# `./__wasm_split.js` imports are never resolved and the app will not start.
#
# Not on by default yet: the chunks are emitted and fetched on demand, but
# dioxus-cli 0.7.9's splitter currently leaves the hacks' own code in the main
# module and the chunks come out at ~500 bytes, so today it is 13 KB of pure
# overhead. See xscreensaver/README.md; this wants solving before the bulk of
# the ports land, not after.
split := "--features split --wasm-split"

# Development builds skip the split: it costs a full extra pass over the wasm
# on every rebuild, and everything still works, just as one module.
dev:
    {{dx}} serve

build:
    {{dx}} build {{release}}
    # dx drops files from assets/ it doesn't recognize, so copy the host's
    # _redirects (SPA fallback + matrix well-knowns) into the served root.
    cp assets/_redirects target/dx/homepage/release/web/public/_redirects

# The same bundle with one wasm chunk per screensaver.
build-split:
    {{dx}} build {{release}} {{split}}
    cp assets/_redirects target/dx/homepage/release/web/public/_redirects

serve:
    {{dx}} serve {{release}}

# Render a screensaver to a PPM to eyeball a port against the original.
# Example: just shot munch 640 480 500
shot slug width="640" height="480" frames="300" query="":
    cargo run --manifest-path xscreensaver/Cargo.toml --release --example render -- \
        {{slug}} {{width}} {{height}} {{frames}} /tmp/{{slug}}.ppm {{query}}

test:
    cargo test --manifest-path xscreensaver/Cargo.toml

clean:
    {{dx}} clean
