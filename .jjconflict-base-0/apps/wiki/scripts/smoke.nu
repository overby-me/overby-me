#!/usr/bin/env nu
# Is the DEPLOYED site serving content to a signed-out visitor?
#
# Not to be confused with `just test-browser` (test-browser.nu), which is the
# real suite: WebDriver, DOM assertions, a WCAG contrast audit, screenshots —
# and which builds and drives its OWN dev server on a fixed local port. That is
# the pre-merge check. This is the post-deploy one, and the difference is the
# target: this points at a URL that is already live, needs no build, no
# WebDriver and no dev server, and finishes in under a minute.
#
# It exists because of what went wrong: the node query asked for a column the
# public role may not select, so EVERY page answered "not available" to anyone
# not signed in. Nothing local could see it — the failure was the deployed
# bundle talking to the real server — and it was caught by a person opening the
# site and looking.
#
# Deliberately shallow. It does not log in, click or measure. It answers the one
# question that has actually gone wrong in production.
#
#   nu scripts/smoke.nu                        # production
#   nu scripts/smoke.nu http://localhost:8080  # a dev server, same check
#
# Needs a Chromium. It is not in the devshell (a browser is a large dependency
# for a check most runs do not do), so point at one:
#
#   CHROMIUM=$(nix build --no-link --print-out-paths nixpkgs#chromium)/bin/chromium \
#     nu scripts/smoke.nu

# What a signed-out reader must be able to see. `expect` is a string that has to
# appear in the rendered DOM — chosen to be content the server had to answer for,
# not chrome the app can draw on its own.
const PAGES = [
    [path, expect, why];
    ["/", "RadikalWiki", "the welcome page renders for someone with no account"]
    ["/blog", "Hello World!", "a public context lists its children"]
    ["/blog/hello_world!", "Test!", "a public document resolves and shows its content"]
    ["/user/login", "", "the login route renders rather than 404ing"]
]

# Text that means the app failed, wherever it appears.
const FORBIDDEN = [
    "Something went wrong"
    "The document is not available"
]

def browser [] {
    let from_env = ($env | get -o CHROMIUM)
    if $from_env != null and ($from_env | path exists) {
        return $from_env
    }
    let on_path = (which chromium | get -o 0.path)
    if $on_path != null {
        return $on_path
    }
    print "No Chromium found. Set CHROMIUM, or:"
    print "  CHROMIUM=$(nix build --no-link --print-out-paths nixpkgs#chromium)/bin/chromium \\"
    print "    nu scripts/smoke.nu"
    exit 2
}

def main [
    base?: string # base URL to test (default: production)
] {
    let base = ($base | default "https://radikal.wiki")
    let chrome = (browser)
    let profile = (mktemp -d)
    mut failures = 0

    print $"Smoke-testing ($base)"
    print ""

    for page in $PAGES {
        let url = $"($base)($page.path)"
        let flags = [
            "--headless=new" "--no-sandbox" "--disable-gpu" "--use-gl=swiftshader"
            "--enable-unsafe-swiftshader" "--no-first-run"
            "--disable-background-networking" "--enable-logging=stderr"
            "--virtual-time-budget=20000" $"--user-data-dir=($profile)" "--dump-dom"
        ]
        let out = (do -i { ^$chrome ...$flags $url } | complete)

        let dom = $out.stdout
        mut problems = []

        if ($dom | str length) < 500 {
            $problems = ($problems | append "the page returned almost nothing")
        }
        if $page.expect != "" and not ($dom | str contains $page.expect) {
            $problems = ($problems | append $"missing (($page.expect)) — ($page.why)")
        }
        for bad in $FORBIDDEN {
            if ($dom | str contains $bad) {
                $problems = ($problems | append $"shows \"($bad)\"")
            }
        }
        # The app's own logger prints `%cERROR`; the browser's unrelated noise
        # (GCM, sandbox warnings) is not ours and is ignored on purpose.
        let app_errors = (
            $out.stderr
            | lines
            | where {|l| ($l | str contains "%cERROR") or ($l | str contains "Uncaught") }
        )
        if ($app_errors | length) > 0 {
            $problems = ($problems | append $"($app_errors | length) console error\(s\)")
        }

        if ($problems | length) == 0 {
            print $"  ok    ($page.path)"
        } else {
            $failures = $failures + 1
            print $"  FAIL  ($page.path)"
            for p in $problems { print $"          ($p)" }
            for e in ($app_errors | first 3) {
                print $"          ($e | str substring 0..160)"
            }
        }
    }

    rm -rf $profile
    print ""
    if $failures > 0 {
        print $"($failures) of ($PAGES | length) pages failed."
        exit 1
    }
    print $"All ($PAGES | length) pages render for a signed-out visitor."
}
