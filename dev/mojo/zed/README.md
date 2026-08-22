# Mojo Zed

A [Zed](https://zed.dev/) extension providing [Mojo](https://www.modular.com/mojo) language support with syntax highlighting and LSP integration.

Fork of [bajrangCoder/zed-mojo](https://github.com/bajrangCoder/zed-mojo), modified to look up `mojo-lsp-server` directly from PATH instead of depending on [pixi](https://pixi.sh/). This makes it compatible with Nix-based environments using [direnv](https://direnv.net/).

## Features

- Syntax highlighting via [tree-sitter-mojo](https://github.com/lsh/tree-sitter-mojo)
- Outlines and text objects
- Language Server Protocol (LSP) support

## How it works

The extension simply looks up `mojo-lsp-server` in PATH via `worktree.which()`. With Zed's `load_direnv` setting enabled, any Mojo installation provided by direnv/Nix will be picked up automatically.

Recommended Zed settings:

```json
{
  "load_direnv": "direct",
  "languages": {
    "Mojo": {
      "formatter": {
        "external": {
          "command": "mojo",
          "arguments": ["format", "-q", "-"]
        }
      },
      "format_on_save": "on"
    }
  }
}
```

## Project structure

```txt
zed-mojo/
├── src/
│   └── mojo.rs                    # Extension entry point — LSP binary lookup
├── languages/
│   └── dev/mojo/
│       ├── config.toml            # Language configuration (file types, brackets, indentation)
│       ├── highlights.scm         # Syntax highlighting queries
│       ├── brackets.scm           # Bracket matching queries
│       ├── indents.scm            # Auto-indentation queries
│       ├── outline.scm            # Symbol outline queries
│       ├── embedding.scm          # Embedding queries
│       ├── overrides.scm          # Scope override queries
│       └── textobjects.scm        # Text object queries (functions, classes, comments)
├── extension.toml                 # Extension metadata and grammar/LSP declarations
├── Cargo.toml                     # Rust project configuration
├── Cargo.lock                     # Dependency lock file
└── default.nix                    # Nix workspace module (package + dev shell)
```

## Installation

This extension is installed as a Zed dev extension via Home Manager. The activation script copies the source into `~/.local/share/zed/dev_extensions/mojo`, where Zed compiles it to WASM.

## Development

Enter the dev shell (requires [Nix](https://nixos.org/)):

```sh
nix develop .#zed-mojo
```

This provides a Rust toolchain with the `wasm32-wasip2` target needed to compile the extension.