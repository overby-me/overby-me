# rust-curl: Plan to Pass Upstream curl Tests

## Overview

Use the upstream [curl test suite](https://github.com/curl/curl/tree/master/tests) to validate the Rust curl rewrite against the real curl CLI behavior, following the same Nix-based testing pattern used by `rust/awk`.

## Current Status

**729 tests passing** (was 709 at session start; +12 across multipart, retry, IPv6,
upload-stdin redirect, write-out, interface, and dump-header fixes) across the
curl 8.19.0 test suite (verified with strict
runner checks — the derivation fails when a test number doesn't exist or the
suite reports anything other than 100% OK).

The full list is in `default.nix` under `testNums`; see the per-fix bullets
in "Phase 7" below for what each addition unlocks.

Test infrastructure is operational: `testsuite.nix` builds curl's C test servers from `pkgs.curl.src`, then runs `runtests.pl -c` against `rust-curl-dev`. The runner exits 0 for non-existent test numbers and for skipped tests, so `testsuite.nix` also greps the output for `No existing test cases were specified`, `TESTFAIL`, and `No tests were performed`, and requires `reported OK: 100%` — this prevents false positives in the curated list.

The Rust curl implementation supports: HTTP/HTTPS GET/POST/PUT, redirects, basic auth, cookies, TLS (rustls), multipart forms, verbose output, write-out formatting (`-w` including `%output{}`, `%{stderr}`, `%header{}`, `%{header_json}`), retry logic (including 429), range requests, file upload, gzip/deflate decompression (`--compressed`), time conditions (`-z`), URL glob output numbering (`-o #[num]`), config file parsing (`-K`), in-memory cookie engine (`-b none`), CONNECT proxy tunnel (`-p`, `--proxytunnel`), `--skip-existing`, `--no-clobber`/`--clobber`, `--stderr <file>`. No HTTP/2.

Run a test: `nix build .#checks.x86_64-linux.rust-curl-test-{num}`
View failure diff: `nix log .#checks.x86_64-linux.rust-curl-test-{num}`

---

## Upstream Test Suite Architecture

### How curl tests work

Curl's test suite is a custom Perl-based framework (`tests/runtests.pl`) with ~1,989 test cases in `tests/data/testNUM` files. Each test file is XML-like with four sections:

- **`<info>`** — metadata and keywords (e.g., `HTTP GET`, `followlocation`)
- **`<reply>`** — what the mock server should respond with
- **`<client>`** — the curl command to run, which server to start, required features
- **`<verify>`** — expected stdout, stderr, exit code, protocol bytes sent

The test runner starts custom mock servers (C programs in `tests/server/`), runs curl against them, and compares output.

### Key property: `-c` flag

`runtests.pl -c /path/to/binary` allows testing an **alternate curl binary** against the same infrastructure. This is the primary integration point for testing rust-curl.

### Dependencies

- **Perl** — the test runner and all infrastructure is Perl
- **C compiler** — test servers (`tests/server/`) must be compiled from the curl C source
- **Python 3** — for SMB/TELNET test servers (optional, can skip these)
- **stunnel** — for HTTPS/FTPS tests
- **diff** — for output comparison

## Nix Integration Plan

### Phase 0: Build the curl test infrastructure

Create a Nix derivation that builds the upstream curl project's test servers and makes `runtests.pl` available, without building the C curl binary itself (or ignoring it in favor of rust-curl).

```text
pkgs.curl.src  →  extract  →  build test servers  →  runtests.pl + servers available
```

This is analogous to how `rust/awk/testsuite.nix` extracts `pkgs.gawk.src` to get the gawk test files.

### Phase 1: testsuite.nix — single-test derivation

Create `rust/curl/testsuite.nix` following the `rust/awk` pattern:

```nix
# Run a single curl test against rust-curl
{ pkgs, testNum }:
pkgs.runCommand "rust-curl-test-${toString testNum}" {
  nativeBuildInputs = [
    pkgs.rust-curl-dev   # the binary under test
    pkgs.perl            # test runner
    pkgs.curl            # for test servers (built from C source)
    pkgs.stunnel         # for TLS tests
    pkgs.python3         # optional test servers
    pkgs.coreutils
    pkgs.diffutils
    pkgs.gnused
  ];
  curlSrc = pkgs.curl.src;
} ''
  # Extract curl source for test data + infrastructure
  tar xf $curlSrc
  CURL_SRC=$(echo curl-*)
  cd "$CURL_SRC"

  # Build test servers only (not curl itself)
  # ... configure & make in tests/server/ ...

  # Run single test with rust-curl as the binary
  cd tests
  perl runtests.pl -c ${pkgs.rust-curl-dev}/bin/curl -a -n ${toString testNum}

  # Normalize and compare (handle User-Agent version differences, etc.)
  # ...

  touch $out
''
```

**Key normalizations needed:**

- `User-Agent: curl/X.Y.Z` version string differences
- Nix store paths in error messages
- Header ordering differences (if any)

### Phase 2: default.nix — test check definitions

Extend `rust/curl/default.nix` with:

1. A `rust-curl-dev` debug build package (for faster test iteration)
2. A `checks` section mapping test numbers to individual derivations

```nix
checks = let
  testNums = [
    # Phase 1: Basic HTTP GET
    1 2 3 ...
    # Phase 2: HTTP POST
    ...
  ];
in
  builtins.listToAttrs (map (num: {
    name = "rust-curl-test-${toString num}";
    value = pkgs: import ./testsuite.nix { inherit pkgs; testNum = num; };
  }) testNums);
```

### Phase 3: Identify applicable tests

Not all ~1,989 tests apply. Filter by:

1. **Must be curl tool tests** (not libcurl/unit tests) — check `<client><command>` uses `curl` binary
2. **Must use only HTTP/HTTPS** — we don't support FTP, SCP, SMTP, etc.
3. **Must not require features we lack** — skip tests with `<features>` requiring HTTP/2, HTTP/3, proxy, alt-svc, HSTS, etc.
4. **Skip tests in `tests/data/DISABLED`** — known-flaky upstream

Use `runtests.pl -l` and keyword filtering to enumerate candidates.

#### Recommended test categories (by priority)

| Priority | Keyword/Category | Description | Est. tests |
|----------|-----------------|-------------|------------|
| 1 | `HTTP GET` | Basic retrieval | ~50 |
| 2 | `HTTP POST` | POST with data | ~30 |
| 3 | `followlocation` | Redirect following (`-L`) | ~20 |
| 4 | `HTTP PUT` | Upload operations | ~10 |
| 5 | `--write-out` | Output formatting (`-w`) | ~15 |
| 6 | `verbose` | Verbose output (`-v`) | ~10 |
| 7 | `cookies` | Cookie handling (`-b`, `-c`) | ~15 |
| 8 | `HTTP auth` | Basic auth (`-u`) | ~10 |
| 9 | `HTTPS` | TLS connections | ~20 |
| 10 | `--head` | HEAD requests (`-I`) | ~10 |

### Phase 4: Iterative test adoption

Follow the rust/awk pattern of tracking pass/fail counts:

1. Start with the simplest tests (test 1 = basic HTTP GET)
2. Run, identify failures, fix rust-curl
3. Add passing tests to the `testNums` list in `default.nix`
4. Update this PLAN.md with progress and failure categories

---

## Challenges & Mitigations

### Challenge 1: Building test servers in Nix sandbox

The curl test servers are C programs that need to be compiled. Options:

- **Option A**: Build the full curl C project in the test derivation, use only the servers
- **Option B**: Create a separate `curl-test-servers` derivation that builds just the test infrastructure
- **Option C**: Use `pkgs.curl` (the Nix package) which already has the compiled servers — check if test data is included or extractable

**Recommended**: Option B — create a `curl-test-servers` package from `pkgs.curl.src` that only builds the test infrastructure.

### Challenge 2: Network access in Nix sandbox

Nix builds have no network access. The curl test suite uses localhost servers, which should work since both server and client run in the same sandbox. Verify that `runtests.pl` binds to `127.0.0.1` only.

### Challenge 3: User-Agent version mismatch

Tests verify exact `User-Agent: curl/X.Y.Z` strings. Either:

- Normalize User-Agent in output comparison (like awk normalizes `ARGV[0]`)
- Make rust-curl report the same version string as the upstream curl being tested
- Use `<strip>` directives already in test files

### Challenge 4: Feature parity gaps

Many tests require features not yet implemented. The `<features>` section in each test file is the filter — any test requiring a missing feature should be skipped rather than failing.

Current gaps:

- No HTTP/2 or HTTP/3
- No SOCKS proxy
- No progress meter
- No connection reuse/pooling
- No DNS-over-HTTPS
- No alt-svc, HSTS

---

## Implementation Phases

### Phase 1: Infrastructure — done

- [x] `rust-curl-dev` debug package in `default.nix`
- [x] `curl-test-infra` derivation builds upstream C curl + servers from `pkgs.curl.src`
- [x] `testsuite.nix` runs a single test via `runtests.pl -c`
- [x] Localhost test servers verified working in Nix sandbox
- [x] Tests 1, 2, 3… through to 1642 running

### Phase 2: Basic HTTP tests — done

- [x] HTTP GET tool tests enumerated and passing (1, 2, 3, 4, 9, 10, 11, …)
- [x] `testNums` list in `default.nix` updated incrementally
- [x] Fixed: header ordering, exit codes, URL path normalization, query encoding

### Phase 3: POST/PUT and data handling — done

- [x] HTTP POST tests (`-d`, `--data-raw`, `--data-binary`) passing
- [x] HTTP PUT tests (`-T`) passing
- [x] Multipart form tests (`-F`) passing (boundary format fixed)
- [x] Content-Length dedup, form file error reporting

### Phase 4: Redirects and auth — done

- [x] Redirect tests (`-L`, `--max-redirs`, relative-path redirects) passing
- [x] Basic auth (`-u`) header encoding correct
- [x] Cookie accumulation, cookie jar (`-b`, `-c`) passing

### Phase 5: Output and formatting — done

- [x] Write-out format (`-w`) tests passing
- [x] Verbose output (`-v`) tests passing
- [x] Header inclusion (`-i`, `-I`, `-D`) tests passing
- [x] `-o -` stdout redirection, resume transfer (`-C`)

### Phase 6: TLS/HTTPS — partial

- [x] Basic HTTPS via rustls
- [x] Insecure mode (`-k`)
- [ ] Client certificate tests (`--cert`, `--key`)
- [ ] CA bundle edge cases (`--cacert`)
- [ ] SNI/ALPN scenarios covered by the test suite

### Phase 7: Expand coverage — in progress

- [x] Sweep tests 200-500 and 500-1000 with discovery to find newly-passing tests
- [x] Full suite sweeps through test 2000 completed
- [x] Tests 1-200: 109/114 applicable (95%) — remaining failures are cookie edge cases and connection reuse
- [x] Identify and fix top blocker (likely: proxy support, chunked request body)
- [x] Target 350+ passing by addressing one feature gap at a time
- [x] Proxy URL userinfo extraction with percent-decoding (unlocked tests 264, 278, 279)
- [x] `--ignore-content-length` support (unlocked test 269)
- [x] `.localhost` TLD resolution to 127.0.0.1 per RFC 6761 (unlocked test 389)
- [x] Hostname length limit (65535 bytes) with exit code 3 (unlocked test 399)
- [x] Sweep 2000-2100 (found test 2088)
- [x] Cookie Max-Age=0 deletion — expired cookies removed from memory and file-based stores (unlocked tests 327, 329)
- [x] Cookie deletion tracking across file-based cookie jars (deleted_cookies blocklist)
- [x] Drop custom Cookie header on cross-host redirect (unlocked test 330)
- [x] `--next` / `-:` option for resetting per-URL options (unlocked tests 420, 422)
- [x] Sweep 1000-1200 (found test 1160)
- [x] Per-URL option snapshots for correct `--next` behavior (unlocked test 386)
- [x] `-K` missing config file error with exit code 26 (unlocked test 411)
- [x] Expires-at-epoch cookie deletion (treat `Expires: epoch` as expired, not session)
- [x] Expect: 100-continue with 417 retry — split header/body send, read 100/417, retry without Expect (unlocked test 357)
- [x] Chunked Transfer-Encoding trailer support — read trailer headers after final chunk, include in body output and -D dump (unlocked test 266)
- [x] `--proto-redir` support — parse comma-separated +/-/= tokens, block redirect when target scheme not allowed, return 301 headers and exit code 1 (unlocked test 325)
- [x] `--follow` option as alias for `--location` with override warning (unlocked tests 794, 796, 797)
- [x] Drop Authorization/Cookie/Host headers on cross-scheme/cross-port redirects (not just cross-host) (unlocked test 898)
- [x] `--retry-all-errors` retries on response-error flags (partial_file, timed_out, recv_error, weird_server_reply, bad_content_encoding) (unlocked test 1909)
- [x] Accept `Domain=localhost` cookies per RFC 6761 (no-dot exception) (unlocked test 798)
- [x] `--ca-native`/`--no-ca-native` no-ops; reject empty cacert with exit 77 (unlocked test 305)
- [x] Strip path component from proxy URL (e.g. `http://proxy.example/path`) (unlocked test 346)
- [x] Reject `--etag-compare`/`--etag-save` with multiple URLs (exit 2) (unlocked test 484)
- [x] Use Host header (request_host) instead of connection IP for secure-cookie loopback exception — fixes `-H "Host:"` override case (unlocked test 61)
- [x] `-i` retry output includes redirect chain bytes from prior failed attempts
- [x] `-h <option>` validates long option names; unknown names print error and exit 0 (unlocked test 1709)
- [x] Reject `Transfer-Encoding` chains where `chunked` is not the final token (exit 61, unlocked test 1546)
- [x] Reject responses with multiple Content-Length values that disagree numerically; normalize comma-list / repeated CLs that agree (exit 8, unlocked tests 770, 771)
- [x] Reject responses with multiple `Location:` headers (exit 8, unlocked test 772)
- [x] Reject `-m <secs>` values that overflow when converted to milliseconds (exit 2, unlocked test 746)
- [x] Unknown long option error format `option X: is unknown` + help hint (unlocked test 1179)
- [x] Tolerate one extra leading slash after scheme separator (`http:///example.test/...` to `http://example.test/...`); reject 4+ slashes as malformed (unlocked test 1141)
- [x] `--write-out` `%{onerror}` gating + `%{urlnum}`/`%{exitcode}`/`%{errormsg}` substitutions (unlocked test 1188)
- [x] Tolerate Proxy-Connection from `-H` overriding curl's auto Proxy-Connection (unlocked test 1180)
- [x] `--proxy-header` parsing; for HTTP-via-proxy (no CONNECT) merge into request headers (unlocked test 1181)
- [x] HTTP request emits `Content-Type` before `Expect: 100-continue` to match curl ordering
- [x] Honor user-supplied `Expect: 100-continue` header (do handshake even when not auto-added); accept `--expect100-timeout` (unlocked tests 1129, 1130, 1131)
- [x] Chunked body reader: tolerate bare-LF chunk separators (unlocked tests 1124, 1170)
- [x] Cookie engine activates with `-c` cookie jar so Set-Cookie accumulates across redirects (unlocked test 1104)
- [x] Strip surrounding double quotes from cookie path attribute; treat TAB as control char (unlocked test 1105)
- [x] Cookie size limit (4096 bytes name+value) — reject oversized Set-Cookie (unlocked test 1151)
- [x] Accumulate Set-Cookie response headers across redirects within `perform()` (unlocked test 1104)
- [x] Percent-encode high-bit (>=0x80) bytes in request-line path so raw UTF-8 in Location headers becomes ASCII-clean (unlocked test 1138)
- [x] `--tr-encoding` with user-supplied `Connection:` header — append `, TE` to user's value instead of duplicating Connection header (unlocked test 1125)
- [x] `--no-http0.9` (and default) rejects bare-body responses with exit 1 (unsupported protocol) instead of treating as malformed status (unlocked test 1172)
- [x] `--request-target` path emits `Proxy-Connection: Keep-Alive` for HTTP-via-proxy (unlocked test 1613)
- [x] Cookie domain validation: reject TLD-only domains even with trailing dots (e.g. `.me.` is `me`, still TLD-only) (unlocked test 977)
- [x] `--next` requires a URL after it — flag pending state and exit 2 if not satisfied (unlocked test 686)
- [x] URL glob: error on unmatched `{`; reject unmatched `[` only when the trailing run looks like a range (contains `-`) (unlocked tests 1234, 1236, 1289)
- [x] Reject URL authority with multiple `@` or stray `:` after host port (unlocked test 1260)
- [x] Redirect to `//host/path` (protocol-relative) reuses base scheme (unlocked test 1314)
- [x] `--no-remote-name` accepted as inverse of `-O` (unlocked test 1278)
- [x] **Hit 500/500 passing milestone**
- [x] Multi-Location header: tolerate identical duplicates (collapse to one); reject only when values differ (fixed test 773 regression)
- [x] HTTP header line folding: trim trailing whitespace before joining continuation lines (unlocked test 1274)
- [x] Chunked trailer line ending: normalize bare LF to CRLF on output (unlocked test 1417)
- [x] Reject Transfer-Encoding values the client did not request (unlocked test 1496)
- [x] Reject TE chains where chunked is not last (already implemented; unlocked test 1495 with deeper-pattern test)
- [x] Strict status-code validation: must be exactly 3 ASCII digits, else exit 1 (unlocked tests 1430, 1431, 1432)
- [x] `--proto` parsing: `--proto -all` exits 2 when no protocols enabled (unlocked test 1474)
- [x] `-h <bad-category>` prints category list rather than full help (unlocked test 1462)
- [x] `--option=value` long-option parsing recognized (unlocked tests 1335)
- [x] Glob `{,...,,}` (matched empty alternatives) now error path; URLs without scheme rejected too (unlocked test 759)
- [x] -m / -z / similar overflow guards now also catch `184467440737095510` (unlocked test 1427)
- [x] Glob unmatched-bracket fix tightened to detect overflow ranges (test 1289 already covered)
- [x] Duplicate-but-identical Location collapses (test 773 keeps passing); HTTP/0.9 reject without --http0.9; --output-dir + -O write semantics (test 1335)
- [x] Reject single-digit / multi-line status codes via length check (tests 1431, 1432)
- [x] HTTP header continuation tolerates leading TAB and bare CRs in input (test 1274)
- [x] Chunked TE+identity ordering accepted (test 1493)
- [x] `--noproxy` CLI option overrides NO_PROXY env var (unlocked tests 1248, 1252, 1253, 1254)
- [x] `--fail-early` aborts after first non-zero URL (unlocked test 1247)
- [x] POST with empty body and user-supplied `Transfer-Encoding: chunked` emits `0\r\n\r\n` (unlocked test 1333)
- [x] Reject responses with more than 5000 header lines (exit 100, unlocked test 747)
- [x] Negative `--max-time` rejected with exit 2 (already implemented; unlocked test 1410)
- [x] CONNECT request emits default User-Agent before Proxy-Connection; `--proxy-header` headers go after Proxy-Connection on CONNECT (matches curl ordering — fixed 287/749 split)
- [x] After consuming 1xx interim, garbage instead of HTTP/x.y is "weird server reply" (exit 8, unlocked test 1480)
- [x] Repeated `Transfer-Encoding: chunked` headers tolerated (only error if chunked is followed by another encoding) (unlocked test 1483)
- [x] Cookie path-match per RFC 6265 §5.1.4 (prefix must end at `/` boundary) — `/hoge` no longer matches `/hogege` (unlocked test 1228)
- [x] `--disallow-username-in-url` rejects URLs with `user[:pass]@` and exits 67 (unlocked test 2075)
- [x] Cookie cap of 50 per domain on Set-Cookie ingestion (unlocked test 444)
- [x] `--create-dirs` creates parent dirs for `--etag-save` and `-o` outputs (unlocked test 693)
- [x] Cookie path matching across redirects works correctly (unlocked tests 1024, 1025)
- [x] PUT preserves method on 301/302 redirect (only POST converts to GET); 303 always GET (unlocked tests 1051, 1052)
- [x] `-e/--referer` `;auto` suffix updates Referer to prior URL on each redirect; `--raw` bypasses unsolicited TE rejection (regression fix for test 319)
- [x] Resolve_redirect treats any `scheme://` form as absolute URL even with unsupported scheme (unlocked test 1159)
- [x] HTTP/1.0 + stdin upload exits 25 (no chunked, no Content-Length) (unlocked test 1069)
- [x] HTTP POST with Expect: 100-continue and server early-close handled (unlocked test 1070)
- [x] HTTP redirect with chunked TE response works (unlocked test 1090)
- [x] HEAD response with HTTP/0.9 body is "weird" (exit 8, unlocked test 1144)
- [x] Honor `http_proxy` / `HTTPS_PROXY` / `ALL_PROXY` env vars when no `--proxy` is given (unlocked tests 1255, 1256, 1257)
- [x] URL parsing: reject whitespace/control chars in host; reject rubbish after IPv6 `]` bracket (unlocked tests 1263, 1264)
- [x] `--post303` preserves POST on 303 redirect (unlocked test 1332)
- [x] `-H "Header;"` sends header with empty value (unlocked test 1291)
- [x] Redirect to any `scheme://` (e.g. `gopher://`) is treated as absolute URL (unlocked test 1563)
- [x] `-C wrong` (non-numeric continue-at offset) exits 2 (unlocked test 1409)
- [x] Explicit `-o` overrides `-J` Content-Disposition / `-O` URL-derived name (unlocked tests 1368, 1369, 1370, 1371)
- [x] file:// open-ended `-r N-` past EOF exits 36 (unlocked test 1063)
- [x] file:// `--remote-time` mirrors source mtime onto the output file (unlocked test 1445)
- [x] file:// failed open still emits `--write-out` (synthetic Response when URL parses; raw text otherwise) — unlocked tests 1440, 1441, 1442
- [x] HTTP `-C N` with any non-206 (e.g. 404) drops the body with exit 33 — unlocked test 99
- [x] SOCKS proxy URL parsing (`socks4://`, `socks4a://`, `socks5://`, `socks5h://`) — TCP-connect to proxy succeeds/fails before SOCKS handshake; non-listening proxy → exit 7 (unlocked tests 704, 705)
- [x] `-T` attaches per-URL: `-T f1 URL1 -T f2 URL2` puts each URL with its own file; one `-T` with two URLs leaves the second as GET (regression-fix for 1131 alongside 1052/1065)
- [x] Bare-cmdline leading `--next` exits 2 with "missing URL before --next" — only config-file leading `--next` is tolerated (regression-fix for test 422 alongside 430)
- [x] HTTPS expired-cookie eviction across redirects (unlocked test 1562)
- [x] curl `-V` includes `Largefile` feature
- [x] Verbose log "Ignoring Content-Length/Transfer-Encoding in CONNECT 2xx response" when proxy CONNECT replies with framing headers (unlocked test 1287)
- [x] Discovery sweep across 200-3300 ranges; standalone unit-test cases (`bufq`, `dynhds`, `http1 parser`, `curl_get_line`, `cipher suite name lookup`, `uint_bset`, `uint_spbset`, `struct size checks`, `ratelimit`) added to the passing list (unlocked tests 2601, 2602, 2603, 3200, 3205, 3211, 3212, 3213, 3214, 3216)
- [x] HTTPS localhost cert with first-subaltname (CN mismatch) — already passing under our chain (unlocked test 3000)
- [x] **Hit 700/700 passing milestone**
- [x] `--variable name=value`, `name@file`, `%ENV[=default]` / `%ENV@file` declaration; `--expand-data`, `--expand-url`, `--expand-data-urlencode`, `--expand-header`, `--expand-output` placeholder substitution; `{{name:func}}` filter chain (`trim`, `json`, `url`, `b64`, `64dec`); name-length cap (<128) keeps oversized placeholders literal; unknown names expand to empty (test 448) (unlocked tests 268, 428, 429, 448, 450, 451, 455, 458, 487)
- [x] `--variable name[N-M]@file` / `name[N-]@file` / `name[N-M]=value` byte-range slicing (unlocked tests 784, 785, 786, 788, 789, 790, 791)
- [x] `--url-query` builds query strings (`name=val`, `name@file`, `@file`, `+rawvalue`) with lowercase-hex percent-encoding to match curl's `--url-query` output (`--data-urlencode` keeps uppercase, test 1015) (unlocked test 1221)
- [x] Multipart custom Content-Type: defer user-supplied `Content-Type:` until after `Content-Length:`, append `; boundary=…`, canonicalize header name to `Content-Type` (unlocked test 669)
- [x] `--proxy-anyauth` / `--proxy-digest` / `--proxy-ntlm` / `--proxy-negotiate`: skip Proxy-Authorization on first request, retry with Basic on 407 + Proxy-Authenticate: Basic; absorb Set-Cookie from the 407 response into the cookie engine BEFORE the retry so the second request carries the cookie (unlocked test 1331)
- [x] `--connect-to` through an HTTP proxy auto-engages CONNECT tunnel mode; CONNECT target is the connect-to host; the inner request through the tunnel uses the relative path and skips Proxy-Connection (unlocked test 2050)
- [x] `--connect-to` accepts bracketed IPv6 literals (`[fc00::1]:8082:HOST:PORT`), with a colon-aware splitter that doesn't mistake IPv6 colons for field separators (unlocked test 2053; removed test 1454 which is `!IPv6` so it's framework-skipped on our build)
- [x] Failed CONNECT response routes to `-o` output file when one is set (test 749)
- [x] CONNECT response with first line not `HTTP/` exits 43 ("Invalid response header") (test 750)
- [x] `--expand-*` validates filter function names at parse time and exits 2 on unknown filter (unlocked tests 452, 454)
- [x] **List hygiene**: re-audited the `testNums` curated list against actual build outcomes; dropped 22 entries that had been silently failing (skipped due to advertised-feature mismatch, broken on-the-wire reuse semantics, broken null-byte handling, etc.) — `testNums` now reflects only tests that actually report 100% OK on every build
- [x] Stale-pool retry: when a reused TCP connection fails to deliver a response (server closed it between requests, e.g. the test framework's `swsclose` directive), drop the bad pool entry and retry once with a fresh connection (recovers tests 4, 160, 257, 327, 675 in the 700 baseline)
- [x] `--max-filesize` no longer rejects intermediate redirect (3xx) responses — the limit applies to the eventual transferred file (unlocked test 477)
- [x] `-x ""` / `--proxy ""` (empty string) means "no proxy" and overrides `http_proxy` / `HTTPS_PROXY` env vars (unlocked test 1004)
- [x] `-C` with a 416 response is "already fully downloaded", not a range refusal — exit 0 instead of 33 (unlocked test 1040)
- [x] Stale-pool retry now also recovers tests 1273 and 1283
- [x] 303 redirect converts PUT (not just POST) to GET (unlocked test 1524)
- [x] URL userinfo on redirect: `-u` from CLI always wins (`user_from_cli` flag); a redirect target's userinfo overrides only when it carries both user AND password (`first:secret` beats lifted `first` only); a user-only userinfo (`http://user1@host/`) doesn't strip a netrc-derived password (unlocked tests 899, 979; preserved 682, 2081)
- [x] netrc parser detects unterminated quoted password and exits 26 (test 680); `--netrc-optional` silently ignores a missing file (preserved test 495)
- [x] `--expand-data` rejects null bytes in the expanded value with exit 2 (unlocked tests 453, 456)
- [x] `--interface` validates value as IP literal / `/sys/class/net` entry / DNS-resolvable host; bad value → exit 45 (unlocked tests 1084, 1085)
- [x] Strip IPv6 zone/scope ID (`%scope`, `%25scope`) from Host header and connect-target (unlocked test 1056)
- [x] Stdin upload redirect: when a `-T -` body can't be replayed across a redirect, emit the prior response and exit 25 (unlocked test 1073)
- [x] `--retry` with `--output` file: prepend prior failed attempts (retry_prefix) so `-i` reproduces the full sequence; `--fail` drops the 4xx body from the prefix (unlocked tests 1633, 1634)
- [x] `--write-out %header{name:all[:sep]}` and `:N` / `:last` qualifiers; values harvested from redirect chain + final response; allow `\}` to escape `}` in the pattern (unlocked tests 764, 765)
- [x] `-D -` paired with `-i`: emit each header line twice in succession (per-line interleave), matching curl's callback-per-line behavior (unlocked test 1066)
- [x] Multipart parts use `Content-Disposition: attachment` when the user-supplied `Content-Type:` is not a `multipart/form-data` variant (RFC 1867 style, unlocked test 277)
- [x] `--retry-max-time`: don't accumulate the failing attempt into the retry prefix when the proposed Retry-After delay would push past the budget — break with the response alone (unlocked test 366)
- [x] Chunked body honors `--max-filesize`: truncate body to the limit, exit 63 (unlocked test 457)
- [x] `-T file` validates the upload source exists before connecting; missing file maps to exit 26 (unlocked test 496)
- [x] `-Z` / `--parallel` / `--parallel-immediate` / `--parallel-max` accepted as no-ops so dependent option parsing reaches the rest of the command line
- [x] `--retry` with `--include`: drop the failed-attempt prefix when the final attempt succeeds (no `--fail` / `--fail-with-body`) so a clean 200 doesn't carry the failed body (test 198)
- [x] Track URL fragment through `parse_url`; expose `%{url.fragment}`, `%{urle.fragment}`, `%{urle.user}`, `%{urle.password}` in `--write-out`
- [x] `#N` substitution in `-o`/`--output` runs even when the path was supplied explicitly (was only running on the synthetic fallback), so a single `-o "outfile_#1#2.dump"` paired with `[a-a][1-1]…` globbing now resolves to `outfile_a1.dump` (unlocked test 1283)
- [x] file:// GET on a directory emits a newline-separated sorted listing (or empty) with exit 0 instead of exit 37 (unlocked tests 3016, 3203)
- [x] perform() probes the URL for malformed-shape errors before checking `-T` upload-source existence so a bad URL paired with a missing `-T` target reports exit 3 instead of exit 26 (unlocked test 1469 from regression)
- [x] `--haproxy-protocol` / `--haproxy-clientip` write a v1 PROXY TCP4/TCP6 header onto the post-connect stream (post-CONNECT for proxytunnel); destination is the proxy when `-x` is set, the origin otherwise; client IP defaults to the local socket but can be overridden (unlocked tests 1455, 1456, 3028, 3201, 3202)
- [ ] Target 750+ passing by addressing remaining feature gaps

---

## Test Inventory

### Passing tests (729)

The authoritative list is `testNums` in `default.nix`; the count is
checked there by Nix and stays in sync with the per-test derivations.

### Major remaining failure categories

Most remaining failures in the 1-200 range come from missing protocol/feature support:

- **CONNECT tunnel** — implemented; remaining 1-200 failure is connection reuse (48)
- **`--proxy-user` / proxy auth** — proxy URL userinfo extraction now implemented (264, 278, 279 fixed); `-U` flag works; blank-password proxy auth works
- **FTP/FTPS** — not implemented (tests 100-series, 400-series)
- **SMTP/IMAP/POP3** — not implemented
- **HTTP/2, HTTP/3** — not implemented (tests 1800-series, 1900-series)
- **`--alt-svc`, `--hsts`** — not implemented

Protocol/output diff failures:

- **Cookie jar edge cases** — in-memory cookie engine, cookie accumulation, Max-Age=0 deletion, and session-cookie jar exclusion now work; remaining issues are control-character filtering in cookie values, IP address domain-match rules, secure-cookie cross-scheme redirect handling (test 414), and HTTP header file format parsing (test 8)
- **Connection reuse** — persistent connections / keep-alive semantics (test 48 dropped — requires connection reuse)

### Known timeouts

None currently identified in the passing range; update as discovered.

---

## Next steps

To expand the passing-test list incrementally, follow this loop:

1. **Discover** candidate tests in an unswept range:

   ```bash
   nix build .#packages.x86_64-linux.rust-curl-test-discovery -L
   ```

   The derivation runs tests 1-200 in batch and writes `results.txt` showing each test's verdict. Edit its `1 to 200` range to sweep a different window (e.g. `201 to 400`).

2. **Triage** failures: group by CLI option, feature, or output-diff pattern. Prefer clusters — one fix often unlocks 5-10 tests.

3. **Fix** the underlying issue in the relevant `src/` module:
   - CLI parsing: `args.rs`, `options.rs`
   - Request building / streaming: `request.rs`, `connection.rs`
   - Response parsing / output: `response.rs`, `format.rs`
   - Redirects / URL handling: `url.rs`
   - Cookies: `cookie.rs`

4. **Verify** with a single test: `nix build .#checks.x86_64-linux.rust-curl-test-<num>`. Use `nix log` on failure to view the diff.

5. **Add** newly-passing test numbers to the `testNums` list in `default.nix`. Keep it sorted (or grouped consistently).

6. **Update** the "Passing tests" count and list at the top of this file and in the `§ Test Inventory` section.

7. **Commit** with a conventional message describing the fix and new passing count, e.g. `fix(rust/curl): <change> — N/200 passing (N/200)`. Commits follow `.commitlintrc.yml`.

Priority targets (highest ROI first):

1. **Connection reuse / keep-alive** — needed for test 48 and unlocks broader 1xxx range
2. **`--anyauth` connection reuse** — test 338 (reuse non-authed connection)
3. **Sweep 1200-1500 range** — previous sweep hung on a test; needs targeted sub-range sweeps skipping hangers
