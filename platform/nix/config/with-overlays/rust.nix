# rust-overlay is optional: only zed's wasm toolchain wants rust-bin, so the
# evaluating tree declares the input (the monorepo root does) or pkgs simply
# has no rust-bin. Flakes cannot say this themselves - NixOS/nix#7205.
final: prev:
if prev.inputs ? rust-overlay
then (import prev.inputs.rust-overlay) final prev
else {}
