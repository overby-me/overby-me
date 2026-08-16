import { getMemory } from "../runtime/memory.ts";
import type { Mutation } from "../runtime/protocol.ts";
import { MutationReader, Op } from "../runtime/protocol.ts";
import { writeStringStruct } from "../runtime/strings.ts";
import type { WasmExports } from "../runtime/types.ts";
import { assert, suite } from "./harness.ts";

type Fns = WasmExports & Record<string, unknown>;

// ── Constants (must match Mojo) ─────────────────────────────────────────────

const TAG_DIV = 0;
const TAG_P = 2;
const TAG_H1 = 10;
const TAG_BUTTON = 19;

const BUF_SIZE = 8192;

// ── Helpers ─────────────────────────────────────────────────────────────────

/** Allocate a mutation buffer via WASM. */
function allocBuf(fns: Fns): bigint {
	return fns.mutation_buf_alloc(BUF_SIZE);
}

function freeBuf(fns: Fns, ptr: bigint): void {
	fns.mutation_buf_free(ptr);
}

/** Read all mutations from buffer [ptr, ptr+len). */
function readMutations(ptr: bigint, len: number): Mutation[] {
	const mem = getMemory();
	const reader = new MutationReader(mem.buffer, Number(ptr), len);
	return reader.readAll();
}

/** Create a runtime + eid allocator + vnode store + writer context. */
function createTestContext(fns: Fns) {
	const rt = fns.runtime_create();
	const eid = fns.eid_alloc_create();
	const store = fns.vnode_store_create();
	const buf = allocBuf(fns);
	const writer = fns.writer_create(buf, BUF_SIZE);
	return { rt, eid, store, buf, writer };
}

function destroyTestContext(
	fns: Fns,
	ctx: { rt: bigint; eid: bigint; store: bigint; buf: bigint; writer: bigint },
) {
	fns.writer_destroy(ctx.writer);
	freeBuf(fns, ctx.buf);
	fns.vnode_store_destroy(ctx.store);
	fns.eid_alloc_destroy(ctx.eid);
	fns.runtime_destroy(ctx.rt);
}

/**
 * Register a simple div template: <div></div>
 * Returns the template ID.
 */
function registerDivTemplate(fns: Fns, rt: bigint, name: string): number {
	const namePtr = writeStringStruct(name);
	const builder = fns.tmpl_builder_create(namePtr);
	fns.tmpl_builder_push_element(builder, TAG_DIV, -1);
	const tmplId = fns.tmpl_builder_register(rt, builder);
	fns.tmpl_builder_destroy(builder);
	return tmplId;
}

/**
 * Register a div template with a dynamic text child:
 *   <div>{dynamic_text_0}</div>
 * Returns the template ID.
 */
function registerDivWithDynText(fns: Fns, rt: bigint, name: string): number {
	const namePtr = writeStringStruct(name);
	const builder = fns.tmpl_builder_create(namePtr);
	const divIdx = fns.tmpl_builder_push_element(builder, TAG_DIV, -1);
	fns.tmpl_builder_push_dynamic_text(builder, 0, divIdx);
	const tmplId = fns.tmpl_builder_register(rt, builder);
	fns.tmpl_builder_destroy(builder);
	return tmplId;
}

/**
 * Register a div template with a dynamic attribute:
 *   <div {dynamic_attr_0}></div>
 * Returns the template ID.
 */
function registerDivWithDynAttr(fns: Fns, rt: bigint, name: string): number {
	const namePtr = writeStringStruct(name);
	const builder = fns.tmpl_builder_create(namePtr);
	const divIdx = fns.tmpl_builder_push_element(builder, TAG_DIV, -1);
	fns.tmpl_builder_push_dynamic_attr(builder, divIdx, 0);
	const tmplId = fns.tmpl_builder_register(rt, builder);
	fns.tmpl_builder_destroy(builder);
	return tmplId;
}

/**
 * Register a div template with static text child:
 *   <div>"hello"</div>
 * Returns the template ID.
 */
function registerDivWithStaticText(fns: Fns, rt: bigint, name: string): number {
	const namePtr = writeStringStruct(name);
	const builder = fns.tmpl_builder_create(namePtr);
	const divIdx = fns.tmpl_builder_push_element(builder, TAG_DIV, -1);
	const textStr = writeStringStruct("hello");
	fns.tmpl_builder_push_text(builder, textStr, divIdx);
	const tmplId = fns.tmpl_builder_register(rt, builder);
	fns.tmpl_builder_destroy(builder);
	return tmplId;
}

/**
 * Register a div template with a Dynamic (full) node slot:
 *   <div>{dynamic_node_0}</div>
 * Returns the template ID.
 */
function registerDivWithDynNode(fns: Fns, rt: bigint, name: string): number {
	const namePtr = writeStringStruct(name);
	const builder = fns.tmpl_builder_create(namePtr);
	const divIdx = fns.tmpl_builder_push_element(builder, TAG_DIV, -1);
	fns.tmpl_builder_push_dynamic(builder, 0, divIdx);
	const tmplId = fns.tmpl_builder_register(rt, builder);
	fns.tmpl_builder_destroy(builder);
	return tmplId;
}

/**
 * Register a more complex template:
 *   <div class="container">
 *     <h1>"Title"</h1>
 *     <p>{dynamic_text_0}</p>
 *     <button {dynamic_attr_0}>{dynamic_text_1}</button>
 *   </div>
 */
function registerComplexTemplate(fns: Fns, rt: bigint, name: string): number {
	const namePtr = writeStringStruct(name);
	const builder = fns.tmpl_builder_create(namePtr);

	// Root div
	const divIdx = fns.tmpl_builder_push_element(builder, TAG_DIV, -1);
	const classStr = writeStringStruct("class");
	const containerStr = writeStringStruct("container");
	fns.tmpl_builder_push_static_attr(builder, divIdx, classStr, containerStr);

	// h1 with static text
	const h1Idx = fns.tmpl_builder_push_element(builder, TAG_H1, divIdx);
	const titleStr = writeStringStruct("Title");
	fns.tmpl_builder_push_text(builder, titleStr, h1Idx);

	// p with dynamic text 0
	const pIdx = fns.tmpl_builder_push_element(builder, TAG_P, divIdx);
	fns.tmpl_builder_push_dynamic_text(builder, 0, pIdx);

	// button with dynamic attr 0 and dynamic text 1
	const btnIdx = fns.tmpl_builder_push_element(builder, TAG_BUTTON, divIdx);
	fns.tmpl_builder_push_dynamic_attr(builder, btnIdx, 0);
	fns.tmpl_builder_push_dynamic_text(builder, 1, btnIdx);

	const tmplId = fns.tmpl_builder_register(rt, builder);
	fns.tmpl_builder_destroy(builder);
	return tmplId;
}

