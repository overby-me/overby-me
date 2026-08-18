// Mutation Protocol Conformance Tests (I-3)
//
// Verifies that:
//   1. The JS MutationBuilder produces byte-identical buffers to Mojo's
//      MutationWriter for the same logical mutation sequence.
//   2. MutationReader correctly round-trips every opcode from MutationBuilder.
//   3. The JS Interpreter produces the expected canonical DOM tree for
//      standardized mutation sequences representing common UI patterns.
//   4. Full app mount sequences (counter, todo-like) produce the
//      expected DOM structure through the Interpreter.
//
// These tests are the "renderer contract" gate: if the JS Interpreter and
// the Mojo MutationWriter agree on the binary format and the JS Interpreter
// produces correct DOM, then the desktop MutationInterpreter (which reads
// the same binary format) can be validated by comparison.

import { parseHTML } from "npm:linkedom";
import { Interpreter, MutationBuilder } from "../runtime/interpreter.ts";
import { alignedAlloc, getMemory } from "../runtime/memory.ts";
import type {
	Mutation,
	MutationRegisterTemplate,
} from "../runtime/protocol.ts";
import { MutationReader, Op } from "../runtime/protocol.ts";
import { writeStringStruct } from "../runtime/strings.ts";
import { Tag } from "../runtime/tags.ts";
import { TemplateCache } from "../runtime/templates.ts";
import type { WasmExports } from "../runtime/types.ts";
import { assert, pass, suite } from "./harness.ts";

type Fns = WasmExports & Record<string, unknown>;

// ── DOM helpers ─────────────────────────────────────────────────────────────

function createDOM() {
	const { document, window } = parseHTML(
		"<!DOCTYPE html><html><body><div id='root'></div></body></html>",
	);
	const root = document.getElementById("root")!;
	return { document, window, root };
}

function createInterpreter(dom?: ReturnType<typeof createDOM>) {
	const { document, root } = dom ?? createDOM();
	const templates = new TemplateCache(document);
	const interp = new Interpreter(root, templates, document);
	return { document, root, templates, interp };
}

// ── Canonical DOM serializer ────────────────────────────────────────────────
//
// Produces a deterministic string representation of a DOM subtree for
// comparison. Format:
//   <tag attr1="val1" attr2="val2">children...</tag>
//   "text content"
//   <!--comment-->
//
// Attributes are sorted alphabetically for determinism. Whitespace-only
// text nodes are omitted.

function serializeDOM(node: Node): string {
	// Element
	if (node.nodeType === 1) {
		const el = node as Element;
		const tag = el.tagName.toLowerCase();

		// Collect and sort attributes
		const attrs: string[] = [];
		for (let i = 0; i < el.attributes.length; i++) {
			const attr = el.attributes[i];
			attrs.push(`${attr.name}="${attr.value}"`);
		}
		attrs.sort();

		const attrStr = attrs.length > 0 ? ` ${attrs.join(" ")}` : "";

		// Serialize children
		const children: string[] = [];
		for (let i = 0; i < el.childNodes.length; i++) {
			const child = serializeDOM(el.childNodes[i]);
			if (child !== "") children.push(child);
		}

		const childStr = children.join("");

		// Void elements
		if (["br", "hr", "img", "input"].includes(tag) && childStr === "") {
			return `<${tag}${attrStr}/>`;
		}

		return `<${tag}${attrStr}>${childStr}</${tag}>`;
	}

	// Text node
	if (node.nodeType === 3) {
		const text = node.textContent ?? "";
		if (text.trim() === "") return "";
		return text;
	}

	// Comment node (placeholders)
	if (node.nodeType === 8) {
		return `<!--${node.textContent ?? ""}-->`;
	}

	return "";
}

/** Serialize only the children of an element (the root mount point). */
function serializeChildren(el: Element): string {
	const parts: string[] = [];
	for (let i = 0; i < el.childNodes.length; i++) {
		const s = serializeDOM(el.childNodes[i]);
		if (s !== "") parts.push(s);
	}
	return parts.join("");
}

// ── RegisterTemplate mutation builder helper ────────────────────────────────
//
// Builds a RegisterTemplate mutation in binary format matching the Mojo
// MutationWriter's register_template() output.

class TemplateMutationBuilder {
	private mb: MutationBuilder;

	constructor(capacity = 4096) {
		this.mb = new MutationBuilder(capacity);
	}

	/**
	 * Register a template with a single element root (no children, no attrs).
	 */
	registerSimpleElement(tmplId: number, name: string, tag: number): this {
		return this.registerElement(tmplId, name, tag, [], []);
	}

	/**
	 * Register a template with a single element root with children.
	 */
	registerElement(
		tmplId: number,
		name: string,
		tag: number,
		children: TemplateNodeSpec[],
		attrs: TemplateAttrSpec[],
	): this {
		// Build the flat node and attr arrays
		const flatNodes: FlatNode[] = [];
		const flatAttrs: FlatAttr[] = [];
		const rootIdx = this.flattenNode(
			{ kind: "element", tag, children, attrs },
			flatNodes,
			flatAttrs,
		);

		this.writeRegisterTemplate(tmplId, name, [rootIdx], flatNodes, flatAttrs);
		return this;
	}

	/**
	 * Register a template from an explicit tree specification with
	 * potentially multiple roots.
	 */
	registerTree(tmplId: number, name: string, roots: TemplateNodeSpec[]): this {
		const flatNodes: FlatNode[] = [];
		const flatAttrs: FlatAttr[] = [];
		const rootIndices = roots.map((r) =>
			this.flattenNode(r, flatNodes, flatAttrs),
		);
		this.writeRegisterTemplate(tmplId, name, rootIndices, flatNodes, flatAttrs);
		return this;
	}

	/** Return the underlying MutationBuilder for chaining additional ops. */
	builder(): MutationBuilder {
		return this.mb;
	}

	build(): { buffer: ArrayBuffer; length: number } {
		return this.mb.build();
	}

	// ── Internal ────────────────────────────────────────────────────

	private flattenNode(
		spec: TemplateNodeSpec,
		nodes: FlatNode[],
		attrs: FlatAttr[],
	): number {
		const idx = nodes.length;

		if (spec.kind === "element") {
			// Reserve slot
			nodes.push(null as unknown as FlatNode);

			// Flatten children first
			const childIndices = (spec.children ?? []).map((c) =>
				this.flattenNode(c, nodes, attrs),
			);

			// Flatten attrs
			const attrFirst = attrs.length;
			for (const a of spec.attrs ?? []) {
				if (a.kind === "static") {
					attrs.push({ kind: 0x00, name: a.name, value: a.value });
				} else {
					attrs.push({ kind: 0x01, dynamicIndex: a.dynamicIndex });
				}
			}
			const attrCount = attrs.length - attrFirst;

			nodes[idx] = {
				kind: 0x00,
				tag: spec.tag,
				childIndices,
				attrFirst,
				attrCount,
			};
		} else if (spec.kind === "text") {
			nodes.push({ kind: 0x01, text: spec.text });
		} else if (spec.kind === "dynamic") {
			nodes.push({ kind: 0x02, dynamicIndex: spec.dynamicIndex });
		} else if (spec.kind === "dynamic_text") {
			nodes.push({ kind: 0x03, dynamicIndex: spec.dynamicIndex });
		}

		return idx;
	}

	private writeRegisterTemplate(
		tmplId: number,
		name: string,
		rootIndices: number[],
		nodes: FlatNode[],
		attrs: FlatAttr[],
	): void {
		// We need to write the raw binary format that MutationWriter produces.
		// Access the internal buffer through the builder.
		const b = this.mb as unknown as {
			buffer: ArrayBuffer;
			view: DataView;
			bytes: Uint8Array;
			offset: number;
		};

		// OP_REGISTER_TEMPLATE
		b.view.setUint8(b.offset, Op.RegisterTemplate);
		b.offset += 1;

		// tmpl_id (u32)
		b.view.setUint32(b.offset, tmplId, true);
		b.offset += 4;

		// name (u16 len + utf8)
		const nameBytes = new TextEncoder().encode(name);
		b.view.setUint16(b.offset, nameBytes.length, true);
		b.offset += 2;
		b.bytes.set(nameBytes, b.offset);
		b.offset += nameBytes.length;

		// root_count (u16)
		b.view.setUint16(b.offset, rootIndices.length, true);
		b.offset += 2;

		// node_count (u16)
		b.view.setUint16(b.offset, nodes.length, true);
		b.offset += 2;

		// attr_count (u16)
		b.view.setUint16(b.offset, attrs.length, true);
		b.offset += 2;

		// Nodes
		for (const node of nodes) {
			b.view.setUint8(b.offset, node.kind);
			b.offset += 1;

			if (node.kind === 0x00) {
				// Element: tag (u8), child_count (u16), [child indices as u16], attr_first (u16), attr_count (u16)
				b.view.setUint8(b.offset, node.tag);
				b.offset += 1;
				b.view.setUint16(b.offset, node.childIndices.length, true);
				b.offset += 2;
				for (const ci of node.childIndices) {
					b.view.setUint16(b.offset, ci, true);
					b.offset += 2;
				}
				b.view.setUint16(b.offset, node.attrFirst, true);
				b.offset += 2;
				b.view.setUint16(b.offset, node.attrCount, true);
				b.offset += 2;
			} else if (node.kind === 0x01) {
				// Text: len (u32) + utf8
				const textBytes = new TextEncoder().encode(node.text);
				b.view.setUint32(b.offset, textBytes.length, true);
				b.offset += 4;
				b.bytes.set(textBytes, b.offset);
				b.offset += textBytes.length;
			} else if (node.kind === 0x02 || node.kind === 0x03) {
				// Dynamic / DynamicText: dynamic_index (u32)
				b.view.setUint32(b.offset, node.dynamicIndex, true);
				b.offset += 4;
			}
		}

		// Attrs
		for (const attr of attrs) {
			b.view.setUint8(b.offset, attr.kind);
			b.offset += 1;

			if (attr.kind === 0x00) {
				// Static: name (u16 len + utf8), value (u32 len + utf8)
				const nameB = new TextEncoder().encode(attr.name!);
				b.view.setUint16(b.offset, nameB.length, true);
				b.offset += 2;
				b.bytes.set(nameB, b.offset);
				b.offset += nameB.length;

				const valB = new TextEncoder().encode(attr.value!);
				b.view.setUint32(b.offset, valB.length, true);
				b.offset += 4;
				b.bytes.set(valB, b.offset);
				b.offset += valB.length;
			} else if (attr.kind === 0x01) {
				// Dynamic: dynamic_index (u32)
				b.view.setUint32(b.offset, attr.dynamicIndex!, true);
				b.offset += 4;
			}
		}

		// Root indices (u16 each)
		for (const ri of rootIndices) {
			b.view.setUint16(b.offset, ri, true);
			b.offset += 2;
		}
	}
}

