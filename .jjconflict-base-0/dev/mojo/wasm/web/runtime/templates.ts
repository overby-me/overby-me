// TemplateCache — Caches template DOM structures for efficient cloning.
//
// Templates are static UI blueprints compiled on the Mojo side.  The JS
// interpreter needs a DOM representation of each template so that
// LoadTemplate mutations can clone them cheaply.
//
// Each template may have multiple root nodes (e.g. a fragment template).
// The cache stores an array of root Nodes per template ID.  On
// instantiation, the requested root is deep-cloned and returned.
//
// Registration can happen in four ways:
//   1. `register(id, roots)` — provide pre-built DOM nodes directly.
//   2. `registerFromHtml(id, html)` — parse an HTML string.
//   3. `registerFromWasm(id, ...)` — query WASM template structure and
//      build the DOM programmatically (for full-app integration).
//   4. `registerFromMutation(m)` — build from a decoded RegisterTemplate
//      mutation (Phase 11 automatic template serialization).

import type {
	MutationRegisterTemplate,
	TemplateAttr,
	TemplateNode,
} from "./protocol.ts";
import { tagName } from "./tags.ts";

// ── Template node kind constants (must match src/vdom/template.mojo) ────────

const TNODE_ELEMENT = 0;
const TNODE_TEXT = 1;
const TNODE_DYNAMIC = 2;
const TNODE_DYNAMIC_TEXT = 3;

// ── Attribute kind constants ────────────────────────────────────────────────

const TATTR_STATIC = 0;

// ── TemplateCache ───────────────────────────────────────────────────────────

/**
 * Caches parsed template DOM structures and clones them on demand.
 *
 * Each template is stored as an array of root `Node` references.
 * `instantiate(id, index)` returns a deep clone of the `index`-th root.
 */
export class TemplateCache {
	private cache: Map<number, Node[]> = new Map();
	private doc: Document;

	/**
	 * @param doc - The Document to use for creating/cloning nodes.
	 *              In a browser this is `document`; in tests it comes
	 *              from a headless DOM implementation (e.g. linkedom).
	 */
	constructor(doc?: Document) {
		this.doc = doc ?? globalThis.document;
	}

	// ── Registration ──────────────────────────────────────────────────

	/**
	 * Register a template from pre-built root nodes.
	 *
	 * The nodes are deep-cloned on storage so that the originals can be
	 * mutated freely by the caller.
	 */
	register(id: number, roots: Node[]): void {
		// Store deep clones so the cache owns its own copies
		const cloned = roots.map((r) => r.cloneNode(true));
		this.cache.set(id, cloned);
	}

	/**
	 * Register a template by parsing an HTML string.
	 *
	 * The string is parsed into a temporary container; all resulting
	 * child nodes become the template's roots.
	 *
	 * Example:
	 *   cache.registerFromHtml(0, '<div class="c"><p></p></div>');
	 *   // Template 0 has one root: the <div>
	 */
	registerFromHtml(id: number, html: string): void {
		const tpl = this.doc.createElement("template");
		tpl.innerHTML = html;

		// For environments where HTMLTemplateElement.content works:
		const frag = (tpl as HTMLTemplateElement).content ?? tpl;
		const roots: Node[] = [];
		for (let i = 0; i < frag.childNodes.length; i++) {
			roots.push(frag.childNodes[i].cloneNode(true));
		}

		// Fallback: if content was empty but tpl has children directly
		// (some headless DOMs behave this way)
		if (roots.length === 0) {
			for (let i = 0; i < tpl.childNodes.length; i++) {
				roots.push(tpl.childNodes[i].cloneNode(true));
			}
		}

		this.cache.set(id, roots);
	}

