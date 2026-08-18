// Free-list Allocator Unit Tests
//
// Tests the size-class map allocator in isolation using a raw
// WebAssembly.Memory (no WASM module required). Exercises: alignment,
// reuse, bump fallback, minimum block size, and rapid alloc/free stability.
//
// Also includes WASM-integrated reuse tests that verify the double-free
// protection works correctly when real WASM code emits duplicate frees.

import {
	alignedAlloc,
	alignedFree,
	getMemory,
	heapStats,
	initTestAllocator,
	restoreAllocator,
	saveAllocator,
	scratchAlloc,
	scratchFreeAll,
	setAllocatorReuse,
} from "../runtime/memory.ts";
import { MutationReader, Op } from "../runtime/protocol.ts";
import { writeStringStruct } from "../runtime/strings.ts";
import type { WasmExports } from "../runtime/types.ts";
import { assert, suite } from "./harness.ts";

type Fns = WasmExports & Record<string, unknown>;

// ── Helpers ─────────────────────────────────────────────────────────────────

/** Shared test memory — created once, reused across all allocator tests. */
const HEAP_BASE = 65536n; // 64 KiB — page 1 (leave page 0 as null guard)
let testMem: WebAssembly.Memory | null = null;

/** Reset the allocator to a clean state using the shared test memory.
 *  Enables reuse — safe here because we control all alloc/free patterns
 *  (no WASM use-after-free concerns in isolated tests). */
const freshAllocator = (): void => {
	if (!testMem) {
		testMem = new WebAssembly.Memory({ initial: 16 });
	}
	initTestAllocator(testMem, HEAP_BASE);
	setAllocatorReuse(true);
};

// ══════════════════════════════════════════════════════════════════════════════

export function testAllocator(): void {
	// Save the real runtime state so we can restore it after tests.
	const saved = saveAllocator();

	try {
		runAllocatorTests();
	} finally {
		restoreAllocator(saved);
	}
}

