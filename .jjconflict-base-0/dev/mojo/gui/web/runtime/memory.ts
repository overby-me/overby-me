import type { WasmExports } from "./types.ts";

// --- WASM runtime state ---

let heapPointer: bigint = 0n;
let wasmExports: WasmExports | null = null;
let memory: WebAssembly.Memory | null = null;

// --- Free-list allocator state ---

/**
 * JS-side size-class map: keys are block sizes (bigint), values are LIFO
 * stacks of free pointers.  Free = push, alloc = pop.  O(1) for both.
 *
 * Reuse is enabled by default.  The compiled WASM code emits double-free
 * calls (the same pointer freed more than once) due to Mojo's destructor
 * mechanics.  The allocator handles this safely: `alignedFree` removes
 * the pointer from `ptrSize` on first free, so subsequent frees of the
 * same pointer are detected as "unknown" and silently ignored.  This
 * prevents duplicate entries in the free list that would hand the same
 * block to two different allocations.
 */
let freeMap: Map<bigint, bigint[]> = new Map();

/**
 * JS-side pointer → size map.  Every pointer returned by `alignedAlloc`
 * is registered here so that `alignedFree` can recover the block size
 * without writing any header into WASM linear memory.
 */
let ptrSize: Map<bigint, bigint> = new Map();

/**
 * Whether `alignedAlloc` may reuse freed blocks from `freeMap`.
 * Enabled by default — double-free protection in `alignedFree` ensures
 * safe reuse even with WASM code that frees the same pointer twice.
 */
let reuseEnabled = true;

/**
 * When true, every alignedAlloc / alignedFree call is logged to the console.
 * Heavy — only enable for targeted debugging of use-after-free issues.
 */
let traceEnabled = false;

// --- Scratch arena state ---

/**
 * Scratch arena: a list of pointers allocated via `scratchAlloc`.
 * These are transient allocations (e.g. `writeStringStruct` per keystroke)
 * that should be bulk-freed after each dispatch+flush cycle.
 *
 * Call `scratchFreeAll()` after the WASM side has consumed the data.
 */
let scratchPtrs: bigint[] = [];

// --- Public API ---

/** Initialize runtime state from a WASM instance. */
export const initialize = (instance: WebAssembly.Instance): void => {
	wasmExports = instance.exports as unknown as WasmExports;
	memory = wasmExports.memory;
	heapPointer = wasmExports.__heap_base.value as bigint;
	freeMap = new Map();
	ptrSize = new Map();
	scratchPtrs = [];
	traceEnabled = false;
};

/** Get the current WASM exports (throws if not initialized). */
export const getExports = (): WasmExports => {
	if (!wasmExports) throw new Error("WASM runtime not initialized");
	return wasmExports;
};

/** Get the current WASM memory (throws if not initialized). */
export const getMemory = (): WebAssembly.Memory => {
	if (!memory) throw new Error("WASM runtime not initialized");
	return memory;
};

/** Get a DataView over the current WASM memory buffer. */
export const getView = (): DataView => new DataView(getMemory().buffer);

/**
 * Enable or disable free-list reuse.
 *
 * When enabled, `alignedAlloc` will pop matching blocks from the free map
 * instead of always bumping.  Enabled by default — the allocator's
 * double-free protection makes reuse safe even with WASM code that
 * emits duplicate free calls.
 */
export const setAllocatorReuse = (on: boolean): void => {
	reuseEnabled = on;
};

/**
 * Enable or disable allocation tracing.
 *
 * When enabled, every alignedAlloc and alignedFree call is logged to
 * stderr with the pointer, size, and whether a free-list block was reused.
 * Very verbose — use only for targeted debugging sessions.
 */
export const setAllocatorTrace = (on: boolean): void => {
	traceEnabled = on;
};

// --- Allocator entry points ---

/**
 * Allocate `size` bytes with the given alignment from the WASM heap.
 *
 * Strategy:
 * 1. If reuse is enabled, check the size-class map for an exact match (O(1)).
 * 2. Fall back to bump allocation from the heap frontier.
 *
 * No header is written into WASM memory — block sizes are tracked in a
 * JS-side Map so the allocator is fully transparent to WASM code.
 */
