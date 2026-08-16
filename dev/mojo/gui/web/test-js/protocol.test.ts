import { alignedAlloc, getMemory } from "../runtime/memory.ts";
import type {
	Mutation,
	MutationAppendChildren,
	MutationAssignId,
	MutationCreatePlaceholder,
	MutationCreateTextNode,
	MutationInsertAfter,
	MutationInsertBefore,
	MutationLoadTemplate,
	MutationNewEventListener,
	MutationPushRoot,
	MutationRegisterTemplate,
	MutationRemove,
	MutationRemoveAttribute,
	MutationRemoveEventListener,
	MutationReplacePlaceholder,
	MutationReplaceWith,
	MutationSetAttribute,
	MutationSetText,
} from "../runtime/protocol.ts";
import { MutationReader, Op } from "../runtime/protocol.ts";
import { writeStringStruct } from "../runtime/strings.ts";
import type { WasmExports } from "../runtime/types.ts";
import { assert, suite } from "./harness.ts";

// deno-lint-ignore no-explicit-any
// biome-ignore lint/suspicious/noExplicitAny: dynamic WASM exports
type WasmExportsExt = WasmExports & Record<string, any>;

// ── Helpers ─────────────────────────────────────────────────────────────────

const BUF_SIZE = 4096;

/** Allocate a mutation buffer via WASM and return the pointer. */
function allocBuf(fns: WasmExports): bigint {
	return fns.mutation_buf_alloc(BUF_SIZE);
}

/** Free a mutation buffer allocated via WASM. */
function freeBuf(fns: WasmExports, ptr: bigint): void {
	fns.mutation_buf_free(ptr);
}

/**
 * Create a MutationReader over the WASM memory region [ptr, ptr+len).
 */
function readerAt(ptr: bigint, len: number): MutationReader {
	const mem = getMemory();
	return new MutationReader(mem.buffer, Number(ptr), len);
}

/**
 * Read exactly one mutation from the buffer.  Asserts that there is exactly
 * one mutation followed by End (or buffer exhaustion).
 */
function readOne(ptr: bigint, len: number): Mutation | null {
	const reader = readerAt(ptr, len);
	return reader.next();
}

/** Write path bytes into WASM memory and return (ptr, len). */
function writePath(path: number[]): { ptr: bigint; len: number } {
	const len = path.length;
	const ptr = alignedAlloc(1n, BigInt(len));
	const mem = getMemory();
	const view = new Uint8Array(mem.buffer, Number(ptr), len);
	for (let i = 0; i < len; i++) view[i] = path[i];
	return { ptr, len };
}

// ── Tests ───────────────────────────────────────────────────────────────────

