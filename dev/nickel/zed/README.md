# nickel-zed

A [Zed](https://zed.dev/) extension providing [Nickel](https://nickel-lang.org/)
language support: syntax highlighting from
[tree-sitter-nickel](https://github.com/nickel-lang/tree-sitter-nickel), and LSP
integration through `nls`, the Nickel Language Server.

Based on [norpadon/zed-nickel-extension](https://github.com/norpadon/zed-nickel-extension).

## Language server

The extension resolves `nls` in this order:

1. `lsp.nls.binary.path` in your Zed settings, if set
2. `nls` on `PATH`

Looking it up on `PATH` rather than downloading a binary is what makes it work
in a Nix devshell, where `nls` is already provided. If neither is found, Zed
reports that `nls` is missing and points at the setting.

## Building

```sh
nix build .#zedExtensions.nickel-zed     # or: cargo build --release
```

The grammar revision is pinned in `extension.toml`, so a grammar update is a
deliberate change rather than something that drifts.
