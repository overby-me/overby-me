// Shared WASM environment — memory management + import object.
//
// Used by all browser examples to avoid duplicating ~70 lines each.
//
// This is the plain-JS port of the TypeScript allocator in runtime/memory.ts.
// See CHANGELOG.md Phase 25 for details.

// ── WASM memory state ───────────────────────────────────────────────────────

let wasmMemory = null;
let heapPointer = 0n;

// ── Free-list allocator state ───────────────────────────────────────────────

/**
 * JS-side size-class map: keys are block sizes (bigint), values are LIFO
 * stacks of free pointers.  Free = push, alloc = pop.  O(1) for both.
 *
 * Reuse is enabled by default.  The compiled WASM code emits double-free
 * calls (the same pointer freed more than once) due to Mojo's destructor
 * mechanics.  The allocator handles this safely: alignedFree removes
 * the pointer from ptrSize on first free, so subsequent frees of the
 * same pointer are detected as "unknown" and silently ignored.  This
 * prevents duplicate entries in the free list that would hand the same
 * block to two different allocations.
 *
 * @type {Map<bigint, bigint[]>}
 */
let freeMap = new Map();

/**
 * JS-side pointer → size map.  Every pointer returned by alignedAlloc
 * is registered here so that alignedFree can recover the block size
 * without writing any header into WASM linear memory.
 *
 * @type {Map<bigint, bigint>}
 */
let ptrSize = new Map();

/**
 * Whether alignedAlloc may reuse freed blocks from freeMap.
 * Enabled by default — double-free protection in alignedFree ensures
 * safe reuse even with WASM code that frees the same pointer twice.
 */
let reuseEnabled = true;

// ── Public API ──────────────────────────────────────────────────────────────

/** Get the current WASM memory (after initMemory has been called). */
export function getMemory() {
	return wasmMemory;
}

/**
 * Enable or disable free-list reuse.
 *
 * When enabled, alignedAlloc will pop matching blocks from the free map
 * instead of always bumping.  Enabled by default — the allocator's
 * double-free protection makes reuse safe even with WASM code that
 * emits duplicate free calls.
 */
