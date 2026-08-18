dx := `which -a dx | grep dioxus | head -1`

# DWARF is not worth its weight in a shipped web bundle: it is ~90 KB gzipped
# and only readable with a browser extension.
release := "--release --debug-symbols false"

# Serve with hot reload.
dev:
    {{dx}} serve

build:
    {{dx}} build {{release}}

serve:
    {{dx}} serve {{release}}

# The firmware, the world and the page it is drawn on.
test:
    cargo test --workspace

lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Fly the drone in the default room and print where it went and what it
# mapped, which is the fastest way to see whether a change to the navigator
# helped. Example: just trace 300
trace seconds="120":
    cargo run --release --manifest-path sim/Cargo.toml --example trace -- {{seconds}}

# Run the built bundle in headless chromium and check it draws. Needs a build
# first.
browser *args:
    nu test-browser.nu {{args}}

clean:
    {{dx}} clean
