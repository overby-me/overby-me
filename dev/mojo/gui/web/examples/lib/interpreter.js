// Shared DOM Interpreter — applies binary mutation buffers to real DOM.
//
// Used by all browser examples to avoid duplicating ~135 lines each.

import { MutationReader, Op } from "./protocol.js";

// Tag ID → tag name lookup (must match src/vdom/tags.mojo)
const TAG_NAMES = [
	"div",
	"span",
	"p",
	"section",
	"header",
	"footer",
	"nav",
	"main",
	"article",
	"aside",
	"h1",
	"h2",
	"h3",
	"h4",
	"h5",
	"h6",
	"ul",
	"ol",
	"li",
	"button",
	"input",
	"form",
	"textarea",
	"select",
	"option",
	"label",
	"a",
	"img",
	"table",
	"thead",
	"tbody",
	"tr",
	"td",
	"th",
	"strong",
	"em",
	"br",
	"hr",
	"pre",
	"code",
];

/**
 * Minimal DOM interpreter that applies binary-encoded mutation buffers
 * (produced by Mojo's MutationWriter) to a live DOM tree.
 *
 * Template roots are provided as a `Map<number, Node[]>` mapping template
 * IDs to arrays of pre-built DOM nodes that are cloned on LoadTemplate.
 *
 * Set `onNewListener` to a callback `(elementId, eventName, handlerId) => listener`
 * to be notified when a NewEventListener mutation is processed. The
 * returned function is attached as the actual DOM event listener.
 */
export class Interpreter {
	constructor(root, templateRoots) {
		this.stack = [];
		this.nodes = new Map();
		this.templateRoots = templateRoots;
		this.doc = root.ownerDocument;
		this.root = root;
		this.listeners = new Map();
		this.onNewListener = null;
		this.nodes.set(0, root);
	}

	/**
	 * Decode and apply all mutations from a binary buffer region.
	 *
	 * @param {ArrayBuffer} buffer   - The underlying buffer (typically WASM memory).
	 * @param {number}      byteOffset - Start of mutation data within `buffer`.
	 * @param {number}      byteLength - Number of bytes of mutation data.
	 */
	applyMutations(buffer, byteOffset, byteLength) {
		const reader = new MutationReader(buffer, byteOffset, byteLength);
		for (let m = reader.next(); m !== null; m = reader.next()) {
			this.handle(m);
		}
	}