function runAllocatorTests(): void {
	// ─── Alignment ──────────────────────────────────────────────────────

	suite("Allocator — alloc returns pointers aligned to requested alignment");
	{
		freshAllocator();
		const p1 = alignedAlloc(1n, 1n);
		assert(p1 % 1n === 0n, true, "align=1 size=1 → 1-byte aligned");

		const p2 = alignedAlloc(4n, 100n);
		assert(p2 % 4n === 0n, true, "align=4 size=100 → 4-byte aligned");

		const p3 = alignedAlloc(8n, 7n);
		assert(p3 % 8n === 0n, true, "align=8 size=7 → 8-byte aligned");

		const p4 = alignedAlloc(16n, 64n);
		assert(p4 % 16n === 0n, true, "align=16 size=64 → 16-byte aligned");
	}

	// ─── Non-overlapping ────────────────────────────────────────────────

	suite("Allocator — consecutive allocs do not overlap");
	{
		freshAllocator();
		const p1 = alignedAlloc(8n, 32n);
		const p2 = alignedAlloc(8n, 64n);
		const p3 = alignedAlloc(8n, 16n);

		// Each pointer should be beyond the previous allocation.
		assert(p2 >= p1 + 32n, true, "p2 starts after p1's 32 bytes");
		assert(p3 >= p2 + 64n, true, "p3 starts after p2's 64 bytes");
	}

	// ─── Minimum block size ─────────────────────────────────────────────

	suite("Allocator — tiny allocs are contiguous (no header overhead)");
	{
		freshAllocator();
		const p1 = alignedAlloc(1n, 1n);
		const p2 = alignedAlloc(1n, 1n);
		// No header, no min block — consecutive 1-byte allocs are 1 byte apart.
		assert(p2 - p1, 1n, "1-byte allocs are exactly 1 byte apart");
	}

	// ─── Bump fallback when free list empty ─────────────────────────────

	suite("Allocator — bump fallback when free list is empty");
	{
		freshAllocator();
		const stats0 = heapStats();
		assert(stats0.freeBlocks, 0, "free list starts empty");

		const p1 = alignedAlloc(8n, 128n);
		assert(p1 > 0n, true, "alloc succeeds from bump");

		const stats1 = heapStats();
		assert(stats1.freeBlocks, 0, "free list still empty after bump alloc");
		assert(
			stats1.heapPointer > stats0.heapPointer,
			true,
			"heap pointer advanced",
		);
	}

	// ─── Free + re-alloc reuses memory ──────────────────────────────────

	suite("Allocator — free + re-alloc reuses memory (same size)");
	{
		freshAllocator();
		const p1 = alignedAlloc(8n, 64n);
		const heapAfterAlloc = heapStats().heapPointer;

		alignedFree(p1);
		assert(heapStats().freeBlocks, 1, "one block on free list after free");
		assert(heapStats().freeBytes, 64n, "64 free bytes");

		// Re-alloc same size should reuse the freed block.
		const p2 = alignedAlloc(8n, 64n);
		assert(p2, p1, "re-alloc returns same pointer (reuse)");

		assert(heapStats().freeBlocks, 0, "free list empty after reuse");
		assert(
			heapStats().heapPointer,
			heapAfterAlloc,
			"heap pointer unchanged (no bump needed)",
		);
	}

	// ─── Different sizes go to different buckets ────────────────────────

	suite("Allocator — different sizes use different buckets");
	{
		freshAllocator();
		const p32 = alignedAlloc(8n, 32n);
		const p64 = alignedAlloc(8n, 64n);

		alignedFree(p32);
		alignedFree(p64);

		assert(heapStats().freeBlocks, 2, "two free blocks (different sizes)");
		assert(heapStats().freeBytes, 96n, "32 + 64 = 96 free bytes");

		// Alloc 64 should get p64 back, not p32.
		const r64 = alignedAlloc(8n, 64n);
		assert(r64, p64, "64-byte alloc reuses 64-byte block");
		assert(heapStats().freeBlocks, 1, "one block left (32-byte)");

		// Alloc 32 should get p32 back.
		const r32 = alignedAlloc(8n, 32n);
		assert(r32, p32, "32-byte alloc reuses 32-byte block");
		assert(heapStats().freeBlocks, 0, "free list empty");
	}

	// ─── Mismatched size falls through to bump ──────────────────────────

	suite("Allocator — mismatched size falls through to bump");
	{
		freshAllocator();
		const p1 = alignedAlloc(8n, 32n);
		alignedFree(p1);

		// Request 64 — the 32-byte free block can't serve it.
		const heapBefore = heapStats().heapPointer;
		const p2 = alignedAlloc(8n, 64n);
		assert(p2 !== p1, true, "64-byte alloc doesn't reuse 32-byte block");
		assert(
			heapStats().heapPointer > heapBefore,
			true,
			"heap pointer advanced (bump fallback)",
		);
		// The 32-byte block is still free.
		assert(heapStats().freeBlocks, 1, "32-byte block still on free list");
	}

	// ─── Free of null / zero pointer is safe ────────────────────────────

	suite("Allocator — free(0) is a safe no-op");
	{
		freshAllocator();
		const result = alignedFree(0n);
		assert(result, 1, "free(0n) returns 1 (success/no-op)");
		assert(heapStats().freeBlocks, 0, "free list still empty");
	}

	// ─── Multiple frees of same size stack up ───────────────────────────

	suite("Allocator — multiple frees of same size stack up");
	{
		freshAllocator();
		const p1 = alignedAlloc(16n, 32n);
		const p2 = alignedAlloc(16n, 32n);
		const p3 = alignedAlloc(16n, 32n);

		alignedFree(p1);
		alignedFree(p2);
		alignedFree(p3);

		assert(heapStats().freeBlocks, 3, "three 32-byte blocks on free list");
		assert(heapStats().freeBytes, 96n, "3 × 32 = 96 free bytes");

		// LIFO: should get p3 first, then p2, then p1.
		const r1 = alignedAlloc(16n, 32n);
		const r2 = alignedAlloc(16n, 32n);
		const r3 = alignedAlloc(16n, 32n);
		assert(r1, p3, "first re-alloc returns p3 (LIFO)");
		assert(r2, p2, "second re-alloc returns p2 (LIFO)");
		assert(r3, p1, "third re-alloc returns p1 (LIFO)");
		assert(heapStats().freeBlocks, 0, "free list empty after reuse");
	}

	// ─── Rapid alloc/free cycles — heap stays bounded ───────────────────

	suite("Allocator — rapid alloc/free cycles keep heap stable");
	{
		freshAllocator();
		// Warm up: single allocation to establish the baseline.
		const warmup = alignedAlloc(8n, 64n);
		alignedFree(warmup);

		const baseline = heapStats().heapPointer;

		// 1000 alloc/free cycles of the same size.
		for (let i = 0; i < 1000; i++) {
			const p = alignedAlloc(8n, 64n);
			alignedFree(p);
		}

		const after = heapStats();
		assert(
			after.heapPointer,
			baseline,
			"heap pointer unchanged after 1000 alloc/free cycles",
		);
		assert(
			after.freeBlocks <= 1,
			true,
			"free list has at most 1 block after cycles",
		);
	}

	// ─── Mixed-size rapid cycles ────────────────────────────────────────

	suite("Allocator — mixed-size rapid cycles stay bounded");
	{
		freshAllocator();
		const sizes = [16n, 32n, 64n, 128n, 256n, 512n];

		// Allocate one of each to establish baseline.
		const ptrs: bigint[] = [];
		for (const sz of sizes) {
			ptrs.push(alignedAlloc(8n, sz));
		}
		for (const p of ptrs) {
			alignedFree(p);
		}

		const baseline = heapStats().heapPointer;

		// 200 rounds of alloc-all then free-all.
		for (let round = 0; round < 200; round++) {
			const batch: bigint[] = [];
			for (const sz of sizes) {
				batch.push(alignedAlloc(8n, sz));
			}
			for (const p of batch) {
				alignedFree(p);
			}
		}

		const after = heapStats();
		assert(
			after.heapPointer,
			baseline,
			"heap pointer unchanged after 200 mixed-size rounds",
		);
	}

	// ─── LIFO alloc/free pattern (stack-like) ───────────────────────────

	suite("Allocator — LIFO (stack-like) alloc/free reuses memory");
	{
		freshAllocator();
		// Allocate 5 blocks of same size, free in reverse, re-allocate.
		const origPtrs: bigint[] = [];
		for (let i = 0; i < 5; i++) {
			origPtrs.push(alignedAlloc(16n, 64n));
		}

		const heapBefore = heapStats().heapPointer;

		// Free all in reverse.
		for (let i = origPtrs.length - 1; i >= 0; i--) {
			alignedFree(origPtrs[i]);
		}

		assert(heapStats().freeBlocks, 5, "5 free blocks of same size");

		// Re-allocate same sizes — all should be served from the free list.
		const newPtrs: bigint[] = [];
		for (let i = 0; i < 5; i++) {
			newPtrs.push(alignedAlloc(16n, 64n));
		}

		assert(
			heapStats().heapPointer,
			heapBefore,
			"heap pointer unchanged (all reused from free list)",
		);
		assert(heapStats().freeBlocks, 0, "free list empty after reuse");
	}

	// ─── Interleaved alloc/free (checkerboard) ──────────────────────────

	suite("Allocator — interleaved alloc/free (checkerboard pattern)");
	{
		freshAllocator();
		// Allocate 10 blocks of 32 bytes each.
		const ptrs: bigint[] = [];
		for (let i = 0; i < 10; i++) {
			ptrs.push(alignedAlloc(16n, 32n));
		}

		// Free every other block (odd indices).
		for (let i = 1; i < 10; i += 2) {
			alignedFree(ptrs[i]);
		}

		assert(heapStats().freeBlocks, 5, "5 free blocks (checkerboard)");
		assert(heapStats().freeBytes, 160n, "5 × 32 = 160 free bytes");

		// Alloc 5 blocks of 32 — should reuse the freed ones.
		const heapBefore = heapStats().heapPointer;
		for (let i = 0; i < 5; i++) {
			alignedAlloc(16n, 32n);
		}

		assert(heapStats().freeBlocks, 0, "all free blocks consumed");
		assert(
			heapStats().heapPointer,
			heapBefore,
			"heap pointer unchanged (all reused)",
		);
	}

	// ─── Same-size reuse for small allocs ────────────────────────────────

	suite("Allocator — same-size small allocs reuse, different sizes don't");
	{
		freshAllocator();
		// Allocate 1 byte, free, re-alloc 1 byte — should reuse.
		const p1 = alignedAlloc(1n, 1n);
		alignedFree(p1);

		const p2 = alignedAlloc(1n, 1n);
		assert(p2, p1, "1-byte re-alloc reuses same pointer");

		// Allocate 8 bytes — different size bucket, should NOT reuse.
		alignedFree(p2);
		const p3 = alignedAlloc(1n, 8n);
		assert(p3 !== p2, true, "8-byte alloc does not reuse 1-byte block");

		// The 1-byte block is still on the free list.
		assert(heapStats().freeBlocks, 1, "1-byte block still free");
		assert(heapStats().freeBytes, 1n, "1 free byte");
	}

	// ─── HeapStats accuracy ─────────────────────────────────────────────

	suite("Allocator — heapStats reports correct counts and bytes");
	{
		freshAllocator();
		assert(heapStats().freeBlocks, 0, "initially 0 free blocks");
		assert(heapStats().freeBytes, 0n, "initially 0 free bytes");

		const p1 = alignedAlloc(8n, 48n);
		const p2 = alignedAlloc(8n, 96n);
		const p3 = alignedAlloc(8n, 48n);

		alignedFree(p1);
		assert(heapStats().freeBlocks, 1, "1 block after first free");
		assert(heapStats().freeBytes, 48n, "48 bytes after first free");

		alignedFree(p2);
		assert(heapStats().freeBlocks, 2, "2 blocks after second free");
		assert(heapStats().freeBytes, 144n, "48 + 96 = 144 bytes");

		alignedFree(p3);
		assert(heapStats().freeBlocks, 3, "3 blocks after third free");
		assert(heapStats().freeBytes, 192n, "48 + 96 + 48 = 192 bytes");

		// Consume them all.
		alignedAlloc(8n, 48n);
		alignedAlloc(8n, 96n);
		alignedAlloc(8n, 48n);

		assert(heapStats().freeBlocks, 0, "0 blocks after consuming all");
		assert(heapStats().freeBytes, 0n, "0 bytes after consuming all");
	}

	// ─── Large allocations work correctly ───────────────────────────────

	suite("Allocator — large allocations (4 KiB) alloc and reuse");
	{
		freshAllocator();
		const p = alignedAlloc(16n, 4096n);
		assert(p % 16n === 0n, true, "4 KiB alloc is 16-byte aligned");

		const heapAfter = heapStats().heapPointer;
		alignedFree(p);

		const p2 = alignedAlloc(16n, 4096n);
		assert(p2, p, "4 KiB re-alloc reuses freed block");
		assert(heapStats().heapPointer, heapAfter, "heap pointer unchanged");
	}

	// ─── Scratch arena: basic alloc + freeAll ───────────────────────────

	suite("Scratch arena — scratchAlloc + scratchFreeAll cycle");
	{
		freshAllocator();

		// scratchAlloc behaves like alignedAlloc (returns valid pointers).
		const s1 = scratchAlloc(8n, 24n);
		const s2 = scratchAlloc(1n, 10n);
		assert(s1 % 8n === 0n, true, "scratch ptr 1 is 8-byte aligned");
		assert(s2 > 0n, true, "scratch ptr 2 is valid");
		assert(s2 !== s1, true, "scratch ptrs are distinct");

		// No free blocks yet — scratch just allocated, not freed.
		assert(heapStats().freeBlocks, 0, "0 free blocks before scratchFreeAll");

		// Bulk-free: both scratch allocations should move to the free list.
		scratchFreeAll();
		assert(heapStats().freeBlocks, 2, "2 free blocks after scratchFreeAll");
		assert(heapStats().freeBytes, 34n, "24 + 10 = 34 free bytes");

		// Re-alloc same sizes: should reuse the freed scratch blocks.
		const heapBefore = heapStats().heapPointer;
		const r1 = scratchAlloc(1n, 10n);
		const r2 = scratchAlloc(8n, 24n);
		assert(r1, s2, "10-byte re-alloc reuses scratch ptr (LIFO)");
		assert(r2, s1, "24-byte re-alloc reuses scratch ptr (LIFO)");
		assert(heapStats().heapPointer, heapBefore, "heap pointer unchanged");

		// Clean up scratch for the next test.
		scratchFreeAll();
	}

	// ─── Scratch arena: empty scratchFreeAll is safe ────────────────────

	suite("Scratch arena — scratchFreeAll on empty arena is a no-op");
	freshAllocator();

	// Calling scratchFreeAll when nothing was scratch-allocated is fine.
	scratchFreeAll();
	assert(heapStats().freeBlocks, 0, "still 0 free blocks");
	assert(heapStats().freeBytes, 0n, "still 0 free bytes");

	// Double-call is also safe.
	scratchFreeAll();
	scratchFreeAll();
	assert(heapStats().freeBlocks, 0, "still 0 after double freeAll");

	// ─── Scratch arena: 1000 alloc/freeAll cycles stay bounded ──────────

	suite("Scratch arena — 1000 writeStringStruct-like cycles stay bounded");
	{
		freshAllocator();

		// Simulate writeStringStruct pattern: 2 scratch allocs per cycle
		// (data buffer + 24-byte struct), then scratchFreeAll after flush.
		scratchAlloc(1n, 6n); // "hello" + null
		scratchAlloc(8n, 24n); // string struct
		scratchFreeAll();

		const baseline = heapStats().heapPointer;

		for (let i = 0; i < 1000; i++) {
			scratchAlloc(1n, 6n);
			scratchAlloc(8n, 24n);
			scratchFreeAll();
		}

		const after = heapStats();
		assert(
			after.heapPointer,
			baseline,
			"heap pointer unchanged after 1000 scratch cycles",
		);
		assert(
			after.freeBlocks <= 2,
			true,
			"free list has at most 2 blocks (one per size bucket)",
		);
	}

	// ─── Scratch arena: mixed scratch + direct allocs ───────────────────

	suite("Scratch arena — scratchFreeAll does not affect direct allocs");
	{
		freshAllocator();

		// Direct alloc (not scratch).
		const directPtr = alignedAlloc(8n, 64n);

		// Scratch allocs.
		scratchAlloc(8n, 32n);
		scratchAlloc(8n, 16n);

		scratchFreeAll();

		// Scratch blocks are freed (2 blocks), direct block is NOT freed.
		assert(heapStats().freeBlocks, 2, "only scratch blocks freed");
		assert(heapStats().freeBytes, 48n, "32 + 16 = 48 scratch bytes freed");

		// Direct alloc of 64 should bump (not reuse 32 or 16).
		const heapBefore = heapStats().heapPointer;
		const d2 = alignedAlloc(8n, 64n);
		assert(d2 !== directPtr, true, "direct re-alloc is a new pointer");
		assert(
			heapStats().heapPointer > heapBefore,
			true,
			"heap pointer advanced (64-byte bucket was empty)",
		);

		// Scratch 32-byte bucket still has its freed block.
		assert(heapStats().freeBlocks, 2, "scratch free blocks untouched");
	}
}

