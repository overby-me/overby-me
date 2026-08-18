// DOM Interpreter — Stack-based mutation processor.
//
// The Interpreter reads decoded mutations (from MutationReader) and applies
// them to a real DOM tree.  It maintains:
//
//   - A node stack (for stack-based mutation operations)
//   - An ElementId → Node map (for targeting specific nodes)
//   - A TemplateCache reference (for LoadTemplate cloning)
//
// This is the JS-side renderer for the Mojo UI framework.  It processes
// the binary mutation buffers emitted by the Mojo CreateEngine and
// DiffEngine, translating them into DOM API calls.
//
// The interpreter is environment-agnostic: it works in browsers (with the
// native `document`) and in headless DOM implementations (linkedom,
// deno-dom, happy-dom) for testing.

import type { Mutation } from "./protocol.ts";
import { MutationReader, Op } from "./protocol.ts";
import type { TemplateCache } from "./templates.ts";

// ── Interpreter ─────────────────────────────────────────────────────────────

/**
 * Stack-based DOM mutation interpreter.
 *
 * Processes mutations emitted by the Mojo VDOM diff/create engines and
 * applies them to a real DOM tree rooted at a given element.
 *
 * Usage:
 *   const interp = new Interpreter(rootEl, templates);
 *   interp.applyMutations(wasmMemory.buffer, offset, length);
 *
 * The root element is pre-registered with ElementId 0.
 */
export class Interpreter {
	/** The virtual stack used by stack-based mutations. */
	private stack: Node[] = [];

	/** ElementId → DOM Node map. */
	private nodes: Map<number, Node> = new Map();

	/** Template cache for LoadTemplate cloning. */
	private templates: TemplateCache;

	/** The Document used for creating new nodes. */
	private doc: Document;

	/** The root mount element (registered as id 0). */
	private root: Element;

	/**
	 * Event listener tracking: `elementId → (eventName → listener)`.
	 * Needed so we can remove listeners on RemoveEventListener.
	 */
	private listeners: Map<number, Map<string, EventListener>> = new Map();

	/**
	 * Optional callback invoked when a NewEventListener mutation is
	 * processed.  The callback receives the element ID, event name,
	 * and handler ID and should return the actual EventListener to
	 * attach.
	 *
	 * If not set, a no-op listener is attached (useful for testing
	 * structure without event wiring).
	 */
	onNewListener:
		| ((
				elementId: number,
				eventName: string,
				handlerId: number,
		  ) => EventListener)
		| null = null;

	/**
	 * @param root      - The mount-point element.  Gets ElementId 0.
	 * @param templates - TemplateCache for LoadTemplate operations.
	 * @param doc       - The Document to use.  Defaults to the root's
	 *                    ownerDocument.
	 */
	constructor(root: Element, templates: TemplateCache, doc?: Document) {
		this.root = root;
		this.templates = templates;
		this.doc = doc ?? root.ownerDocument!;

		// Register the root element with id 0
		this.nodes.set(0, root);
	}

	// ── Public API ────────────────────────────────────────────────────

	/**
	 * Decode and apply all mutations from a binary buffer region.
	 *
	 * @param buffer     - The ArrayBuffer (typically WASM linear memory).
	 * @param byteOffset - Start of mutation data within `buffer`.
	 * @param byteLength - Number of bytes of mutation data.
	 */
	applyMutations(
		buffer: ArrayBuffer,
		byteOffset: number,
		byteLength: number,
	): void {
		const reader = new MutationReader(buffer, byteOffset, byteLength);
		for (let m = reader.next(); m !== null; m = reader.next()) {
			this.handleMutation(m);
		}
	}