	handle(m) {
		switch (m.op) {
			case Op.PushRoot:
				this.stack.push(this.nodes.get(m.id));
				break;

			case Op.AppendChildren: {
				const parent = this.nodes.get(m.id);
				const children = this.stack.splice(-m.m, m.m);
				for (const c of children) parent.appendChild(c);
				break;
			}

			case Op.CreateTextNode: {
				const n = this.doc.createTextNode(m.text);
				this.nodes.set(m.id, n);
				this.stack.push(n);
				break;
			}

			case Op.CreatePlaceholder: {
				const n = this.doc.createComment("placeholder");
				this.nodes.set(m.id, n);
				this.stack.push(n);
				break;
			}

			case Op.LoadTemplate: {
				const roots = this.templateRoots.get(m.tmplId);
				if (!roots) throw new Error(`Template ${m.tmplId} not registered`);
				const n = roots[m.index].cloneNode(true);
				this.nodes.set(m.id, n);
				this.stack.push(n);
				break;
			}

			case Op.AssignId: {
				let n = this.stack[this.stack.length - 1];
				for (const idx of m.path) n = n.childNodes[idx];
				this.nodes.set(m.id, n);
				break;
			}

			case Op.SetAttribute: {
				const n = this.nodes.get(m.id);
				if (n?.setAttribute) n.setAttribute(m.name, m.value);
				break;
			}

			case Op.RemoveAttribute: {
				const n = this.nodes.get(m.id);
				if (n?.removeAttribute) n.removeAttribute(m.name);
				break;
			}

			case Op.SetText: {
				const n = this.nodes.get(m.id);
				if (n) n.textContent = m.text;
				break;
			}

			case Op.NewEventListener: {
				const el = this.nodes.get(m.id);
				if (!el) break;
				const listener = this.onNewListener
					? this.onNewListener(m.id, m.name, m.handlerId)
					: () => {};
				let elMap = this.listeners.get(m.id);
				if (!elMap) {
					elMap = new Map();
					this.listeners.set(m.id, elMap);
				}
				const prev = elMap.get(m.name);
				if (prev) el.removeEventListener(m.name, prev);
				el.addEventListener(m.name, listener);
				elMap.set(m.name, listener);
				break;
			}

			case Op.RemoveEventListener: {
				const el = this.nodes.get(m.id);
				if (!el) break;
				const elMap = this.listeners.get(m.id);
				if (!elMap) break;
				const fn = elMap.get(m.name);
				if (fn) {
					el.removeEventListener(m.name, fn);
					elMap.delete(m.name);
				}
				break;
			}

			case Op.Remove: {
				const n = this.nodes.get(m.id);
				if (n?.parentNode) n.parentNode.removeChild(n);
				break;
			}

			case Op.ReplaceWith: {
				const old = this.nodes.get(m.id);
				const reps = this.stack.splice(-m.m, m.m);
				const parent = old.parentNode;
				if (parent) {
					parent.replaceChild(reps[0], old);
					for (let i = 1; i < reps.length; i++) {
						parent.insertBefore(reps[i], reps[i - 1].nextSibling);
					}
				}
				break;
			}

			case Op.ReplacePlaceholder: {
				const reps = this.stack.splice(-m.m, m.m);
				let target = this.stack[this.stack.length - 1];
				for (const idx of m.path) target = target.childNodes[idx];
				const parent = target.parentNode;
				if (parent) {
					for (const r of reps) parent.insertBefore(r, target);
					parent.removeChild(target);
				}
				break;
			}

			case Op.InsertAfter: {
				const ref = this.nodes.get(m.id);
				const news = this.stack.splice(-m.m, m.m);
				const parent = ref.parentNode;
				let point = ref.nextSibling;
				for (const n of news) {
					parent.insertBefore(n, point);
					point = n.nextSibling;
				}
				break;
			}

			case Op.InsertBefore: {
				const ref = this.nodes.get(m.id);
				const news = this.stack.splice(-m.m, m.m);
				const parent = ref.parentNode;
				for (const n of news) parent.insertBefore(n, ref);
				break;
			}

			case Op.RegisterTemplate: {
				const roots = [];
				for (const rootIdx of m.rootIndices) {
					roots.push(this.buildTemplateNode(rootIdx, m.nodes, m.attrs));
				}
				this.templateRoots.set(m.tmplId, roots);
				break;
			}
		}
	}

	/**
	 * Recursively build a DOM node from a decoded RegisterTemplate mutation's
	 * serialized node/attr arrays.
	 *
	 * @param {number} nodeIdx - Index into the nodes array.
	 * @param {Array}  nodes   - Flat array of serialized template nodes.
	 * @param {Array}  attrs   - Flat array of serialized template attributes.
	 * @returns {Node}
	 */
	buildTemplateNode(nodeIdx, nodes, attrs) {
		const node = nodes[nodeIdx];
		switch (node.kind) {
			case 0x00: {
				// Element
				const tag = TAG_NAMES[node.tag] || "unknown";
				const el = this.doc.createElement(tag);
				// Static attributes
				for (let a = 0; a < node.attrCount; a++) {
					const attr = attrs[node.attrFirst + a];
					if (attr.kind === 0x00) {
						el.setAttribute(attr.name, attr.value);
					}
					// Dynamic attrs are filled at render time — skip
				}
				// Children
				for (const childIdx of node.children) {
					el.appendChild(this.buildTemplateNode(childIdx, nodes, attrs));
				}
				return el;
			}
			case 0x01:
				// Static text
				return this.doc.createTextNode(node.text);
			case 0x02:
				// Dynamic node placeholder
				return this.doc.createComment("placeholder");
			case 0x03:
				// Dynamic text placeholder
				return this.doc.createTextNode("");
			default:
				throw new Error(`Unknown template node kind ${node.kind}`);
		}
	}
}