export const alignedAlloc = (align: bigint, size: bigint): bigint => {
	const actual = size < 1n ? 1n : size;

	// --- Size-class map path (O(1) pop) ---
	if (reuseEnabled) {
		const bucket = freeMap.get(actual);
		if (bucket !== undefined && bucket.length > 0) {
			const reused = bucket.pop()!;
			// Re-register in ptrSize so a future free can look up the size.
			ptrSize.set(reused, actual);
			if (traceEnabled) {
				console.error(
					`[alloc] REUSE ptr=${reused} size=${actual} align=${align}`,
				);
			}
			return reused;
		}
	}

	// --- Bump-allocator fallback ---
	const remainder = heapPointer % align;
	if (remainder !== 0n) {
		heapPointer += align - remainder;
	}
	const ptr = heapPointer;
	heapPointer += actual;

	// Track the size in JS so alignedFree can find it later.
	ptrSize.set(ptr, actual);

	if (traceEnabled) {
		console.error(`[alloc] BUMP  ptr=${ptr} size=${actual} align=${align}`);
	}

	return ptr;
};

/**
 * Free a previously allocated block.
 *
 * Looks up the block size from the JS-side pointer map and pushes the
 * pointer onto the size-class free list.  If reuse is disabled the block
 * is still tracked (visible in `heapStats`) but won't be handed out.
 */
export const alignedFree = (ptr: bigint): number => {
	if (ptr === 0n) return 1;

	const size = ptrSize.get(ptr);
	if (size === undefined) {
		if (traceEnabled) {
			console.error(
				`[free]  UNKNOWN ptr=${ptr} (not in ptrSize map — likely double-free)`,
			);
		}
		return 1; // unknown or already-freed pointer — ignore
	}

	// Remove from ptrSize so that a second free of the same pointer
	// (double-free from WASM) is detected as "unknown" and ignored.
	ptrSize.delete(ptr);

	if (traceEnabled) {
		console.error(`[free]  ptr=${ptr} size=${size}`);
	}

	// Push onto the size-class bucket (O(1)).
	let bucket = freeMap.get(size);
	if (bucket === undefined) {
		bucket = [];
		freeMap.set(size, bucket);
	}
	bucket.push(ptr);

	return 1;
};

// --- Scratch arena ---

/**
 * Allocate from the main allocator and record the pointer in the scratch
 * arena.  Use this for transient JS→WASM data (e.g. `writeStringStruct`)
 * that should be bulk-freed after each dispatch+flush cycle.
 */
export const scratchAlloc = (align: bigint, size: bigint): bigint => {
	const ptr = alignedAlloc(align, size);
	scratchPtrs.push(ptr);
	return ptr;
};

/**
 * Free every pointer in the scratch arena.  Call this after the WASM side
 * has consumed the transient data (typically after flush).
 */
export const scratchFreeAll = (): void => {
	for (const ptr of scratchPtrs) {
		alignedFree(ptr);
	}
	scratchPtrs = [];
};

// --- Diagnostics (for tests) ---

export interface HeapStats {
	heapPointer: bigint;
	freeBlocks: number;
	freeBytes: bigint;
}

/** Walk the size-class map and return summary statistics. */
export const heapStats = (): HeapStats => {
	let blocks = 0;
	let bytes = 0n;

	for (const [size, bucket] of freeMap) {
		blocks += bucket.length;
		bytes += size * BigInt(bucket.length);
	}

	return { heapPointer, freeBlocks: blocks, freeBytes: bytes };
};

// --- Test isolation helpers ---

interface AllocatorSnapshot {
	heapPointer: bigint;
	freeMap: Map<bigint, bigint[]>;
	ptrSize: Map<bigint, bigint>;
	reuseEnabled: boolean;
	scratchPtrs: bigint[];
	wasmExports: WasmExports | null;
	memory: WebAssembly.Memory | null;
}

/**
 * Snapshot the current allocator state so it can be restored later.
 * Useful for running allocator unit tests without disturbing the
 * main WASM instance.
 */
export const saveAllocator = (): AllocatorSnapshot => ({
	heapPointer,
	freeMap,
	ptrSize,
	reuseEnabled,
	scratchPtrs,
	wasmExports,
	memory,
});

/** Restore a previously saved allocator snapshot. */
export const restoreAllocator = (snap: AllocatorSnapshot): void => {
	heapPointer = snap.heapPointer;
	freeMap = snap.freeMap;
	ptrSize = snap.ptrSize;
	reuseEnabled = snap.reuseEnabled;
	scratchPtrs = snap.scratchPtrs;
	wasmExports = snap.wasmExports;
	memory = snap.memory;
};

/**
 * Initialize the allocator with a raw Memory and heap base for testing.
 * Does not require a full WASM instance.
 */
export const initTestAllocator = (
	mem: WebAssembly.Memory,
	base: bigint,
): void => {
	memory = mem;
	wasmExports = null;
	heapPointer = base;
	freeMap = new Map();
	ptrSize = new Map();
	scratchPtrs = [];
	traceEnabled = false;
};