export function setAllocatorReuse(on) {
	reuseEnabled = on;
}

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
export function alignedAlloc(align, size) {
	const actual = size < 1n ? 1n : size;

	// --- Size-class map path (O(1) pop) ---
	if (reuseEnabled) {
		const bucket = freeMap.get(actual);
		if (bucket !== undefined && bucket.length > 0) {
			const reused = bucket.pop();
			// Re-register in ptrSize so a future free can look up the size.
			ptrSize.set(reused, actual);
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

	return ptr;
}

/**
 * Free a previously allocated block.
 *
 * Looks up the block size from the JS-side pointer map and pushes the
 * pointer onto the size-class free list.  If reuse is disabled the block
 * is still tracked (visible in heapStats) but won't be handed out.
 */
export function alignedFree(ptr) {
	if (ptr === 0n) return 1;

	const size = ptrSize.get(ptr);
	if (size === undefined) return 1; // unknown or already-freed pointer — ignore

	// Remove from ptrSize so that a second free of the same pointer
	// (double-free from WASM) is detected as "unknown" and ignored.
	ptrSize.delete(ptr);

	// Push onto the size-class bucket (O(1)).
	let bucket = freeMap.get(size);
	if (bucket === undefined) {
		bucket = [];
		freeMap.set(size, bucket);
	}
	bucket.push(ptr);

	return 1;
}

// ── Scratch arena ───────────────────────────────────────────────────────────

/**
 * Scratch arena: a list of pointers allocated via scratchAlloc.
 * These are transient allocations (e.g. writeStringStruct per keystroke)
 * that should be bulk-freed after each dispatch+flush cycle.
 *
 * Call scratchFreeAll() after the WASM side has consumed the data.
 */
let scratchPtrs = [];

/**
 * Allocate from the main allocator and record the pointer in the scratch
 * arena.  Use this for transient JS→WASM data (e.g. writeStringStruct)
 * that should be bulk-freed after each dispatch+flush cycle.
 */
export function scratchAlloc(align, size) {
	const ptr = alignedAlloc(align, size);
	scratchPtrs.push(ptr);
	return ptr;
}

/**
 * Free every pointer in the scratch arena.  Call this after the WASM side
 * has consumed the transient data (typically after flush).
 */
export function scratchFreeAll() {
	for (const ptr of scratchPtrs) {
		alignedFree(ptr);
	}
	scratchPtrs = [];
}

// ── Diagnostics ─────────────────────────────────────────────────────────────

/**
 * Walk the size-class map and return summary statistics.
 *
 * @returns {{ heapPointer: bigint, freeBlocks: number, freeBytes: bigint }}
 */
export function heapStats() {
	let blocks = 0;
	let bytes = 0n;

	for (const [size, bucket] of freeMap) {
		blocks += bucket.length;
		bytes += size * BigInt(bucket.length);
	}

	return { heapPointer, freeBlocks: blocks, freeBytes: bytes };
}

/** Initialize memory state from WASM exports (called after instantiation). */
export function initMemory(exports) {
	wasmMemory = exports.memory;
	heapPointer = exports.__heap_base.value;
	freeMap = new Map();
	ptrSize = new Map();
	scratchPtrs = [];
}

// ── WASM import object ──────────────────────────────────────────────────────

export const env = {
	memory: new WebAssembly.Memory({ initial: 4096 }),
	__cxa_atexit: () => 0,
	KGEN_CompilerRT_AlignedAlloc: alignedAlloc,
	KGEN_CompilerRT_AlignedFree: alignedFree,
	KGEN_CompilerRT_GetStackTrace: () => 0n,
	KGEN_CompilerRT_fprintf: () => 0,
	write: (_fd, ptr, len) => {
		if (len === 0n || !wasmMemory) return 0;
		const text = new TextDecoder().decode(
			new Uint8Array(wasmMemory.buffer, Number(ptr), Number(len)),
		);
		console.log(text);
		return Number(len);
	},
	free: () => 1,
	dup: () => 1,
	fdopen: () => 1,
	fflush: () => 1,
	fclose: () => 1,
	__multi3: (resultPtr, aLo, aHi, bLo, bHi) => {
		if (!wasmMemory) return;
		const mask = 0xffffffffffffffffn;
		const product =
			(((aHi & mask) << 64n) | (aLo & mask)) *
			(((bHi & mask) << 64n) | (bLo & mask));
		const view = new DataView(wasmMemory.buffer);
		view.setBigInt64(Number(resultPtr), product & mask, true);
		view.setBigInt64(Number(resultPtr) + 8, (product >> 64n) & mask, true);
	},
	performance_now: () => performance.now(),

	// client-side routing (P30.2) — push/replace browser history
	push_state: (pathPtr) => {
		if (!wasmMemory) return;
		// Read the Mojo String struct: ptr (i64) + len (i64) at pathPtr
		const view = new DataView(wasmMemory.buffer);
		const strPtr = view.getBigInt64(Number(pathPtr), true);
		const strLen = view.getBigInt64(Number(pathPtr) + 8, true);
		if (strLen > 0n) {
			const bytes = new Uint8Array(
				wasmMemory.buffer,
				Number(strPtr),
				Number(strLen),
			);
			const path = new TextDecoder().decode(bytes);
			try {
				history.pushState(null, "", path);
			} catch (_) {
				/* no-op in non-browser */
			}
		}
	},
	replace_state: (pathPtr) => {
		if (!wasmMemory) return;
		const view = new DataView(wasmMemory.buffer);
		const strPtr = view.getBigInt64(Number(pathPtr), true);
		const strLen = view.getBigInt64(Number(pathPtr) + 8, true);
		if (strLen > 0n) {
			const bytes = new Uint8Array(
				wasmMemory.buffer,
				Number(strPtr),
				Number(strLen),
			);
			const path = new TextDecoder().decode(bytes);
			try {
				history.replaceState(null, "", path);
			} catch (_) {
				/* no-op in non-browser */
			}
		}
	},

	fmaf: (x, y, z) => Math.fround(Math.fround(x * y) + z),
	fminf: (x, y) => (x > y ? y : x),
	fmaxf: (x, y) => (x > y ? x : y),
	fma: (x, y, z) => x * y + z,
	fmin: (x, y) => (x > y ? y : x),
	fmax: (x, y) => (x > y ? x : y),
};

// ── WASM loader ─────────────────────────────────────────────────────────────

/**
 * Load and instantiate a Mojo WASM binary.
 *
 * @param {string|URL} wasmUrl - URL to the .wasm file.
 * @returns {Promise<object>} The WASM instance exports.
 */
export async function loadWasm(wasmUrl) {
	const wasmBuffer = await fetch(wasmUrl).then((r) => r.arrayBuffer());
	const { instance } = await WebAssembly.instantiate(wasmBuffer, { env });
	const fns = instance.exports;
	initMemory(fns);
	return fns;
}