// ══════════════════════════════════════════════════════════════════════════════
// WASM-integrated reuse tests
//
// These run against the real WASM module to verify that the double-free
// protection in alignedFree prevents memory corruption when the compiled
// WASM code frees the same pointer more than once.
// ══════════════════════════════════════════════════════════════════════════════

const WASM_TAG_DIV = 0;
const WASM_BUF_SIZE = 8192;

function wasmAllocBuf(fns: Fns): bigint {
	return fns.mutation_buf_alloc(WASM_BUF_SIZE);
}

function wasmFreeBuf(fns: Fns, ptr: bigint): void {
	fns.mutation_buf_free(ptr);
}

function wasmReadMutations(ptr: bigint, len: number) {
	const mem = getMemory();
	const reader = new MutationReader(mem.buffer, Number(ptr), len);
	return reader.readAll();
}

function wasmCreateCtx(fns: Fns) {
	const rt = fns.runtime_create();
	const eid = fns.eid_alloc_create();
	const store = fns.vnode_store_create();
	const buf = wasmAllocBuf(fns);
	const writer = fns.writer_create(buf, WASM_BUF_SIZE);
	return { rt, eid, store, buf, writer };
}

function wasmDestroyCtx(
	fns: Fns,
	ctx: { rt: bigint; eid: bigint; store: bigint; buf: bigint; writer: bigint },
) {
	fns.writer_destroy(ctx.writer);
	wasmFreeBuf(fns, ctx.buf);
	fns.vnode_store_destroy(ctx.store);
	fns.eid_alloc_destroy(ctx.eid);
	fns.runtime_destroy(ctx.rt);
}