	/**
	 * Apply a single decoded mutation to the DOM.
	 *
	 * Exposed publicly so callers can feed mutations one at a time
	 * (useful for debugging or testing).
	 */
	handleMutation(m: Mutation): void {
		switch (m.op) {
			case Op.PushRoot:
				this.opPushRoot(m.id);
				break;

			case Op.AppendChildren:
				this.opAppendChildren(m.id, m.m);
				break;

			case Op.CreateTextNode:
				this.opCreateTextNode(m.id, m.text);
				break;

			case Op.CreatePlaceholder:
				this.opCreatePlaceholder(m.id);
				break;

			case Op.LoadTemplate:
				this.opLoadTemplate(m.tmplId, m.index, m.id);
				break;

			case Op.AssignId:
				this.opAssignId(m.path, m.id);
				break;

			case Op.SetAttribute:
				this.opSetAttribute(m.id, m.ns, m.name, m.value);
				break;

			case Op.RemoveAttribute:
				this.opRemoveAttribute(m.id, m.ns, m.name);
				break;

			case Op.SetText:
				this.opSetText(m.id, m.text);
				break;

			case Op.NewEventListener:
				this.opNewEventListener(m.id, m.name, m.handlerId);
				break;

			case Op.RemoveEventListener:
				this.opRemoveEventListener(m.id, m.name);
				break;

			case Op.Remove:
				this.opRemove(m.id);
				break;

			case Op.ReplaceWith:
				this.opReplaceWith(m.id, m.m);
				break;

			case Op.ReplacePlaceholder:
				this.opReplacePlaceholder(m.path, m.m);
				break;

			case Op.InsertAfter:
				this.opInsertAfter(m.id, m.m);
				break;

			case Op.InsertBefore:
				this.opInsertBefore(m.id, m.m);
				break;

			case Op.RegisterTemplate:
				this.templates.registerFromMutation(m);
				break;

			default: {
				const _exhaustive: never = m;
				throw new Error(
					`Interpreter: unhandled mutation op ${(m as Mutation).op}`,
				);
			}
		}
	}

	// ── Introspection (for testing) ───────────────────────────────────

	/** Get the DOM node for an ElementId, or undefined if not tracked. */
	getNode(id: number): Node | undefined {
		return this.nodes.get(id);
	}

	/** Get the current stack depth. */
	getStackSize(): number {
		return this.stack.length;
	}

	/** Peek at the top of the stack without popping. */
	stackTop(): Node | undefined {
		return this.stack[this.stack.length - 1];
	}

	/** Get the root element. */
	getRoot(): Element {
		return this.root;
	}

	/** Get the number of tracked nodes (ElementId → Node mappings). */
	getNodeCount(): number {
		return this.nodes.size;
	}

	// ── Stack operations ──────────────────────────────────────────────

	private push(node: Node): void {
		this.stack.push(node);
	}

	/**
	 * Pop `count` nodes from the stack.
	 * Returns them in the order they were pushed (first pushed = index 0).
	 */
	private popMany(count: number): Node[] {
		if (count > this.stack.length) {
			throw new Error(
				`Interpreter: stack underflow — need ${count}, have ${this.stack.length}`,
			);
		}
		// splice removes from the end
		return this.stack.splice(this.stack.length - count, count);
	}

	// ── Mutation implementations ──────────────────────────────────────

	/**
	 * PushRoot: push nodes[id] onto the stack.
	 */
	private opPushRoot(id: number): void {
		const node = this.nodes.get(id);
		if (!node) {
			throw new Error(`Interpreter: PushRoot — unknown id ${id}`);
		}
		this.push(node);
	}

	/**
	 * AppendChildren: pop `m` nodes from the stack and append them as
	 * children of nodes[id].
	 */
	private opAppendChildren(id: number, m: number): void {
		const parent = this.nodes.get(id);
		if (!parent) {
			throw new Error(`Interpreter: AppendChildren — unknown id ${id}`);
		}
		const children = this.popMany(m);
		for (const child of children) {
			parent.appendChild(child);
		}
	}

	/**
	 * CreateTextNode: create a text node, store it in nodes[id], push
	 * it onto the stack.
	 */
	private opCreateTextNode(id: number, text: string): void {
		const node = this.doc.createTextNode(text);
		this.nodes.set(id, node);
		this.push(node);
	}

