dx := `which -a dx | grep dioxus | head -1`

dev:
    {{dx}} serve

build:
    {{dx}} build --release
    # dx drops files from assets/ it doesn't recognize, so copy the host's
    # _redirects (SPA fallback + matrix well-knowns) into the served root.
    cp assets/_redirects target/dx/homepage/release/web/public/_redirects

serve:
    {{dx}} serve --release

clean:
    {{dx}} clean
