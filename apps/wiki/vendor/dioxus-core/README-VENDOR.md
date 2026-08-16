# Vendored dioxus-core

`dioxus-core` 0.7.9, unpacked from crates.io (MIT OR Apache-2.0, DioxusLabs;
`README.md` and `docs/` are upstream's and stay). Wired in by
`[patch.crates-io]` in `../../Cargo.toml`, so every crate in the tree that asks
for `dioxus-core` gets this copy.

`tests/` was dropped: the packaged manifest declares no dev-dependencies, so
those files cannot compile outside the upstream workspace.

## The one change

`Runtime::handle_event` (`src/runtime.rs`) took a shared borrow of
`runtime.elements` and held it across the listener call:

```rust
let elements = self.elements.borrow();
if let Some(Some(parent_path)) = elements.get(element.0).copied() {
    self.handle_bubbling_event(parent_path, name, event);
```

`elements` is borrowed mutably by `VirtualDom::next_element`, which every node
creation goes through. So a listener that caused a render panicked with
`RefCell already borrowed` at `arena.rs:59` and, under `panic = "abort"`, took
the whole app down. It reached us from `?app=speak`: tapping the join FAB on
iOS killed the page, and that screen re-renders every second, so a render was
nearly always pending when a tap landed.

The fix reads the path out and drops the borrow before calling user code. It is
what `handle_bubbling_event` already does, twenty lines below, for the sibling
`mounts` borrow:

> We do this in its own block to prevent mounts from staying open while we call
> user code

Still unfixed upstream in 0.8.0-alpha.0 and on `main`, so an upgrade does not
remove this. Sent as [DioxusLabs/dioxus#5729](https://github.com/DioxusLabs/dioxus/pull/5729).

## On upgrading dioxus

Re-vendor from the new crates.io release, drop `tests/`, and re-apply the
change (or drop the `[patch.crates-io]` entry entirely once upstream ships it).
