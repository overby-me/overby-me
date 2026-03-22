build:
    cargo build

build-release:
    cargo build --release

test:
    cargo test

check:
    cargo check

clippy:
    cargo clippy -- -D warnings

fmt:
    cargo fmt

fmt-check:
    cargo fmt -- --check

doc:
    cargo doc --no-deps --open

clean:
    cargo clean

# Transcode an H.264 file to AV1
transcode input output="" speed="6" quantizer="100":
    cargo run --release -- {{ input }} -o {{ if output != "" { output } else { input + ".av1.ivf" } }} -s {{ speed }} -q {{ quantizer }}

# Transcode with verbose logging
transcode-verbose input output="" speed="6" quantizer="100":
    cargo run --release -- {{ input }} -o {{ if output != "" { output } else { input + ".av1.ivf" } }} -s {{ speed }} -q {{ quantizer }} -v

# Quick transcode with fastest speed preset
transcode-fast input output="":
    cargo run --release -- {{ input }} -o {{ if output != "" { output } else { input + ".av1.ivf" } }} -s 10 -q 128

# High quality transcode
transcode-hq input output="":
    cargo run --release -- {{ input }} -o {{ if output != "" { output } else { input + ".av1.ivf" } }} -s 3 -q 60

all: fmt-check clippy test build