	/**
	 * CreatePlaceholder: create a placeholder node (empty comment),
	 * store in nodes[id], push onto stack.
	 *
	 * Placeholders are implemented as comment nodes so they're invisible
	 * in the rendered output but can be targeted for replacement.
	 */
	private opCreatePlaceholder(id: number): void {
		const node = this.doc.createComment("placeholder");
		this.nodes.set(id, node);
		this.push(node);
	}

	/**
	 * LoadTemplate: clone the `index`-th root of template `tmplId`,
	 * store in nodes[id], push onto stack.
	 */
	private opLoadTemplate(tmplId: number, index: number, id: number): void {
		const node = this.templates.instantiate(tmplId, index);
		this.nodes.set(id, node);
		this.push(node);
	}

	/**
	 * AssignId: walk the `path` from the top of the stack (without
	 * popping) and assign the reached node to nodes[id].
	 *
	 * The path is a sequence of child indices.  For example, path [1, 0]
	 * means: from the stack top, take child 1, then child 0 of that.
	 *
	 * For templates with multiple roots loaded onto the stack, the path
	 * navigates from the top-most root.  Multi-root path selection (where
	 * the first byte is a stack offset) may be added in a future phase.
	 */
	private opAssignId(path: Uint8Array, id: number): void {
		let node = this.stack[this.stack.length - 1];
		if (!node) {
			throw new Error("Interpreter: AssignId — empty stack");
		}

		for (let i = 0; i < path.length; i++) {
			const childIndex = path[i];
			const children = node.childNodes;
			if (childIndex >= children.length) {
				throw new Error(
					`Interpreter: AssignId — path step ${i} index ${childIndex} ` +
						`out of bounds (${children.length} children)`,
				);
			}
			node = children[childIndex];
		}

		this.nodes.set(id, node);
	}

	/**
	 * SetAttribute: set an attribute on the element at nodes[id].
	 *
	 * `ns` is a namespace tag (0 = no namespace).
	 * Non-zero namespace values are reserved for xlink, xml, xmlns.
	 */
	private opSetAttribute(
		id: number,
		ns: number,
		name: string,
		value: string,
	): void {
		const node = this.nodes.get(id);
		if (!node) {
			throw new Error(`Interpreter: SetAttribute — unknown id ${id}`);
		}

		// Must be an Element to have attributes
		if (node.nodeType !== 1 /* ELEMENT_NODE */) {
			// Silently skip for non-elements (defensive)
			return;
		}

		const el = node as Element;

		if (ns === 0) {
			el.setAttribute(name, value);
		} else {
			// Namespace mapping (simplified)
			const nsUri = this.resolveNs(ns);
			if (nsUri) {
				el.setAttributeNS(nsUri, name, value);
			} else {
				el.setAttribute(name, value);
			}
		}
	}

	/**
	 * SetText: update the text content of nodes[id].
	 */
	private opRemoveAttribute(id: number, ns: number, name: string): void {
		const node = this.nodes.get(id);
		if (!node) {
			throw new Error(`Interpreter: RemoveAttribute — unknown id ${id}`);
		}

		// Must be an Element to have attributes
		if (node.nodeType !== 1 /* ELEMENT_NODE */) {
			// Silently skip for non-elements (defensive)
			return;
		}

		const el = node as Element;

		if (ns === 0) {
			el.removeAttribute(name);
		} else {
			const nsUri = this.resolveNs(ns);
			if (nsUri) {
				el.removeAttributeNS(nsUri, name);
			} else {
				el.removeAttribute(name);
			}
		}
	}

	private opSetText(id: number, text: string): void {
		const node = this.nodes.get(id);
		if (!node) {
			throw new Error(`Interpreter: SetText — unknown id ${id}`);
		}
		node.textContent = text;
	}

