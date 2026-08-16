"""Pre-compile the WASM binary into a serialized .cwasm module.

This script compiles build/out.wasm using the wasmtime engine and
serializes the result to build/out.cwasm.  The .cwasm file can then
be loaded by the test harness via Module.deserialize_file, which is
essentially an mmap and skips the expensive compilation step entirely.

Usage:
    mojo run -I ../mojo-wasmtime/src build/precompile.mojo
"""

from pathlib import Path

from wasmtime_mojo import Engine, Module


fn main() raises:
    var wasm_path = "build/out.wasm"
    var cwasm_path = "build/out.cwasm"

    if not Path(wasm_path).exists():
        raise Error(
            "WASM binary not found at "
            + wasm_path
            + " — run the build step first."
        )

    print("Compiling " + wasm_path + " ...")
    var engine = Engine(cache=True)
    var wasm_bytes = Path(wasm_path).read_bytes()
    var module = Module(engine.ptr(), wasm_bytes)

    print("Serializing to " + cwasm_path + " ...")
    module.serialize(cwasm_path)

    print("Done — " + cwasm_path + " is ready.")
