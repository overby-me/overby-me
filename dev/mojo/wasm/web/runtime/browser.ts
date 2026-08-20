// Browser entry point for the web runtime.
//
// ../examples/lib/env.js is generated from this file by scripts/bundle-env.ts and
// is not checked in. Before that, the browser half of the env glue was a
// hand-written JavaScript port of env.ts and memory.ts kept beside the
// TypeScript it copied, and the two drifted: the port was missing
// clock_gettime_nsec_np, which the Deno suites never noticed because they
// instantiate the TypeScript, and which a browser reported as a LinkError with
// no app on the page.
//
// Everything here is a re-export of the runtime proper, except loadWasm.
// mod.ts already has an instantiate() doing the same job, but it reads the
// binary with Deno.readFile; fetch is the browser's equivalent, and that one
// difference is the only reason this file exists.

import { env as denoEnv, setMemory } from "./env.ts";
import { getExports, getMemory, initialize } from "./memory.ts";
import type { WasmExports } from "./types.ts";

const decoder = new TextDecoder();

/**
 * The one import that cannot be shared. env.ts writes through Deno.stdout and
 * Deno.stderr because the Deno suites read the harness output off those two
 * streams by descriptor; a browser has neither. console.log is the equivalent
 * sink, and the descriptor stops mattering once both go to one console.
 */
const write = (_fd: bigint, ptr: bigint, len: bigint): number => {
	if (len === 0n) return 0;
	const mem = getMemory();
	const text = decoder.decode(
		new Uint8Array(mem.buffer, Number(ptr), Number(len)),
	);
	console.log(text);
	return Number(len);
};

/** The runtime's imports, with the host-specific write swapped in. */
export const env: WebAssembly.ModuleImports = { ...denoEnv, write };

export {
	alignedAlloc,
	alignedFree,
	getMemory,
	heapStats,
	scratchAlloc,
	scratchFreeAll,
	setAllocatorReuse,
} from "./memory.ts";
export { setMemory };

/**
 * Fetch and instantiate a Mojo WASM binary, wiring the shared heap and the
 * write stub to the new instance.
 *
 * @param wasmUrl - URL of the `.wasm` binary.
 */
export async function loadWasm(wasmUrl: string | URL): Promise<WasmExports> {
	const wasmBuffer = await fetch(wasmUrl).then((r) => r.arrayBuffer());
	const { instance } = await WebAssembly.instantiate(wasmBuffer, { env });
	initialize(instance);
	const exports = getExports();
	setMemory(exports.memory);
	return exports;
}
