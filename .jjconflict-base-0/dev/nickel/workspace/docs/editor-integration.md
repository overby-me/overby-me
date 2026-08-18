# Editor Integration

nix-workspace leverages the [Nickel Language Server (NLS)](https://github.com/tweag/nickel/tree/master/lsp) to provide real-time diagnostics, completion, and hover documentation directly in your editor. This guide covers setup for popular editors and how to get the most out of the integration.

## Table of Contents

- [Prerequisites](#prerequisites)
- [How it works](#how-it-works)
- [VS Code](#vs-code)
- [Neovim](#neovim)
- [Helix](#helix)
- [Zed](#zed)
- [Emacs](#emacs)
- [Diagnostics mapping](#diagnostics-mapping)
- [Tips and tricks](#tips-and-tricks)
- [Troubleshooting](#troubleshooting)

---

## Prerequisites

1. **Nickel CLI** — Install via `nix profile install nixpkgs#nickel` or include it in your dev shell.
2. **Nickel Language Server (NLS)** — Install via `nix profile install nixpkgs#nls` or include it in your dev shell.

If you're using nix-workspace's own dev shell, both are already available:

```bash
nix develop github:example/nix-workspace
# or, if you have direnv:
direnv allow
```

Verify installation:

```bash
nickel --version
nls --version
```

---

## How it works

nix-workspace configuration files (`.ncl`) are standard Nickel files. The Nickel Language Server provides:

| Feature | Description |
|---------|-------------|
| **Diagnostics** | Real-time contract violations, type errors, and parse errors shown as editor squiggles |
| **Hover** | Documentation for fields (from `\| doc` annotations) shown on hover |
| **Completion** | Field name completion inside records with known contracts |
| **Go to definition** | Navigate to contract definitions in `contracts/*.ncl` |
| **Formatting** | `nickel format` integration for consistent code style |

When you edit `workspace.ncl`, `packages/my-tool.ncl`, or any other `.ncl` file, the LSP evaluates the Nickel contracts and surfaces errors immediately — before you ever run `nix build`.

### Contract awareness

Because nix-workspace `.ncl` files apply contracts (e.g. `| WorkspaceConfig`), the LSP knows the expected shape of every field. This means:

- Typing `sys` inside a workspace record suggests `systems`.
- Hovering over `build-system` shows the valid values (`"rust"`, `"go"`, `"generic"`).
- Entering an invalid system string shows an inline error with the "did you mean?" hint.

---

## VS Code

### Installation

1. Install the Nickel extension from the VS Code marketplace:
   - Open VS Code → Extensions → Search "Nickel" → Install

2. Ensure `nls` is on your `$PATH`. If you're using direnv or `nix develop`, VS Code should pick it up automatically when launched from the terminal:

   ```bash
   cd my-workspace
   code .
   ```

### Configuration

Add to your `.vscode/settings.json` for workspace-specific settings:

```json
{
  "nickel.server.path": "nls",
  "nickel.format.enable": true,
  "[nickel]": {
    "editor.formatOnSave": true
  }
}
```

If `nls` is in a non-standard location (e.g. a Nix store path):

```json
{
  "nickel.server.path": "/nix/store/...-nls/bin/nls"
}
```

### Usage

- Open any `.ncl` file — diagnostics appear automatically.
- Hover over fields to see documentation.
- Use `Ctrl+Space` for completions.
- Errors show as red squiggles with full contract violation details in the Problems panel.

---

## Neovim

### With nvim-lspconfig

The easiest setup uses [nvim-lspconfig](https://github.com/neovim/nvim-lspconfig), which has built-in support for NLS.

```lua
-- In your Neovim config (init.lua or lua/plugins/lsp.lua)
local lspconfig = require('lspconfig')

lspconfig.nickel_ls.setup({
  -- Optional: customize the command if nls isn't on $PATH
  -- cmd = { "/path/to/nls" },

  on_attach = function(client, bufnr)
    -- Standard LSP keymaps
    local opts = { buffer = bufnr, noremap = true, silent = true }
    vim.keymap.set('n', 'gd', vim.lsp.buf.definition, opts)
    vim.keymap.set('n', 'K', vim.lsp.buf.hover, opts)
    vim.keymap.set('n', '<leader>e', vim.diagnostic.open_float, opts)
    vim.keymap.set('n', '[d', vim.diagnostic.goto_prev, opts)
    vim.keymap.set('n', ']d', vim.diagnostic.goto_next, opts)
  end,

  -- File patterns to activate on
  filetypes = { "nickel" },
})
```

### With lazy.nvim

If you manage plugins with [lazy.nvim](https://github.com/folke/lazy.nvim):

```lua
{
  "neovim/nvim-lspconfig",
  config = function()
    require('lspconfig').nickel_ls.setup({})
  end,
}
```

---

## Helix

[Helix](https://helix-editor.com/) has built-in LSP support and ships with a Nickel language definition.

### Configuration

Add to `~/.config/helix/languages.toml`:

```toml
[[language]]
name = "nickel"
language-servers = ["nickel-ls"]

[language-server.nickel-ls]
command = "nls"
```

Helix will automatically start `nls` when you open a `.ncl` file.

### Usage

- `Space + k` — Hover documentation
- `gd` — Go to definition
- `Space + d` — Show diagnostics
- `]d` / `[d` — Next/previous diagnostic

---

## Zed

[Zed](https://zed.dev/) supports LSP servers natively.

### Configuration

Zed discovers `nls` automatically if it's on your `$PATH`. For explicit configuration, add to your Zed settings (`~/.config/zed/settings.json`):

```json
{
  "lsp": {
    "nickel-ls": {
      "binary": {
        "path": "nls"
      }
    }
  }
}
```

### Usage

Open any `.ncl` file — diagnostics, hover, and completions work out of the box. Errors appear inline and in the diagnostics panel.

---

## Emacs

### With lsp-mode

```elisp
(use-package nickel-mode
  :ensure t)

(use-package lsp-mode
  :ensure t
  :hook (nickel-mode . lsp-deferred)
  :config
  (lsp-register-client
   (make-lsp-client
    :new-connection (lsp-stdio-connection '("nls"))
    :activation-fn (lsp-activate-on "nickel")
    :server-id 'nickel-ls)))
```

### With eglot (Emacs 29+)

```elisp
(use-package nickel-mode :ensure t)

(add-to-list 'eglot-server-programs '(nickel-mode "nls"))
(add-hook 'nickel-mode-hook #'eglot-ensure)
```

---

## Diagnostics mapping

When nix-workspace emits structured diagnostics (via `nix-workspace check --format json`), the diagnostic codes map to LSP severity levels:

| NW Code Range | LSP Severity | Editor Display |
|---------------|-------------|----------------|
| `NW0xx` (Contract violations) | Error | Red squiggle / ✗ |
| `NW1xx` (Discovery errors) | Error | Red squiggle / ✗ |
| `NW2xx` (Namespace conflicts) | Error | Red squiggle / ✗ |
| `NW3xx` (Dependency errors) | Error | Red squiggle / ✗ |
| `NW4xx` (Plugin errors) | Error | Red squiggle / ✗ |
| `NW5xx` (CLI errors) | Warning | Yellow squiggle / ⚠ |

### Integrating CLI diagnostics with your editor

For diagnostics that come from the Nix evaluation layer (not directly from Nickel), you can pipe `nix-workspace check --format json` output into your editor's diagnostic system. For example, with Neovim's diagnostic API:

```lua
-- Example: run nix-workspace check and populate diagnostics
local function nw_check()
  vim.fn.jobstart({ "nix-workspace", "check", "--format", "json" }, {
    stdout_buffered = true,
    on_stdout = function(_, data)
      local json = table.concat(data, "\n")
      local ok, report = pcall(vim.json.decode, json)
      if ok and report.diagnostics then
        local ns = vim.api.nvim_create_namespace("nix-workspace")
        vim.diagnostic.reset(ns)
        for _, d in ipairs(report.diagnostics) do
          if d.file then
            local bufnr = vim.fn.bufnr(d.file)
            if bufnr ~= -1 then
              vim.diagnostic.set(ns, bufnr, {{
                lnum = (d.line or 1) - 1,
                col = (d.column or 1) - 1,
                message = d.message,
                severity = d.severity == "error"
                  and vim.diagnostic.severity.ERROR
                  or vim.diagnostic.severity.WARN,
                source = "nix-workspace",
                code = d.code,
              }})
            end
          end
        end
      end
    end,
  })
end

vim.api.nvim_create_user_command("NixWorkspaceCheck", nw_check, {})
```

---

## Tips and tricks

### 1. Include nls in your project's dev shell

Ensure every contributor has the LSP available by adding it to your workspace's shell:

```nickel
# shells/default.ncl
{
  packages = ["nickel", "nls"],
}
```

### 2. Use direnv for automatic environment loading

With [direnv](https://direnv.net/) and [nix-direnv](https://github.com/nix-community/nix-direnv), your editor automatically picks up `nls` when you open the project:

```bash
# .envrc
use flake
```

### 3. Format on save

Configure your editor to run `nickel format` on save for consistent `.ncl` file formatting. Most editors support this via the LSP `textDocument/formatting` capability.

### 4. Validate before committing

Add `nix-workspace check` to your pre-commit hooks to catch configuration errors before they reach CI:

```yaml
# .pre-commit-config.yaml
repos:
  - repo: local
    hooks:
      - id: nix-workspace-check
        name: Validate workspace config
        entry: nix-workspace check
        language: system
        pass_filenames: false
        files: '\.ncl$'
```

---

## Troubleshooting

### NLS not starting

- Verify `nls` is on your `$PATH`: `which nls`
- Check that the editor is launched from a shell with `nls` available (e.g., from a `nix develop` shell or with direnv active).
- Check editor LSP logs for connection errors.

### Contract errors not showing

If you see parse errors but not contract violations, ensure that:

- Your `.ncl` file applies a contract (e.g., `| WorkspaceConfig` at the end).
- The contract imports resolve correctly (relative paths to `contracts/` are correct).
- NLS can find the imported files — check that your working directory is the workspace root.

### Slow diagnostics

Nickel evaluation is generally fast, but complex workspaces with many imports may cause a brief delay. If diagnostics feel slow:

- Check if there are circular imports (NLS will report this as an error).
- Simplify deeply nested contract hierarchies.
- Ensure you're running the latest version of `nls`.

### Import resolution failures

If NLS reports "cannot find import" for `contracts/*.ncl`:

- Make sure you're editing from the workspace root directory.
- Verify the import paths are correct relative to the file being edited.
- For standalone `.ncl` files in convention directories, imports use paths relative to the file's location (e.g., `import "../contracts/workspace.ncl"`).