// ── Template node/attr spec types ───────────────────────────────────────────

interface ElementSpec {
	kind: "element";
	tag: number;
	children?: TemplateNodeSpec[];
	attrs?: TemplateAttrSpec[];
}
interface TextSpec {
	kind: "text";
	text: string;
}
interface DynamicSpec {
	kind: "dynamic";
	dynamicIndex: number;
}
interface DynamicTextSpec {
	kind: "dynamic_text";
	dynamicIndex: number;
}
type TemplateNodeSpec = ElementSpec | TextSpec | DynamicSpec | DynamicTextSpec;

interface StaticAttrSpec {
	kind: "static";
	name: string;
	value: string;
}
interface DynamicAttrSpec {
	kind: "dynamic";
	dynamicIndex: number;
}
type TemplateAttrSpec = StaticAttrSpec | DynamicAttrSpec;

// Flat representations for binary encoding
interface FlatElementNode {
	kind: 0x00;
	tag: number;
	childIndices: number[];
	attrFirst: number;
	attrCount: number;
}
interface FlatTextNode {
	kind: 0x01;
	text: string;
}
interface FlatDynamicNode {
	kind: 0x02;
	dynamicIndex: number;
}
interface FlatDynamicTextNode {
	kind: 0x03;
	dynamicIndex: number;
}
type FlatNode =
	| FlatElementNode
	| FlatTextNode
	| FlatDynamicNode
	| FlatDynamicTextNode;

interface FlatStaticAttr {
	kind: 0x00;
	name: string;
	value: string;
	dynamicIndex?: never;
}
interface FlatDynamicAttr {
	kind: 0x01;
	dynamicIndex: number;
	name?: never;
	value?: never;
}
type FlatAttr = FlatStaticAttr | FlatDynamicAttr;

// ── WASM buffer helpers ─────────────────────────────────────────────────────

const BUF_SIZE = 8192;

function allocBuf(fns: Fns): bigint {
	return fns.mutation_buf_alloc(BUF_SIZE);
}

function freeBuf(fns: Fns, ptr: bigint): void {
	fns.mutation_buf_free(ptr);
}

// ── Byte comparison helper ──────────────────────────────────────────────────

function compareBytes(
	label: string,
	actual: Uint8Array,
	expected: Uint8Array,
	length: number,
): boolean {
	if (actual.length < length || expected.length < length) {
		return false;
	}
	for (let i = 0; i < length; i++) {
		if (actual[i] !== expected[i]) {
			return false;
		}
	}
	return true;
}

function hexDump(bytes: Uint8Array, length: number, maxBytes = 64): string {
	const parts: string[] = [];
	const limit = Math.min(length, maxBytes);
	for (let i = 0; i < limit; i++) {
		parts.push(bytes[i].toString(16).padStart(2, "0"));
	}
	const suffix = length > maxBytes ? `... (${length} bytes total)` : "";
	return parts.join(" ") + suffix;
}

// ══════════════════════════════════════════════════════════════════════════════
// Test entry point
// ══════════════════════════════════════════════════════════════════════════════

