# rust-curl: Plan to Pass Upstream curl Tests

## Overview

Use the upstream [curl test suite](https://github.com/curl/curl/tree/master/tests) to validate the Rust curl rewrite against the real curl CLI behavior, following the same Nix-based testing pattern used by `rust/awk`.

## Current Status

**418 tests passing** across the curl 8.19.0 test suite (verified with strict
runner checks — the derivation fails when a test number doesn't exist or the
suite reports anything other than 100% OK).

Passing: 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 66, 71, 73, 74, 75, 77, 78, 80, 82, 83, 84, 85, 86, 87, 92, 93, 94, 95, 97, 98, 129, 151, 152, 156, 157, 158, 160, 163, 164, 166, 171, 172, 173, 174, 178, 179, 180, 181, 183, 184, 185, 186, 187, 188, 189, 192, 193, 194, 197, 198, 199, 207, 214, 218, 219, 220, 221, 222, 224, 230, 232, 233, 234, 249, 256, 260, 262, 274, 276, 281, 282, 292, 293, 300, 301, 302, 303, 304, 306, 309, 310, 317, 318, 319, 326, 328, 331, 333, 334, 339, 341, 342, 343, 344, 345, 347, 349, 360, 361, 364, 365, 366, 367, 368, 370, 371, 372, 373, 374, 376, 378, 379, 383, 384, 385, 387, 391, 392, 393, 394, 395, 398, 410, 415, 418, 419, 421, 425, 426, 434, 443, 449, 452, 453, 454, 456, 460, 461, 462, 463, 467, 468, 469, 470, 473, 477, 481, 482, 485, 497, 498, 499, 518, 537, 662, 663, 675, 678, 686, 690, 691, 692, 697, 708, 722, 723, 724, 743, 752, 767, 768, 769, 773, 787, 899, 978, 979, 990, 991, 994, 995, 996, 998, 999, 1004, 1011, 1015, 1027, 1029, 1031, 1032, 1033, 1040, 1041, 1042, 1043, 1053, 1058, 1064, 1068, 1080, 1081, 1089, 1101, 1109, 1110, 1111, 1115, 1117, 1118, 1121, 1122, 1123, 1126, 1127, 1128, 1143, 1147, 1150, 1155, 1157, 1161, 1164, 1166, 1168, 1169, 1174, 1175, 1176, 1178, 1182, 1183, 1184, 1197, 1200, 1201, 1202, 1205, 1210, 1213, 1214, 1216, 1218, 1223, 1231, 1232, 1235, 1237, 1240, 1241, 1246, 1249, 1251, 1258, 1259, 1261, 1266, 1267, 1268, 1269, 1270, 1271, 1272, 1273, 1275, 1276, 1280, 1283, 1290, 1292, 1296, 1297, 1298, 1299, 1300, 1302, 1303, 1304, 1305, 1306, 1309, 1311, 1317, 1318, 1322, 1323, 1325, 1334, 1336, 1337, 1338, 1339, 1340, 1341, 1342, 1343, 1344, 1345, 1346, 1347, 1364, 1365, 1366, 1367, 1372, 1373, 1374, 1375, 1376, 1377, 1395, 1396, 1397, 1398, 1399, 1411, 1413, 1416, 1424, 1429, 1433, 1434, 1438, 1439, 1457, 1466, 1471, 1472, 1473, 1475, 1484, 1487, 1489, 1494, 1497, 1524, 1544, 1584, 1585, 1601, 1602, 1603, 1605, 1606, 1607, 1608, 1609, 1610, 1611, 1612, 1614, 1615, 1616, 1620, 1635, 1636, 1650, 1651, 1652, 1653, 1655, 1656, 1657, 1658, 1661, 1663, 1664, 1665, 1670, 1671, 1680, 1681, 1682, 1683, 1979, 1980, 2080

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
- [ ] Target 350+ passing by addressing one feature gap at a time

---

## Test Inventory

### Passing tests (439)

1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 66, 71, 73, 74, 75, 77, 78, 80, 82, 83, 84, 85, 86, 87, 92, 93, 94, 95, 97, 98, 129, 151, 152, 156, 157, 158, 160, 163, 164, 166, 171, 172, 173, 174, 178, 179, 180, 181, 183, 184, 185, 186, 187, 188, 189, 192, 193, 194, 197, 198, 199, 207, 214, 218, 219, 220, 221, 222, 224, 230, 232, 233, 234, 249, 256, 260, 262, 274, 276, 281, 282, 292, 293, 300, 301, 302, 303, 304, 306, 309, 310, 317, 318, 319, 326, 328, 331, 333, 334, 339, 341, 342, 343, 344, 345, 347, 349, 360, 361, 364, 365, 366, 367, 368, 370, 371, 372, 373, 374, 376, 378, 379, 383, 384, 385, 387, 391, 392, 393, 394, 395, 398, 410, 415, 418, 419, 421, 425, 426, 434, 443, 449, 452, 453, 454, 456, 460, 461, 462, 463, 467, 468, 469, 470, 473, 477, 481, 482, 485, 497, 498, 499, 518, 537, 662, 663, 675, 678, 686, 690, 691, 692, 697, 708, 722, 723, 724, 743, 752, 767, 768, 769, 773, 787, 899, 978, 979, 990, 991, 994, 995, 996, 998, 999, 1004, 1011, 1015, 1027, 1029, 1031, 1032, 1033, 1040, 1041, 1042, 1043, 1053, 1058, 1064, 1068, 1080, 1081, 1089, 1101, 1109, 1110, 1111, 1115, 1117, 1118, 1121, 1122, 1123, 1126, 1127, 1128, 1143, 1147, 1150, 1155, 1157, 1161, 1164, 1166, 1168, 1169, 1174, 1175, 1176, 1178, 1182, 1183, 1184, 1197, 1200, 1201, 1202, 1205, 1210, 1213, 1214, 1216, 1218, 1223, 1231, 1232, 1235, 1237, 1240, 1241, 1246, 1249, 1251, 1258, 1259, 1261, 1266, 1267, 1268, 1269, 1270, 1271, 1272, 1273, 1275, 1276, 1280, 1283, 1290, 1292, 1296, 1297, 1298, 1299, 1300, 1302, 1303, 1304, 1305, 1306, 1309, 1311, 1317, 1318, 1322, 1323, 1325, 1334, 1336, 1337, 1338, 1339, 1340, 1341, 1342, 1343, 1344, 1345, 1346, 1347, 1364, 1365, 1366, 1367, 1372, 1373, 1374, 1375, 1376, 1377, 1395, 1396, 1397, 1398, 1399, 1411, 1413, 1416, 1424, 1429, 1433, 1434, 1438, 1439, 1457, 1466, 1471, 1472, 1473, 1475, 1484, 1487, 1489, 1494, 1497, 1524, 1544, 1584, 1585, 1601, 1602, 1603, 1605, 1606, 1607, 1608, 1609, 1610, 1611, 1612, 1614, 1615, 1616, 1620, 1635, 1636, 1650, 1651, 1652, 1653, 1655, 1656, 1657, 1658, 1661, 1663, 1664, 1665, 1670, 1671, 1680, 1681, 1682, 1683, 1979, 1980, 2080

### Major remaining failure categories

Most remaining failures in the 1-200 range come from missing protocol/feature support:

- **CONNECT tunnel** — implemented; remaining 1-200 failure is connection reuse (48)
- **`--proxy-user` / proxy auth** — tests 85, ...
- **FTP/FTPS** — not implemented (tests 100-series, 400-series)
- **SMTP/IMAP/POP3** — not implemented
- **HTTP/2, HTTP/3** — not implemented (tests 1800-series, 1900-series)
- **`--alt-svc`, `--hsts`** — not implemented

Protocol/output diff failures:

- **Cookie jar edge cases** — in-memory cookie engine and cookie accumulation across URLs now work; remaining issues are control-character filtering in cookie values, IP address domain-match rules, and HTTP header file format parsing (test 8)
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
2. **Sweep 700-900 and 1500-2000 windows** — use the discovery derivation to find newly-passing tests after the cookie engine rewrite