	/**
	 * NewEventListener: attach an event listener to the element at
	 * nodes[id] for the given event name.
	 *
	 * The actual handler function comes from `onNewListener` if set;
	 * otherwise a no-op handler is used.
	 */
	private opNewEventListener(
		id: number,
		name: string,
		handlerId: number,
	): void {
		const node = this.nodes.get(id);
		if (!node || node.nodeType !== 1) return;

		const el = node as Element;
		const listener: EventListener = this.onNewListener
			? this.onNewListener(id, name, handlerId)
			: () => {};

		// Track the listener so we can remove it later
		let elListeners = this.listeners.get(id);
		if (!elListeners) {
			elListeners = new Map();
			this.listeners.set(id, elListeners);
		}

		// Remove previous listener for this event if any
		const prev = elListeners.get(name);
		if (prev) {
			el.removeEventListener(name, prev);
		}

		elListeners.set(name, listener);
		el.addEventListener(name, listener);
	}

	/**
	 * RemoveEventListener: remove the event listener for `name` from
	 * the element at nodes[id].
	 */
	private opRemoveEventListener(id: number, name: string): void {
		const node = this.nodes.get(id);
		if (!node || node.nodeType !== 1) return;

		const el = node as Element;
		const elListeners = this.listeners.get(id);
		if (elListeners) {
			const listener = elListeners.get(name);
			if (listener) {
				el.removeEventListener(name, listener);
				elListeners.delete(name);
			}
		}
	}

	/**
	 * Remove: remove nodes[id] from its parent.
	 *
	 * Also cleans up the id→node mapping and any tracked listeners.
	 */
	private opRemove(id: number): void {
		const node = this.nodes.get(id);
		if (!node) return;

		if (node.parentNode) {
			node.parentNode.removeChild(node);
		}

		this.nodes.delete(id);
		this.listeners.delete(id);
	}

	/**
	 * ReplaceWith: pop `m` nodes from the stack and use them to replace
	 * nodes[id] in the DOM.
	 *
	 * The old node is removed from the id→node map.  If `m === 1`, the
	 * replacement inherits the position directly.  If `m > 1`, all
	 * replacement nodes are inserted at the old node's position.
	 */
	private opReplaceWith(id: number, m: number): void {
		const oldNode = this.nodes.get(id);
		if (!oldNode) {
			throw new Error(`Interpreter: ReplaceWith — unknown id ${id}`);
		}

		const replacements = this.popMany(m);
		const parent = oldNode.parentNode;

		if (parent) {
			if (replacements.length === 1) {
				parent.replaceChild(replacements[0], oldNode);
			} else if (replacements.length > 0) {
				// Insert all replacements before the old node, then remove it
				for (const r of replacements) {
					parent.insertBefore(r, oldNode);
				}
				parent.removeChild(oldNode);
			} else {
				// m === 0: just remove
				parent.removeChild(oldNode);
			}
		}

		this.nodes.delete(id);
		this.listeners.delete(id);
	}

	/**
	 * ReplacePlaceholder: pop `m` replacement nodes from the stack,
	 * then navigate `path` from the new stack top (the template root)
	 * and replace the node at that path with the popped nodes.
	 *
	 * This is used during initial template creation to replace
	 * comment-node placeholders with dynamically created content.
	 *
	 * Stack state before: [..., template_root, replacement_nodes (m)]
	 * Stack state after:  [..., template_root]   (template root stays)
	 */
	private opReplacePlaceholder(path: Uint8Array, m: number): void {
		// The m replacement nodes are at the TOP of the stack.
		// The template root is BELOW them.
		if (this.stack.length < m + 1) {
			throw new Error(
				`Interpreter: ReplacePlaceholder — stack underflow ` +
					`(need ${m + 1}, have ${this.stack.length})`,
			);
		}

		// Pop m replacement nodes from the top
		const replacements = this.popMany(m);

		// The template root is now at the top of the stack (peek, don't pop)
		const templateRoot = this.stack[this.stack.length - 1];

		// Navigate the path to find the placeholder
		let target: Node = templateRoot;
		for (let i = 0; i < path.length; i++) {
			const childIndex = path[i];
			const children = target.childNodes;
			if (childIndex >= children.length) {
				throw new Error(
					`Interpreter: ReplacePlaceholder — path step ${i} ` +
						`index ${childIndex} out of bounds ` +
						`(${children.length} children)`,
				);
			}
			target = children[childIndex];
		}

		// Replace the placeholder node with the replacement nodes
		const parent = target.parentNode;
		if (parent) {
			if (replacements.length === 1) {
				parent.replaceChild(replacements[0], target);
			} else if (replacements.length > 0) {
				for (const r of replacements) {
					parent.insertBefore(r, target);
				}
				parent.removeChild(target);
			} else {
				parent.removeChild(target);
			}
		}
	}

