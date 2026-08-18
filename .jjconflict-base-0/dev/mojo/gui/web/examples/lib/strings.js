// Shared Mojo String ABI helpers — write JS strings into WASM linear memory.
//
// Mojo String is a 24-byte struct: { data_ptr: i64, len: i64, capacity: i64 }
// We allocate this struct in WASM linear memory and return a pointer to it.
//
// Used by examples that need to pass strings to WASM (e.g. todo app).

import { getMemory, scratchAlloc } from "./env.js";

const encoder = new TextEncoder();

/**
 * Allocate a Mojo String struct in WASM memory populated with the given
 * JS string. The string data is written as UTF-8 with a null terminator.
 *
 * @param {string} str - The JS string to write.
 * @returns {bigint} Pointer to the 24-byte String struct in WASM memory.
 */
export function writeStringStruct(str) {
	const bytes = encoder.encode(str);
	const dataLen = BigInt(bytes.length);
	const memory = getMemory();

	// Allocate buffer for string data (with null terminator)
	const dataPtr = scratchAlloc(1n, dataLen + 1n);
	new Uint8Array(memory.buffer).set(bytes, Number(dataPtr));
	new Uint8Array(memory.buffer)[Number(dataPtr + dataLen)] = 0;

	// Allocate 24-byte String struct
	const structPtr = scratchAlloc(8n, 24n);
	const view = new DataView(memory.buffer);
	view.setBigInt64(Number(structPtr), dataPtr, true); // data_ptr
	view.setBigInt64(Number(structPtr) + 8, dataLen, true); // len
	view.setBigInt64(Number(structPtr) + 16, dataLen + 1n, true); // capacity

	return structPtr;
}
