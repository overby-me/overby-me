// Shared boot helpers — low-level API for browser examples.
//
// Provides convenience functions that tie together env, interpreter,
// and WASM loading.  Most examples should use `launch()` from `app.js`
// instead — this module is for advanced use cases (e.g. bench) that
// need direct control over the boot sequence.

export { launch } from "./app.js";
export { alignedAlloc, getMemory, loadWasm } from "./env.js";
export { EventBridge } from "./events.js";
export { Interpreter } from "./interpreter.js";
export { Op } from "./protocol.js";
export { writeStringStruct } from "./strings.js";

import { alignedAlloc, getMemory } from "./env.js";
import { Interpreter } from "./interpreter.js";

/**
 * Create an Interpreter wired to a DOM root element with the given
 * template roots.
 *
 * @param {Element}              root          - The mount-point DOM element.
 * @param {Map<number, Node[]>}  templateRoots - Template ID → cloneable root nodes.
 * @returns {Interpreter}
 */
export function createInterpreter(root, templateRoots) {
	return new Interpreter(root, templateRoots);
}

/**
 * Allocate a mutation buffer in WASM linear memory.
 *
 * @param {number} capacity - Buffer size in bytes.
 * @returns {bigint} Pointer to the buffer in WASM memory.
 */
export function allocBuffer(capacity) {
	return alignedAlloc(8n, BigInt(capacity));
}

/**
 * Apply a mutation buffer to an interpreter.
 *
 * @param {Interpreter} interpreter - The DOM interpreter.
 * @param {bigint}      bufPtr      - Pointer to the mutation buffer.
 * @param {number}      byteLen     - Number of bytes of mutation data.
 */
export function applyMutations(interpreter, bufPtr, byteLen) {
	const mem = getMemory();
	interpreter.applyMutations(mem.buffer, Number(bufPtr), byteLen);
}