function wasmRegisterDivWithDynText(
	fns: Fns,
	rt: bigint,
	name: string,
): number {
	const namePtr = writeStringStruct(name);
	const builder = fns.tmpl_builder_create(namePtr);
	const divIdx = fns.tmpl_builder_push_element(builder, WASM_TAG_DIV, -1);
	fns.tmpl_builder_push_dynamic_text(builder, 0, divIdx);
	const tmplId = fns.tmpl_builder_register(rt, builder);
	fns.tmpl_builder_destroy(builder);
	return tmplId;
}

function wasmRegisterDivWithDynAttr(
	fns: Fns,
	rt: bigint,
	name: string,
): number {
	const namePtr = writeStringStruct(name);
	const builder = fns.tmpl_builder_create(namePtr);
	const divIdx = fns.tmpl_builder_push_element(builder, WASM_TAG_DIV, -1);
	fns.tmpl_builder_push_dynamic_attr(builder, divIdx, 0);
	const tmplId = fns.tmpl_builder_register(rt, builder);
	fns.tmpl_builder_destroy(builder);
	return tmplId;
}

function runWasmReuseTests(fns: Fns): void {
	// ── Reuse ON: diff with changed dynamic text ─────────────────────
	suite("Reuse WASM — diff text changed → SetText");
	{
		setAllocatorReuse(true);
		const ctx = wasmCreateCtx(fns);
		const tmplId = wasmRegisterDivWithDynText(fns, ctx.rt, "reuse-txt");

		const oldIdx = fns.vnode_push_template_ref(ctx.store, tmplId);
		const dt1 = writeStringStruct("old text");
		fns.vnode_push_dynamic_text_node(ctx.store, oldIdx, dt1);
		fns.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		const newIdx = fns.vnode_push_template_ref(ctx.store, tmplId);
		const dt2 = writeStringStruct("new text");
		fns.vnode_push_dynamic_text_node(ctx.store, newIdx, dt2);

		fns.writer_destroy(ctx.writer);
		const buf2 = wasmAllocBuf(fns);
		const writer2 = fns.writer_create(buf2, WASM_BUF_SIZE);
		fns.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = fns.writer_finalize(writer2);
		const mutations = wasmReadMutations(buf2, offset);

		assert(mutations.length, 1, "reuse ON → 1 mutation");
		assert(mutations[0]?.op, Op.SetText, "reuse ON → SetText");
		if (mutations[0]?.op === Op.SetText) {
			assert(mutations[0].text, "new text", "reuse ON → correct text");
		}

		fns.writer_destroy(writer2);
		wasmFreeBuf(fns, buf2);
		wasmDestroyCtx(fns, ctx);
		setAllocatorReuse(false);
	}

	// ── Reuse ON: same values → 0 mutations ─────────────────────────
	suite("Reuse WASM — same dynamic text → 0 mutations");
	{
		setAllocatorReuse(true);
		const ctx = wasmCreateCtx(fns);
		const tmplId = wasmRegisterDivWithDynText(fns, ctx.rt, "reuse-same");

		const oldIdx = fns.vnode_push_template_ref(ctx.store, tmplId);
		const dt1 = writeStringStruct("same");
		fns.vnode_push_dynamic_text_node(ctx.store, oldIdx, dt1);
		fns.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		const newIdx = fns.vnode_push_template_ref(ctx.store, tmplId);
		const dt2 = writeStringStruct("same");
		fns.vnode_push_dynamic_text_node(ctx.store, newIdx, dt2);

		fns.writer_destroy(ctx.writer);
		const buf2 = wasmAllocBuf(fns);
		const writer2 = fns.writer_create(buf2, WASM_BUF_SIZE);
		fns.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = fns.writer_finalize(writer2);
		const mutations = wasmReadMutations(buf2, offset);

		assert(mutations.length, 0, "reuse ON → 0 mutations (same text)");

		fns.writer_destroy(writer2);
		wasmFreeBuf(fns, buf2);
		wasmDestroyCtx(fns, ctx);
		setAllocatorReuse(false);
	}

	// ── Reuse ON: attribute diff ─────────────────────────────────────
	suite("Reuse WASM — dynamic attribute changed → SetAttribute");
	{
		setAllocatorReuse(true);
		const ctx = wasmCreateCtx(fns);
		const tmplId = wasmRegisterDivWithDynAttr(fns, ctx.rt, "reuse-attr");

		const oldIdx = fns.vnode_push_template_ref(ctx.store, tmplId);
		const n1 = writeStringStruct("class");
		const v1 = writeStringStruct("old-class");
		fns.vnode_push_dynamic_attr_text(ctx.store, oldIdx, n1, v1, 0);
		fns.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		const newIdx = fns.vnode_push_template_ref(ctx.store, tmplId);
		const n2 = writeStringStruct("class");
		const v2 = writeStringStruct("new-class");
		fns.vnode_push_dynamic_attr_text(ctx.store, newIdx, n2, v2, 0);

		fns.writer_destroy(ctx.writer);
		const buf2 = wasmAllocBuf(fns);
		const writer2 = fns.writer_create(buf2, WASM_BUF_SIZE);
		fns.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = fns.writer_finalize(writer2);
		const mutations = wasmReadMutations(buf2, offset);

		assert(mutations.length, 1, "reuse ON attr → 1 mutation");
		assert(mutations[0]?.op, Op.SetAttribute, "reuse ON → SetAttribute");

		fns.writer_destroy(writer2);
		wasmFreeBuf(fns, buf2);
		wasmDestroyCtx(fns, ctx);
		setAllocatorReuse(false);
	}

	// ── Reuse ON: sequential diffs (state chain) ─────────────────────
	suite("Reuse WASM — sequential diffs (state chain)");
	{
		setAllocatorReuse(true);
		const ctx = wasmCreateCtx(fns);
		const tmplId = wasmRegisterDivWithDynText(fns, ctx.rt, "reuse-chain");

		const texts = ["count: 0", "count: 1", "count: 2", "count: 3", "count: 4"];
		let prevIdx = -1;
		let currentWriter = ctx.writer;
		let currentBuf = ctx.buf;

		for (let i = 0; i < texts.length; i++) {
			const idx = fns.vnode_push_template_ref(ctx.store, tmplId);
			const dt = writeStringStruct(texts[i]);
			fns.vnode_push_dynamic_text_node(ctx.store, idx, dt);

			if (prevIdx === -1) {
				fns.create_vnode(currentWriter, ctx.eid, ctx.rt, ctx.store, idx);
			} else {
				fns.writer_destroy(currentWriter);
				wasmFreeBuf(fns, currentBuf);
				currentBuf = wasmAllocBuf(fns);
				currentWriter = fns.writer_create(currentBuf, WASM_BUF_SIZE);

				fns.diff_vnodes(
					currentWriter,
					ctx.eid,
					ctx.rt,
					ctx.store,
					prevIdx,
					idx,
				);
				const off = fns.writer_finalize(currentWriter);
				const muts = wasmReadMutations(currentBuf, off);

				assert(muts.length, 1, `diff ${i - 1}→${i} → 1 mutation`);
				if (muts[0]?.op === Op.SetText) {
					assert(muts[0].text, texts[i], `diff ${i - 1}→${i} → correct text`);
				}
			}

			prevIdx = idx;
		}

		fns.writer_destroy(currentWriter);
		wasmFreeBuf(fns, currentBuf);
		fns.vnode_store_destroy(ctx.store);
		fns.eid_alloc_destroy(ctx.eid);
		fns.runtime_destroy(ctx.rt);
		setAllocatorReuse(false);
	}

	// ── Reuse ON: text VNode diff (both before create — the original
	//    failing pattern that exposed the double-free bug) ────────────
	suite("Reuse WASM — text VNode diff (both created before create_vnode)");
	{
		setAllocatorReuse(true);
		const ctx = wasmCreateCtx(fns);

		const t1 = writeStringStruct("aaa");
		const t2 = writeStringStruct("bbb");
		const oldIdx = fns.vnode_push_text(ctx.store, t1);
		const newIdx = fns.vnode_push_text(ctx.store, t2);

		fns.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		fns.writer_destroy(ctx.writer);
		const buf2 = wasmAllocBuf(fns);
		const writer2 = fns.writer_create(buf2, WASM_BUF_SIZE);
		fns.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = fns.writer_finalize(writer2);
		const mutations = wasmReadMutations(buf2, offset);

		assert(mutations.length, 1, "text diff (both before) → 1 mutation");
		assert(mutations[0]?.op, Op.SetText, "text diff (both before) → SetText");
		if (mutations[0]?.op === Op.SetText) {
			assert(
				mutations[0].text,
				"bbb",
				"text diff (both before) → correct text",
			);
		}

		fns.writer_destroy(writer2);
		wasmFreeBuf(fns, buf2);
		wasmDestroyCtx(fns, ctx);
		setAllocatorReuse(false);
	}

	// ── Reuse ON: text VNode diff (new after create) ─────────────────
	suite("Reuse WASM — text VNode diff (new created after create_vnode)");
	{
		setAllocatorReuse(true);
		const ctx = wasmCreateCtx(fns);

		const t1 = writeStringStruct("aaa");
		const oldIdx = fns.vnode_push_text(ctx.store, t1);

		fns.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		const t2 = writeStringStruct("bbb");
		const newIdx = fns.vnode_push_text(ctx.store, t2);

		fns.writer_destroy(ctx.writer);
		const buf2 = wasmAllocBuf(fns);
		const writer2 = fns.writer_create(buf2, WASM_BUF_SIZE);
		fns.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = fns.writer_finalize(writer2);
		const mutations = wasmReadMutations(buf2, offset);

		assert(mutations.length, 1, "text diff (new after) → 1 mutation");
		assert(mutations[0]?.op, Op.SetText, "text diff (new after) → SetText");
		if (mutations[0]?.op === Op.SetText) {
			assert(mutations[0].text, "bbb", "text diff (new after) → correct text");
		}

		fns.writer_destroy(writer2);
		wasmFreeBuf(fns, buf2);
		wasmDestroyCtx(fns, ctx);
		setAllocatorReuse(false);
	}

	// ── Reuse ON: fragment diff ──────────────────────────────────────
	suite("Reuse WASM — fragment children text changed");
	{
		setAllocatorReuse(true);
		const ctx = wasmCreateCtx(fns);

		const oldFrag = fns.vnode_push_fragment(ctx.store);
		const oa = writeStringStruct("alpha");
		const ob = writeStringStruct("beta");
		const oaIdx = fns.vnode_push_text(ctx.store, oa);
		const obIdx = fns.vnode_push_text(ctx.store, ob);
		fns.vnode_push_fragment_child(ctx.store, oldFrag, oaIdx);
		fns.vnode_push_fragment_child(ctx.store, oldFrag, obIdx);

		fns.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldFrag);

		const newFrag = fns.vnode_push_fragment(ctx.store);
		const na = writeStringStruct("alpha");
		const nc = writeStringStruct("gamma");
		const naIdx = fns.vnode_push_text(ctx.store, na);
		const ncIdx = fns.vnode_push_text(ctx.store, nc);
		fns.vnode_push_fragment_child(ctx.store, newFrag, naIdx);
		fns.vnode_push_fragment_child(ctx.store, newFrag, ncIdx);

		fns.writer_destroy(ctx.writer);
		const buf2 = wasmAllocBuf(fns);
		const writer2 = fns.writer_create(buf2, WASM_BUF_SIZE);
		fns.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldFrag, newFrag);
		const offset = fns.writer_finalize(writer2);
		const mutations = wasmReadMutations(buf2, offset);

		assert(mutations.length, 1, "fragment diff → 1 mutation");
		assert(mutations[0]?.op, Op.SetText, "fragment diff → SetText");
		if (mutations[0]?.op === Op.SetText) {
			assert(mutations[0].text, "gamma", "fragment diff → correct text");
		}

		fns.writer_destroy(writer2);
		wasmFreeBuf(fns, buf2);
		wasmDestroyCtx(fns, ctx);
		setAllocatorReuse(false);
	}

	// ── Heap stats: frees tracked during WASM operations ─────────────
	suite("Reuse WASM — heap stats track frees during WASM ops");
	{
		setAllocatorReuse(false);
		const statsBefore = heapStats();

		const ctx = wasmCreateCtx(fns);
		const tmplId = wasmRegisterDivWithDynText(fns, ctx.rt, "heap-track");

		const idx = fns.vnode_push_template_ref(ctx.store, tmplId);
		const dt = writeStringStruct("heap test");
		fns.vnode_push_dynamic_text_node(ctx.store, idx, dt);
		fns.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, idx);

		const statsAfter = heapStats();

		assert(
			statsAfter.heapPointer > statsBefore.heapPointer,
			true,
			"heap pointer advanced",
		);
		assert(
			statsAfter.freeBlocks >= statsBefore.freeBlocks,
			true,
			"free blocks did not decrease",
		);

		wasmDestroyCtx(fns, ctx);
	}

	// ── Reuse ON: placeholder diff → 0 mutations ────────────────────
	suite("Reuse WASM — placeholder diff → 0 mutations");
	{
		setAllocatorReuse(true);
		const ctx = wasmCreateCtx(fns);

		const oldIdx = fns.vnode_push_placeholder(ctx.store, 0);
		const newIdx = fns.vnode_push_placeholder(ctx.store, 0);

		fns.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		fns.writer_destroy(ctx.writer);
		const buf2 = wasmAllocBuf(fns);
		const writer2 = fns.writer_create(buf2, WASM_BUF_SIZE);
		fns.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = fns.writer_finalize(writer2);
		const mutations = wasmReadMutations(buf2, offset);

		assert(mutations.length, 0, "placeholder diff → 0 mutations");

		fns.writer_destroy(writer2);
		wasmFreeBuf(fns, buf2);
		wasmDestroyCtx(fns, ctx);
		setAllocatorReuse(false);
	}

	// ── Reuse ON: TemplateRef diff (both before create) ──────────────
	suite("Reuse WASM — TemplateRef diff (both created before create_vnode)");
	{
		setAllocatorReuse(true);
		const ctx = wasmCreateCtx(fns);
		const tmplId = wasmRegisterDivWithDynText(fns, ctx.rt, "reuse-both-tmpl");

		const oldIdx = fns.vnode_push_template_ref(ctx.store, tmplId);
		const dt1 = writeStringStruct("old tmpl");
		fns.vnode_push_dynamic_text_node(ctx.store, oldIdx, dt1);

		const newIdx = fns.vnode_push_template_ref(ctx.store, tmplId);
		const dt2 = writeStringStruct("new tmpl");
		fns.vnode_push_dynamic_text_node(ctx.store, newIdx, dt2);

		fns.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		fns.writer_destroy(ctx.writer);
		const buf2 = wasmAllocBuf(fns);
		const writer2 = fns.writer_create(buf2, WASM_BUF_SIZE);
		fns.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = fns.writer_finalize(writer2);
		const mutations = wasmReadMutations(buf2, offset);

		assert(mutations.length, 1, "tmplref (both before) → 1 mutation");
		assert(mutations[0]?.op, Op.SetText, "tmplref (both before) → SetText");

		fns.writer_destroy(writer2);
		wasmFreeBuf(fns, buf2);
		wasmDestroyCtx(fns, ctx);
		setAllocatorReuse(false);
	}
}

/** Run WASM-integrated reuse tests (requires a live WASM instance). */
export function testAllocatorReuse(fns: WasmExports): void {
	runWasmReuseTests(fns as Fns);
}