	/**
	 * InsertAfter: pop `m` nodes from the stack and insert them after
	 * nodes[id] in the DOM.
	 */
	private opInsertAfter(id: number, m: number): void {
		const ref = this.nodes.get(id);
		if (!ref) {
			throw new Error(`Interpreter: InsertAfter — unknown id ${id}`);
		}

		const newNodes = this.popMany(m);
		const parent = ref.parentNode;
		if (!parent) return;

		// Insert after ref: insertBefore(node, ref.nextSibling)
		let insertPoint = ref.nextSibling;
		for (const node of newNodes) {
			parent.insertBefore(node, insertPoint);
			// Each subsequent node goes after the one we just inserted
			insertPoint = node.nextSibling;
		}
	}

	/**
	 * InsertBefore: pop `m` nodes from the stack and insert them before
	 * nodes[id] in the DOM.
	 */
	private opInsertBefore(id: number, m: number): void {
		const ref = this.nodes.get(id);
		if (!ref) {
			throw new Error(`Interpreter: InsertBefore — unknown id ${id}`);
		}

		const newNodes = this.popMany(m);
		const parent = ref.parentNode;
		if (!parent) return;

		for (const node of newNodes) {
			parent.insertBefore(node, ref);
		}
	}

	// ── Helpers ───────────────────────────────────────────────────────

	/**
	 * Resolve a namespace tag to a URI string.
	 * Returns null if the namespace is not recognized.
	 */
	private resolveNs(ns: number): string | null {
		switch (ns) {
			case 1:
				return "http://www.w3.org/1999/xlink";
			case 2:
				return "http://www.w3.org/XML/1998/namespace";
			case 3:
				return "http://www.w3.org/2000/xmlns/";
			default:
				return null;
		}
	}
}

// ── MutationBuilder ─────────────────────────────────────────────────────────
//
// A JS-side mutation buffer writer for tests.  Mirrors the binary format
// produced by Mojo's MutationWriter so that tests can construct mutation
// buffers without going through WASM.

const encoder = new TextEncoder();

/**
 * Builds binary mutation buffers in JS for testing the Interpreter
 * without requiring WASM compilation.
 *
 * Usage:
 *   const { buffer, length } = new MutationBuilder(1024)
 *     .loadTemplate(0, 0, 1)
 *     .assignId([0], 2)
 *     .setText(2, "hello")
 *     .appendChildren(0, 1)
 *     .end()
 *     .build();
 *
 *   interpreter.applyMutations(buffer, 0, length);
 */
export class MutationBuilder {
	private buffer: ArrayBuffer;
	private view: DataView;
	private bytes: Uint8Array;
	private offset: number;

	constructor(capacity: number = 4096) {
		this.buffer = new ArrayBuffer(capacity);
		this.view = new DataView(this.buffer);
		this.bytes = new Uint8Array(this.buffer);
		this.offset = 0;
	}

	// ── Primitive writers ─────────────────────────────────────────────

	private writeU8(v: number): void {
		this.view.setUint8(this.offset, v);
		this.offset += 1;
	}

	private writeU16(v: number): void {
		this.view.setUint16(this.offset, v, true); // little-endian
		this.offset += 2;
	}

	private writeU32(v: number): void {
		this.view.setUint32(this.offset, v, true); // little-endian
		this.offset += 4;
	}

