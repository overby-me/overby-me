# rust-curl: Plan to Pass Upstream curl Tests

## Overview

Use the upstream [curl test suite](https://github.com/curl/curl/tree/master/tests) to validate the Rust curl rewrite against the real curl CLI behavior, following the same Nix-based testing pattern used by `rust/awk`.

## Current Status

**34/200 tests passing** (17%) — from the curl 8.18.0 test suite.

Passing: 1, 2, 3, 4, 7, 10, 11, 12, 13, 15, 19, 22, 23, 28, 34, 35, 40, 42, 47, 49, 50, 51, 52, 54, 55, 57, 97, 151, 152, 160, 164, 180, 181, 198

Test infrastructure is operational: `testsuite.nix` builds curl's C test servers from `pkgs.curl.src`, then runs `runtests.pl -c` against `rust-curl-dev`.

The Rust curl implementation supports: HTTP/HTTPS GET/POST/PUT, redirects, basic auth, cookies, TLS (rustls), multipart forms, verbose output, write-out formatting, retry logic, range requests, and file upload. No HTTP/2.

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
- No compression/decompression (`--compressed` parsed but not functional)
- No progress meter
- No connection reuse/pooling
- No DNS-over-HTTPS
- No alt-svc, HSTS

---

## Implementation Phases

### Phase 1: Infrastructure (est. +0 tests)

- [ ] Add `rust-curl-dev` package to `default.nix`
- [ ] Create `curl-test-servers` derivation or determine how to build test infra
- [ ] Create `testsuite.nix` that runs a single test via `runtests.pl -c`
- [ ] Verify localhost test servers work in Nix sandbox
- [ ] Run test 1 (simplest HTTP GET) as proof of concept

### Phase 2: Basic HTTP tests (est. ~30-50 tests)

- [ ] Enumerate all HTTP GET tool tests that don't need special features
- [ ] Add passing tests to `default.nix` checks
- [ ] Fix basic compatibility issues found (headers, exit codes, output format)

### Phase 3: POST/PUT and data handling (est. ~20-30 tests)

- [ ] Add HTTP POST tests (`-d`, `--data-raw`, `--data-binary`)
- [ ] Add HTTP PUT tests (`-T`)
- [ ] Add multipart form tests (`-F`)
- [ ] Fix any data encoding/boundary issues

### Phase 4: Redirects and auth (est. ~20-30 tests)

- [ ] Add redirect tests (`-L`, `--max-redirs`)
- [ ] Add basic auth tests (`-u`)
- [ ] Add cookie tests (`-b`, `-c`)

### Phase 5: Output and formatting (est. ~20 tests)

- [ ] Add write-out format tests (`-w`)
- [ ] Add verbose output tests (`-v`)
- [ ] Add header inclusion tests (`-i`, `-I`, `-D`)

### Phase 6: TLS/HTTPS (est. ~15-20 tests)

- [ ] Add HTTPS tests (requires stunnel in sandbox)
- [ ] Add client certificate tests (`--cert`, `--key`)
- [ ] Add CA bundle tests (`--cacert`)
- [ ] Add insecure mode tests (`-k`)

---

## Test Inventory

### Passing tests (34)

1, 2, 3, 4, 7, 10, 11, 12, 13, 15, 19, 22, 23, 28, 34, 35, 40, 42, 47, 49, 50, 51, 52, 54, 55, 57, 97, 151, 152, 160, 164, 180, 181, 198

### Major failure categories (from tests 1-200)

Most failures (exit code 2) are due to unrecognized CLI options used by the test suite:

- **`-x` / `--proxy`** — proxy support (tests 5, 30, 84, 85, ...)
- **`-K` / `--config`** — config file from stdin (tests 56, 71, ...)
- **`--resolve`** — custom DNS resolution (tests 46, ...)
- **`--proxy-user`** — proxy auth (test 85, ...)

Protocol/output diff failures:

- **URL normalization** — `../../` in redirect paths not resolved (test 50)
- **Redirect `-i` output** — intermediate responses not shown (test 55)
- **Cookie handling** — cookie send/receive/jar (tests 6, 7, 8, 24, 25, ...)
- **Auth headers** — basic auth encoding (tests 2, 3, 15, ...)

### Timeouts

14, 24 (likely redirect loops or server interaction issues)
