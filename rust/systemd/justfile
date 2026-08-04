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

# Run the in-process differential oracles against the C systemd binaries
# (task #22/#21): compare rust-systemd against the same input through real
# systemd. Resolves the C binaries from nixpkgs and sets the env vars that gate
# each differential test (they skip when unset), turning upstream drift into a
# failing test without a VM.
differential:
    #!/usr/bin/env bash
    set -euo pipefail
    nix build --no-link 'nixpkgs#systemd'
    sysd=$(nix eval --raw 'nixpkgs#systemd.outPath')
    echo "using C systemd at $sysd"
    # journal export-format parser vs systemd-journal-remote
    SYSTEMD_JOURNAL_REMOTE="$sysd/lib/systemd/systemd-journal-remote" \
      JOURNALCTL="$sysd/bin/journalctl" \
      cargo test -p libsystemd differential_ -- --nocapture
    # unit-name escaping vs systemd-escape
    SYSTEMD_ESCAPE="$sysd/bin/systemd-escape" \
      cargo test -p systemd-escape --test differential_vs_c -- --nocapture
    # timespan / calendar parsing vs systemd-analyze
    SYSTEMD_ANALYZE="$sysd/bin/systemd-analyze" \
      cargo test -p systemd-analyze --test differential_vs_c -- --nocapture
    # well-known / GPT-partition-type ID table vs systemd-id128
    SYSTEMD_ID128="$sysd/bin/systemd-id128" \
      cargo test -p systemd-id128 --test differential_vs_c -- --nocapture
    # systemd-creds list exit-status (ENXIO) contract vs systemd-creds
    SYSTEMD_CREDS="$sysd/bin/systemd-creds" \
      cargo test -p systemd-creds --test differential_vs_c -- --nocapture
    # kernel-command-line → .network generation vs systemd-network-generator
    SYSTEMD_NETWORK_GENERATOR="$sysd/lib/systemd/systemd-network-generator" \
      cargo test -p systemd-network-generator --test differential_vs_c -- --nocapture