// ── Tests ───────────────────────────────────────────────────────────────────

export function testMutations(fns: WasmExports): void {
	const ext = fns as Fns;

	// ═══════════════════════════════════════════════════════════════════
	// CREATE ENGINE TESTS
	// ═══════════════════════════════════════════════════════════════════

	// ── Create: Text VNode ──────────────────────────────────────────
	suite("Create — Text VNode");
	{
		const ctx = createTestContext(ext);

		const textStr = writeStringStruct("hello world");
		const vnIdx = ext.vnode_push_text(ctx.store, textStr);

		const numRoots = ext.create_vnode(
			ctx.writer,
			ctx.eid,
			ctx.rt,
			ctx.store,
			vnIdx,
		);
		const offset = ext.writer_finalize(ctx.writer);

		assert(numRoots, 1, "text vnode creates 1 root");

		const mutations = readMutations(ctx.buf, offset);
		assert(mutations.length, 1, "1 mutation emitted for text vnode");
		assert(mutations[0].op, Op.CreateTextNode, "mutation is CreateTextNode");
		if (mutations[0].op === Op.CreateTextNode) {
			assert(mutations[0].text, "hello world", "text content is correct");
			assert(mutations[0].id > 0, true, "element id is non-zero");
		}

		// Check mount state
		assert(ext.vnode_is_mounted(ctx.store, vnIdx), 1, "text vnode is mounted");
		assert(
			ext.vnode_root_id_count(ctx.store, vnIdx),
			1,
			"text vnode has 1 root id",
		);
		const rootId = ext.vnode_get_root_id(ctx.store, vnIdx, 0);
		assert(rootId > 0, true, "root id is non-zero");

		destroyTestContext(ext, ctx);
	}

	// ── Create: Placeholder VNode ───────────────────────────────────
	suite("Create — Placeholder VNode");
	{
		const ctx = createTestContext(ext);

		const vnIdx = ext.vnode_push_placeholder(ctx.store, 0);

		const numRoots = ext.create_vnode(
			ctx.writer,
			ctx.eid,
			ctx.rt,
			ctx.store,
			vnIdx,
		);
		const offset = ext.writer_finalize(ctx.writer);

		assert(numRoots, 1, "placeholder creates 1 root");

		const mutations = readMutations(ctx.buf, offset);
		assert(mutations.length, 1, "1 mutation for placeholder");
		assert(
			mutations[0].op,
			Op.CreatePlaceholder,
			"mutation is CreatePlaceholder",
		);

		assert(ext.vnode_is_mounted(ctx.store, vnIdx), 1, "placeholder is mounted");
		assert(
			ext.vnode_root_id_count(ctx.store, vnIdx),
			1,
			"placeholder has 1 root id",
		);

		destroyTestContext(ext, ctx);
	}

	// ── Create: Simple TemplateRef (no dynamic content) ─────────────
	suite("Create — Simple TemplateRef (static div)");
	{
		const ctx = createTestContext(ext);

		const tmplId = registerDivTemplate(ext, ctx.rt, "simple-div");
		const vnIdx = ext.vnode_push_template_ref(ctx.store, tmplId);

		const numRoots = ext.create_vnode(
			ctx.writer,
			ctx.eid,
			ctx.rt,
			ctx.store,
			vnIdx,
		);
		const offset = ext.writer_finalize(ctx.writer);

		assert(numRoots, 1, "single root template creates 1 root");

		const mutations = readMutations(ctx.buf, offset);
		assert(mutations.length, 1, "1 mutation for static template");
		assert(mutations[0].op, Op.LoadTemplate, "mutation is LoadTemplate");
		if (mutations[0].op === Op.LoadTemplate) {
			assert(mutations[0].tmplId, tmplId, "correct template id");
			assert(mutations[0].index, 0, "root index 0");
			assert(mutations[0].id > 0, true, "assigned element id > 0");
		}

		assert(
			ext.vnode_root_id_count(ctx.store, vnIdx),
			1,
			"template ref has 1 root id",
		);

		destroyTestContext(ext, ctx);
	}

	// ── Create: TemplateRef with dynamic text ───────────────────────
	suite("Create — TemplateRef with DynamicText");
	{
		const ctx = createTestContext(ext);

		const tmplId = registerDivWithDynText(ext, ctx.rt, "dyn-text-div");
		const vnIdx = ext.vnode_push_template_ref(ctx.store, tmplId);

		// Add a dynamic text node with content "Count: 5"
		const dynTextStr = writeStringStruct("Count: 5");
		ext.vnode_push_dynamic_text_node(ctx.store, vnIdx, dynTextStr);

		const numRoots = ext.create_vnode(
			ctx.writer,
			ctx.eid,
			ctx.rt,
			ctx.store,
			vnIdx,
		);
		const offset = ext.writer_finalize(ctx.writer);

		assert(numRoots, 1, "1 root for template with dyn text");

		const mutations = readMutations(ctx.buf, offset);

		// Expect: LoadTemplate, AssignId (to dynamic text position), SetText
		assert(mutations.length >= 2, true, "at least 2 mutations for dyn text");

		// First should be LoadTemplate
		assert(mutations[0].op, Op.LoadTemplate, "first mutation is LoadTemplate");

		// Should have a dyn node ID assigned
		assert(
			ext.vnode_dyn_node_id_count(ctx.store, vnIdx),
			1,
			"1 dynamic node id assigned",
		);
		const dynNodeId = ext.vnode_get_dyn_node_id(ctx.store, vnIdx, 0);
		assert(dynNodeId > 0, true, "dynamic node id is non-zero");

		// Verify that one of the mutations is AssignId and one is SetText
		let hasAssignId = false;
		let hasSetText = false;
		for (const m of mutations) {
			if (m.op === Op.AssignId) hasAssignId = true;
			if (m.op === Op.SetText) {
				hasSetText = true;
				assert(m.text, "Count: 5", "SetText has correct content");
			}
		}
		assert(hasAssignId, true, "has AssignId mutation for dyn text");
		assert(hasSetText, true, "has SetText mutation for dyn text content");

		destroyTestContext(ext, ctx);
	}

	// ── Create: TemplateRef with dynamic attribute ──────────────────
	suite("Create — TemplateRef with dynamic attribute");
	{
		const ctx = createTestContext(ext);

		const tmplId = registerDivWithDynAttr(ext, ctx.rt, "dyn-attr-div");
		const vnIdx = ext.vnode_push_template_ref(ctx.store, tmplId);

		// Add a dynamic text attribute: class="active"
		const nameStr = writeStringStruct("class");
		const valueStr = writeStringStruct("active");
		ext.vnode_push_dynamic_attr_text(ctx.store, vnIdx, nameStr, valueStr, 0);

		const numRoots = ext.create_vnode(
			ctx.writer,
			ctx.eid,
			ctx.rt,
			ctx.store,
			vnIdx,
		);
		const offset = ext.writer_finalize(ctx.writer);

		assert(numRoots, 1, "1 root for template with dyn attr");

		const mutations = readMutations(ctx.buf, offset);

		// Expect: LoadTemplate, AssignId (to element with dyn attr), SetAttribute
		assert(mutations.length >= 2, true, "at least 2 mutations for dyn attr");
		assert(mutations[0].op, Op.LoadTemplate, "first is LoadTemplate");

		// Check dyn attr id was assigned
		assert(
			ext.vnode_dyn_attr_id_count(ctx.store, vnIdx),
			1,
			"1 dynamic attr id",
		);
		const dynAttrId = ext.vnode_get_dyn_attr_id(ctx.store, vnIdx, 0);
		assert(dynAttrId > 0, true, "dynamic attr id is non-zero");

		// Verify SetAttribute mutation exists
		let hasSetAttr = false;
		for (const m of mutations) {
			if (m.op === Op.SetAttribute) {
				hasSetAttr = true;
				assert(m.name, "class", "attribute name is 'class'");
				assert(m.value, "active", "attribute value is 'active'");
			}
		}
		assert(hasSetAttr, true, "has SetAttribute mutation");

		destroyTestContext(ext, ctx);
	}

	// ── Create: TemplateRef with Dynamic node (text) ────────────────
	suite("Create — TemplateRef with Dynamic node (text replacement)");
	{
		const ctx = createTestContext(ext);

		const tmplId = registerDivWithDynNode(ext, ctx.rt, "dyn-node-div");
		const vnIdx = ext.vnode_push_template_ref(ctx.store, tmplId);

		// Add a dynamic text node
		const dynStr = writeStringStruct("inserted text");
		ext.vnode_push_dynamic_text_node(ctx.store, vnIdx, dynStr);

		const numRoots = ext.create_vnode(
			ctx.writer,
			ctx.eid,
			ctx.rt,
			ctx.store,
			vnIdx,
		);
		const offset = ext.writer_finalize(ctx.writer);

		assert(numRoots, 1, "1 root for template with dyn node");

		const mutations = readMutations(ctx.buf, offset);

		// Expect: LoadTemplate, CreateTextNode, ReplacePlaceholder
		assert(mutations.length >= 3, true, "at least 3 mutations for dyn node");
		assert(mutations[0].op, Op.LoadTemplate, "first is LoadTemplate");

		let hasCreateText = false;
		let hasReplacePlaceholder = false;
		for (const m of mutations) {
			if (m.op === Op.CreateTextNode) {
				hasCreateText = true;
				assert(m.text, "inserted text", "created text content correct");
			}
			if (m.op === Op.ReplacePlaceholder) {
				hasReplacePlaceholder = true;
				assert(m.m, 1, "replaces with 1 node");
			}
		}
		assert(hasCreateText, true, "has CreateTextNode for dyn node");
		assert(hasReplacePlaceholder, true, "has ReplacePlaceholder for dyn node");

		// Check mount state
		assert(
			ext.vnode_dyn_node_id_count(ctx.store, vnIdx),
			1,
			"1 dyn node id assigned",
		);

		destroyTestContext(ext, ctx);
	}

	// ── Create: TemplateRef with Dynamic placeholder node ───────────
	suite("Create — TemplateRef with Dynamic placeholder node");
	{
		const ctx = createTestContext(ext);

		const tmplId = registerDivWithDynNode(ext, ctx.rt, "dyn-ph-div");
		const vnIdx = ext.vnode_push_template_ref(ctx.store, tmplId);

		// Add a dynamic placeholder node
		ext.vnode_push_dynamic_placeholder(ctx.store, vnIdx);

		const numRoots = ext.create_vnode(
			ctx.writer,
			ctx.eid,
			ctx.rt,
			ctx.store,
			vnIdx,
		);
		const offset = ext.writer_finalize(ctx.writer);

		assert(numRoots, 1, "1 root");

		const mutations = readMutations(ctx.buf, offset);

		let hasCreatePlaceholder = false;
		let hasReplacePlaceholder = false;
		for (const m of mutations) {
			if (m.op === Op.CreatePlaceholder) hasCreatePlaceholder = true;
			if (m.op === Op.ReplacePlaceholder) hasReplacePlaceholder = true;
		}
		assert(hasCreatePlaceholder, true, "has CreatePlaceholder for dyn node");
		assert(hasReplacePlaceholder, true, "has ReplacePlaceholder for dyn node");

		destroyTestContext(ext, ctx);
	}

	// ── Create: Fragment VNode ──────────────────────────────────────
	suite("Create — Fragment VNode");
	{
		const ctx = createTestContext(ext);

		// Create a fragment with 3 text children
		const fragIdx = ext.vnode_push_fragment(ctx.store);
		const t1Str = writeStringStruct("alpha");
		const t2Str = writeStringStruct("beta");
		const t3Str = writeStringStruct("gamma");
		const c1 = ext.vnode_push_text(ctx.store, t1Str);
		const c2 = ext.vnode_push_text(ctx.store, t2Str);
		const c3 = ext.vnode_push_text(ctx.store, t3Str);
		ext.vnode_push_fragment_child(ctx.store, fragIdx, c1);
		ext.vnode_push_fragment_child(ctx.store, fragIdx, c2);
		ext.vnode_push_fragment_child(ctx.store, fragIdx, c3);

		const numRoots = ext.create_vnode(
			ctx.writer,
			ctx.eid,
			ctx.rt,
			ctx.store,
			fragIdx,
		);
		const offset = ext.writer_finalize(ctx.writer);

		assert(numRoots, 3, "fragment with 3 text children creates 3 roots");

		const mutations = readMutations(ctx.buf, offset);
		assert(mutations.length, 3, "3 CreateTextNode mutations");
		for (let i = 0; i < 3; i++) {
			assert(
				mutations[i].op,
				Op.CreateTextNode,
				`fragment child ${i} is CreateTextNode`,
			);
		}
		if (mutations[0].op === Op.CreateTextNode)
			assert(mutations[0].text, "alpha", "first child text");
		if (mutations[1].op === Op.CreateTextNode)
			assert(mutations[1].text, "beta", "second child text");
		if (mutations[2].op === Op.CreateTextNode)
			assert(mutations[2].text, "gamma", "third child text");

		// Each child should be mounted
		assert(ext.vnode_is_mounted(ctx.store, c1), 1, "child 1 mounted");
		assert(ext.vnode_is_mounted(ctx.store, c2), 1, "child 2 mounted");
		assert(ext.vnode_is_mounted(ctx.store, c3), 1, "child 3 mounted");

		destroyTestContext(ext, ctx);
	}

	// ── Create: TemplateRef with event handler ──────────────────────
	suite("Create — TemplateRef with event handler attribute");
	{
		const ctx = createTestContext(ext);

		const tmplId = registerDivWithDynAttr(ext, ctx.rt, "event-div");
		const vnIdx = ext.vnode_push_template_ref(ctx.store, tmplId);

		// Add a dynamic event attribute: onclick
		const nameStr = writeStringStruct("click");
		ext.vnode_push_dynamic_attr_event(ctx.store, vnIdx, nameStr, 42, 0);

		const numRoots = ext.create_vnode(
			ctx.writer,
			ctx.eid,
			ctx.rt,
			ctx.store,
			vnIdx,
		);
		const offset = ext.writer_finalize(ctx.writer);

		assert(numRoots, 1, "1 root");

		const mutations = readMutations(ctx.buf, offset);

		let hasNewEventListener = false;
		for (const m of mutations) {
			if (m.op === Op.NewEventListener) {
				hasNewEventListener = true;
				assert(m.name, "click", "event name is 'click'");
			}
		}
		assert(hasNewEventListener, true, "has NewEventListener mutation");

		destroyTestContext(ext, ctx);
	}

	// ── Create: Mount state ElementId uniqueness ────────────────────
	suite("Create — ElementId uniqueness across multiple creates");
	{
		const ctx = createTestContext(ext);

		const tmplId = registerDivTemplate(ext, ctx.rt, "eid-uniq-div");

		const vn1 = ext.vnode_push_template_ref(ctx.store, tmplId);
		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, vn1);

		const vn2 = ext.vnode_push_template_ref(ctx.store, tmplId);
		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, vn2);

		const vn3 = ext.vnode_push_template_ref(ctx.store, tmplId);
		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, vn3);

		const id1 = ext.vnode_get_root_id(ctx.store, vn1, 0);
		const id2 = ext.vnode_get_root_id(ctx.store, vn2, 0);
		const id3 = ext.vnode_get_root_id(ctx.store, vn3, 0);

		assert(id1 !== id2, true, "vnode 1 and 2 have different ids");
		assert(id2 !== id3, true, "vnode 2 and 3 have different ids");
		assert(id1 !== id3, true, "vnode 1 and 3 have different ids");

		destroyTestContext(ext, ctx);
	}

	// ═══════════════════════════════════════════════════════════════════
	// DIFF ENGINE TESTS
	// ═══════════════════════════════════════════════════════════════════

	// ── Diff: Same text — 0 mutations ───────────────────────────────
	suite("Diff — Same text produces 0 mutations");
	{
		const ctx = createTestContext(ext);

		const t1 = writeStringStruct("hello");
		const t2 = writeStringStruct("hello");
		const oldIdx = ext.vnode_push_text(ctx.store, t1);
		const newIdx = ext.vnode_push_text(ctx.store, t2);

		// Create old
		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		// Reset writer for diff
		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);
		assert(mutations.length, 0, "same text → 0 mutations");

		// New node should inherit mount state
		assert(ext.vnode_is_mounted(ctx.store, newIdx), 1, "new text is mounted");

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Diff: Text changed — 1 SetText mutation ─────────────────────
	suite("Diff — Text changed produces SetText");
	{
		const ctx = createTestContext(ext);

		const t1 = writeStringStruct("hello");
		const t2 = writeStringStruct("world");
		const oldIdx = ext.vnode_push_text(ctx.store, t1);
		const newIdx = ext.vnode_push_text(ctx.store, t2);

		// Create old
		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		// Diff
		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);
		assert(mutations.length, 1, "text change → 1 mutation");
		assert(mutations[0].op, Op.SetText, "mutation is SetText");
		if (mutations[0].op === Op.SetText) {
			assert(mutations[0].text, "world", "new text is 'world'");
			// The target id should be the old element's id
			const oldRootId = ext.vnode_get_root_id(ctx.store, oldIdx, 0);
			assert(mutations[0].id, oldRootId, "SetText targets old element");
		}

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Diff: Text empty to non-empty ───────────────────────────────
	suite("Diff — Text '' → 'hello' produces SetText");
	{
		const ctx = createTestContext(ext);

		const t1 = writeStringStruct("");
		const t2 = writeStringStruct("hello");
		const oldIdx = ext.vnode_push_text(ctx.store, t1);
		const newIdx = ext.vnode_push_text(ctx.store, t2);

		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);
		assert(mutations.length, 1, "empty→hello → 1 mutation");
		assert(mutations[0].op, Op.SetText, "mutation is SetText");
		if (mutations[0].op === Op.SetText) {
			assert(mutations[0].text, "hello", "SetText content correct");
		}

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Diff: Placeholder → Placeholder — 0 mutations ───────────────
	suite("Diff — Placeholder → Placeholder produces 0 mutations");
	{
		const ctx = createTestContext(ext);

		const oldIdx = ext.vnode_push_placeholder(ctx.store, 0);
		const newIdx = ext.vnode_push_placeholder(ctx.store, 0);

		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);
		assert(mutations.length, 0, "placeholder→placeholder → 0 mutations");

		assert(
			ext.vnode_is_mounted(ctx.store, newIdx),
			1,
			"new placeholder is mounted",
		);

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Diff: Same TemplateRef, same dynamic values — 0 mutations ───
	suite("Diff — Same template, same dynamic values → 0 mutations");
	{
		const ctx = createTestContext(ext);

		const tmplId = registerDivWithDynText(ext, ctx.rt, "same-dyn-div");

		const oldIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const dt1 = writeStringStruct("hello");
		ext.vnode_push_dynamic_text_node(ctx.store, oldIdx, dt1);

		// Create old
		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		const newIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const dt2 = writeStringStruct("hello");
		ext.vnode_push_dynamic_text_node(ctx.store, newIdx, dt2);

		// Diff
		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);
		assert(mutations.length, 0, "same template + same values → 0 mutations");

		// New node should inherit mount state
		assert(
			ext.vnode_root_id_count(ctx.store, newIdx),
			1,
			"new node has root id",
		);
		assert(
			ext.vnode_dyn_node_id_count(ctx.store, newIdx),
			1,
			"new node has dyn node id",
		);

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Diff: Same TemplateRef, dynamic text changed ────────────────
	suite("Diff — Same template, dynamic text changed → SetText");
	{
		const ctx = createTestContext(ext);

		const tmplId = registerDivWithDynText(ext, ctx.rt, "chg-dyn-div");

		const oldIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const dt1 = writeStringStruct("old text");
		ext.vnode_push_dynamic_text_node(ctx.store, oldIdx, dt1);

		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		const newIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const dt2 = writeStringStruct("new text");
		ext.vnode_push_dynamic_text_node(ctx.store, newIdx, dt2);

		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);
		assert(mutations.length, 1, "dynamic text change → 1 mutation");
		assert(mutations[0].op, Op.SetText, "mutation is SetText");
		if (mutations[0].op === Op.SetText) {
			assert(mutations[0].text, "new text", "SetText has new content");
			// Should target the dyn_node_id from the old VNode
			const oldDynId = ext.vnode_get_dyn_node_id(ctx.store, oldIdx, 0);
			assert(mutations[0].id, oldDynId, "targets old dynamic node element");
		}

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Diff: Same TemplateRef, dynamic attribute changed ───────────
	suite("Diff — Same template, dynamic attribute changed → SetAttribute");
	{
		const ctx = createTestContext(ext);

		const tmplId = registerDivWithDynAttr(ext, ctx.rt, "chg-attr-div");

		const oldIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const n1 = writeStringStruct("class");
		const v1 = writeStringStruct("old-class");
		ext.vnode_push_dynamic_attr_text(ctx.store, oldIdx, n1, v1, 0);

		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		const newIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const n2 = writeStringStruct("class");
		const v2 = writeStringStruct("new-class");
		ext.vnode_push_dynamic_attr_text(ctx.store, newIdx, n2, v2, 0);

		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);
		assert(mutations.length, 1, "attr change → 1 mutation");
		assert(mutations[0].op, Op.SetAttribute, "mutation is SetAttribute");
		if (mutations[0].op === Op.SetAttribute) {
			assert(mutations[0].name, "class", "attr name is 'class'");
			assert(mutations[0].value, "new-class", "attr value is 'new-class'");
		}

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Diff: Same TemplateRef, attribute unchanged — 0 mutations ───
	suite("Diff — Same template, attribute unchanged → 0 mutations");
	{
		const ctx = createTestContext(ext);

		const tmplId = registerDivWithDynAttr(ext, ctx.rt, "same-attr-div");

		const oldIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const n1 = writeStringStruct("class");
		const v1 = writeStringStruct("stable");
		ext.vnode_push_dynamic_attr_text(ctx.store, oldIdx, n1, v1, 0);

		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		const newIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const n2 = writeStringStruct("class");
		const v2 = writeStringStruct("stable");
		ext.vnode_push_dynamic_attr_text(ctx.store, newIdx, n2, v2, 0);

		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);
		assert(mutations.length, 0, "unchanged attr → 0 mutations");

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Diff: Bool attribute changed ────────────────────────────────
	suite("Diff — Bool attribute changed");
	{
		const ctx = createTestContext(ext);

		const tmplId = registerDivWithDynAttr(ext, ctx.rt, "bool-attr-div");

		const oldIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const n1 = writeStringStruct("disabled");
		ext.vnode_push_dynamic_attr_bool(ctx.store, oldIdx, n1, 0, 0);

		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		const newIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const n2 = writeStringStruct("disabled");
		ext.vnode_push_dynamic_attr_bool(ctx.store, newIdx, n2, 1, 0);

		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);
		assert(mutations.length, 1, "bool change → 1 mutation");
		assert(mutations[0].op, Op.SetAttribute, "mutation is SetAttribute");
		if (mutations[0].op === Op.SetAttribute) {
			assert(mutations[0].name, "disabled", "attr name is 'disabled'");
			assert(mutations[0].value, "true", "attr value is 'true'");
		}

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Diff: Text → Placeholder (different kind) — replacement ─────
	suite("Diff — Text → Placeholder (different kind) → replacement");
	{
		const ctx = createTestContext(ext);

		const t1 = writeStringStruct("some text");
		const oldIdx = ext.vnode_push_text(ctx.store, t1);
		const newIdx = ext.vnode_push_placeholder(ctx.store, 0);

		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);
		// Should have CreatePlaceholder and ReplaceWith
		assert(mutations.length >= 2, true, "replacement produces >= 2 mutations");

		let hasCreate = false;
		let hasReplace = false;
		for (const m of mutations) {
			if (m.op === Op.CreatePlaceholder) hasCreate = true;
			if (m.op === Op.ReplaceWith) hasReplace = true;
		}
		assert(hasCreate, true, "has CreatePlaceholder for new node");
		assert(hasReplace, true, "has ReplaceWith to swap old with new");

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Diff: Different templates — replacement ─────────────────────
	suite("Diff — Different templates → replacement");
	{
		const ctx = createTestContext(ext);

		const tmplA = registerDivTemplate(ext, ctx.rt, "tmpl-a");
		const tmplB = registerDivWithStaticText(ext, ctx.rt, "tmpl-b");

		const oldIdx = ext.vnode_push_template_ref(ctx.store, tmplA);
		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		const newIdx = ext.vnode_push_template_ref(ctx.store, tmplB);

		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);

		// Should have LoadTemplate (for new) and ReplaceWith (for swapping)
		let hasLoad = false;
		let hasReplace = false;
		for (const m of mutations) {
			if (m.op === Op.LoadTemplate) hasLoad = true;
			if (m.op === Op.ReplaceWith) hasReplace = true;
		}
		assert(hasLoad, true, "has LoadTemplate for new template");
		assert(hasReplace, true, "has ReplaceWith to swap templates");

		// New node should be mounted
		assert(
			ext.vnode_is_mounted(ctx.store, newIdx),
			1,
			"new template ref is mounted",
		);

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Diff: Fragment — same children count, text changed ──────────
	suite("Diff — Fragment children text changed");
	{
		const ctx = createTestContext(ext);

		// Old fragment: [text("a"), text("b")]
		const oldFrag = ext.vnode_push_fragment(ctx.store);
		const oa = writeStringStruct("a");
		const ob = writeStringStruct("b");
		const oaIdx = ext.vnode_push_text(ctx.store, oa);
		const obIdx = ext.vnode_push_text(ctx.store, ob);
		ext.vnode_push_fragment_child(ctx.store, oldFrag, oaIdx);
		ext.vnode_push_fragment_child(ctx.store, oldFrag, obIdx);

		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldFrag);

		// New fragment: [text("a"), text("c")]
		const newFrag = ext.vnode_push_fragment(ctx.store);
		const na = writeStringStruct("a");
		const nc = writeStringStruct("c");
		const naIdx = ext.vnode_push_text(ctx.store, na);
		const ncIdx = ext.vnode_push_text(ctx.store, nc);
		ext.vnode_push_fragment_child(ctx.store, newFrag, naIdx);
		ext.vnode_push_fragment_child(ctx.store, newFrag, ncIdx);

		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldFrag, newFrag);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);
		// "a" == "a" → no mutation, "b" → "c" → SetText
		assert(mutations.length, 1, "fragment diff → 1 SetText");
		assert(mutations[0].op, Op.SetText, "mutation is SetText");
		if (mutations[0].op === Op.SetText) {
			assert(mutations[0].text, "c", "updated text is 'c'");
		}

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Diff: Fragment — children removed ───────────────────────────
	suite("Diff — Fragment children removed");
	{
		const ctx = createTestContext(ext);

		// Old: [text("a"), text("b"), text("c")]
		const oldFrag = ext.vnode_push_fragment(ctx.store);
		const oa = writeStringStruct("a");
		const ob = writeStringStruct("b");
		const oc = writeStringStruct("c");
		const oaIdx = ext.vnode_push_text(ctx.store, oa);
		const obIdx = ext.vnode_push_text(ctx.store, ob);
		const ocIdx = ext.vnode_push_text(ctx.store, oc);
		ext.vnode_push_fragment_child(ctx.store, oldFrag, oaIdx);
		ext.vnode_push_fragment_child(ctx.store, oldFrag, obIdx);
		ext.vnode_push_fragment_child(ctx.store, oldFrag, ocIdx);

		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldFrag);

		// New: [text("a")]
		const newFrag = ext.vnode_push_fragment(ctx.store);
		const na = writeStringStruct("a");
		const naIdx = ext.vnode_push_text(ctx.store, na);
		ext.vnode_push_fragment_child(ctx.store, newFrag, naIdx);

		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldFrag, newFrag);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);

		// "a" == "a" → 0, "b" removed, "c" removed → 2 Remove mutations
		let removeCount = 0;
		for (const m of mutations) {
			if (m.op === Op.Remove) removeCount++;
		}
		assert(removeCount, 2, "2 Remove mutations for dropped children");

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Diff: Fragment — children added ─────────────────────────────
	suite("Diff — Fragment children added");
	{
		const ctx = createTestContext(ext);

		// Old: [text("a")]
		const oldFrag = ext.vnode_push_fragment(ctx.store);
		const oa = writeStringStruct("a");
		const oaIdx = ext.vnode_push_text(ctx.store, oa);
		ext.vnode_push_fragment_child(ctx.store, oldFrag, oaIdx);

		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldFrag);

		// New: [text("a"), text("b"), text("c")]
		const newFrag = ext.vnode_push_fragment(ctx.store);
		const na = writeStringStruct("a");
		const nb = writeStringStruct("b");
		const nc = writeStringStruct("c");
		const naIdx = ext.vnode_push_text(ctx.store, na);
		const nbIdx = ext.vnode_push_text(ctx.store, nb);
		const ncIdx = ext.vnode_push_text(ctx.store, nc);
		ext.vnode_push_fragment_child(ctx.store, newFrag, naIdx);
		ext.vnode_push_fragment_child(ctx.store, newFrag, nbIdx);
		ext.vnode_push_fragment_child(ctx.store, newFrag, ncIdx);

		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldFrag, newFrag);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);

		// "a" == "a" → 0, then create "b" and "c" + insert after
		let createCount = 0;
		let hasInsertAfter = false;
		for (const m of mutations) {
			if (m.op === Op.CreateTextNode) createCount++;
			if (m.op === Op.InsertAfter) hasInsertAfter = true;
		}
		assert(createCount, 2, "2 CreateTextNode for new children");
		assert(hasInsertAfter, true, "has InsertAfter to place new children");

		// New children should be mounted
		assert(ext.vnode_is_mounted(ctx.store, nbIdx), 1, "child b mounted");
		assert(ext.vnode_is_mounted(ctx.store, ncIdx), 1, "child c mounted");

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Diff: Attribute type changed (text → bool) ──────────────────
	suite("Diff — Attribute type changed (text → bool)");
	{
		const ctx = createTestContext(ext);

		const tmplId = registerDivWithDynAttr(ext, ctx.rt, "type-chg-div");

		const oldIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const n1 = writeStringStruct("data-active");
		const v1 = writeStringStruct("yes");
		ext.vnode_push_dynamic_attr_text(ctx.store, oldIdx, n1, v1, 0);

		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		const newIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const n2 = writeStringStruct("data-active");
		ext.vnode_push_dynamic_attr_bool(ctx.store, newIdx, n2, 1, 0);

		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);
		assert(mutations.length, 1, "type change → 1 mutation");
		assert(mutations[0].op, Op.SetAttribute, "mutation is SetAttribute");
		if (mutations[0].op === Op.SetAttribute) {
			assert(mutations[0].name, "data-active", "attr name preserved");
			assert(mutations[0].value, "true", "new bool value as string");
		}

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Diff: Attribute removed (text → none) ───────────────────────
	suite("Diff — Attribute removed (text → none)");
	{
		const ctx = createTestContext(ext);

		const tmplId = registerDivWithDynAttr(ext, ctx.rt, "rm-attr-div");

		const oldIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const n1 = writeStringStruct("title");
		const v1 = writeStringStruct("tooltip");
		ext.vnode_push_dynamic_attr_text(ctx.store, oldIdx, n1, v1, 0);

		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		const newIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const n2 = writeStringStruct("title");
		ext.vnode_push_dynamic_attr_none(ctx.store, newIdx, n2, 0);

		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);
		assert(mutations.length, 1, "attr removal → 1 mutation");
		assert(
			mutations[0].op,
			Op.RemoveAttribute,
			"mutation is RemoveAttribute (removal)",
		);

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Diff: Event listener swap ───────────────────────────────────
	suite("Diff — Event listener handler id changed");
	{
		const ctx = createTestContext(ext);

		const tmplId = registerDivWithDynAttr(ext, ctx.rt, "ev-swap-div");

		const oldIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const n1 = writeStringStruct("click");
		ext.vnode_push_dynamic_attr_event(ctx.store, oldIdx, n1, 10, 0);

		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		const newIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const n2 = writeStringStruct("click");
		ext.vnode_push_dynamic_attr_event(ctx.store, newIdx, n2, 20, 0);

		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);
		// Should have RemoveEventListener + NewEventListener
		let hasRemoveListener = false;
		let hasNewListener = false;
		for (const m of mutations) {
			if (m.op === Op.RemoveEventListener) {
				hasRemoveListener = true;
				assert(m.name, "click", "remove listener name is 'click'");
			}
			if (m.op === Op.NewEventListener) {
				hasNewListener = true;
				assert(m.name, "click", "new listener name is 'click'");
			}
		}
		assert(hasRemoveListener, true, "has RemoveEventListener");
		assert(hasNewListener, true, "has NewEventListener");

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Diff: Event listener same — 0 mutations ────────────────────
	suite("Diff — Same event listener → 0 mutations");
	{
		const ctx = createTestContext(ext);

		const tmplId = registerDivWithDynAttr(ext, ctx.rt, "ev-same-div");

		const oldIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const n1 = writeStringStruct("click");
		ext.vnode_push_dynamic_attr_event(ctx.store, oldIdx, n1, 42, 0);

		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		const newIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const n2 = writeStringStruct("click");
		ext.vnode_push_dynamic_attr_event(ctx.store, newIdx, n2, 42, 0);

		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);
		assert(mutations.length, 0, "same event → 0 mutations");

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Create: Complex template with dynamic text + dynamic attr ───
	suite("Create — Complex template with multiple dynamic slots");
	{
		const ctx = createTestContext(ext);

		const tmplId = registerComplexTemplate(ext, ctx.rt, "complex-tmpl");

		const vnIdx = ext.vnode_push_template_ref(ctx.store, tmplId);

		// dynamic_text_0 → "Description"
		const dtStr = writeStringStruct("Description");
		ext.vnode_push_dynamic_text_node(ctx.store, vnIdx, dtStr);

		// dynamic_text_1 → "Click me"
		const dt2Str = writeStringStruct("Click me");
		ext.vnode_push_dynamic_text_node(ctx.store, vnIdx, dt2Str);

		// dynamic_attr_0 → onclick handler
		const evName = writeStringStruct("click");
		ext.vnode_push_dynamic_attr_event(ctx.store, vnIdx, evName, 99, 0);

		const numRoots = ext.create_vnode(
			ctx.writer,
			ctx.eid,
			ctx.rt,
			ctx.store,
			vnIdx,
		);
		const offset = ext.writer_finalize(ctx.writer);

		assert(numRoots, 1, "complex template creates 1 root");

		const mutations = readMutations(ctx.buf, offset);

		// Verify LoadTemplate is first
		assert(mutations[0].op, Op.LoadTemplate, "first is LoadTemplate");

		// Count mutation types
		let loadCount = 0;
		let assignCount = 0;
		let setTextCount = 0;
		let newListenerCount = 0;
		for (const m of mutations) {
			if (m.op === Op.LoadTemplate) loadCount++;
			if (m.op === Op.AssignId) assignCount++;
			if (m.op === Op.SetText) setTextCount++;
			if (m.op === Op.NewEventListener) newListenerCount++;
		}

		assert(loadCount, 1, "1 LoadTemplate");
		// Should have AssignId for each dynamic text slot + dynamic attr target
		assert(assignCount >= 2, true, "at least 2 AssignId mutations");
		assert(setTextCount, 2, "2 SetText mutations for dyn texts");
		assert(newListenerCount, 1, "1 NewEventListener for click");

		// Mount state
		assert(ext.vnode_root_id_count(ctx.store, vnIdx), 1, "has 1 root id");
		assert(
			ext.vnode_dyn_node_id_count(ctx.store, vnIdx),
			2,
			"has 2 dyn node ids",
		);
		assert(
			ext.vnode_dyn_attr_id_count(ctx.store, vnIdx),
			1,
			"has 1 dyn attr id",
		);

		destroyTestContext(ext, ctx);
	}

	// ── Multiple diffs: create → diff → diff (state chain) ─────────
	suite("Diff — Sequential diffs (state chain)");
	{
		const ctx = createTestContext(ext);

		const tmplId = registerDivWithDynText(ext, ctx.rt, "chain-div");

		// Create v0 with "alpha"
		const v0 = ext.vnode_push_template_ref(ctx.store, tmplId);
		const t0 = writeStringStruct("alpha");
		ext.vnode_push_dynamic_text_node(ctx.store, v0, t0);
		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, v0);

		// Diff v0 → v1 ("alpha" → "beta")
		const v1 = ext.vnode_push_template_ref(ctx.store, tmplId);
		const t1 = writeStringStruct("beta");
		ext.vnode_push_dynamic_text_node(ctx.store, v1, t1);

		fns.writer_destroy(ctx.writer);
		const buf1 = allocBuf(ext);
		const w1 = ext.writer_create(buf1, BUF_SIZE);
		ext.diff_vnodes(w1, ctx.eid, ctx.rt, ctx.store, v0, v1);
		const off1 = ext.writer_finalize(w1);
		const muts1 = readMutations(buf1, off1);
		assert(muts1.length, 1, "first diff: 1 SetText");

		// Diff v1 → v2 ("beta" → "gamma")
		const v2 = ext.vnode_push_template_ref(ctx.store, tmplId);
		const t2 = writeStringStruct("gamma");
		ext.vnode_push_dynamic_text_node(ctx.store, v2, t2);

		ext.writer_destroy(w1);
		const buf2 = allocBuf(ext);
		const w2 = ext.writer_create(buf2, BUF_SIZE);
		ext.diff_vnodes(w2, ctx.eid, ctx.rt, ctx.store, v1, v2);
		const off2 = ext.writer_finalize(w2);
		const muts2 = readMutations(buf2, off2);
		assert(muts2.length, 1, "second diff: 1 SetText");
		if (muts2[0].op === Op.SetText) {
			assert(muts2[0].text, "gamma", "second diff text is 'gamma'");
		}

		// Diff v2 → v3 ("gamma" → "gamma") — no change
		const v3 = ext.vnode_push_template_ref(ctx.store, tmplId);
		const t3 = writeStringStruct("gamma");
		ext.vnode_push_dynamic_text_node(ctx.store, v3, t3);

		ext.writer_destroy(w2);
		const buf3 = allocBuf(ext);
		const w3 = ext.writer_create(buf3, BUF_SIZE);
		ext.diff_vnodes(w3, ctx.eid, ctx.rt, ctx.store, v2, v3);
		const off3 = ext.writer_finalize(w3);
		const muts3 = readMutations(buf3, off3);
		assert(muts3.length, 0, "third diff: 0 mutations (no change)");

		// Mount state should be consistently transferred
		assert(
			ext.vnode_dyn_node_id_count(ctx.store, v3),
			1,
			"v3 has dyn node id from chain",
		);

		ext.writer_destroy(w3);
		freeBuf(ext, buf3);
		freeBuf(ext, buf2);
		freeBuf(ext, buf1);
		destroyTestContext(ext, ctx);
	}

	// ── Create: Empty fragment — 0 roots ────────────────────────────
	suite("Create — Empty fragment");
	{
		const ctx = createTestContext(ext);

		const fragIdx = ext.vnode_push_fragment(ctx.store);

		const numRoots = ext.create_vnode(
			ctx.writer,
			ctx.eid,
			ctx.rt,
			ctx.store,
			fragIdx,
		);
		const offset = ext.writer_finalize(ctx.writer);

		assert(numRoots, 0, "empty fragment creates 0 roots");

		const mutations = readMutations(ctx.buf, offset);
		assert(mutations.length, 0, "0 mutations for empty fragment");

		destroyTestContext(ext, ctx);
	}

	// ── Diff: Fragment — empty → populated ──────────────────────────
	suite("Diff — Fragment empty → populated");
	{
		const ctx = createTestContext(ext);

		// Old: empty fragment
		const oldFrag = ext.vnode_push_fragment(ctx.store);
		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldFrag);

		// New: fragment with 2 children
		const newFrag = ext.vnode_push_fragment(ctx.store);
		const na = writeStringStruct("x");
		const nb = writeStringStruct("y");
		const naIdx = ext.vnode_push_text(ctx.store, na);
		const nbIdx = ext.vnode_push_text(ctx.store, nb);
		ext.vnode_push_fragment_child(ctx.store, newFrag, naIdx);
		ext.vnode_push_fragment_child(ctx.store, newFrag, nbIdx);

		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldFrag, newFrag);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);

		// Should create 2 text nodes
		let createCount = 0;
		for (const m of mutations) {
			if (m.op === Op.CreateTextNode) createCount++;
		}
		assert(createCount, 2, "2 CreateTextNode for new children");

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Diff: Fragment — populated → empty ──────────────────────────
	suite("Diff — Fragment populated → empty");
	{
		const ctx = createTestContext(ext);

		// Old: fragment with 2 children
		const oldFrag = ext.vnode_push_fragment(ctx.store);
		const oa = writeStringStruct("x");
		const ob = writeStringStruct("y");
		const oaIdx = ext.vnode_push_text(ctx.store, oa);
		const obIdx = ext.vnode_push_text(ctx.store, ob);
		ext.vnode_push_fragment_child(ctx.store, oldFrag, oaIdx);
		ext.vnode_push_fragment_child(ctx.store, oldFrag, obIdx);

		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldFrag);

		// New: empty fragment
		const newFrag = ext.vnode_push_fragment(ctx.store);

		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldFrag, newFrag);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);

		let removeCount = 0;
		for (const m of mutations) {
			if (m.op === Op.Remove) removeCount++;
		}
		assert(removeCount, 2, "2 Remove for dropped children");

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Diff: Dynamic node kind changed (text → placeholder) ────────
	suite("Diff — Dynamic node text → placeholder");
	{
		const ctx = createTestContext(ext);

		const tmplId = registerDivWithDynNode(ext, ctx.rt, "dn-chg-div");

		const oldIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const dynStr = writeStringStruct("visible");
		ext.vnode_push_dynamic_text_node(ctx.store, oldIdx, dynStr);

		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		const newIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		ext.vnode_push_dynamic_placeholder(ctx.store, newIdx);

		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);

		// Should have CreatePlaceholder + ReplaceWith
		let hasCreatePh = false;
		let hasReplaceWith = false;
		for (const m of mutations) {
			if (m.op === Op.CreatePlaceholder) hasCreatePh = true;
			if (m.op === Op.ReplaceWith) hasReplaceWith = true;
		}
		assert(hasCreatePh, true, "has CreatePlaceholder for new dynamic node");
		assert(hasReplaceWith, true, "has ReplaceWith to swap dynamic node");

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Diff: Integer attribute value changed ───────────────────────
	suite("Diff — Integer attribute value changed");
	{
		const ctx = createTestContext(ext);

		const tmplId = registerDivWithDynAttr(ext, ctx.rt, "int-attr-div");

		const oldIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const n1 = writeStringStruct("tabindex");
		ext.vnode_push_dynamic_attr_int(ctx.store, oldIdx, n1, 1, 0);

		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		const newIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const n2 = writeStringStruct("tabindex");
		ext.vnode_push_dynamic_attr_int(ctx.store, newIdx, n2, 5, 0);

		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		const offset = ext.writer_finalize(writer2);

		const mutations = readMutations(buf2, offset);
		assert(mutations.length, 1, "int attr change → 1 mutation");
		assert(mutations[0].op, Op.SetAttribute, "mutation is SetAttribute");
		if (mutations[0].op === Op.SetAttribute) {
			assert(mutations[0].name, "tabindex", "attr name correct");
			assert(mutations[0].value, "5", "int value serialized as string");
		}

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}

	// ── Diff: Mount state transfer preserves IDs ────────────────────
	suite("Diff — Mount state transfer preserves element IDs");
	{
		const ctx = createTestContext(ext);

		const tmplId = registerDivWithDynText(ext, ctx.rt, "id-preserve");

		const oldIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const t1 = writeStringStruct("text1");
		ext.vnode_push_dynamic_text_node(ctx.store, oldIdx, t1);

		ext.create_vnode(ctx.writer, ctx.eid, ctx.rt, ctx.store, oldIdx);

		const oldRootId = ext.vnode_get_root_id(ctx.store, oldIdx, 0);
		const oldDynId = ext.vnode_get_dyn_node_id(ctx.store, oldIdx, 0);

		const newIdx = ext.vnode_push_template_ref(ctx.store, tmplId);
		const t2 = writeStringStruct("text2");
		ext.vnode_push_dynamic_text_node(ctx.store, newIdx, t2);

		fns.writer_destroy(ctx.writer);
		const buf2 = allocBuf(ext);
		const writer2 = ext.writer_create(buf2, BUF_SIZE);

		ext.diff_vnodes(writer2, ctx.eid, ctx.rt, ctx.store, oldIdx, newIdx);
		ext.writer_finalize(writer2);

		// New node should have same IDs (transferred from old)
		const newRootId = ext.vnode_get_root_id(ctx.store, newIdx, 0);
		const newDynId = ext.vnode_get_dyn_node_id(ctx.store, newIdx, 0);

		assert(newRootId, oldRootId, "root ID preserved after diff");
		assert(newDynId, oldDynId, "dyn node ID preserved after diff");

		ext.writer_destroy(writer2);
		freeBuf(ext, buf2);
		destroyTestContext(ext, ctx);
	}
}