export function testProtocol(fns: WasmExports): void {
	const ext = fns as WasmExportsExt;

	// ── Debug: pointer round-trip ────────────────────────────────────
	suite("Protocol — debug: pointer round-trip");
	{
		const buf = allocBuf(fns);
		const bufNum = Number(buf);
		console.log(`    [debug] buf ptr = ${buf} (${bufNum})`);

		// Check round-trip: int→ptr→int should be identity
		if (ext.debug_ptr_roundtrip) {
			const rt = ext.debug_ptr_roundtrip(buf) as bigint;
			console.log(`    [debug] round-trip = ${rt}`);
			assert(rt, buf, "ptr round-trip preserves address");
		} else {
			console.log("    [debug] debug_ptr_roundtrip not exported, skipping");
		}

		// Check that we can write and read individual bytes
		if (ext.debug_write_byte && ext.debug_read_byte) {
			const off1 = ext.debug_write_byte(buf, 0, 0xab) as number;
			assert(off1, 1, "debug_write_byte returns offset+1");

			const val = ext.debug_read_byte(buf, 0) as number;
			assert(val, 0xab, "debug_read_byte reads back 0xAB");

			// Also verify from JS side
			const mem = getMemory();
			const jsVal = new Uint8Array(mem.buffer, bufNum, 1)[0];
			assert(jsVal, 0xab, "JS reads same byte 0xAB from WASM memory");
		} else {
			console.log("    [debug] debug_write/read_byte not exported, skipping");
		}

		freeBuf(fns, buf);
	}

	// ── End sentinel ──────────────────────────────────────────────────
	suite("Protocol — End sentinel");
	{
		const buf = allocBuf(fns);
		const off = fns.write_op_end(buf, 0);
		assert(off, 1, "End writes 1 byte");

		const reader = readerAt(buf, off);
		const m = reader.next();
		assert(m, null, "End sentinel returns null");
		freeBuf(fns, buf);
	}

	// ── Empty buffer ─────────────────────────────────────────────────
	suite("Protocol — empty buffer");
	{
		const buf = allocBuf(fns);
		// Don't write anything — reader over 0 bytes
		const reader = readerAt(buf, 0);
		const m = reader.next();
		assert(m, null, "Empty buffer returns null");

		const all = readerAt(buf, 0).readAll();
		assert(all.length, 0, "readAll on empty buffer returns []");
		freeBuf(fns, buf);
	}

	// ── Buffer with only End sentinel ────────────────────────────────
	suite("Protocol — only End sentinel");
	{
		const buf = allocBuf(fns);
		fns.write_op_end(buf, 0);
		const all = readerAt(buf, 1).readAll();
		assert(all.length, 0, "readAll with only End returns []");
		freeBuf(fns, buf);
	}

	// ── AppendChildren ───────────────────────────────────────────────
	suite("Protocol — AppendChildren");
	{
		const buf = allocBuf(fns);
		let off = 0;
		off = fns.write_op_append_children(buf, off, 42, 3);
		assert(off, 9, "AppendChildren writes 1 + 4 + 4 = 9 bytes");

		const m = readOne(buf, off) as MutationAppendChildren;
		assert(m.op, Op.AppendChildren, "op is AppendChildren");
		assert(m.id, 42, "id is 42");
		assert(m.m, 3, "m is 3");
		freeBuf(fns, buf);
	}

	// ── AppendChildren with zero children ────────────────────────────
	suite("Protocol — AppendChildren (zero children)");
	{
		const buf = allocBuf(fns);
		const off = fns.write_op_append_children(buf, 0, 1, 0);
		const m = readOne(buf, off) as MutationAppendChildren;
		assert(m.op, Op.AppendChildren, "op is AppendChildren");
		assert(m.id, 1, "id is 1");
		assert(m.m, 0, "m is 0 (zero children)");
		freeBuf(fns, buf);
	}

	// ── CreatePlaceholder ────────────────────────────────────────────
	suite("Protocol — CreatePlaceholder");
	{
		const buf = allocBuf(fns);
		let off = 0;
		off = fns.write_op_create_placeholder(buf, off, 99);
		assert(off, 5, "CreatePlaceholder writes 1 + 4 = 5 bytes");

		const m = readOne(buf, off) as MutationCreatePlaceholder;
		assert(m.op, Op.CreatePlaceholder, "op is CreatePlaceholder");
		assert(m.id, 99, "id is 99");
		freeBuf(fns, buf);
	}

	// ── LoadTemplate ─────────────────────────────────────────────────
	suite("Protocol — LoadTemplate");
	{
		const buf = allocBuf(fns);
		let off = 0;
		off = fns.write_op_load_template(buf, off, 5, 0, 100);
		assert(off, 13, "LoadTemplate writes 1 + 4 + 4 + 4 = 13 bytes");

		const m = readOne(buf, off) as MutationLoadTemplate;
		assert(m.op, Op.LoadTemplate, "op is LoadTemplate");
		assert(m.tmplId, 5, "tmplId is 5");
		assert(m.index, 0, "index is 0");
		assert(m.id, 100, "id is 100");
		freeBuf(fns, buf);
	}

	// ── ReplaceWith ──────────────────────────────────────────────────
	suite("Protocol — ReplaceWith");
	{
		const buf = allocBuf(fns);
		const off = fns.write_op_replace_with(buf, 0, 7, 2);
		assert(off, 9, "ReplaceWith writes 9 bytes");

		const m = readOne(buf, off) as MutationReplaceWith;
		assert(m.op, Op.ReplaceWith, "op is ReplaceWith");
		assert(m.id, 7, "id is 7");
		assert(m.m, 2, "m is 2");
		freeBuf(fns, buf);
	}

	// ── InsertAfter ──────────────────────────────────────────────────
	suite("Protocol — InsertAfter");
	{
		const buf = allocBuf(fns);
		const off = fns.write_op_insert_after(buf, 0, 20, 4);
		const m = readOne(buf, off) as MutationInsertAfter;
		assert(m.op, Op.InsertAfter, "op is InsertAfter");
		assert(m.id, 20, "id is 20");
		assert(m.m, 4, "m is 4");
		freeBuf(fns, buf);
	}

	// ── InsertBefore ─────────────────────────────────────────────────
	suite("Protocol — InsertBefore");
	{
		const buf = allocBuf(fns);
		const off = fns.write_op_insert_before(buf, 0, 30, 1);
		const m = readOne(buf, off) as MutationInsertBefore;
		assert(m.op, Op.InsertBefore, "op is InsertBefore");
		assert(m.id, 30, "id is 30");
		assert(m.m, 1, "m is 1");
		freeBuf(fns, buf);
	}

	// ── Remove ───────────────────────────────────────────────────────
	suite("Protocol — Remove");
	{
		const buf = allocBuf(fns);
		const off = fns.write_op_remove(buf, 0, 55);
		assert(off, 5, "Remove writes 1 + 4 = 5 bytes");

		const m = readOne(buf, off) as MutationRemove;
		assert(m.op, Op.Remove, "op is Remove");
		assert(m.id, 55, "id is 55");
		freeBuf(fns, buf);
	}

	// ── PushRoot ─────────────────────────────────────────────────────
	suite("Protocol — PushRoot");
	{
		const buf = allocBuf(fns);
		const off = fns.write_op_push_root(buf, 0, 77);
		assert(off, 5, "PushRoot writes 1 + 4 = 5 bytes");

		const m = readOne(buf, off) as MutationPushRoot;
		assert(m.op, Op.PushRoot, "op is PushRoot");
		assert(m.id, 77, "id is 77");
		freeBuf(fns, buf);
	}

	// ── CreateTextNode ───────────────────────────────────────────────
	suite("Protocol — CreateTextNode");
	{
		const buf = allocBuf(fns);
		const textPtr = writeStringStruct("hello");
		const off = fns.write_op_create_text_node(buf, 0, 11, textPtr);
		// 1 (op) + 4 (id) + 4 (len) + 5 (text) = 14
		assert(off, 14, "CreateTextNode('hello') writes 14 bytes");

		const m = readOne(buf, off) as MutationCreateTextNode;
		assert(m.op, Op.CreateTextNode, "op is CreateTextNode");
		assert(m.id, 11, "id is 11");
		assert(m.text, "hello", "text is 'hello'");
		freeBuf(fns, buf);
	}

	// ── CreateTextNode with empty string ─────────────────────────────
	suite("Protocol — CreateTextNode (empty string)");
	{
		const buf = allocBuf(fns);
		const textPtr = writeStringStruct("");
		const off = fns.write_op_create_text_node(buf, 0, 12, textPtr);
		// 1 (op) + 4 (id) + 4 (len=0) = 9
		assert(off, 9, "CreateTextNode('') writes 9 bytes");

		const m = readOne(buf, off) as MutationCreateTextNode;
		assert(m.op, Op.CreateTextNode, "op is CreateTextNode");
		assert(m.id, 12, "id is 12");
		assert(m.text, "", "text is empty string");
		freeBuf(fns, buf);
	}

	// ── CreateTextNode with unicode ──────────────────────────────────
	suite("Protocol — CreateTextNode (unicode)");
	{
		const buf = allocBuf(fns);
		const textPtr = writeStringStruct("Hello 🔥 Mojo");
		const off = fns.write_op_create_text_node(buf, 0, 13, textPtr);

		const m = readOne(buf, off) as MutationCreateTextNode;
		assert(m.op, Op.CreateTextNode, "op is CreateTextNode");
		assert(m.text, "Hello 🔥 Mojo", "text is 'Hello 🔥 Mojo'");
		freeBuf(fns, buf);
	}

	// ── SetText ──────────────────────────────────────────────────────
	suite("Protocol — SetText");
	{
		const buf = allocBuf(fns);
		const textPtr = writeStringStruct("updated");
		const off = fns.write_op_set_text(buf, 0, 44, textPtr);
		// 1 (op) + 4 (id) + 4 (len) + 7 (text) = 16
		assert(off, 16, "SetText('updated') writes 16 bytes");

		const m = readOne(buf, off) as MutationSetText;
		assert(m.op, Op.SetText, "op is SetText");
		assert(m.id, 44, "id is 44");
		assert(m.text, "updated", "text is 'updated'");
		freeBuf(fns, buf);
	}

	// ── SetAttribute ─────────────────────────────────────────────────
	suite("Protocol — SetAttribute");
	{
		const buf = allocBuf(fns);
		const namePtr = writeStringStruct("class");
		const valPtr = writeStringStruct("container");
		const off = fns.write_op_set_attribute(buf, 0, 50, 0, namePtr, valPtr);
		// 1 (op) + 4 (id) + 1 (ns) + 2 (name_len) + 5 (name) + 4 (val_len) + 9 (val) = 26
		assert(off, 26, "SetAttribute('class','container') writes 26 bytes");

		const m = readOne(buf, off) as MutationSetAttribute;
		assert(m.op, Op.SetAttribute, "op is SetAttribute");
		assert(m.id, 50, "id is 50");
		assert(m.ns, 0, "ns is 0 (no namespace)");
		assert(m.name, "class", "name is 'class'");
		assert(m.value, "container", "value is 'container'");
		freeBuf(fns, buf);
	}

	// ── SetAttribute with namespace ──────────────────────────────────
	suite("Protocol — SetAttribute (with namespace)");
	{
		const buf = allocBuf(fns);
		const namePtr = writeStringStruct("href");
		const valPtr = writeStringStruct("http://example.com");
		const off = fns.write_op_set_attribute(buf, 0, 51, 1, namePtr, valPtr);

		const m = readOne(buf, off) as MutationSetAttribute;
		assert(m.ns, 1, "ns is 1 (xlink namespace)");
		assert(m.name, "href", "name is 'href'");
		assert(m.value, "http://example.com", "value is 'http://example.com'");
		freeBuf(fns, buf);
	}

	// ── SetAttribute with empty value ────────────────────────────────
	suite("Protocol — SetAttribute (empty value)");
	{
		const buf = allocBuf(fns);
		const namePtr = writeStringStruct("disabled");
		const valPtr = writeStringStruct("");
		const off = fns.write_op_set_attribute(buf, 0, 52, 0, namePtr, valPtr);

		const m = readOne(buf, off) as MutationSetAttribute;
		assert(m.name, "disabled", "name is 'disabled'");
		assert(m.value, "", "value is empty string");
		freeBuf(fns, buf);
	}

	// ── RemoveAttribute ──────────────────────────────────────────────
	suite("Protocol — RemoveAttribute");
	{
		const buf = allocBuf(fns);
		const namePtr = writeStringStruct("disabled");
		const off = ext.write_op_remove_attribute(buf, 0, 53, 0, namePtr) as number;
		// 1 (op) + 4 (id) + 1 (ns) + 2 (name_len) + 8 (name) = 16
		assert(off, 16, "RemoveAttribute('disabled') writes 16 bytes");

		const m = readOne(buf, off) as MutationRemoveAttribute;
		assert(m.op, Op.RemoveAttribute, "op is RemoveAttribute");
		assert(m.id, 53, "id is 53");
		assert(m.ns, 0, "ns is 0 (no namespace)");
		assert(m.name, "disabled", "name is 'disabled'");
		freeBuf(fns, buf);
	}

	// ── RemoveAttribute with namespace ───────────────────────────────
	suite("Protocol — RemoveAttribute (with namespace)");
	{
		const buf = allocBuf(fns);
		const namePtr = writeStringStruct("href");
		const off = ext.write_op_remove_attribute(buf, 0, 54, 1, namePtr) as number;
		// 1 (op) + 4 (id) + 1 (ns) + 2 (name_len) + 4 (name) = 12
		assert(off, 12, "RemoveAttribute('href', ns=1) writes 12 bytes");

		const m = readOne(buf, off) as MutationRemoveAttribute;
		assert(m.op, Op.RemoveAttribute, "op is RemoveAttribute");
		assert(m.id, 54, "id is 54");
		assert(m.ns, 1, "ns is 1 (xlink namespace)");
		assert(m.name, "href", "name is 'href'");
		freeBuf(fns, buf);
	}

	// ── RemoveAttribute in sequence (Set → Remove round-trip) ────────
	suite("Protocol — RemoveAttribute in Set→Remove sequence");
	{
		const buf = allocBuf(fns);
		let off = 0;
		const namePtr1 = writeStringStruct("hidden");
		const valPtr1 = writeStringStruct("");
		off = fns.write_op_set_attribute(buf, off, 55, 0, namePtr1, valPtr1);
		const namePtr2 = writeStringStruct("hidden");
		off = ext.write_op_remove_attribute(buf, off, 55, 0, namePtr2) as number;
		fns.write_op_end(buf, off);

		const reader = readerAt(buf, off + 1);
		const mutations = reader.readAll();
		assert(mutations.length, 2, "two mutations in buffer");
		const m0 = mutations[0] as MutationSetAttribute;
		assert(m0.op, Op.SetAttribute, "first is SetAttribute");
		assert(m0.name, "hidden", "first name is 'hidden'");
		assert(m0.value, "", "first value is empty");
		const m1 = mutations[1] as MutationRemoveAttribute;
		assert(m1.op, Op.RemoveAttribute, "second is RemoveAttribute");
		assert(m1.name, "hidden", "second name is 'hidden'");
		freeBuf(fns, buf);
	}

	// ── NewEventListener ─────────────────────────────────────────────
	suite("Protocol — NewEventListener");
	{
		const buf = allocBuf(fns);
		const namePtr = writeStringStruct("click");
		const off = fns.write_op_new_event_listener(buf, 0, 60, 77, namePtr);
		// 1 (op) + 4 (id) + 4 (handler_id) + 2 (name_len) + 5 (name) = 16
		assert(off, 16, "NewEventListener('click') writes 16 bytes");

		const m = readOne(buf, off) as MutationNewEventListener;
		assert(m.op, Op.NewEventListener, "op is NewEventListener");
		assert(m.id, 60, "id is 60");
		assert(m.handlerId, 77, "handlerId is 77");
		assert(m.name, "click", "name is 'click'");
		freeBuf(fns, buf);
	}

	// ── RemoveEventListener ──────────────────────────────────────────
	suite("Protocol — RemoveEventListener");
	{
		const buf = allocBuf(fns);
		const namePtr = writeStringStruct("input");
		const off = fns.write_op_remove_event_listener(buf, 0, 61, namePtr);
		// 1 (op) + 4 (id) + 2 (name_len) + 5 (name) = 12
		assert(off, 12, "RemoveEventListener('input') writes 12 bytes");

		const m = readOne(buf, off) as MutationRemoveEventListener;
		assert(m.op, Op.RemoveEventListener, "op is RemoveEventListener");
		assert(m.id, 61, "id is 61");
		assert(m.name, "input", "name is 'input'");
		freeBuf(fns, buf);
	}

	// ── AssignId ─────────────────────────────────────────────────────
	suite("Protocol — AssignId");
	{
		const buf = allocBuf(fns);
		const { ptr: pathPtr, len: pathLen } = writePath([0, 1, 2]);
		const off = fns.write_op_assign_id(buf, 0, pathPtr, pathLen, 88);
		// 1 (op) + 1 (path_len) + 3 (path) + 4 (id) = 9
		assert(off, 9, "AssignId([0,1,2], 88) writes 9 bytes");

		const m = readOne(buf, off) as MutationAssignId;
		assert(m.op, Op.AssignId, "op is AssignId");
		assert(m.path.length, 3, "path has 3 elements");
		assert(m.path[0], 0, "path[0] is 0");
		assert(m.path[1], 1, "path[1] is 1");
		assert(m.path[2], 2, "path[2] is 2");
		assert(m.id, 88, "id is 88");
		freeBuf(fns, buf);
	}

	// ── AssignId with empty path ─────────────────────────────────────
	suite("Protocol — AssignId (empty path)");
	{
		const buf = allocBuf(fns);
		const { ptr: pathPtr, len: pathLen } = writePath([]);
		const off = fns.write_op_assign_id(buf, 0, pathPtr, pathLen, 89);
		// 1 (op) + 1 (path_len=0) + 4 (id) = 6
		assert(off, 6, "AssignId([], 89) writes 6 bytes");

		const m = readOne(buf, off) as MutationAssignId;
		assert(m.path.length, 0, "path is empty");
		assert(m.id, 89, "id is 89");
		freeBuf(fns, buf);
	}

	// ── ReplacePlaceholder ───────────────────────────────────────────
	suite("Protocol — ReplacePlaceholder");
	{
		const buf = allocBuf(fns);
		const { ptr: pathPtr, len: pathLen } = writePath([0, 2]);
		const off = fns.write_op_replace_placeholder(buf, 0, pathPtr, pathLen, 3);
		// 1 (op) + 1 (path_len) + 2 (path) + 4 (m) = 8
		assert(off, 8, "ReplacePlaceholder([0,2], 3) writes 8 bytes");

		const m = readOne(buf, off) as MutationReplacePlaceholder;
		assert(m.op, Op.ReplacePlaceholder, "op is ReplacePlaceholder");
		assert(m.path.length, 2, "path has 2 elements");
		assert(m.path[0], 0, "path[0] is 0");
		assert(m.path[1], 2, "path[1] is 2");
		assert(m.m, 3, "m is 3");
		freeBuf(fns, buf);
	}

	// ── Multiple mutations in sequence ───────────────────────────────
	suite("Protocol — multiple mutations in sequence");
	{
		const buf = allocBuf(fns);
		let off = 0;
		off = fns.write_op_push_root(buf, off, 1);
		off = fns.write_op_create_placeholder(buf, off, 2);
		off = fns.write_op_append_children(buf, off, 1, 1);
		off = fns.write_op_remove(buf, off, 3);
		off = fns.write_op_end(buf, off);

		const mutations = readerAt(buf, off).readAll();
		assert(mutations.length, 4, "4 mutations before End");

		assert(mutations[0].op, Op.PushRoot, "mutation[0] is PushRoot");
		assert((mutations[0] as MutationPushRoot).id, 1, "PushRoot id=1");

		assert(
			mutations[1].op,
			Op.CreatePlaceholder,
			"mutation[1] is CreatePlaceholder",
		);
		assert(
			(mutations[1] as MutationCreatePlaceholder).id,
			2,
			"CreatePlaceholder id=2",
		);

		assert(mutations[2].op, Op.AppendChildren, "mutation[2] is AppendChildren");
		assert(
			(mutations[2] as MutationAppendChildren).id,
			1,
			"AppendChildren id=1",
		);
		assert((mutations[2] as MutationAppendChildren).m, 1, "AppendChildren m=1");

		assert(mutations[3].op, Op.Remove, "mutation[3] is Remove");
		assert((mutations[3] as MutationRemove).id, 3, "Remove id=3");

		freeBuf(fns, buf);
	}

	// ── Mixed mutations with strings ─────────────────────────────────
	suite("Protocol — mixed mutations with strings");
	{
		const buf = allocBuf(fns);
		let off = 0;

		off = fns.write_op_load_template(buf, off, 1, 0, 10);
		const textPtr = writeStringStruct("Count: 0");
		off = fns.write_op_create_text_node(buf, off, 11, textPtr);
		off = fns.write_op_append_children(buf, off, 10, 1);
		const evtPtr = writeStringStruct("click");
		off = fns.write_op_new_event_listener(buf, off, 10, 0, evtPtr);
		off = fns.write_op_push_root(buf, off, 10);
		off = fns.write_op_end(buf, off);

		const mutations = readerAt(buf, off).readAll();
		assert(mutations.length, 5, "5 mutations before End");

		const lt = mutations[0] as MutationLoadTemplate;
		assert(lt.op, Op.LoadTemplate, "first op is LoadTemplate");
		assert(lt.tmplId, 1, "tmplId is 1");
		assert(lt.id, 10, "element id is 10");

		const ct = mutations[1] as MutationCreateTextNode;
		assert(ct.op, Op.CreateTextNode, "second op is CreateTextNode");
		assert(ct.text, "Count: 0", "text is 'Count: 0'");

		const ac = mutations[2] as MutationAppendChildren;
		assert(ac.op, Op.AppendChildren, "third op is AppendChildren");

		const el = mutations[3] as MutationNewEventListener;
		assert(el.op, Op.NewEventListener, "fourth op is NewEventListener");
		assert(el.name, "click", "listener name is 'click'");
		assert(typeof el.handlerId, "number", "handlerId is a number");

		assert(mutations[4].op, Op.PushRoot, "fifth op is PushRoot");

		freeBuf(fns, buf);
	}

	// ── Composite test sequence ──────────────────────────────────────
	suite("Protocol — composite test sequence (write_test_sequence)");
	{
		const buf = allocBuf(fns);
		const totalBytes = fns.write_test_sequence(buf);

		const mutations = readerAt(buf, totalBytes).readAll();
		assert(
			mutations.length,
			4,
			"test sequence has 4 mutations (End not counted)",
		);

		const m0 = mutations[0] as MutationLoadTemplate;
		assert(m0.op, Op.LoadTemplate, "seq[0] is LoadTemplate");
		assert(m0.tmplId, 1, "seq[0] tmplId=1");
		assert(m0.index, 0, "seq[0] index=0");
		assert(m0.id, 10, "seq[0] id=10");

		const m1 = mutations[1] as MutationCreateTextNode;
		assert(m1.op, Op.CreateTextNode, "seq[1] is CreateTextNode");
		assert(m1.id, 11, "seq[1] id=11");
		assert(m1.text, "hello", "seq[1] text='hello'");

		const m2 = mutations[2] as MutationAppendChildren;
		assert(m2.op, Op.AppendChildren, "seq[2] is AppendChildren");
		assert(m2.id, 10, "seq[2] id=10");
		assert(m2.m, 1, "seq[2] m=1");

		const m3 = mutations[3] as MutationPushRoot;
		assert(m3.op, Op.PushRoot, "seq[3] is PushRoot");
		assert(m3.id, 10, "seq[3] id=10");

		freeBuf(fns, buf);
	}

	// ── Edge case: max u32 values ────────────────────────────────────
	suite("Protocol — max u32 values");
	{
		const buf = allocBuf(fns);
		// 0xFFFFFFFF = 4294967295, but Int32 in Mojo is signed.
		// Max signed Int32 is 2147483647 (0x7FFFFFFF).
		// Let's test with the max positive Int32 value.
		const maxI32 = 0x7fffffff;
		const off = fns.write_op_append_children(buf, 0, maxI32, maxI32);

		const m = readOne(buf, off) as MutationAppendChildren;
		assert(m.id, maxI32, "id is max i32 (0x7FFFFFFF)");
		assert(m.m, maxI32, "m is max i32 (0x7FFFFFFF)");
		freeBuf(fns, buf);
	}

	// ── Edge case: zero IDs ──────────────────────────────────────────
	suite("Protocol — zero IDs");
	{
		const buf = allocBuf(fns);
		const off = fns.write_op_push_root(buf, 0, 0);
		const m = readOne(buf, off) as MutationPushRoot;
		assert(m.id, 0, "id is 0");
		freeBuf(fns, buf);
	}

	// ── Offset threading ─────────────────────────────────────────────
	suite("Protocol — offset threading (non-zero start offset)");
	{
		const buf = allocBuf(fns);
		// Write first mutation at offset 0
		let off = 0;
		off = fns.write_op_push_root(buf, off, 1);
		const firstEnd = off;
		// Write second mutation at the returned offset
		off = fns.write_op_push_root(buf, off, 2);
		off = fns.write_op_end(buf, off);

		// Read all from the beginning
		const all = readerAt(buf, off).readAll();
		assert(all.length, 2, "two PushRoot mutations");
		assert((all[0] as MutationPushRoot).id, 1, "first PushRoot id=1");
		assert((all[1] as MutationPushRoot).id, 2, "second PushRoot id=2");

		// Read starting from the second mutation
		const second = readerAt(buf + BigInt(firstEnd), off - firstEnd).readAll();
		assert(second.length, 1, "one mutation from second offset");
		assert((second[0] as MutationPushRoot).id, 2, "second PushRoot id=2");

		freeBuf(fns, buf);
	}

	// ── Long string payload ──────────────────────────────────────────
	suite("Protocol — long string payload (1KB)");
	{
		const buf = allocBuf(fns);
		const longStr = "x".repeat(1024);
		const textPtr = writeStringStruct(longStr);
		const off = fns.write_op_create_text_node(buf, 0, 99, textPtr);
		// 1 + 4 + 4 + 1024 = 1033
		assert(off, 1033, "CreateTextNode with 1KB text writes 1033 bytes");

		const m = readOne(buf, off) as MutationCreateTextNode;
		assert(m.text.length, 1024, "decoded text has 1024 chars");
		assert(m.text, longStr, "decoded text matches original");
		freeBuf(fns, buf);
	}

	// ── All opcodes in one buffer ────────────────────────────────────
	suite("Protocol — all opcodes in one buffer");
	{
		const buf = allocBuf(fns);
		let off = 0;

		// 1. AppendChildren
		off = fns.write_op_append_children(buf, off, 1, 2);
		// 2. AssignId
		const { ptr: p1, len: l1 } = writePath([0]);
		off = fns.write_op_assign_id(buf, off, p1, l1, 3);
		// 3. CreatePlaceholder
		off = fns.write_op_create_placeholder(buf, off, 4);
		// 4. CreateTextNode
		const t1 = writeStringStruct("node");
		off = fns.write_op_create_text_node(buf, off, 5, t1);
		// 5. LoadTemplate
		off = fns.write_op_load_template(buf, off, 6, 0, 7);
		// 6. ReplaceWith
		off = fns.write_op_replace_with(buf, off, 8, 1);
		// 7. ReplacePlaceholder
		const { ptr: p2, len: l2 } = writePath([1, 0]);
		off = fns.write_op_replace_placeholder(buf, off, p2, l2, 2);
		// 8. InsertAfter
		off = fns.write_op_insert_after(buf, off, 9, 3);
		// 9. InsertBefore
		off = fns.write_op_insert_before(buf, off, 10, 1);
		// 10. SetAttribute
		const n1 = writeStringStruct("id");
		const v1 = writeStringStruct("main");
		off = fns.write_op_set_attribute(buf, off, 11, 0, n1, v1);
		// 11. SetText
		const t2 = writeStringStruct("text");
		off = fns.write_op_set_text(buf, off, 12, t2);
		// 12. NewEventListener
		const e1 = writeStringStruct("click");
		off = fns.write_op_new_event_listener(buf, off, 13, 0, e1);
		// 13. RemoveEventListener
		const e2 = writeStringStruct("click");
		off = fns.write_op_remove_event_listener(buf, off, 14, e2);
		// 14. Remove
		off = fns.write_op_remove(buf, off, 15);
		// 15. PushRoot
		off = fns.write_op_push_root(buf, off, 16);
		// 16. RemoveAttribute
		const ra1 = writeStringStruct("hidden");
		off = ext.write_op_remove_attribute(buf, off, 17, 0, ra1) as number;
		// End
		off = fns.write_op_end(buf, off);

		const all = readerAt(buf, off).readAll();
		assert(all.length, 16, "16 mutations (all opcodes)");

		assert(all[0].op, Op.AppendChildren, "op[0] AppendChildren");
		assert(all[1].op, Op.AssignId, "op[1] AssignId");
		assert(all[2].op, Op.CreatePlaceholder, "op[2] CreatePlaceholder");
		assert(all[3].op, Op.CreateTextNode, "op[3] CreateTextNode");
		assert(all[4].op, Op.LoadTemplate, "op[4] LoadTemplate");
		assert(all[5].op, Op.ReplaceWith, "op[5] ReplaceWith");
		assert(all[6].op, Op.ReplacePlaceholder, "op[6] ReplacePlaceholder");
		assert(all[7].op, Op.InsertAfter, "op[7] InsertAfter");
		assert(all[8].op, Op.InsertBefore, "op[8] InsertBefore");
		assert(all[9].op, Op.SetAttribute, "op[9] SetAttribute");
		assert(all[10].op, Op.SetText, "op[10] SetText");
		assert(all[11].op, Op.NewEventListener, "op[11] NewEventListener");
		assert(all[12].op, Op.RemoveEventListener, "op[12] RemoveEventListener");
		assert(all[13].op, Op.Remove, "op[13] Remove");
		assert(all[14].op, Op.PushRoot, "op[14] PushRoot");
		assert(all[15].op, Op.RemoveAttribute, "op[15] RemoveAttribute");

		// Spot-check a few payloads
		assert(
			(all[3] as MutationCreateTextNode).text,
			"node",
			"CreateTextNode text='node'",
		);
		assert(
			(all[9] as MutationSetAttribute).name,
			"id",
			"SetAttribute name='id'",
		);
		assert(
			(all[9] as MutationSetAttribute).value,
			"main",
			"SetAttribute value='main'",
		);
		assert(
			(all[11] as MutationNewEventListener).name,
			"click",
			"NewEventListener name='click'",
		);
		assert(
			(all[11] as MutationNewEventListener).handlerId,
			0,
			"NewEventListener handlerId=0",
		);
		assert(
			(all[15] as MutationRemoveAttribute).name,
			"hidden",
			"RemoveAttribute name='hidden'",
		);

		freeBuf(fns, buf);
	}

	// ── MutationReader: unknown opcode ───────────────────────────────
	suite("Protocol — MutationReader unknown opcode");
	{
		// Manually write an invalid opcode (0xFF) into a buffer
		const buf = allocBuf(fns);
		const mem = getMemory();
		new Uint8Array(mem.buffer, Number(buf), 1)[0] = 0xff;

		let threw = false;
		try {
			readerAt(buf, 1).next();
		} catch (_e) {
			threw = true;
		}
		assert(threw, true, "Unknown opcode 0xFF throws an error");
		freeBuf(fns, buf);
	}

	// ── MutationReader: readAll stops at End ─────────────────────────
	suite("Protocol — readAll stops at End, ignores trailing data");
	{
		const buf = allocBuf(fns);
		let off = 0;
		off = fns.write_op_push_root(buf, off, 1);
		off = fns.write_op_end(buf, off);
		// Write extra data after End (should be ignored)
		off = fns.write_op_push_root(buf, off, 999);

		const all = readerAt(buf, off).readAll();
		assert(all.length, 1, "readAll returns 1 mutation (stops at End)");
		assert((all[0] as MutationPushRoot).id, 1, "only mutation has id=1");
		freeBuf(fns, buf);
	}

	// ── RegisterTemplate: minimal (div > dyn_text[0]) ───────────────
	suite("Protocol — RegisterTemplate: minimal template");
	{
		// Create a runtime and build a template: div > dyn_text[0]
		const rt = ext.runtime_create() as bigint;
		const namePtr = writeStringStruct("min");
		const b = ext.tmpl_builder_create(namePtr) as bigint;
		ext.tmpl_builder_push_element(b, 0, -1); // div, root
		ext.tmpl_builder_push_dynamic_text(b, 0, 0); // dyn_text[0], child of div
		const tmplId = ext.tmpl_builder_register(rt, b) as number;
		ext.tmpl_builder_destroy(b);

		const buf = allocBuf(fns);
		const off = ext.write_op_register_template(buf, 0, rt, tmplId) as number;
		assert(off > 0, true, "wrote bytes");

		const m = readOne(buf, off) as MutationRegisterTemplate;
		assert(m !== null, true, "decoded a mutation");
		assert(m.op, Op.RegisterTemplate, "opcode is RegisterTemplate");
		assert(m.tmplId, tmplId, "template ID matches");
		assert(m.name, "min", "template name");
		assert(m.rootCount, 1, "1 root");
		assert(m.nodeCount, 2, "2 nodes (div + dyn_text)");
		assert(m.attrCount, 0, "0 attributes");

		// Node 0: Element(div)
		assert(m.nodes[0].kind, 0x00, "node[0] is element");
		const n0 = m.nodes[0] as {
			kind: 0x00;
			tag: number;
			children: number[];
			attrFirst: number;
			attrCount: number;
		};
		assert(n0.tag, 0, "node[0] tag is div (0)");
		assert(n0.children.length, 1, "node[0] has 1 child");
		assert(n0.children[0], 1, "node[0] child is index 1");
		assert(n0.attrCount, 0, "node[0] no attrs");

		// Node 1: DynamicText(index=0)
		assert(m.nodes[1].kind, 0x03, "node[1] is dynamic text");
		const n1 = m.nodes[1] as { kind: 0x03; dynamicIndex: number };
		assert(n1.dynamicIndex, 0, "node[1] dynamic index is 0");

		// Root indices
		assert(m.rootIndices.length, 1, "1 root index");
		assert(m.rootIndices[0], 0, "root[0] is node 0");

		freeBuf(fns, buf);
		ext.runtime_destroy(rt);
	}

	// ── RegisterTemplate: with text, static attr, dynamic attr ──────
	suite("Protocol — RegisterTemplate: text + attrs");
	{
		const rt = ext.runtime_create() as bigint;
		const namePtr = writeStringStruct("ctr");
		const b = ext.tmpl_builder_create(namePtr) as bigint;
		// div > [span > dyn_text[0], button > text("+")]
		ext.tmpl_builder_push_element(b, 0, -1); // 0: div
		ext.tmpl_builder_push_element(b, 1, 0); // 1: span
		ext.tmpl_builder_push_element(b, 19, 0); // 2: button (TAG_BUTTON=19)
		ext.tmpl_builder_push_dynamic_text(b, 0, 1); // 3: dyn_text in span
		const plusPtr = writeStringStruct("+");
		ext.tmpl_builder_push_text(b, plusPtr, 2); // 4: text "+" in button
		// static attr on button: type="submit"
		const typePtr = writeStringStruct("type");
		const submitPtr = writeStringStruct("submit");
		ext.tmpl_builder_push_static_attr(b, 2, typePtr, submitPtr);
		// dynamic attr on button: index=0
		ext.tmpl_builder_push_dynamic_attr(b, 2, 0);
		const tmplId = ext.tmpl_builder_register(rt, b) as number;
		ext.tmpl_builder_destroy(b);

		const buf = allocBuf(fns);
		const off = ext.write_op_register_template(buf, 0, rt, tmplId) as number;

		const m = readOne(buf, off) as MutationRegisterTemplate;
		assert(m.op, Op.RegisterTemplate, "opcode");
		assert(m.name, "ctr", "name");
		assert(m.nodeCount, 5, "5 nodes");
		assert(m.attrCount, 2, "2 attrs (1 static + 1 dynamic)");

		// Node 0: div with 2 children [1, 2]
		const n0 = m.nodes[0] as { kind: 0x00; children: number[] };
		assert(n0.kind, 0x00, "n0 element");
		assert(n0.children.length, 2, "n0 has 2 children");
		assert(n0.children[0], 1, "n0 child[0] is span");
		assert(n0.children[1], 2, "n0 child[1] is button");

		// Node 3: DynamicText
		assert(m.nodes[3].kind, 0x03, "n3 is dynamic text");

		// Node 4: Text("+")
		assert(m.nodes[4].kind, 0x01, "n4 is text");
		const n4 = m.nodes[4] as { kind: 0x01; text: string };
		assert(n4.text, "+", "n4 text is '+'");

		// Attr 0: Static("type", "submit")
		assert(m.attrs[0].kind, 0x00, "a0 is static");
		const a0 = m.attrs[0] as { kind: 0x00; name: string; value: string };
		assert(a0.name, "type", "a0 name");
		assert(a0.value, "submit", "a0 value");

		// Attr 1: Dynamic(index=0)
		assert(m.attrs[1].kind, 0x01, "a1 is dynamic");
		const a1 = m.attrs[1] as { kind: 0x01; dynamicIndex: number };
		assert(a1.dynamicIndex, 0, "a1 dynamic index");

		freeBuf(fns, buf);
		ext.runtime_destroy(rt);
	}

	// ── RegisterTemplate: Dynamic node (not DynamicText) ────────────
	suite("Protocol — RegisterTemplate: dynamic node slot");
	{
		const rt = ext.runtime_create() as bigint;
		const namePtr = writeStringStruct("dyn");
		const b = ext.tmpl_builder_create(namePtr) as bigint;
		ext.tmpl_builder_push_element(b, 0, -1); // 0: div
		ext.tmpl_builder_push_dynamic(b, 0, 0); // 1: dynamic[0]
		const tmplId = ext.tmpl_builder_register(rt, b) as number;
		ext.tmpl_builder_destroy(b);

		const buf = allocBuf(fns);
		const off = ext.write_op_register_template(buf, 0, rt, tmplId) as number;

		const m = readOne(buf, off) as MutationRegisterTemplate;
		assert(m.nodeCount, 2, "2 nodes");
		assert(m.nodes[1].kind, 0x02, "node[1] is Dynamic");
		const n1 = m.nodes[1] as { kind: 0x02; dynamicIndex: number };
		assert(n1.dynamicIndex, 0, "dynamic index is 0");

		freeBuf(fns, buf);
		ext.runtime_destroy(rt);
	}

	// ── RegisterTemplate: readAll includes RegisterTemplate ─────────
	suite("Protocol — RegisterTemplate in readAll sequence");
	{
		const rt = ext.runtime_create() as bigint;
		const namePtr = writeStringStruct("seq");
		const b = ext.tmpl_builder_create(namePtr) as bigint;
		ext.tmpl_builder_push_element(b, 0, -1); // div
		const tmplId = ext.tmpl_builder_register(rt, b) as number;
		ext.tmpl_builder_destroy(b);

		const buf = allocBuf(fns);
		let off = 0;
		off = ext.write_op_register_template(buf, off, rt, tmplId) as number;
		off = fns.write_op_push_root(buf, off, 1);
		off = fns.write_op_end(buf, off);

		const all = readerAt(buf, off).readAll();
		assert(all.length, 2, "readAll returns 2 mutations");
		assert(all[0].op, Op.RegisterTemplate, "first is RegisterTemplate");
		assert(all[1].op, Op.PushRoot, "second is PushRoot");

		freeBuf(fns, buf);
		ext.runtime_destroy(rt);
	}
}
