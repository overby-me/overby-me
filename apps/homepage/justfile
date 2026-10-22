dx := `which -a dx | grep dioxus | head -1`

# DWARF is not worth its weight in a shipped web bundle: it is ~90 KB gzipped
# and only readable with a browser extension.
release := "--release --debug-symbols false"

dev:
    {{dx}} serve

build:
    {{dx}} build {{release}}
    # dx drops files from assets/ it doesn't recognize, so copy the host's
    # _redirects (SPA fallback + matrix well-knowns) and the pages that forward
    # /screensaver and /cardioid links into the served root.
    cp assets/_redirects assets/screensaver.html assets/cardioid.html target/dx/homepage/release/web/public/

serve:
    {{dx}} serve {{release}}

# Upload the built bundle to statichost. Needs a fresh `just build` and the
# API key in the environment; the key is never written into this repository:
#   STATICHOST_APIKEY=... just deploy
deploy site="overby-me":
    nu deploy.nu {{site}} target/dx/homepage/release/web/public

clean:
    {{dx}} clean
