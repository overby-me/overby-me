build:
    cargo build --workspace

test:
    cargo test --workspace

clippy:
    cargo clippy --workspace

# Run differential tests comparing rust-systemd against real systemd
difftest *ARGS:
    cargo test --package difftest -- {{ARGS}}

# Run differential tests with full report output
difftest-report:
    DIFFTEST_JUNIT_PATH=result/difftest-results.xml \
    DIFFTEST_JSON_PATH=result/difftest-results.json \
    DIFFTEST_MARKDOWN_PATH=result/difftest-results.md \
    cargo test --package difftest -- --nocapture

# Update differential test snapshots (approve current outputs as golden)
difftest-update-snapshots:
    DIFFTEST_UPDATE_SNAPSHOTS=1 cargo test --package difftest

# List all registered differential tests
difftest-list:
    cargo test --package difftest -- --list

# Run the in-process differential parser oracles against the C systemd binaries
# (task #22/#21). Resolves systemd-journal-remote + journalctl from nixpkgs and
# runs the `differential_*` tests, which skip when those env vars are unset. This
# turns upstream parser drift into a failing test without a VM.
differential-parsers:
    #!/usr/bin/env bash
    set -euo pipefail
    nix build --no-link 'nixpkgs#systemd'
    sysd=$(nix eval --raw 'nixpkgs#systemd.outPath')
    echo "using C systemd at $sysd"
    SYSTEMD_JOURNAL_REMOTE="$sysd/lib/systemd/systemd-journal-remote" \
      JOURNALCTL="$sysd/bin/journalctl" \
      cargo test -p libsystemd differential_ -- --nocapture
