# rust-pipewire

A from-scratch Rust reimplementation of [PipeWire](https://pipewire.org/) —
the multimedia (audio + video) graph daemon and its surrounding CLI tools —
aimed at passing the upstream PipeWire test suite.

## Status

- **75/79 Nix-level tests passing** (the 4 still-failing SMF tests have a
  null-byte / shell-quoting issue in their fixtures, unrelated to the
  parser)
- **39 internal Rust unit tests passing** (POD round-trip, SMF parsing,
  protocol framing, dict encode/decode, JSON formatting, decoders for
  every Info-event type)
- **18/18 byte-identical POD encode comparisons** vs the C `spa_pod_builder_*`
  API
- Native protocol client round-trips with a real C `pipewire` daemon
- `pw-cli list-objects` and `pw-cli info` produce byte-identical output to
  the upstream C `pw-cli` against the same daemon, for Core / Module /
  Factory / Client / Node / Port / Device, per-interface registry walks,
  and `info all`
- `pw-dump` emits structurally-correct JSON for the registry contents
- `spa-json-dump`, `pw-mididump`, `pw-config paths` produce output identical
  to the upstream tools on every test fixture

A multicall binary dispatches on `argv[0]` (or `argv[1]` for
`rust-pipewire <tool>`) so the same Rust program serves all the symlinked
tool names under `bin/` (`pipewire`, `pw-cli`, `spa-json-dump`, ...).

See [`PLAN.md`](./PLAN.md) for the phased roadmap and current milestone.

## Architecture

### 1. Custom comparison tests (`testsuite.nix`)

A Nix derivation that runs a single named test, comparing `rust-pipewire`
output against reference `pkgs.pipewire` output. Takes
`{ pkgs, tool, name }` as arguments. Each test is a shell script in
`tests/${tool}/${name}.sh` that uses `$REF`, `$RUST`, `$TMPDIR`, and the
`compare` helper.

### 2. POD comparison tests (`pod-testsuite.nix`)

Compiles a small libspa-linked C helper inside the test sandbox, encodes
a set of named sample values via `spa_pod_builder_*`, and `cmp`s the
bytes against rust-pipewire's encoder. This is the closest thing to
"upstream tests" — the encoder is verified bit-exact against libspa.

### 3. Daemon-interop tests (`proto-testsuite.nix`)

Spawns a real C `pipewire` daemon in a private XDG_RUNTIME_DIR sandbox,
then connects with rust-pipewire's protocol-native client and verifies
the expected handshake (`Core.Hello` → `Core.Info` + `Registry.Global`
events).

### 4. Daemon-comparison tests (`daemon-testsuite.nix`)

Spawns a real C `pipewire` daemon, then runs both the C reference tool
(`pkgs.pipewire/bin/pw-cli`, `pw-dump`, ...) and rust-pipewire's tool
against it and `diff`s the output. This is the M7-style end-to-end
verification of the protocol-native client side: every byte of every
`Registry.Global` and `*.Info` event has to round-trip correctly.

### 5. `default.nix`

- `rust-pipewire` package — release multicall binary with symlinks for
  every tool name in `postInstall`
- `rust-pipewire-dev` package — debug build for fast iteration
- `checks` attribute set wiring per-test custom tests

## Running the tests

```sh
# Run a single comparison test
nix build .#checks.x86_64-linux.rust-pipewire-test-pw-cli-help
nix build .#checks.x86_64-linux.rust-pipewire-test-spa-json-dump-conf-jack
nix build .#checks.x86_64-linux.rust-pipewire-test-pw-mididump-tempo-meta

# POD encoder vs libspa
nix build .#checks.x86_64-linux.rust-pipewire-pod-test-encode-cases

# Daemon round-trip
nix build .#checks.x86_64-linux.rust-pipewire-proto-test-hello-info

# Daemon-comparison: same daemon, both pw-cli binaries, diff output
nix build .#checks.x86_64-linux.rust-pipewire-daemon-test-pw-cli-info-all
nix build .#checks.x86_64-linux.rust-pipewire-daemon-test-pw-cli-ls-module

# Run everything
nix flake check
```

## Workflow

1. Pick the next failing test (`grep "^FAIL:" log`).
2. Reproduce locally:
   `nix build .#checks.x86_64-linux.rust-pipewire-test-<tool>-<name>`.
3. Read the failure under `nix log <drv>` and compare against the
   expected reference output.
4. Fix the code, rebuild, re-run the check.
5. Commit and push the `pipewire-rust` bookmark.