	/** Write a u32-length-prefixed UTF-8 string. */
	private writeStr(s: string): void {
		const encoded = encoder.encode(s);
		this.writeU32(encoded.length);
		this.bytes.set(encoded, this.offset);
		this.offset += encoded.length;
	}

	/** Write a u16-length-prefixed UTF-8 string (for short names). */
	private writeShortStr(s: string): void {
		const encoded = encoder.encode(s);
		this.writeU16(encoded.length);
		this.bytes.set(encoded, this.offset);
		this.offset += encoded.length;
	}

	/** Write a u8-length-prefixed path. */
	private writePath(path: number[]): void {
		this.writeU8(path.length);
		for (const p of path) {
			this.writeU8(p);
		}
	}

	// ── Mutation builders ─────────────────────────────────────────────

	end(): this {
		this.writeU8(Op.End);
		return this;
	}

	pushRoot(id: number): this {
		this.writeU8(Op.PushRoot);
		this.writeU32(id);
		return this;
	}

	appendChildren(id: number, m: number): this {
		this.writeU8(Op.AppendChildren);
		this.writeU32(id);
		this.writeU32(m);
		return this;
	}

	createTextNode(id: number, text: string): this {
		this.writeU8(Op.CreateTextNode);
		this.writeU32(id);
		this.writeStr(text);
		return this;
	}

	createPlaceholder(id: number): this {
		this.writeU8(Op.CreatePlaceholder);
		this.writeU32(id);
		return this;
	}

	loadTemplate(tmplId: number, index: number, id: number): this {
		this.writeU8(Op.LoadTemplate);
		this.writeU32(tmplId);
		this.writeU32(index);
		this.writeU32(id);
		return this;
	}

	assignId(path: number[], id: number): this {
		this.writeU8(Op.AssignId);
		this.writePath(path);
		this.writeU32(id);
		return this;
	}

	setAttribute(id: number, ns: number, name: string, value: string): this {
		this.writeU8(Op.SetAttribute);
		this.writeU32(id);
		this.writeU8(ns);
		this.writeShortStr(name);
		this.writeStr(value);
		return this;
	}

	removeAttribute(id: number, ns: number, name: string): this {
		this.writeU8(Op.RemoveAttribute);
		this.writeU32(id);
		this.writeU8(ns);
		this.writeShortStr(name);
		return this;
	}

	setText(id: number, text: string): this {
		this.writeU8(Op.SetText);
		this.writeU32(id);
		this.writeStr(text);
		return this;
	}

	newEventListener(id: number, name: string, handlerId = 0): this {
		this.writeU8(Op.NewEventListener);
		this.writeU32(id);
		this.writeU32(handlerId);
		this.writeShortStr(name);
		return this;
	}

	removeEventListener(id: number, name: string): this {
		this.writeU8(Op.RemoveEventListener);
		this.writeU32(id);
		this.writeShortStr(name);
		return this;
	}

	remove(id: number): this {
		this.writeU8(Op.Remove);
		this.writeU32(id);
		return this;
	}

	replaceWith(id: number, m: number): this {
		this.writeU8(Op.ReplaceWith);
		this.writeU32(id);
		this.writeU32(m);
		return this;
	}

	replacePlaceholder(path: number[], m: number): this {
		this.writeU8(Op.ReplacePlaceholder);
		this.writePath(path);
		this.writeU32(m);
		return this;
	}

	insertAfter(id: number, m: number): this {
		this.writeU8(Op.InsertAfter);
		this.writeU32(id);
		this.writeU32(m);
		return this;
	}

	insertBefore(id: number, m: number): this {
		this.writeU8(Op.InsertBefore);
		this.writeU32(id);
		this.writeU32(m);
		return this;
	}

	// ── Build ─────────────────────────────────────────────────────────

	/**
	 * Finalize and return the mutation buffer.
	 *
	 * Automatically appends an End sentinel if the last written op
	 * wasn't End.
	 */
	build(): { buffer: ArrayBuffer; length: number } {
		return { buffer: this.buffer, length: this.offset };
	}
}