export function testConformance(fns: Fns): void {
	// ═════════════════════════════════════════════════════════════════════
	// Section 1: Binary protocol round-trip — every opcode
	// ═════════════════════════════════════════════════════════════════════

	suite("Conformance — MutationBuilder → MutationReader round-trip: PushRoot");
	{
		const { buffer, length } = new MutationBuilder().pushRoot(42).end().build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation decoded");
		assert(muts[0].op, Op.PushRoot, "op is PushRoot");
		assert((muts[0] as { id: number }).id, 42, "id is 42");
	}

	suite("Conformance — round-trip: AppendChildren");
	{
		const { buffer, length } = new MutationBuilder()
			.appendChildren(5, 3)
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		assert(muts[0].op, Op.AppendChildren, "op is AppendChildren");
		const m = muts[0] as { id: number; m: number };
		assert(m.id, 5, "id is 5");
		assert(m.m, 3, "m is 3");
	}

	suite("Conformance — round-trip: CreateTextNode");
	{
		const { buffer, length } = new MutationBuilder()
			.createTextNode(7, "Hello, World!")
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		assert(muts[0].op, Op.CreateTextNode, "op is CreateTextNode");
		const m = muts[0] as { id: number; text: string };
		assert(m.id, 7, "id is 7");
		assert(m.text, "Hello, World!", "text is correct");
	}

	suite("Conformance — round-trip: CreateTextNode with unicode");
	{
		const { buffer, length } = new MutationBuilder()
			.createTextNode(1, "こんにちは 🌍")
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		const m = muts[0] as { id: number; text: string };
		assert(m.text, "こんにちは 🌍", "unicode text preserved");
	}

	suite("Conformance — round-trip: CreateTextNode with empty string");
	{
		const { buffer, length } = new MutationBuilder()
			.createTextNode(2, "")
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		const m = muts[0] as { id: number; text: string };
		assert(m.text, "", "empty string preserved");
	}

	suite("Conformance — round-trip: CreatePlaceholder");
	{
		const { buffer, length } = new MutationBuilder()
			.createPlaceholder(99)
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		assert(muts[0].op, Op.CreatePlaceholder, "op is CreatePlaceholder");
		assert((muts[0] as { id: number }).id, 99, "id is 99");
	}

	suite("Conformance — round-trip: LoadTemplate");
	{
		const { buffer, length } = new MutationBuilder()
			.loadTemplate(10, 0, 50)
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		assert(muts[0].op, Op.LoadTemplate, "op is LoadTemplate");
		const m = muts[0] as { tmplId: number; index: number; id: number };
		assert(m.tmplId, 10, "tmplId is 10");
		assert(m.index, 0, "index is 0");
		assert(m.id, 50, "id is 50");
	}

	suite("Conformance — round-trip: AssignId");
	{
		const { buffer, length } = new MutationBuilder()
			.assignId([0, 1], 20)
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		assert(muts[0].op, Op.AssignId, "op is AssignId");
		const m = muts[0] as { path: Uint8Array; id: number };
		assert(m.path.length, 2, "path length is 2");
		assert(m.path[0], 0, "path[0] is 0");
		assert(m.path[1], 1, "path[1] is 1");
		assert(m.id, 20, "id is 20");
	}

	suite("Conformance — round-trip: AssignId with empty path");
	{
		const { buffer, length } = new MutationBuilder()
			.assignId([], 30)
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		const m = muts[0] as { path: Uint8Array; id: number };
		assert(m.path.length, 0, "empty path");
		assert(m.id, 30, "id is 30");
	}

	suite("Conformance — round-trip: SetAttribute");
	{
		const { buffer, length } = new MutationBuilder()
			.setAttribute(3, 0, "class", "active")
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		assert(muts[0].op, Op.SetAttribute, "op is SetAttribute");
		const m = muts[0] as {
			id: number;
			ns: number;
			name: string;
			value: string;
		};
		assert(m.id, 3, "id is 3");
		assert(m.ns, 0, "ns is 0");
		assert(m.name, "class", "name is 'class'");
		assert(m.value, "active", "value is 'active'");
	}

	suite("Conformance — round-trip: SetAttribute with namespace");
	{
		const { buffer, length } = new MutationBuilder()
			.setAttribute(4, 1, "href", "http://example.com")
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		const m = muts[0] as { ns: number; name: string; value: string };
		assert(m.ns, 1, "ns is 1 (xlink)");
		assert(m.name, "href", "name is href");
		assert(m.value, "http://example.com", "value preserved");
	}

	suite("Conformance — round-trip: RemoveAttribute");
	{
		const { buffer, length } = new MutationBuilder()
			.removeAttribute(5, 0, "disabled")
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		assert(muts[0].op, Op.RemoveAttribute, "op is RemoveAttribute");
		const m = muts[0] as { id: number; ns: number; name: string };
		assert(m.id, 5, "id is 5");
		assert(m.ns, 0, "ns is 0");
		assert(m.name, "disabled", "name is 'disabled'");
	}

	suite("Conformance — round-trip: SetText");
	{
		const { buffer, length } = new MutationBuilder()
			.setText(8, "Updated text")
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		assert(muts[0].op, Op.SetText, "op is SetText");
		const m = muts[0] as { id: number; text: string };
		assert(m.id, 8, "id is 8");
		assert(m.text, "Updated text", "text is correct");
	}

	suite("Conformance — round-trip: NewEventListener");
	{
		const { buffer, length } = new MutationBuilder()
			.newEventListener(6, "click", 100)
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		assert(muts[0].op, Op.NewEventListener, "op is NewEventListener");
		const m = muts[0] as { id: number; handlerId: number; name: string };
		assert(m.id, 6, "id is 6");
		assert(m.handlerId, 100, "handlerId is 100");
		assert(m.name, "click", "name is 'click'");
	}

	suite("Conformance — round-trip: RemoveEventListener");
	{
		const { buffer, length } = new MutationBuilder()
			.removeEventListener(6, "click")
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		assert(muts[0].op, Op.RemoveEventListener, "op is RemoveEventListener");
		const m = muts[0] as { id: number; name: string };
		assert(m.id, 6, "id is 6");
		assert(m.name, "click", "name is 'click'");
	}

	suite("Conformance — round-trip: Remove");
	{
		const { buffer, length } = new MutationBuilder().remove(15).end().build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		assert(muts[0].op, Op.Remove, "op is Remove");
		assert((muts[0] as { id: number }).id, 15, "id is 15");
	}

	suite("Conformance — round-trip: ReplaceWith");
	{
		const { buffer, length } = new MutationBuilder()
			.replaceWith(10, 2)
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		assert(muts[0].op, Op.ReplaceWith, "op is ReplaceWith");
		const m = muts[0] as { id: number; m: number };
		assert(m.id, 10, "id is 10");
		assert(m.m, 2, "m is 2");
	}

	suite("Conformance — round-trip: ReplacePlaceholder");
	{
		const { buffer, length } = new MutationBuilder()
			.replacePlaceholder([0, 2], 1)
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		assert(muts[0].op, Op.ReplacePlaceholder, "op is ReplacePlaceholder");
		const m = muts[0] as { path: Uint8Array; m: number };
		assert(m.path.length, 2, "path length is 2");
		assert(m.path[0], 0, "path[0] is 0");
		assert(m.path[1], 2, "path[1] is 2");
		assert(m.m, 1, "m is 1");
	}

	suite("Conformance — round-trip: InsertAfter");
	{
		const { buffer, length } = new MutationBuilder()
			.insertAfter(11, 3)
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		assert(muts[0].op, Op.InsertAfter, "op is InsertAfter");
		const m = muts[0] as { id: number; m: number };
		assert(m.id, 11, "id is 11");
		assert(m.m, 3, "m is 3");
	}

	suite("Conformance — round-trip: InsertBefore");
	{
		const { buffer, length } = new MutationBuilder()
			.insertBefore(12, 2)
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		assert(muts[0].op, Op.InsertBefore, "op is InsertBefore");
		const m = muts[0] as { id: number; m: number };
		assert(m.id, 12, "id is 12");
		assert(m.m, 2, "m is 2");
	}

	suite("Conformance — round-trip: all opcodes in one buffer");
	{
		const { buffer, length } = new MutationBuilder()
			.pushRoot(1)
			.appendChildren(0, 1)
			.createTextNode(2, "hi")
			.createPlaceholder(3)
			.loadTemplate(0, 0, 4)
			.assignId([0], 5)
			.setAttribute(5, 0, "id", "x")
			.removeAttribute(5, 0, "id")
			.setText(2, "bye")
			.newEventListener(5, "click", 10)
			.removeEventListener(5, "click")
			.remove(3)
			.replaceWith(4, 1)
			.replacePlaceholder([1], 1)
			.insertAfter(1, 1)
			.insertBefore(1, 1)
			.end()
			.build();

		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 16, "16 mutations decoded");

		assert(muts[0].op, Op.PushRoot, "op[0] = PushRoot");
		assert(muts[1].op, Op.AppendChildren, "op[1] = AppendChildren");
		assert(muts[2].op, Op.CreateTextNode, "op[2] = CreateTextNode");
		assert(muts[3].op, Op.CreatePlaceholder, "op[3] = CreatePlaceholder");
		assert(muts[4].op, Op.LoadTemplate, "op[4] = LoadTemplate");
		assert(muts[5].op, Op.AssignId, "op[5] = AssignId");
		assert(muts[6].op, Op.SetAttribute, "op[6] = SetAttribute");
		assert(muts[7].op, Op.RemoveAttribute, "op[7] = RemoveAttribute");
		assert(muts[8].op, Op.SetText, "op[8] = SetText");
		assert(muts[9].op, Op.NewEventListener, "op[9] = NewEventListener");
		assert(muts[10].op, Op.RemoveEventListener, "op[10] = RemoveEventListener");
		assert(muts[11].op, Op.Remove, "op[11] = Remove");
		assert(muts[12].op, Op.ReplaceWith, "op[12] = ReplaceWith");
		assert(muts[13].op, Op.ReplacePlaceholder, "op[13] = ReplacePlaceholder");
		assert(muts[14].op, Op.InsertAfter, "op[14] = InsertAfter");
		assert(muts[15].op, Op.InsertBefore, "op[15] = InsertBefore");
	}

	suite("Conformance — round-trip: End sentinel terminates reading");
	{
		const { buffer, length } = new MutationBuilder()
			.createTextNode(1, "before end")
			.end()
			.createTextNode(2, "after end")
			.end()
			.build();

		// Reader should stop at first End sentinel
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "only one mutation before End");
		assert(
			(muts[0] as { text: string }).text,
			"before end",
			"correct mutation",
		);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 2: RegisterTemplate round-trip
	// ═════════════════════════════════════════════════════════════════════

	suite("Conformance — RegisterTemplate round-trip: simple element");
	{
		const tmb = new TemplateMutationBuilder();
		tmb.registerSimpleElement(0, "test-div", Tag.DIV);
		const { buffer, length } = tmb.build();

		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		assert(muts[0].op, Op.RegisterTemplate, "op is RegisterTemplate");

		const m = muts[0] as MutationRegisterTemplate;
		assert(m.tmplId, 0, "tmplId is 0");
		assert(m.name, "test-div", "name is 'test-div'");
		assert(m.rootCount, 1, "one root");
		assert(m.nodeCount, 1, "one node");
		assert(m.attrCount, 0, "no attrs");
		assert(m.rootIndices[0], 0, "root index is 0");
		assert(m.nodes[0].kind, 0x00, "root is element");
	}

	suite("Conformance — RegisterTemplate round-trip: element with children");
	{
		const tmb = new TemplateMutationBuilder();
		tmb.registerTree(0, "div-with-children", [
			{
				kind: "element",
				tag: Tag.DIV,
				children: [
					{ kind: "text", text: "Hello" },
					{
						kind: "element",
						tag: Tag.SPAN,
						children: [],
						attrs: [{ kind: "static", name: "class", value: "inner" }],
					},
				],
			},
		]);
		const { buffer, length } = tmb.build();

		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		const m = muts[0] as MutationRegisterTemplate;
		assert(m.nodeCount, 3, "3 nodes (div, text, span)");
		assert(m.attrCount, 1, "1 attr (class on span)");

		// Root is div with 2 children
		const root = m.nodes[m.rootIndices[0]];
		assert(root.kind, 0x00, "root is element");
		if (root.kind === 0x00) {
			assert(root.tag, Tag.DIV, "root tag is div");
			assert(root.children.length, 2, "div has 2 children");
		}
	}

	suite("Conformance — RegisterTemplate round-trip: dynamic text slot");
	{
		const tmb = new TemplateMutationBuilder();
		tmb.registerTree(1, "div-dyn-text", [
			{
				kind: "element",
				tag: Tag.DIV,
				children: [{ kind: "dynamic_text", dynamicIndex: 0 }],
			},
		]);
		const { buffer, length } = tmb.build();

		const muts = new MutationReader(buffer, 0, length).readAll();
		const m = muts[0] as MutationRegisterTemplate;
		assert(m.nodeCount, 2, "2 nodes (div + dynamic_text)");

		// Find the dynamic text node
		const dynNode = m.nodes.find((n) => n.kind === 0x03);
		assert(dynNode !== undefined, true, "found dynamic text node");
		if (dynNode && dynNode.kind === 0x03) {
			assert(dynNode.dynamicIndex, 0, "dynamicIndex is 0");
		}
	}

	suite("Conformance — RegisterTemplate round-trip: dynamic node slot");
	{
		const tmb = new TemplateMutationBuilder();
		tmb.registerTree(2, "div-dyn-node", [
			{
				kind: "element",
				tag: Tag.DIV,
				children: [
					{ kind: "text", text: "before" },
					{ kind: "dynamic", dynamicIndex: 0 },
					{ kind: "text", text: "after" },
				],
			},
		]);
		const { buffer, length } = tmb.build();

		const muts = new MutationReader(buffer, 0, length).readAll();
		const m = muts[0] as MutationRegisterTemplate;
		assert(m.nodeCount, 4, "4 nodes (div + text + dynamic + text)");

		const dynNode = m.nodes.find((n) => n.kind === 0x02);
		assert(dynNode !== undefined, true, "found dynamic node");
		if (dynNode && dynNode.kind === 0x02) {
			assert(dynNode.dynamicIndex, 0, "dynamicIndex is 0");
		}
	}

	suite("Conformance — RegisterTemplate round-trip: dynamic attribute");
	{
		const tmb = new TemplateMutationBuilder();
		tmb.registerTree(3, "div-dyn-attr", [
			{
				kind: "element",
				tag: Tag.DIV,
				attrs: [
					{ kind: "static", name: "id", value: "main" },
					{ kind: "dynamic", dynamicIndex: 0 },
				],
			},
		]);
		const { buffer, length } = tmb.build();

		const muts = new MutationReader(buffer, 0, length).readAll();
		const m = muts[0] as MutationRegisterTemplate;
		assert(m.attrCount, 2, "2 attrs (static + dynamic)");
		assert(m.attrs[0].kind, 0x00, "first attr is static");
		assert(m.attrs[1].kind, 0x01, "second attr is dynamic");
		if (m.attrs[0].kind === 0x00) {
			assert(m.attrs[0].name, "id", "static attr name");
			assert(m.attrs[0].value, "main", "static attr value");
		}
		if (m.attrs[1].kind === 0x01) {
			assert(m.attrs[1].dynamicIndex, 0, "dynamic attr index");
		}
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 3: Interpreter DOM conformance — canonical output
	// ═════════════════════════════════════════════════════════════════════

	suite("Conformance — DOM: mount text node into root");
	{
		const { root, interp } = createInterpreter();
		const { buffer, length } = new MutationBuilder()
			.createTextNode(1, "Hello, World!")
			.pushRoot(1)
			.appendChildren(0, 1)
			.end()
			.build();

		interp.applyMutations(buffer, 0, length);
		assert(serializeChildren(root), "Hello, World!", "text node mounted");
	}

	suite("Conformance — DOM: mount placeholder into root");
	{
		const { root, interp } = createInterpreter();
		const { buffer, length } = new MutationBuilder()
			.createPlaceholder(1)
			.pushRoot(1)
			.appendChildren(0, 1)
			.end()
			.build();

		interp.applyMutations(buffer, 0, length);
		assert(
			serializeChildren(root),
			"<!--placeholder-->",
			"placeholder comment mounted",
		);
	}

	suite("Conformance — DOM: mount template element");
	{
		const dom = createDOM();
		const { root, templates, interp } = createInterpreter(dom);
		const div = dom.document.createElement("div");
		templates.register(0, [div]);

		const { buffer, length } = new MutationBuilder()
			.loadTemplate(0, 0, 1)
			.pushRoot(1)
			.appendChildren(0, 1)
			.end()
			.build();

		interp.applyMutations(buffer, 0, length);
		assert(serializeChildren(root), "<div></div>", "div template mounted");
	}

	suite("Conformance — DOM: template + dynamic text + SetText");
	{
		const { root, templates, interp, document } = createInterpreter();

		// Register template: <div><!-- dynamic text (empty) --></div>
		const tmb = new TemplateMutationBuilder();
		tmb.registerTree(0, "div-dyn", [
			{
				kind: "element",
				tag: Tag.DIV,
				children: [{ kind: "dynamic_text", dynamicIndex: 0 }],
			},
		]);
		const regBuf = tmb.build();
		interp.applyMutations(regBuf.buffer, 0, regBuf.length);

		// Mount: LoadTemplate → AssignId (dynamic text node) → SetText → AppendChildren
		const { buffer, length } = new MutationBuilder()
			.loadTemplate(0, 0, 1)
			.assignId([0], 2) // div's first child (the dynamic text node)
			.setText(2, "Count: 5")
			.pushRoot(1)
			.appendChildren(0, 1)
			.end()
			.build();

		interp.applyMutations(buffer, 0, length);
		assert(
			serializeChildren(root),
			"<div>Count: 5</div>",
			"dynamic text rendered",
		);
	}

	suite("Conformance — DOM: template + dynamic attr + SetAttribute");
	{
		const { root, interp } = createInterpreter();

		// Register: <div></div> with one dynamic attribute
		const tmb = new TemplateMutationBuilder();
		tmb.registerTree(0, "div-attr", [
			{
				kind: "element",
				tag: Tag.DIV,
				attrs: [{ kind: "dynamic", dynamicIndex: 0 }],
			},
		]);
		const regBuf = tmb.build();
		interp.applyMutations(regBuf.buffer, 0, regBuf.length);

		// Mount with attribute
		const { buffer, length } = new MutationBuilder()
			.loadTemplate(0, 0, 1)
			.assignId([], 1) // assign id to the root div itself
			.setAttribute(1, 0, "class", "active")
			.pushRoot(1)
			.appendChildren(0, 1)
			.end()
			.build();

		interp.applyMutations(buffer, 0, length);
		assert(
			serializeChildren(root),
			'<div class="active"></div>',
			"dynamic attr rendered",
		);
	}

	suite("Conformance — DOM: template with static text and attributes");
	{
		const { root, interp } = createInterpreter();

		// Register: <button type="submit">Click me</button>
		const tmb = new TemplateMutationBuilder();
		tmb.registerTree(0, "button-tmpl", [
			{
				kind: "element",
				tag: Tag.BUTTON,
				children: [{ kind: "text", text: "Click me" }],
				attrs: [{ kind: "static", name: "type", value: "submit" }],
			},
		]);
		const regBuf = tmb.build();
		interp.applyMutations(regBuf.buffer, 0, regBuf.length);

		const { buffer, length } = new MutationBuilder()
			.loadTemplate(0, 0, 1)
			.pushRoot(1)
			.appendChildren(0, 1)
			.end()
			.build();

		interp.applyMutations(buffer, 0, length);
		assert(
			serializeChildren(root),
			'<button type="submit">Click me</button>',
			"static template rendered",
		);
	}

	suite("Conformance — DOM: SetText updates existing text");
	{
		const { root, interp } = createInterpreter();

		// Mount a text node
		const buf1 = new MutationBuilder()
			.createTextNode(1, "v1")
			.pushRoot(1)
			.appendChildren(0, 1)
			.end()
			.build();
		interp.applyMutations(buf1.buffer, 0, buf1.length);
		assert(serializeChildren(root), "v1", "initial text");

		// Update via SetText
		const buf2 = new MutationBuilder().setText(1, "v2").end().build();
		interp.applyMutations(buf2.buffer, 0, buf2.length);
		assert(serializeChildren(root), "v2", "text updated");
	}

	suite("Conformance — DOM: SetAttribute + RemoveAttribute cycle");
	{
		const { root, interp } = createInterpreter();

		const tmb = new TemplateMutationBuilder();
		tmb.registerTree(0, "div-cycle", [{ kind: "element", tag: Tag.DIV }]);
		const reg = tmb.build();
		interp.applyMutations(reg.buffer, 0, reg.length);

		// Mount div
		const mount = new MutationBuilder()
			.loadTemplate(0, 0, 1)
			.assignId([], 1)
			.pushRoot(1)
			.appendChildren(0, 1)
			.end()
			.build();
		interp.applyMutations(mount.buffer, 0, mount.length);
		assert(serializeChildren(root), "<div></div>", "div mounted");

		// Set attribute
		const set1 = new MutationBuilder()
			.setAttribute(1, 0, "hidden", "")
			.end()
			.build();
		interp.applyMutations(set1.buffer, 0, set1.length);
		assert(serializeChildren(root), '<div hidden=""></div>', "attribute set");

		// Remove attribute
		const rm = new MutationBuilder()
			.removeAttribute(1, 0, "hidden")
			.end()
			.build();
		interp.applyMutations(rm.buffer, 0, rm.length);
		assert(serializeChildren(root), "<div></div>", "attribute removed");

		// Set again
		const set2 = new MutationBuilder()
			.setAttribute(1, 0, "class", "shown")
			.end()
			.build();
		interp.applyMutations(set2.buffer, 0, set2.length);
		assert(
			serializeChildren(root),
			'<div class="shown"></div>',
			"attribute re-added",
		);
	}

	suite("Conformance — DOM: Remove deletes node from parent");
	{
		const { root, interp } = createInterpreter();

		// Mount two text nodes
		const mount = new MutationBuilder()
			.createTextNode(1, "keep")
			.createTextNode(2, "remove")
			.pushRoot(1)
			.pushRoot(2)
			.appendChildren(0, 2)
			.end()
			.build();
		interp.applyMutations(mount.buffer, 0, mount.length);
		assert(serializeChildren(root), "keepremove", "both nodes mounted");

		// Remove second node
		const rm = new MutationBuilder().remove(2).end().build();
		interp.applyMutations(rm.buffer, 0, rm.length);
		assert(serializeChildren(root), "keep", "second node removed");
	}

	suite("Conformance — DOM: ReplaceWith swaps node");
	{
		const { root, interp } = createInterpreter();

		// Mount a text node
		const mount = new MutationBuilder()
			.createTextNode(1, "old")
			.pushRoot(1)
			.appendChildren(0, 1)
			.end()
			.build();
		interp.applyMutations(mount.buffer, 0, mount.length);
		assert(serializeChildren(root), "old", "old text mounted");

		// Replace with new text
		const replace = new MutationBuilder()
			.createTextNode(2, "new")
			.pushRoot(2)
			.replaceWith(1, 1)
			.end()
			.build();
		interp.applyMutations(replace.buffer, 0, replace.length);
		assert(serializeChildren(root), "new", "text replaced");
	}

	suite("Conformance — DOM: InsertAfter places node correctly");
	{
		const { root, interp } = createInterpreter();

		// Mount A
		const mount = new MutationBuilder()
			.createTextNode(1, "A")
			.pushRoot(1)
			.appendChildren(0, 1)
			.end()
			.build();
		interp.applyMutations(mount.buffer, 0, mount.length);

		// Insert B after A
		const insert = new MutationBuilder()
			.createTextNode(2, "B")
			.pushRoot(2)
			.insertAfter(1, 1)
			.end()
			.build();
		interp.applyMutations(insert.buffer, 0, insert.length);
		assert(serializeChildren(root), "AB", "B inserted after A");
	}

	suite("Conformance — DOM: InsertBefore places node correctly");
	{
		const { root, interp } = createInterpreter();

		// Mount B
		const mount = new MutationBuilder()
			.createTextNode(2, "B")
			.pushRoot(2)
			.appendChildren(0, 1)
			.end()
			.build();
		interp.applyMutations(mount.buffer, 0, mount.length);

		// Insert A before B
		const insert = new MutationBuilder()
			.createTextNode(1, "A")
			.pushRoot(1)
			.insertBefore(2, 1)
			.end()
			.build();
		interp.applyMutations(insert.buffer, 0, insert.length);
		assert(serializeChildren(root), "AB", "A inserted before B");
	}

	suite("Conformance — DOM: multiple InsertAfter preserves order");
	{
		const { root, interp } = createInterpreter();

		// Mount A
		const mount = new MutationBuilder()
			.createTextNode(1, "A")
			.pushRoot(1)
			.appendChildren(0, 1)
			.end()
			.build();
		interp.applyMutations(mount.buffer, 0, mount.length);

		// Insert B, C after A (push both, insertAfter m=2)
		const insert = new MutationBuilder()
			.createTextNode(2, "B")
			.createTextNode(3, "C")
			.pushRoot(2)
			.pushRoot(3)
			.insertAfter(1, 2)
			.end()
			.build();
		interp.applyMutations(insert.buffer, 0, insert.length);
		assert(serializeChildren(root), "ABC", "B and C inserted after A in order");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 4: Complex UI pattern conformance
	// ═════════════════════════════════════════════════════════════════════

	suite("Conformance — DOM: counter-like mount sequence");
	{
		const { root, interp } = createInterpreter();

		// Register template:
		// <div>
		//   <h1><!-- dynamic text for "Count: N" --></h1>
		//   <button>Up!</button>
		//   <button>Down!</button>
		// </div>
		const tmb = new TemplateMutationBuilder();
		tmb.registerTree(0, "counter", [
			{
				kind: "element",
				tag: Tag.DIV,
				children: [
					{
						kind: "element",
						tag: Tag.H1,
						children: [{ kind: "dynamic_text", dynamicIndex: 0 }],
					},
					{
						kind: "element",
						tag: Tag.BUTTON,
						children: [{ kind: "text", text: "Up!" }],
					},
					{
						kind: "element",
						tag: Tag.BUTTON,
						children: [{ kind: "text", text: "Down!" }],
					},
				],
			},
		]);
		const reg = tmb.build();
		interp.applyMutations(reg.buffer, 0, reg.length);

		// Mount: load template, assign IDs, set text, add events, append
		const mount = new MutationBuilder()
			.loadTemplate(0, 0, 1) // div → stack, id=1
			.assignId([0, 0], 2) // h1's dynamic text child → id=2
			.assignId([1], 3) // first button → id=3
			.assignId([2], 4) // second button → id=4
			.setText(2, "Count: 0")
			.newEventListener(3, "click", 100)
			.newEventListener(4, "click", 101)
			.pushRoot(1)
			.appendChildren(0, 1)
			.end()
			.build();

		interp.applyMutations(mount.buffer, 0, mount.length);

		const expected =
			"<div>" +
			"<h1>Count: 0</h1>" +
			"<button>Up!</button>" +
			"<button>Down!</button>" +
			"</div>";
		assert(serializeChildren(root), expected, "counter mounted correctly");

		// Simulate increment: update the text
		const update = new MutationBuilder().setText(2, "Count: 1").end().build();
		interp.applyMutations(update.buffer, 0, update.length);

		const expectedAfter =
			"<div>" +
			"<h1>Count: 1</h1>" +
			"<button>Up!</button>" +
			"<button>Down!</button>" +
			"</div>";
		assert(
			serializeChildren(root),
			expectedAfter,
			"counter updated after increment",
		);
	}

	suite("Conformance — DOM: todo-like mount + add + remove items");
	{
		const { root, interp } = createInterpreter();

		// Register templates:
		// 0: <div><h2><!-- dyn text --></h2><ul></ul></div>  (app shell)
		// 1: <li><!-- dyn text --></li>                       (list item)
		const tmb = new TemplateMutationBuilder(8192);
		tmb.registerTree(0, "todo-shell", [
			{
				kind: "element",
				tag: Tag.DIV,
				children: [
					{
						kind: "element",
						tag: Tag.H2,
						children: [{ kind: "dynamic_text", dynamicIndex: 0 }],
					},
					{
						kind: "element",
						tag: Tag.UL,
						children: [{ kind: "dynamic", dynamicIndex: 1 }],
					},
				],
			},
		]);
		const reg1 = tmb.build();
		interp.applyMutations(reg1.buffer, 0, reg1.length);

		const tmb2 = new TemplateMutationBuilder();
		tmb2.registerTree(1, "todo-item", [
			{
				kind: "element",
				tag: Tag.LI,
				children: [{ kind: "dynamic_text", dynamicIndex: 0 }],
			},
		]);
		const reg2 = tmb2.build();
		interp.applyMutations(reg2.buffer, 0, reg2.length);

		// Mount shell with no items (placeholder for UL's dynamic slot)
		const mount = new MutationBuilder()
			.loadTemplate(0, 0, 1) // div → stack, id=1
			.assignId([0, 0], 2) // h2's dyn text → id=2
			.assignId([1, 0], 3) // ul's dynamic placeholder → id=3
			.assignId([1], 4) // ul → id=4
			.setText(2, "Items: 0")
			.pushRoot(1)
			.appendChildren(0, 1)
			.end()
			.build();
		interp.applyMutations(mount.buffer, 0, mount.length);

		const expected0 =
			"<div>" + "<h2>Items: 0</h2>" + "<ul><!--placeholder--></ul>" + "</div>";
		assert(
			serializeChildren(root),
			expected0,
			"todo shell mounted with placeholder",
		);

		// Add first item: replace placeholder with an <li>
		const add1 = new MutationBuilder()
			.loadTemplate(1, 0, 10) // li → stack, id=10
			.assignId([0], 11) // li's dyn text → id=11
			.setText(11, "Buy milk")
			.pushRoot(10)
			.replaceWith(3, 1) // replace placeholder (id=3) with li
			.setText(2, "Items: 1")
			.end()
			.build();
		interp.applyMutations(add1.buffer, 0, add1.length);

		const expected1 =
			"<div>" + "<h2>Items: 1</h2>" + "<ul><li>Buy milk</li></ul>" + "</div>";
		assert(serializeChildren(root), expected1, "first item added");

		// Add second item after the first
		const add2 = new MutationBuilder()
			.loadTemplate(1, 0, 20)
			.assignId([0], 21)
			.setText(21, "Walk dog")
			.pushRoot(20)
			.insertAfter(10, 1)
			.setText(2, "Items: 2")
			.end()
			.build();
		interp.applyMutations(add2.buffer, 0, add2.length);

		const expected2 =
			"<div>" +
			"<h2>Items: 2</h2>" +
			"<ul><li>Buy milk</li><li>Walk dog</li></ul>" +
			"</div>";
		assert(serializeChildren(root), expected2, "second item added");

		// Remove first item
		const rm = new MutationBuilder()
			.remove(10)
			.setText(2, "Items: 1")
			.end()
			.build();
		interp.applyMutations(rm.buffer, 0, rm.length);

		const expected3 =
			"<div>" + "<h2>Items: 1</h2>" + "<ul><li>Walk dog</li></ul>" + "</div>";
		assert(serializeChildren(root), expected3, "first item removed");
	}

	suite("Conformance — DOM: conditional rendering (show/hide detail)");
	{
		const { root, interp } = createInterpreter();

		// Register template: <div><h1>Title</h1><!-- dynamic slot --></div>
		const tmb = new TemplateMutationBuilder();
		tmb.registerTree(0, "cond-root", [
			{
				kind: "element",
				tag: Tag.DIV,
				children: [
					{
						kind: "element",
						tag: Tag.H1,
						children: [{ kind: "text", text: "Title" }],
					},
					{ kind: "dynamic", dynamicIndex: 0 },
				],
			},
		]);
		const reg = tmb.build();
		interp.applyMutations(reg.buffer, 0, reg.length);

		// Detail template: <p>Detail info</p>
		const tmb2 = new TemplateMutationBuilder();
		tmb2.registerTree(1, "detail", [
			{
				kind: "element",
				tag: Tag.P,
				children: [{ kind: "text", text: "Detail info" }],
			},
		]);
		const reg2 = tmb2.build();
		interp.applyMutations(reg2.buffer, 0, reg2.length);

		// Mount with placeholder (detail hidden)
		const mount = new MutationBuilder()
			.loadTemplate(0, 0, 1)
			.assignId([1], 2) // dynamic placeholder → id=2
			.pushRoot(1)
			.appendChildren(0, 1)
			.end()
			.build();
		interp.applyMutations(mount.buffer, 0, mount.length);

		const hidden = "<div><h1>Title</h1><!--placeholder--></div>";
		assert(serializeChildren(root), hidden, "initial: detail hidden");

		// Show detail: replace placeholder with <p>
		const show = new MutationBuilder()
			.loadTemplate(1, 0, 3)
			.pushRoot(3)
			.replaceWith(2, 1)
			.end()
			.build();
		interp.applyMutations(show.buffer, 0, show.length);

		const shown = "<div><h1>Title</h1><p>Detail info</p></div>";
		assert(serializeChildren(root), shown, "detail shown");

		// Hide detail: replace <p> with placeholder
		const hide = new MutationBuilder()
			.createPlaceholder(4)
			.pushRoot(4)
			.replaceWith(3, 1)
			.end()
			.build();
		interp.applyMutations(hide.buffer, 0, hide.length);

		const hiddenAgain = "<div><h1>Title</h1><!--placeholder--></div>";
		assert(serializeChildren(root), hiddenAgain, "detail hidden again");
	}

	suite("Conformance — DOM: nested template with multiple dynamic slots");
	{
		const { root, interp } = createInterpreter();

		// Template: <div><h1><!-- dyn[0] --></h1><p><!-- dyn[1] --></p></div>
		const tmb = new TemplateMutationBuilder();
		tmb.registerTree(0, "multi-dyn", [
			{
				kind: "element",
				tag: Tag.DIV,
				children: [
					{
						kind: "element",
						tag: Tag.H1,
						children: [{ kind: "dynamic_text", dynamicIndex: 0 }],
					},
					{
						kind: "element",
						tag: Tag.P,
						children: [{ kind: "dynamic_text", dynamicIndex: 1 }],
					},
				],
			},
		]);
		const reg = tmb.build();
		interp.applyMutations(reg.buffer, 0, reg.length);

		const mount = new MutationBuilder()
			.loadTemplate(0, 0, 1)
			.assignId([0, 0], 2) // h1's dyn text
			.assignId([1, 0], 3) // p's dyn text
			.setText(2, "Welcome")
			.setText(3, "Description here")
			.pushRoot(1)
			.appendChildren(0, 1)
			.end()
			.build();

		interp.applyMutations(mount.buffer, 0, mount.length);

		const expected = "<div><h1>Welcome</h1><p>Description here</p></div>";
		assert(serializeChildren(root), expected, "multiple dyn slots rendered");
	}

	suite("Conformance — DOM: keyed list reorder simulation");
	{
		const { root, interp } = createInterpreter();

		// Register <li><!-- dyn text --></li>
		const tmb = new TemplateMutationBuilder();
		tmb.registerTree(0, "li-item", [
			{
				kind: "element",
				tag: Tag.LI,
				children: [{ kind: "dynamic_text", dynamicIndex: 0 }],
			},
		]);
		const reg = tmb.build();
		interp.applyMutations(reg.buffer, 0, reg.length);

		// Mount 3 items: A, B, C
		const mount = new MutationBuilder()
			.loadTemplate(0, 0, 1)
			.assignId([0], 11)
			.setText(11, "A")
			.loadTemplate(0, 0, 2)
			.assignId([0], 12)
			.setText(12, "B")
			.loadTemplate(0, 0, 3)
			.assignId([0], 13)
			.setText(13, "C")
			.pushRoot(1)
			.pushRoot(2)
			.pushRoot(3)
			.appendChildren(0, 3)
			.end()
			.build();
		interp.applyMutations(mount.buffer, 0, mount.length);

		assert(
			serializeChildren(root),
			"<li>A</li><li>B</li><li>C</li>",
			"initial list: A, B, C",
		);

		// Reorder to C, A, B by:
		// 1. Move C (id=3) before A (id=1)
		const reorder = new MutationBuilder()
			.pushRoot(3) // push C onto stack
			.insertBefore(1, 1) // insert C before A
			.end()
			.build();
		interp.applyMutations(reorder.buffer, 0, reorder.length);

		assert(
			serializeChildren(root),
			"<li>C</li><li>A</li><li>B</li>",
			"reordered to C, A, B",
		);
	}

	suite("Conformance — DOM: multiple templates in same buffer");
	{
		const { root, interp } = createInterpreter();

		// Register template 0: <div></div>
		const tmb0 = new TemplateMutationBuilder();
		tmb0.registerSimpleElement(0, "div-tmpl", Tag.DIV);
		const r0 = tmb0.build();
		interp.applyMutations(r0.buffer, 0, r0.length);

		// Register template 1: <span></span>
		const tmb1 = new TemplateMutationBuilder();
		tmb1.registerSimpleElement(1, "span-tmpl", Tag.SPAN);
		const r1 = tmb1.build();
		interp.applyMutations(r1.buffer, 0, r1.length);

		// Mount both
		const mount = new MutationBuilder()
			.loadTemplate(0, 0, 1) // div
			.loadTemplate(1, 0, 2) // span
			.pushRoot(1)
			.pushRoot(2)
			.appendChildren(0, 2)
			.end()
			.build();
		interp.applyMutations(mount.buffer, 0, mount.length);

		assert(
			serializeChildren(root),
			"<div></div><span></span>",
			"two different templates mounted",
		);
	}

	suite("Conformance — DOM: empty mutation buffer (End only)");
	{
		const { root, interp } = createInterpreter();
		const { buffer, length } = new MutationBuilder().end().build();
		interp.applyMutations(buffer, 0, length);
		assert(serializeChildren(root), "", "root unchanged after empty buffer");
	}

	suite("Conformance — DOM: accumulation across multiple apply calls");
	{
		const { root, interp } = createInterpreter();

		// First batch: add text A
		const b1 = new MutationBuilder()
			.createTextNode(1, "A")
			.pushRoot(1)
			.appendChildren(0, 1)
			.end()
			.build();
		interp.applyMutations(b1.buffer, 0, b1.length);
		assert(serializeChildren(root), "A", "first text mounted");

		// Second batch: add text B
		const b2 = new MutationBuilder()
			.createTextNode(2, "B")
			.pushRoot(2)
			.appendChildren(0, 1)
			.end()
			.build();
		interp.applyMutations(b2.buffer, 0, b2.length);
		assert(serializeChildren(root), "AB", "second text appended");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 5: WASM ↔ JS binary format conformance
	// ═════════════════════════════════════════════════════════════════════
	//
	// These tests build mutation buffers via WASM (Mojo write_op_*
	// exports) and via JS (MutationBuilder), then compare the decoded
	// mutations to ensure both sides agree on the wire format.
	//
	// The WASM exports use the write_op_*(buf, off, ...) pattern which
	// creates an inline MutationWriter at the given offset. String
	// arguments are passed as Mojo String structs via writeStringStruct().

	suite("Conformance — WASM vs JS: PushRoot byte-level match");
	{
		const buf = allocBuf(fns);

		// WASM side: write_op_push_root(buf, 0, 42) + end
		let off = 0;
		off = fns.write_op_push_root(buf, off, 42);
		off = fns.write_op_end(buf, off);
		const wasmOffset = off;

		// Read WASM bytes
		const mem = getMemory();
		const wasmBytes = new Uint8Array(mem.buffer, Number(buf), wasmOffset);

		// JS side: build the same mutation
		const { buffer, length: jsLength } = new MutationBuilder()
			.pushRoot(42)
			.end()
			.build();
		const jsBytes = new Uint8Array(buffer, 0, jsLength);

		assert(wasmOffset, jsLength, "WASM and JS buffer lengths match");
		assert(
			compareBytes("PushRoot", wasmBytes, jsBytes, jsLength),
			true,
			`bytes match: [${hexDump(wasmBytes, wasmOffset)}]`,
		);

		// Also verify decoded mutations match
		const wasmMuts = new MutationReader(
			mem.buffer,
			Number(buf),
			wasmOffset,
		).readAll();
		const jsMuts = new MutationReader(buffer, 0, jsLength).readAll();
		assert(wasmMuts.length, jsMuts.length, "same mutation count");
		assert(wasmMuts[0].op, jsMuts[0].op, "same opcode");
		assert(
			(wasmMuts[0] as { id: number }).id,
			(jsMuts[0] as { id: number }).id,
			"same id",
		);

		freeBuf(fns, buf);
	}

	suite("Conformance — WASM vs JS: CreateTextNode byte-level match");
	{
		const buf = allocBuf(fns);

		const textPtr = writeStringStruct("hello");
		let off = 0;
		off = fns.write_op_create_text_node(buf, off, 5, textPtr);
		off = fns.write_op_end(buf, off);
		const wasmOffset = off;

		const mem = getMemory();
		const wasmBytes = new Uint8Array(mem.buffer, Number(buf), wasmOffset);

		const { buffer, length: jsLength } = new MutationBuilder()
			.createTextNode(5, "hello")
			.end()
			.build();
		const jsBytes = new Uint8Array(buffer, 0, jsLength);

		assert(wasmOffset, jsLength, "lengths match");
		assert(
			compareBytes("CreateTextNode", wasmBytes, jsBytes, jsLength),
			true,
			`bytes match: [${hexDump(wasmBytes, wasmOffset)}]`,
		);

		freeBuf(fns, buf);
	}

	suite("Conformance — WASM vs JS: SetAttribute byte-level match");
	{
		const buf = allocBuf(fns);

		const namePtr = writeStringStruct("class");
		const valPtr = writeStringStruct("active");
		let off = 0;
		off = fns.write_op_set_attribute(buf, off, 3, 0, namePtr, valPtr);
		off = fns.write_op_end(buf, off);
		const wasmOffset = off;

		const mem = getMemory();
		const wasmBytes = new Uint8Array(mem.buffer, Number(buf), wasmOffset);

		const { buffer, length: jsLength } = new MutationBuilder()
			.setAttribute(3, 0, "class", "active")
			.end()
			.build();
		const jsBytes = new Uint8Array(buffer, 0, jsLength);

		assert(wasmOffset, jsLength, "lengths match");
		assert(
			compareBytes("SetAttribute", wasmBytes, jsBytes, jsLength),
			true,
			`bytes match: [${hexDump(wasmBytes, wasmOffset)}]`,
		);

		freeBuf(fns, buf);
	}

	suite("Conformance — WASM vs JS: AppendChildren byte-level match");
	{
		const buf = allocBuf(fns);

		let off = 0;
		off = fns.write_op_append_children(buf, off, 0, 3);
		off = fns.write_op_end(buf, off);
		const wasmOffset = off;

		const mem = getMemory();
		const wasmBytes = new Uint8Array(mem.buffer, Number(buf), wasmOffset);

		const { buffer, length: jsLength } = new MutationBuilder()
			.appendChildren(0, 3)
			.end()
			.build();
		const jsBytes = new Uint8Array(buffer, 0, jsLength);

		assert(wasmOffset, jsLength, "lengths match");
		assert(
			compareBytes("AppendChildren", wasmBytes, jsBytes, jsLength),
			true,
			`bytes match`,
		);

		freeBuf(fns, buf);
	}

	suite("Conformance — WASM vs JS: SetText byte-level match");
	{
		const buf = allocBuf(fns);

		const textPtr = writeStringStruct("Count: 42");
		let off = 0;
		off = fns.write_op_set_text(buf, off, 7, textPtr);
		off = fns.write_op_end(buf, off);
		const wasmOffset = off;

		const mem = getMemory();
		const wasmBytes = new Uint8Array(mem.buffer, Number(buf), wasmOffset);

		const { buffer, length: jsLength } = new MutationBuilder()
			.setText(7, "Count: 42")
			.end()
			.build();
		const jsBytes = new Uint8Array(buffer, 0, jsLength);

		assert(wasmOffset, jsLength, "lengths match");
		assert(
			compareBytes("SetText", wasmBytes, jsBytes, jsLength),
			true,
			`bytes match`,
		);

		freeBuf(fns, buf);
	}

	suite("Conformance — WASM vs JS: NewEventListener byte-level match");
	{
		const buf = allocBuf(fns);

		const namePtr = writeStringStruct("click");
		let off = 0;
		off = fns.write_op_new_event_listener(buf, off, 10, 200, namePtr);
		off = fns.write_op_end(buf, off);
		const wasmOffset = off;

		const mem = getMemory();
		const wasmBytes = new Uint8Array(mem.buffer, Number(buf), wasmOffset);

		const { buffer, length: jsLength } = new MutationBuilder()
			.newEventListener(10, "click", 200)
			.end()
			.build();
		const jsBytes = new Uint8Array(buffer, 0, jsLength);

		assert(wasmOffset, jsLength, "lengths match");
		assert(
			compareBytes("NewEventListener", wasmBytes, jsBytes, jsLength),
			true,
			`bytes match`,
		);

		freeBuf(fns, buf);
	}

	suite("Conformance — WASM vs JS: Remove byte-level match");
	{
		const buf = allocBuf(fns);

		let off = 0;
		off = fns.write_op_remove(buf, off, 99);
		off = fns.write_op_end(buf, off);
		const wasmOffset = off;

		const mem = getMemory();
		const wasmBytes = new Uint8Array(mem.buffer, Number(buf), wasmOffset);

		const { buffer, length: jsLength } = new MutationBuilder()
			.remove(99)
			.end()
			.build();
		const jsBytes = new Uint8Array(buffer, 0, jsLength);

		assert(wasmOffset, jsLength, "lengths match");
		assert(
			compareBytes("Remove", wasmBytes, jsBytes, jsLength),
			true,
			`bytes match`,
		);

		freeBuf(fns, buf);
	}

	suite("Conformance — WASM vs JS: LoadTemplate byte-level match");
	{
		const buf = allocBuf(fns);

		let off = 0;
		off = fns.write_op_load_template(buf, off, 5, 0, 20);
		off = fns.write_op_end(buf, off);
		const wasmOffset = off;

		const mem = getMemory();
		const wasmBytes = new Uint8Array(mem.buffer, Number(buf), wasmOffset);

		const { buffer, length: jsLength } = new MutationBuilder()
			.loadTemplate(5, 0, 20)
			.end()
			.build();
		const jsBytes = new Uint8Array(buffer, 0, jsLength);

		assert(wasmOffset, jsLength, "lengths match");
		assert(
			compareBytes("LoadTemplate", wasmBytes, jsBytes, jsLength),
			true,
			`bytes match`,
		);

		freeBuf(fns, buf);
	}

	suite("Conformance — WASM vs JS: CreatePlaceholder byte-level match");
	{
		const buf = allocBuf(fns);

		let off = 0;
		off = fns.write_op_create_placeholder(buf, off, 33);
		off = fns.write_op_end(buf, off);
		const wasmOffset = off;

		const mem = getMemory();
		const wasmBytes = new Uint8Array(mem.buffer, Number(buf), wasmOffset);

		const { buffer, length: jsLength } = new MutationBuilder()
			.createPlaceholder(33)
			.end()
			.build();
		const jsBytes = new Uint8Array(buffer, 0, jsLength);

		assert(wasmOffset, jsLength, "lengths match");
		assert(
			compareBytes("CreatePlaceholder", wasmBytes, jsBytes, jsLength),
			true,
			`bytes match`,
		);

		freeBuf(fns, buf);
	}

	suite("Conformance — WASM vs JS: ReplaceWith byte-level match");
	{
		const buf = allocBuf(fns);

		let off = 0;
		off = fns.write_op_replace_with(buf, off, 15, 2);
		off = fns.write_op_end(buf, off);
		const wasmOffset = off;

		const mem = getMemory();
		const wasmBytes = new Uint8Array(mem.buffer, Number(buf), wasmOffset);

		const { buffer, length: jsLength } = new MutationBuilder()
			.replaceWith(15, 2)
			.end()
			.build();
		const jsBytes = new Uint8Array(buffer, 0, jsLength);

		assert(wasmOffset, jsLength, "lengths match");
		assert(
			compareBytes("ReplaceWith", wasmBytes, jsBytes, jsLength),
			true,
			`bytes match`,
		);

		freeBuf(fns, buf);
	}

	suite("Conformance — WASM vs JS: InsertAfter byte-level match");
	{
		const buf = allocBuf(fns);

		let off = 0;
		off = fns.write_op_insert_after(buf, off, 8, 1);
		off = fns.write_op_end(buf, off);
		const wasmOffset = off;

		const mem = getMemory();
		const wasmBytes = new Uint8Array(mem.buffer, Number(buf), wasmOffset);

		const { buffer, length: jsLength } = new MutationBuilder()
			.insertAfter(8, 1)
			.end()
			.build();
		const jsBytes = new Uint8Array(buffer, 0, jsLength);

		assert(wasmOffset, jsLength, "lengths match");
		assert(
			compareBytes("InsertAfter", wasmBytes, jsBytes, jsLength),
			true,
			`bytes match`,
		);

		freeBuf(fns, buf);
	}

	suite("Conformance — WASM vs JS: InsertBefore byte-level match");
	{
		const buf = allocBuf(fns);

		let off = 0;
		off = fns.write_op_insert_before(buf, off, 9, 1);
		off = fns.write_op_end(buf, off);
		const wasmOffset = off;

		const mem = getMemory();
		const wasmBytes = new Uint8Array(mem.buffer, Number(buf), wasmOffset);

		const { buffer, length: jsLength } = new MutationBuilder()
			.insertBefore(9, 1)
			.end()
			.build();
		const jsBytes = new Uint8Array(buffer, 0, jsLength);

		assert(wasmOffset, jsLength, "lengths match");
		assert(
			compareBytes("InsertBefore", wasmBytes, jsBytes, jsLength),
			true,
			`bytes match`,
		);

		freeBuf(fns, buf);
	}

	suite("Conformance — WASM vs JS: RemoveAttribute byte-level match");
	{
		const buf = allocBuf(fns);

		const namePtr = writeStringStruct("hidden");
		let off = 0;
		off = fns.write_op_remove_attribute(buf, off, 4, 0, namePtr);
		off = fns.write_op_end(buf, off);
		const wasmOffset = off;

		const mem = getMemory();
		const wasmBytes = new Uint8Array(mem.buffer, Number(buf), wasmOffset);

		const { buffer, length: jsLength } = new MutationBuilder()
			.removeAttribute(4, 0, "hidden")
			.end()
			.build();
		const jsBytes = new Uint8Array(buffer, 0, jsLength);

		assert(wasmOffset, jsLength, "lengths match");
		assert(
			compareBytes("RemoveAttribute", wasmBytes, jsBytes, jsLength),
			true,
			`bytes match`,
		);

		freeBuf(fns, buf);
	}

	suite("Conformance — WASM vs JS: RemoveEventListener byte-level match");
	{
		const buf = allocBuf(fns);

		const namePtr = writeStringStruct("click");
		let off = 0;
		off = fns.write_op_remove_event_listener(buf, off, 6, namePtr);
		off = fns.write_op_end(buf, off);
		const wasmOffset = off;

		const mem = getMemory();
		const wasmBytes = new Uint8Array(mem.buffer, Number(buf), wasmOffset);

		const { buffer, length: jsLength } = new MutationBuilder()
			.removeEventListener(6, "click")
			.end()
			.build();
		const jsBytes = new Uint8Array(buffer, 0, jsLength);

		assert(wasmOffset, jsLength, "lengths match");
		assert(
			compareBytes("RemoveEventListener", wasmBytes, jsBytes, jsLength),
			true,
			`bytes match`,
		);

		freeBuf(fns, buf);
	}

	suite("Conformance — WASM vs JS: multi-op sequence decoded identically");
	{
		const buf = allocBuf(fns);

		// Write a realistic sequence via WASM
		const hiPtr = writeStringStruct("Hi");
		const byePtr = writeStringStruct("Bye");
		let off = 0;
		off = fns.write_op_create_text_node(buf, off, 1, hiPtr);
		off = fns.write_op_push_root(buf, off, 1);
		off = fns.write_op_append_children(buf, off, 0, 1);
		off = fns.write_op_set_text(buf, off, 1, byePtr);
		off = fns.write_op_end(buf, off);
		const wasmOffset = off;

		// Same sequence via JS
		const { buffer, length: jsLength } = new MutationBuilder()
			.createTextNode(1, "Hi")
			.pushRoot(1)
			.appendChildren(0, 1)
			.setText(1, "Bye")
			.end()
			.build();

		// Decode both and compare
		const mem = getMemory();
		const wasmMuts = new MutationReader(
			mem.buffer,
			Number(buf),
			wasmOffset,
		).readAll();
		const jsMuts = new MutationReader(buffer, 0, jsLength).readAll();

		assert(wasmMuts.length, jsMuts.length, "same mutation count");
		assert(wasmMuts.length, 4, "4 mutations");

		for (let i = 0; i < wasmMuts.length; i++) {
			assert(wasmMuts[i].op, jsMuts[i].op, `mut[${i}] opcodes match`);
		}

		// Verify CreateTextNode content
		const wCtx = wasmMuts[0] as { text: string };
		const jCtx = jsMuts[0] as { text: string };
		assert(wCtx.text, jCtx.text, "CreateTextNode text matches");

		// Verify SetText content
		const wSt = wasmMuts[3] as { text: string };
		const jSt = jsMuts[3] as { text: string };
		assert(wSt.text, jSt.text, "SetText text matches");

		freeBuf(fns, buf);
	}

	suite("Conformance — WASM vs JS: unicode text byte-level match");
	{
		const buf = allocBuf(fns);

		const textPtr = writeStringStruct("日本語テスト 🎉");
		let off = 0;
		off = fns.write_op_create_text_node(buf, off, 1, textPtr);
		off = fns.write_op_end(buf, off);
		const wasmOffset = off;

		const mem = getMemory();
		const wasmBytes = new Uint8Array(mem.buffer, Number(buf), wasmOffset);

		const { buffer, length: jsLength } = new MutationBuilder()
			.createTextNode(1, "日本語テスト 🎉")
			.end()
			.build();
		const jsBytes = new Uint8Array(buffer, 0, jsLength);

		assert(wasmOffset, jsLength, "lengths match for unicode");
		assert(
			compareBytes("Unicode", wasmBytes, jsBytes, jsLength),
			true,
			"unicode bytes match",
		);

		freeBuf(fns, buf);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 6: WASM-generated mutations through Interpreter
	// ═════════════════════════════════════════════════════════════════════
	//
	// End-to-end: Mojo write_op_* → binary buffer → JS Interpreter → DOM

	suite("Conformance — E2E: WASM-written mount sequence renders correct DOM");
	{
		const { root, interp, templates } = createInterpreter();

		// Register a simple <div></div> template via JS
		const dom = createDOM();
		const div = dom.document.createElement("div");
		templates.register(0, [div]);

		// Use WASM write_op_* to produce mount sequence
		const buf = allocBuf(fns);
		let off = 0;
		off = fns.write_op_load_template(buf, off, 0, 0, 1);
		off = fns.write_op_push_root(buf, off, 1);
		off = fns.write_op_append_children(buf, off, 0, 1);
		off = fns.write_op_end(buf, off);
		const wasmOffset = off;

		// Apply WASM-generated buffer to Interpreter
		const mem = getMemory();
		interp.applyMutations(mem.buffer, Number(buf), wasmOffset);

		assert(serializeChildren(root), "<div></div>", "WASM mount rendered div");

		freeBuf(fns, buf);
	}

	suite("Conformance — E2E: WASM-written text + update renders correctly");
	{
		const { root, interp } = createInterpreter();

		// Mount text node via WASM
		const buf = allocBuf(fns);
		const initialPtr = writeStringStruct("Initial");
		let off = 0;
		off = fns.write_op_create_text_node(buf, off, 1, initialPtr);
		off = fns.write_op_push_root(buf, off, 1);
		off = fns.write_op_append_children(buf, off, 0, 1);
		off = fns.write_op_end(buf, off);
		const offset1 = off;

		const mem = getMemory();
		interp.applyMutations(mem.buffer, Number(buf), offset1);
		assert(serializeChildren(root), "Initial", "initial WASM text rendered");

		// Update text via WASM (reuse buf from a fresh offset)
		const buf2 = allocBuf(fns);
		const updatedPtr = writeStringStruct("Updated");
		let off2 = 0;
		off2 = fns.write_op_set_text(buf2, off2, 1, updatedPtr);
		off2 = fns.write_op_end(buf2, off2);
		const offset2 = off2;

		interp.applyMutations(mem.buffer, Number(buf2), offset2);
		assert(serializeChildren(root), "Updated", "updated WASM text rendered");

		freeBuf(fns, buf);
		freeBuf(fns, buf2);
	}

	suite("Conformance — E2E: WASM attribute set/remove renders correctly");
	{
		const { root, interp, templates } = createInterpreter();

		const dom = createDOM();
		const div = dom.document.createElement("div");
		templates.register(0, [div]);

		// Mount div + set attribute via WASM
		const buf = allocBuf(fns);
		const namePtr = writeStringStruct("data-value");
		const valPtr = writeStringStruct("42");
		let off = 0;
		off = fns.write_op_load_template(buf, off, 0, 0, 1);
		off = fns.write_op_set_attribute(buf, off, 1, 0, namePtr, valPtr);
		off = fns.write_op_push_root(buf, off, 1);
		off = fns.write_op_append_children(buf, off, 0, 1);
		off = fns.write_op_end(buf, off);
		const offset1 = off;

		const mem = getMemory();
		interp.applyMutations(mem.buffer, Number(buf), offset1);
		assert(
			serializeChildren(root),
			'<div data-value="42"></div>',
			"WASM attribute set",
		);

		// Remove attribute via WASM
		const buf2 = allocBuf(fns);
		const rmNamePtr = writeStringStruct("data-value");
		let off2 = 0;
		off2 = fns.write_op_remove_attribute(buf2, off2, 1, 0, rmNamePtr);
		off2 = fns.write_op_end(buf2, off2);
		const offset2 = off2;

		interp.applyMutations(mem.buffer, Number(buf2), offset2);
		assert(serializeChildren(root), "<div></div>", "WASM attribute removed");

		freeBuf(fns, buf);
		freeBuf(fns, buf2);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 7: Opcode binary layout verification
	// ═════════════════════════════════════════════════════════════════════
	//
	// Verify exact byte layout of each opcode matches the protocol spec.

	suite("Conformance — binary layout: PushRoot is [0x0F, id:u32le]");
	{
		const { buffer, length } = new MutationBuilder()
			.pushRoot(0x01020304)
			.build();
		const bytes = new Uint8Array(buffer, 0, length);
		assert(bytes[0], 0x0f, "opcode 0x0F");
		assert(bytes[1], 0x04, "id byte 0 (LE)");
		assert(bytes[2], 0x03, "id byte 1");
		assert(bytes[3], 0x02, "id byte 2");
		assert(bytes[4], 0x01, "id byte 3");
		assert(length, 5, "total 5 bytes");
	}

	suite(
		"Conformance — binary layout: AppendChildren is [0x01, id:u32le, m:u32le]",
	);
	{
		const { buffer, length } = new MutationBuilder()
			.appendChildren(1, 2)
			.build();
		const bytes = new Uint8Array(buffer, 0, length);
		assert(bytes[0], 0x01, "opcode 0x01");
		// id = 1 in LE: 01 00 00 00
		assert(bytes[1], 0x01, "id[0]");
		assert(bytes[2], 0x00, "id[1]");
		assert(bytes[3], 0x00, "id[2]");
		assert(bytes[4], 0x00, "id[3]");
		// m = 2 in LE: 02 00 00 00
		assert(bytes[5], 0x02, "m[0]");
		assert(bytes[6], 0x00, "m[1]");
		assert(length, 9, "total 9 bytes");
	}

	suite(
		"Conformance — binary layout: CreateTextNode is [0x04, id:u32le, len:u32le, text]",
	);
	{
		const { buffer, length } = new MutationBuilder()
			.createTextNode(1, "AB")
			.build();
		const bytes = new Uint8Array(buffer, 0, length);
		assert(bytes[0], 0x04, "opcode 0x04");
		// id = 1 LE
		assert(bytes[1], 0x01, "id[0]");
		// len = 2 LE
		assert(bytes[5], 0x02, "text len[0]");
		assert(bytes[6], 0x00, "text len[1]");
		// text = "AB"
		assert(bytes[9], 0x41, "text[0] = 'A'");
		assert(bytes[10], 0x42, "text[1] = 'B'");
		assert(length, 11, "total 11 bytes");
	}

	suite(
		"Conformance — binary layout: SetAttribute is [0x0A, id:u32le, ns:u8, name_len:u16le, name, val_len:u32le, val]",
	);
	{
		const { buffer, length } = new MutationBuilder()
			.setAttribute(2, 0, "id", "x")
			.build();
		const bytes = new Uint8Array(buffer, 0, length);
		assert(bytes[0], 0x0a, "opcode 0x0A");
		// id = 2 LE
		assert(bytes[1], 0x02, "id[0]");
		// ns = 0
		assert(bytes[5], 0x00, "ns");
		// name_len = 2 LE (u16)
		assert(bytes[6], 0x02, "name_len[0]");
		assert(bytes[7], 0x00, "name_len[1]");
		// name = "id"
		assert(bytes[8], 0x69, "'i'");
		assert(bytes[9], 0x64, "'d'");
		// val_len = 1 LE (u32)
		assert(bytes[10], 0x01, "val_len[0]");
		// val = "x"
		assert(bytes[14], 0x78, "'x'");
		assert(length, 15, "total 15 bytes");
	}

	suite("Conformance — binary layout: End sentinel is [0x00]");
	{
		const { buffer, length } = new MutationBuilder().end().build();
		const bytes = new Uint8Array(buffer, 0, length);
		assert(bytes[0], 0x00, "End = 0x00");
		assert(length, 1, "1 byte");
	}

	suite("Conformance — binary layout: Remove is [0x0E, id:u32le]");
	{
		const { buffer, length } = new MutationBuilder().remove(7).build();
		const bytes = new Uint8Array(buffer, 0, length);
		assert(bytes[0], 0x0e, "opcode 0x0E");
		assert(bytes[1], 0x07, "id[0]");
		assert(bytes[2], 0x00, "id[1]");
		assert(bytes[3], 0x00, "id[2]");
		assert(bytes[4], 0x00, "id[3]");
		assert(length, 5, "total 5 bytes");
	}

	suite(
		"Conformance — binary layout: AssignId is [0x02, path_len:u8, path, id:u32le]",
	);
	{
		const { buffer, length } = new MutationBuilder()
			.assignId([0, 1, 2], 50)
			.build();
		const bytes = new Uint8Array(buffer, 0, length);
		assert(bytes[0], 0x02, "opcode 0x02");
		assert(bytes[1], 0x03, "path_len = 3");
		assert(bytes[2], 0x00, "path[0] = 0");
		assert(bytes[3], 0x01, "path[1] = 1");
		assert(bytes[4], 0x02, "path[2] = 2");
		assert(bytes[5], 0x32, "id[0] = 50");
		assert(bytes[6], 0x00, "id[1]");
		assert(length, 9, "total 9 bytes");
	}

	suite(
		"Conformance — binary layout: LoadTemplate is [0x05, tmplId:u32le, index:u32le, id:u32le]",
	);
	{
		const { buffer, length } = new MutationBuilder()
			.loadTemplate(10, 0, 20)
			.build();
		const bytes = new Uint8Array(buffer, 0, length);
		assert(bytes[0], 0x05, "opcode 0x05");
		assert(bytes[1], 0x0a, "tmplId[0] = 10");
		assert(bytes[5], 0x00, "index[0] = 0");
		assert(bytes[9], 0x14, "id[0] = 20");
		assert(length, 13, "total 13 bytes");
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 8: Edge cases and robustness
	// ═════════════════════════════════════════════════════════════════════

	suite("Conformance — edge: large element ID values");
	{
		const largeId = 0xffffffff; // max u32
		const { buffer, length } = new MutationBuilder()
			.pushRoot(largeId)
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		// JS uses 32-bit unsigned via DataView.getUint32
		assert(
			(muts[0] as { id: number }).id,
			largeId >>> 0,
			"max u32 ID preserved",
		);
	}

	suite("Conformance — edge: empty attribute name and value");
	{
		const { buffer, length } = new MutationBuilder()
			.setAttribute(1, 0, "", "")
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		const m = muts[0] as { name: string; value: string };
		assert(m.name, "", "empty name preserved");
		assert(m.value, "", "empty value preserved");
	}

	suite("Conformance — edge: long string in text node");
	{
		const longText = "x".repeat(10000);
		const { buffer, length } = new MutationBuilder(65536)
			.createTextNode(1, longText)
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(muts.length, 1, "one mutation");
		assert(
			(muts[0] as { text: string }).text.length,
			10000,
			"10000-char text preserved",
		);
		assert(
			(muts[0] as { text: string }).text,
			longText,
			"text content matches",
		);
	}

	suite("Conformance — edge: deep path in AssignId");
	{
		const deepPath = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
		const { buffer, length } = new MutationBuilder()
			.assignId(deepPath, 100)
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		const m = muts[0] as { path: Uint8Array };
		assert(m.path.length, 10, "path length is 10");
		for (let i = 0; i < 10; i++) {
			assert(m.path[i], i, `path[${i}] is ${i}`);
		}
	}

	suite("Conformance — edge: special chars in attribute values");
	{
		const { buffer, length } = new MutationBuilder()
			.setAttribute(1, 0, "style", 'color: "red"; font-size: 12px')
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		const m = muts[0] as { value: string };
		assert(
			m.value,
			'color: "red"; font-size: 12px',
			"special chars preserved in attribute",
		);
	}

	suite("Conformance — edge: newlines and tabs in text");
	{
		const text = "line1\nline2\ttab";
		const { buffer, length } = new MutationBuilder()
			.createTextNode(1, text)
			.end()
			.build();
		const muts = new MutationReader(buffer, 0, length).readAll();
		assert(
			(muts[0] as { text: string }).text,
			text,
			"whitespace chars preserved",
		);
	}

	suite("Conformance — DOM: complex app with all opcode types exercised");
	{
		const { root, interp } = createInterpreter();

		// Register <section><header><!-- dyn[0] --></header><main><!-- dyn[1] --></main></section>
		const tmb = new TemplateMutationBuilder(8192);
		tmb.registerTree(0, "app-shell", [
			{
				kind: "element",
				tag: Tag.SECTION,
				attrs: [{ kind: "static", name: "class", value: "app" }],
				children: [
					{
						kind: "element",
						tag: Tag.HEADER,
						children: [{ kind: "dynamic_text", dynamicIndex: 0 }],
					},
					{
						kind: "element",
						tag: Tag.MAIN,
						children: [{ kind: "dynamic", dynamicIndex: 1 }],
					},
				],
			},
		]);
		const r0 = tmb.build();
		interp.applyMutations(r0.buffer, 0, r0.length);

		// Register <p><!-- dyn[0] --></p> (content template)
		const tmb2 = new TemplateMutationBuilder();
		tmb2.registerTree(1, "content", [
			{
				kind: "element",
				tag: Tag.P,
				children: [{ kind: "dynamic_text", dynamicIndex: 0 }],
			},
		]);
		const r1 = tmb2.build();
		interp.applyMutations(r1.buffer, 0, r1.length);

		// 1. Mount shell
		const mount = new MutationBuilder()
			.loadTemplate(0, 0, 1) // section
			.assignId([0, 0], 2) // header's dyn text
			.assignId([1, 0], 3) // main's dyn placeholder
			.assignId([1], 4) // main element
			.setText(2, "My App")
			.pushRoot(1)
			.appendChildren(0, 1)
			.end()
			.build();
		interp.applyMutations(mount.buffer, 0, mount.length);

		const step1 =
			'<section class="app">' +
			"<header>My App</header>" +
			"<main><!--placeholder--></main>" +
			"</section>";
		assert(serializeChildren(root), step1, "step 1: shell mounted");

		// 2. Show content (replace placeholder)
		const show = new MutationBuilder()
			.loadTemplate(1, 0, 10)
			.assignId([0], 11)
			.setText(11, "Welcome!")
			.pushRoot(10)
			.replaceWith(3, 1)
			.end()
			.build();
		interp.applyMutations(show.buffer, 0, show.length);

		const step2 =
			'<section class="app">' +
			"<header>My App</header>" +
			"<main><p>Welcome!</p></main>" +
			"</section>";
		assert(serializeChildren(root), step2, "step 2: content shown");

		// 3. Update header text
		const update1 = new MutationBuilder().setText(2, "My App v2").end().build();
		interp.applyMutations(update1.buffer, 0, update1.length);

		const step3 =
			'<section class="app">' +
			"<header>My App v2</header>" +
			"<main><p>Welcome!</p></main>" +
			"</section>";
		assert(serializeChildren(root), step3, "step 3: header updated");

		// 4. Update content text
		const update2 = new MutationBuilder().setText(11, "Goodbye!").end().build();
		interp.applyMutations(update2.buffer, 0, update2.length);

		const step4 =
			'<section class="app">' +
			"<header>My App v2</header>" +
			"<main><p>Goodbye!</p></main>" +
			"</section>";
		assert(serializeChildren(root), step4, "step 4: content updated");

		// 5. Set attribute on section
		const setAttr = new MutationBuilder()
			.setAttribute(1, 0, "data-version", "2")
			.end()
			.build();
		interp.applyMutations(setAttr.buffer, 0, setAttr.length);

		const step5 =
			'<section class="app" data-version="2">' +
			"<header>My App v2</header>" +
			"<main><p>Goodbye!</p></main>" +
			"</section>";
		assert(serializeChildren(root), step5, "step 5: attribute added");

		// 6. Replace content with placeholder (hide)
		const hide = new MutationBuilder()
			.createPlaceholder(20)
			.pushRoot(20)
			.replaceWith(10, 1)
			.end()
			.build();
		interp.applyMutations(hide.buffer, 0, hide.length);

		const step6 =
			'<section class="app" data-version="2">' +
			"<header>My App v2</header>" +
			"<main><!--placeholder--></main>" +
			"</section>";
		assert(serializeChildren(root), step6, "step 6: content hidden");
	}
}