	/**
	 * Register a template by querying WASM template structure exports.
	 *
	 * Builds the DOM tree programmatically from the Mojo template
	 * registry.  This is the primary registration path for a full app.
	 *
	 * @param id      - Template ID.
	 * @param fns     - WASM exports object.
	 * @param rtPtr   - Pointer to the Mojo Runtime.
	 * @param readStr - Function that reads a Mojo String struct from
	 *                  WASM memory: `(structPtr: bigint) => string`.
	 */
	registerFromWasm(
		id: number,
		fns: WasmTemplateExports,
		rtPtr: bigint,
		readStr: (ptr: bigint) => string,
		allocStr: () => bigint,
	): void {
		const rootCount = fns.tmpl_root_count(rtPtr, id);
		const roots: Node[] = [];

		for (let r = 0; r < rootCount; r++) {
			const rootNodeIdx = fns.tmpl_get_root_index(rtPtr, id, r);
			roots.push(
				this.buildNodeFromWasm(id, rootNodeIdx, fns, rtPtr, readStr, allocStr),
			);
		}

		this.cache.set(id, roots);
	}

	/**
	 * Register a template from a decoded RegisterTemplate mutation.
	 *
	 * Builds the DOM tree purely from the serialized template data —
	 * no WASM calls needed.  This is the primary registration path
	 * when using automatic template serialization (Phase 11).
	 *
	 * @param m - The decoded MutationRegisterTemplate mutation.
	 */
	registerFromMutation(m: MutationRegisterTemplate): void {
		const roots: Node[] = [];
		for (const rootIdx of m.rootIndices) {
			roots.push(this.buildNodeFromMutation(rootIdx, m.nodes, m.attrs));
		}
		this.cache.set(m.tmplId, roots);
	}

	// ── Instantiation ─────────────────────────────────────────────────

	/**
	 * Clone and return the `index`-th root of template `id`.
	 *
	 * @throws if the template or index is not registered.
	 */
	instantiate(id: number, index: number): Node {
		const roots = this.cache.get(id);
		if (!roots) {
			throw new Error(`TemplateCache: template ${id} not registered`);
		}
		if (index < 0 || index >= roots.length) {
			throw new Error(
				`TemplateCache: template ${id} has ${roots.length} roots, ` +
					`requested index ${index}`,
			);
		}
		return roots[index].cloneNode(true);
	}

	// ── Queries ───────────────────────────────────────────────────────

	/** Check whether a template is registered. */
	has(id: number): boolean {
		return this.cache.has(id);
	}

	/** Return the number of root nodes for a registered template. */
	rootCount(id: number): number {
		return this.cache.get(id)?.length ?? 0;
	}

	/** Return the total number of registered templates. */
	get size(): number {
		return this.cache.size;
	}

	// ── Internal: build DOM from WASM queries ─────────────────────────

	private buildNodeFromWasm(
		tmplId: number,
		nodeIdx: number,
		fns: WasmTemplateExports,
		rtPtr: bigint,
		readStr: (ptr: bigint) => string,
		allocStr: () => bigint,
	): Node {
		const kind = fns.tmpl_node_kind(rtPtr, tmplId, nodeIdx);

		switch (kind) {
			case TNODE_ELEMENT: {
				const tagId = fns.tmpl_node_tag(rtPtr, tmplId, nodeIdx);
				const tag = tagName(tagId);
				const el = this.doc.createElement(tag);

				// Static attributes
				const firstAttr = fns.tmpl_node_first_attr(rtPtr, tmplId, nodeIdx);
				const attrCount = fns.tmpl_node_attr_count(rtPtr, tmplId, nodeIdx);
				for (let a = 0; a < attrCount; a++) {
					const attrIdx = firstAttr + a;
					const attrKind = fns.tmpl_attr_kind(rtPtr, tmplId, attrIdx);
					if (attrKind === TATTR_STATIC) {
						const outName = allocStr();
						const nameRaw = fns.tmpl_attr_name(rtPtr, tmplId, attrIdx);
						const name =
							typeof nameRaw === "string" ? nameRaw : readStr(outName);

						const outVal = allocStr();
						const valRaw = fns.tmpl_attr_value(rtPtr, tmplId, attrIdx);
						const value = typeof valRaw === "string" ? valRaw : readStr(outVal);

						el.setAttribute(name, value);
					}
					// Dynamic attrs become placeholders — nothing to set now
				}

				// Children
				const childCount = fns.tmpl_node_child_count(rtPtr, tmplId, nodeIdx);
				for (let c = 0; c < childCount; c++) {
					const childIdx = fns.tmpl_node_child_at(rtPtr, tmplId, nodeIdx, c);
					el.appendChild(
						this.buildNodeFromWasm(
							tmplId,
							childIdx,
							fns,
							rtPtr,
							readStr,
							allocStr,
						),
					);
				}

				return el;
			}

			case TNODE_TEXT: {
				// Static text node
				const outPtr = allocStr();
				const raw = fns.tmpl_node_text(rtPtr, tmplId, nodeIdx);
				const text = typeof raw === "string" ? raw : readStr(outPtr);
				return this.doc.createTextNode(text);
			}

			case TNODE_DYNAMIC: {
				// Dynamic node placeholder — use a comment so it can be
				// found and replaced by ReplacePlaceholder mutations.
				return this.doc.createComment("placeholder");
			}

			case TNODE_DYNAMIC_TEXT: {
				// Dynamic text placeholder — an empty text node whose
				// content will be set via SetText mutations.
				return this.doc.createTextNode("");
			}

			default:
				throw new Error(`TemplateCache: unknown template node kind ${kind}`);
		}
	}

