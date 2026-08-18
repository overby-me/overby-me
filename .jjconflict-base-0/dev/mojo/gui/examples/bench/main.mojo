# Benchmark example — shared entry point for all platforms.
#
# This file compiles identically for web (WASM) and desktop (Blitz):
#
#   # Web (WASM):
#   mojo build main.mojo --target wasm64-wasi -I ../../core/src -I ../../web/src -I ..
#
#   # Desktop (native):
#   mojo build main.mojo -I ../../core/src -I ../../desktop/src -I ..
#
# On WASM targets, launch() stores the config and returns — the JS
# runtime drives the event loop via @export wrappers in web/src/main.mojo.
#
# On native targets, launch() calls desktop_launch[BenchmarkApp](config)
# which creates a Blitz window, mounts the DOM, and enters the event loop.

from platform.launch import launch, AppConfig
from bench import BenchmarkApp


fn main() raises:
    launch[BenchmarkApp](
        AppConfig(
            title="js-framework-benchmark — mojo-gui",
            width=1000,
            height=800,
        )
    )
