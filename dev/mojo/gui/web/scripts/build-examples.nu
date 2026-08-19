#!/usr/bin/env nu
# build-examples.nu — Build all shared examples for the web (WASM) target.
#
# This script discovers example apps in mojo-gui/examples/ and the existing
# web/examples/ directory, compiles the WASM module (which bundles all apps
# via main.mojo), and copies web-specific assets to the output directory.
#
# Architecture:
#
#   Currently, all example apps are compiled into a single WASM binary
#   (build/out.wasm) via web/src/main.mojo, which imports and re-exports
#   each app's lifecycle functions as @export WASM wrappers. The per-example
#   web/ assets (index.html, main.js) load this shared WASM binary and
#   use convention-based export discovery (e.g. "counter_init") to boot
#   the correct app.
#
#   Future: When apps are refactored to use launch(), each example will
#   compile to its own WASM binary, and this script will build them
#   individually.
#
# Usage:
#
#   cd mojo-gui/web
#   nu scripts/build-examples.nu
#
#   # Or build a specific example:
#   nu scripts/build-examples.nu counter
#
#   # Or build multiple:
#   nu scripts/build-examples.nu counter todo
#
# Prerequisites:
#
#   - mojo (Mojo compiler)
#   - wasm-ld (WebAssembly linker)
#   - The mojo-gui/core source tree at ../core/
#
# Output:
#
#   build/out.wasm              — compiled WASM binary (all apps)
#   build/examples/<name>/      — per-example output directories
#     ├── index.html            — HTML shell (copied from examples/<name>/web/ or web/examples/<name>/)
#     └── main.js               — JS entry point (copied)
#
# Environment variables:
#
#   MOJO_FLAGS    — extra flags to pass to `mojo build` (default: none during the 1.0 migration)
#   BUILD_DIR     — output directory (default: build)
#   INITIAL_MEM   — WASM initial memory in bytes (default: 268435456 = 256 MiB)

# All known examples (from web/examples/ directory)
const ALL_EXAMPLES = [counter todo bench app]

# ── Helpers ───────────────────────────────────────────────────────────────

def log [msg: string] {
    print $"==> ($msg)"
}

def err [msg: string] {
    print -e $"ERROR: ($msg)"
    exit 1
}

def check-tool [tool: string] {
    if (which $tool | is-empty) {
        err $"'($tool)' not found in PATH. Please install it."
    }
}

def main [...examples: string] {
    # ── Configuration ─────────────────────────────────────────────────────

    let script_dir = $env.FILE_PWD
    let web_dir = $script_dir | path dirname
    let root_dir = $web_dir | path dirname
    let core_dir = $root_dir | path join core
    let examples_dir = $root_dir | path join examples
    let web_examples_dir = $web_dir | path join examples

    let build_dir = $env.BUILD_DIR? | default ($web_dir | path join build)
    let mojo_flags = $env.MOJO_FLAGS? | default "" | split row -r '\s+' | where {|f| $f != "" }
    let initial_mem = $env.INITIAL_MEM? | default "268435456"

    let examples = if ($examples | is-empty) { $ALL_EXAMPLES } else { $examples }

    # ── Preflight checks ──────────────────────────────────────────────────

    check-tool mojo
    check-tool wasm-ld

    if not (($core_dir | path join src) | path exists) {
        err $"Core source not found at ($core_dir)/src — run from mojo-gui/web/"
    }
    if not (($web_dir | path join src main.mojo) | path exists) {
        err $"main.mojo not found at ($web_dir)/src/main.mojo"
    }

    # ── Step 1: Build the shared WASM binary ──────────────────────────────
    #
    # All apps are bundled into a single WASM binary via main.mojo.
    # This matches the existing `just build` workflow.

    log "Building WASM binary (all apps)..."

    mkdir $build_dir
    rm -f ($build_dir | path join out.cwasm)

    # Compile Mojo → WASM object (the SDK compiler targets wasm directly)
    let out_obj = $build_dir | path join out.o
    mojo build ...$mojo_flags --emit object --target-triple wasm64-wasi -I ($core_dir | path join src) -I $examples_dir -o $out_obj ($web_dir | path join src main.mojo)

    # Link → WASM binary
    let out_wasm = $build_dir | path join out.wasm
    (wasm-ld
        --no-entry
        --export-all
        --allow-undefined
        -mwasm64
        -z stack-size=8388608
        $"--initial-memory=($initial_mem)"
        -o $out_wasm
        ($build_dir | path join out.o))

    log $"WASM binary built: ($out_wasm)"

    # ── Step 2: Copy per-example web assets ───────────────────────────────
    #
    # Each example needs its HTML shell and JS entry point. We look in two
    # locations (shared examples first, then web-specific examples):
    #
    #   1. mojo-gui/examples/<name>/web/  (shared examples — target structure)
    #   2. mojo-gui/web/examples/<name>/  (current web examples)

    log $"Copying web assets for examples: ($examples | str join ' ')"

    for name in $examples {
        let out = $build_dir | path join examples $name
        mkdir $out

        # Find the HTML and JS assets (shared examples directory first)
        let html_src = [
            ($examples_dir | path join $name web index.html)
            ($web_examples_dir | path join $name index.html)
        ] | where {|p| $p | path exists } | get 0?

        let js_src = [
            ($examples_dir | path join $name web main.js)
            ($web_examples_dir | path join $name main.js)
        ] | where {|p| $p | path exists } | get 0?

        if $html_src == null {
            print $"  SKIP ($name) — no index.html found"
            continue
        }

        cp $html_src ($out | path join index.html)
        if $js_src != null {
            cp $js_src ($out | path join main.js)
        }

        print $"  OK   ($name) → ($out)/"
    }

    # ── Step 3: Copy shared JS library ────────────────────────────────────
    #
    # The examples/lib/ directory contains shared JS modules used by all
    # example entry points (app.js, env.js, events.js, interpreter.js, etc.)

    if (($web_examples_dir | path join lib) | path exists) {
        log "Copying shared JS library..."
        let lib_out = $build_dir | path join examples lib
        mkdir $lib_out
        cp ...(glob ($web_examples_dir | path join lib "*.js")) $lib_out
        print $"  OK   lib/ → ($lib_out)/"
    }

    # ── Summary ───────────────────────────────────────────────────────────

    let wasm_size = ls $out_wasm | get 0.size | into int
    log "Build complete!"
    print $"  WASM binary: ($out_wasm) \(($wasm_size) bytes\)"
    print $"  Examples:    ($examples | str join ' ')"
    print ""
    print "Serve locally with:"
    print $"  cd ($web_dir); deno run --allow-net --allow-read jsr:@std/http/file-server"
    print ""
    print "Then open:"
    for name in $examples {
        if (($build_dir | path join examples $name index.html) | path exists) {
            print $"  http://localhost:4507/examples/($name)/"
        }
    }
}
