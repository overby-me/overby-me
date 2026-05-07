# rust-curl: Plan to Pass Upstream curl Tests

## Overview

Use the upstream [curl test suite](https://github.com/curl/curl/tree/master/tests) to validate the Rust curl rewrite against the real curl CLI behavior, following the same Nix-based testing pattern used by `rust/awk`.

## Current Status

**569 tests passing** across the curl 8.19.0 test suite (verified with strict
runner checks — the derivation fails when a test number doesn't exist or the
suite reports anything other than 100% OK).

Passing: 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 66, 71, 73, 74, 75, 77, 78, 80, 82, 83, 84, 85, 86, 87, 92, 93, 94, 95, 97, 98, 129, 151, 152, 156, 157, 158, 160, 163, 164, 166, 171, 172, 173, 174, 178, 179, 180, 181, 183, 184, 185, 186, 187, 188, 189, 192, 193, 194, 197, 198, 199, 207, 214, 218, 219, 220, 221, 222, 224, 230, 232, 233, 234, 249, 256, 260, 262, 264, 266, 269, 274, 276, 278, 279, 281, 282, 292, 293, 300, 301, 302, 303, 304, 305, 306, 309, 310, 317, 318, 319, 325, 326, 327, 328, 329, 330, 331, 333, 334, 339, 341, 342, 343, 344, 345, 346, 347, 349, 357, 360, 361, 364, 365, 366, 367, 368, 370, 371, 372, 373, 374, 376, 378, 379, 383, 384, 385, 386, 387, 389, 391, 392, 393, 394, 395, 398, 399, 410, 411, 415, 418, 419, 420, 421, 422, 425, 426, 434, 443, 444, 449, 452, 453, 454, 456, 460, 461, 462, 463, 467, 468, 469, 470, 473, 477, 481, 482, 484, 485, 497, 498, 499, 518, 537, 662, 663, 675, 678, 681, 686, 690, 691, 692, 693, 697, 708, 722, 723, 724, 743, 746, 747, 752, 759, 767, 768, 769, 770, 771, 772, 773, 787, 794, 796, 797, 798, 898, 899, 977, 978, 979, 990, 991, 994, 995, 996, 998, 999, 1004, 1011, 1012, 1015, 1024, 1025, 1027, 1029, 1031, 1032, 1033, 1040, 1041, 1042, 1043, 1051, 1052, 1053, 1054, 1058, 1064, 1068, 1069, 1070, 1076, 1080, 1081, 1089, 1090, 1101, 1104, 1105, 1109, 1110, 1111, 1115, 1116, 1117, 1118, 1121, 1122, 1123, 1124, 1125, 1126, 1127, 1128, 1129, 1130, 1131, 1138, 1141, 1143, 1144, 1147, 1150, 1151, 1155, 1157, 1159, 1160, 1161, 1164, 1166, 1168, 1169, 1170, 1172, 1174, 1175, 1176, 1178, 1179, 1180, 1181, 1182, 1183, 1184, 1188, 1197, 1200, 1201, 1202, 1205, 1210, 1213, 1214, 1216, 1218, 1223, 1228, 1231, 1232, 1234, 1235, 1236, 1237, 1240, 1241, 1246, 1247, 1248, 1249, 1251, 1252, 1253, 1254, 1255, 1256, 1257, 1258, 1259, 1260, 1261, 1263, 1264, 1266, 1267, 1268, 1269, 1270, 1271, 1272, 1273, 1274, 1275, 1276, 1278, 1280, 1281, 1283, 1289, 1290, 1291, 1292, 1296, 1297, 1298, 1299, 1300, 1302, 1303, 1304, 1305, 1306, 1309, 1310, 1311, 1312, 1313, 1314, 1317, 1318, 1322, 1323, 1325, 1332, 1333, 1334, 1335, 1336, 1337, 1338, 1339, 1340, 1341, 1342, 1343, 1344, 1345, 1346, 1347, 1364, 1365, 1366, 1367, 1368, 1369, 1370, 1371, 1372, 1373, 1374, 1375, 1376, 1377, 1395, 1396, 1397, 1398, 1399, 1409, 1410, 1411, 1413, 1416, 1417, 1424, 1427, 1429, 1430, 1431, 1432, 1433, 1434, 1438, 1439, 1443, 1457, 1462, 1466, 1471, 1472, 1473, 1474, 1475, 1480, 1483, 1484, 1487, 1489, 1493, 1494, 1495, 1496, 1497, 1524, 1544, 1546, 1563, 1584, 1585, 1601, 1602, 1603, 1605, 1606, 1607, 1608, 1609, 1610, 1611, 1612, 1613, 1614, 1615, 1616, 1620, 1635, 1636, 1650, 1651, 1652, 1653, 1655, 1656, 1657, 1658, 1661, 1663, 1664, 1665, 1670, 1671, 1680, 1681, 1682, 1683, 1709, 1909, 1979, 1980, 2075, 2080, 2088

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
- [ ] Target 600+ passing by addressing remaining feature gaps

---

## Test Inventory

### Passing tests (569)

1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 66, 71, 73, 74, 75, 77, 78, 80, 82, 83, 84, 85, 86, 87, 92, 93, 94, 95, 97, 98, 129, 151, 152, 156, 157, 158, 160, 163, 164, 166, 171, 172, 173, 174, 178, 179, 180, 181, 183, 184, 185, 186, 187, 188, 189, 192, 193, 194, 197, 198, 199, 207, 214, 218, 219, 220, 221, 222, 224, 230, 232, 233, 234, 249, 256, 260, 262, 264, 266, 269, 274, 276, 278, 279, 281, 282, 292, 293, 300, 301, 302, 303, 304, 305, 306, 309, 310, 317, 318, 319, 325, 326, 327, 328, 329, 330, 331, 333, 334, 339, 341, 342, 343, 344, 345, 346, 347, 349, 357, 360, 361, 364, 365, 366, 367, 368, 370, 371, 372, 373, 374, 376, 378, 379, 383, 384, 385, 386, 387, 389, 391, 392, 393, 394, 395, 398, 399, 410, 411, 415, 418, 419, 420, 421, 422, 425, 426, 434, 443, 444, 449, 452, 453, 454, 456, 460, 461, 462, 463, 467, 468, 469, 470, 473, 477, 481, 482, 484, 485, 497, 498, 499, 518, 537, 662, 663, 675, 678, 681, 686, 690, 691, 692, 693, 697, 708, 722, 723, 724, 743, 746, 747, 752, 759, 767, 768, 769, 770, 771, 772, 773, 787, 794, 796, 797, 798, 898, 899, 977, 978, 979, 990, 991, 994, 995, 996, 998, 999, 1004, 1011, 1012, 1015, 1024, 1025, 1027, 1029, 1031, 1032, 1033, 1040, 1041, 1042, 1043, 1051, 1052, 1053, 1054, 1058, 1064, 1068, 1069, 1070, 1076, 1080, 1081, 1089, 1090, 1101, 1104, 1105, 1109, 1110, 1111, 1115, 1116, 1117, 1118, 1121, 1122, 1123, 1124, 1125, 1126, 1127, 1128, 1129, 1130, 1131, 1138, 1141, 1143, 1144, 1147, 1150, 1151, 1155, 1157, 1159, 1160, 1161, 1164, 1166, 1168, 1169, 1170, 1172, 1174, 1175, 1176, 1178, 1179, 1180, 1181, 1182, 1183, 1184, 1188, 1197, 1200, 1201, 1202, 1205, 1210, 1213, 1214, 1216, 1218, 1223, 1228, 1231, 1232, 1234, 1235, 1236, 1237, 1240, 1241, 1246, 1247, 1248, 1249, 1251, 1252, 1253, 1254, 1255, 1256, 1257, 1258, 1259, 1260, 1261, 1263, 1264, 1266, 1267, 1268, 1269, 1270, 1271, 1272, 1273, 1274, 1275, 1276, 1278, 1280, 1281, 1283, 1289, 1290, 1291, 1292, 1296, 1297, 1298, 1299, 1300, 1302, 1303, 1304, 1305, 1306, 1309, 1310, 1311, 1312, 1313, 1314, 1317, 1318, 1322, 1323, 1325, 1332, 1333, 1334, 1335, 1336, 1337, 1338, 1339, 1340, 1341, 1342, 1343, 1344, 1345, 1346, 1347, 1364, 1365, 1366, 1367, 1368, 1369, 1370, 1371, 1372, 1373, 1374, 1375, 1376, 1377, 1395, 1396, 1397, 1398, 1399, 1409, 1410, 1411, 1413, 1416, 1417, 1424, 1427, 1429, 1430, 1431, 1432, 1433, 1434, 1438, 1439, 1443, 1457, 1462, 1466, 1471, 1472, 1473, 1474, 1475, 1480, 1483, 1484, 1487, 1489, 1493, 1494, 1495, 1496, 1497, 1524, 1544, 1546, 1563, 1584, 1585, 1601, 1602, 1603, 1605, 1606, 1607, 1608, 1609, 1610, 1611, 1612, 1613, 1614, 1615, 1616, 1620, 1635, 1636, 1650, 1651, 1652, 1653, 1655, 1656, 1657, 1658, 1661, 1663, 1664, 1665, 1670, 1671, 1680, 1681, 1682, 1683, 1709, 1909, 1979, 1980, 2075, 2080, 2088

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
