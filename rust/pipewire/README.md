# rust-pipewire

A from-scratch Rust reimplementation of [PipeWire](https://pipewire.org/) —
the multimedia (audio + video) graph daemon and its surrounding CLI tools —
aimed at passing the upstream PipeWire test suite.

## Status

Scaffolding only — see [`PLAN.md`](./PLAN.md) for the phased roadmap.

A multicall binary dispatches on `argv[0]` (or `argv[1]` for
`rust-pipewire <tool>`) so the same Rust program serves all the symlinked
tool names under `bin/` (`pipewire`, `pw-cli`, `spa-json-dump`, ...).

## Architecture

### 1. Custom comparison tests (`testsuite.nix`)

A Nix derivation that runs a single named test, comparing `rust-pipewire`
output against reference `pkgs.pipewire` output. Takes
`{ pkgs, tool, name }` as arguments. Each test is a shell script in
`tests/${tool}/${name}.sh` that uses `$REF`, `$RUST`, `$TMPDIR`, and the
`compare` helper.

### 2. `default.nix`

- `rust-pipewire` package — release multicall binary with symlinks for
  every tool name in `postInstall`
- `rust-pipewire-dev` package — debug build for fast iteration
- `checks` attribute set wiring per-test custom tests

## Running the tests

```sh
# Run a single test
nix build .#checks.x86_64-linux.rust-pipewire-test-pw-cli-help
nix build .#checks.x86_64-linux.rust-pipewire-test-spa-json-dump-help
```

## Workflow

1. Pick the next failing test (`grep "^FAIL:" log`).
2. Reproduce locally:
   `nix build .#checks.x86_64-linux.rust-pipewire-test-<tool>-<name>`.
3. Read the failure under `nix log <drv>` and compare against the
   expected reference output.
4. Fix the code, rebuild, re-run the check.
5. Commit and push the `pipewire-rust` bookmark.
