build:
    cargo build --workspace

test:
    cargo test --workspace

check:
    cargo clippy --workspace -- -D warnings
    cargo fmt --check

fmt:
    cargo fmt --all

run *ARGS:
    cargo run -p tangled-spindle -- {{ARGS}}

clean:
    cargo clean
