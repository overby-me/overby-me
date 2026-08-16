import { alignedAlloc, getMemory, getView, scratchAlloc } from "./memory.ts";

const encoder = new TextEncoder();
const decoder = new TextDecoder();

// Mojo String ABI: 24-byte struct { data_ptr: i64, len: i64, capacity: i64 }
const STRING_STRUCT_SIZE = 24n;
const STRING_STRUCT_ALIGN = 8n;

/**
 * Allocate a Mojo String struct in WASM memory and populate it with the given
 * JS string. The string data is written as UTF-8 with a null terminator.
 */
export const writeStringStruct = (str: string): bigint => {
	const bytes = encoder.encode(str);
	const dataLen = BigInt(bytes.length);

	// Allocate buffer for string data (with null terminator)
	const dataPtr = scratchAlloc(1n, dataLen + 1n);
	const mem = getMemory();
	new Uint8Array(mem.buffer).set(bytes, Number(dataPtr));
	new Uint8Array(mem.buffer)[Number(dataPtr + dataLen)] = 0;

	// Allocate 24-byte String struct
	const structPtr = scratchAlloc(STRING_STRUCT_ALIGN, STRING_STRUCT_SIZE);
	const view = getView();
	// data_ptr at offset 0
	view.setBigInt64(Number(structPtr), dataPtr, true);
	// len at offset 8 (string byte count, excludes null terminator)
	view.setBigInt64(Number(structPtr) + 8, dataLen, true);
	// capacity at offset 16
	view.setBigInt64(Number(structPtr) + 16, dataLen + 1n, true);

	return structPtr;
};

/**
 * Allocate an empty (zero-initialized) Mojo String struct to be used as an
 * output parameter for WASM functions that return strings.
 */
export const allocStringStruct = (): bigint => {
	const structPtr = alignedAlloc(STRING_STRUCT_ALIGN, STRING_STRUCT_SIZE);
	// Zero-initialize
	const view = getView();
	view.setBigInt64(Number(structPtr), 0n, true);
	view.setBigInt64(Number(structPtr) + 8, 0n, true);
	view.setBigInt64(Number(structPtr) + 16, 0n, true);

	return structPtr;
};

/**
 * Read a JS string from a Mojo String struct in WASM memory.
 *
 * Handles Mojo's Small String Optimization (SSO):
 * - Bit 63 of the capacity field is the SSO flag.
 * - When set, the string data is stored inline at the struct address
 *   and bits 56-60 of the capacity encode the byte length.
 * - When clear, data_ptr (offset 0) and len (offset 8) are used directly.
 */
export const readStringStruct = (structPtr: bigint): string => {
	const view = getView();
	const capacity = view.getBigUint64(Number(structPtr) + 16, true);

	const SSO_FLAG = 0x8000000000000000n;
	const SSO_LEN_MASK = 0x1f00000000000000n;

	let dataPtr: bigint;
	let len: bigint;

	if ((capacity & SSO_FLAG) !== 0n) {
		// SSO: data is stored inline starting at the struct itself
		dataPtr = structPtr;
		len = (capacity & SSO_LEN_MASK) >> 56n;
	} else {
		dataPtr = view.getBigInt64(Number(structPtr), true);
		len = view.getBigInt64(Number(structPtr) + 8, true);
	}

	if (len <= 0n) return "";

	const mem = getMemory();
	const bytes = new Uint8Array(mem.buffer, Number(dataPtr), Number(len));
	return decoder.decode(bytes);
};