	// ── Internal: build DOM from decoded mutation data ─────────────────

	private buildNodeFromMutation(
		nodeIdx: number,
		nodes: TemplateNode[],
		attrs: TemplateAttr[],
	): Node {
		const node = nodes[nodeIdx];

		switch (node.kind) {
			case 0x00: {
				// Element
				const tag = tagName(node.tag);
				const el = this.doc.createElement(tag);

				// Static attributes
				for (let a = 0; a < node.attrCount; a++) {
					const attr = attrs[node.attrFirst + a];
					if (attr.kind === 0x00) {
						// Static attribute
						el.setAttribute(attr.name, attr.value);
					}
					// Dynamic attrs are placeholders — nothing to set now
				}

				// Children
				for (const childIdx of node.children) {
					el.appendChild(this.buildNodeFromMutation(childIdx, nodes, attrs));
				}

				return el;
			}

			case 0x01: {
				// Static text node
				return this.doc.createTextNode(node.text);
			}

			case 0x02: {
				// Dynamic node placeholder — comment for ReplacePlaceholder
				return this.doc.createComment("placeholder");
			}

			case 0x03: {
				// Dynamic text placeholder — empty text node for SetText
				return this.doc.createTextNode("");
			}

			default:
				throw new Error(
					`TemplateCache: unknown template node kind ${(node as TemplateNode).kind}`,
				);
		}
	}
}

// ── Minimal WASM export interface needed for template queries ────────────────

/**
 * Subset of WasmExports used by TemplateCache.registerFromWasm().
 * Avoids a circular import with types.ts.
 */
export interface WasmTemplateExports {
	tmpl_root_count(rtPtr: bigint, tmplId: number): number;
	tmpl_get_root_index(rtPtr: bigint, tmplId: number, rootPos: number): number;
	tmpl_node_kind(rtPtr: bigint, tmplId: number, nodeIdx: number): number;
	tmpl_node_tag(rtPtr: bigint, tmplId: number, nodeIdx: number): number;
	tmpl_node_child_count(rtPtr: bigint, tmplId: number, nodeIdx: number): number;
	tmpl_node_child_at(
		rtPtr: bigint,
		tmplId: number,
		nodeIdx: number,
		childPos: number,
	): number;
	tmpl_node_attr_count(rtPtr: bigint, tmplId: number, nodeIdx: number): number;
	tmpl_node_first_attr(rtPtr: bigint, tmplId: number, nodeIdx: number): number;
	tmpl_attr_kind(rtPtr: bigint, tmplId: number, attrIdx: number): number;
	tmpl_node_text(
		rtPtr: bigint,
		tmplId: number,
		nodeIdx: number,
	): bigint | string;
	tmpl_attr_name(
		rtPtr: bigint,
		tmplId: number,
		attrIdx: number,
	): bigint | string;
	tmpl_attr_value(
		rtPtr: bigint,
		tmplId: number,
		attrIdx: number,
	): bigint | string;
}
