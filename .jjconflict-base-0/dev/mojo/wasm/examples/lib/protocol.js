// Shared mutation protocol — Op constants + MutationReader.
//
// Decodes binary-encoded DOM mutations written by the Mojo MutationWriter.
// All multi-byte integers are little-endian. Strings are length-prefixed UTF-8.

const decoder = new TextDecoder();

// ── Opcodes (must match src/bridge/protocol.mojo) ───────────────────────────

export const Op = {
	End: 0x00,
	AppendChildren: 0x01,
	AssignId: 0x02,
	CreatePlaceholder: 0x03,
	CreateTextNode: 0x04,
	LoadTemplate: 0x05,
	ReplaceWith: 0x06,
	ReplacePlaceholder: 0x07,
	InsertAfter: 0x08,
	InsertBefore: 0x09,
	SetAttribute: 0x0a,
	SetText: 0x0b,
	NewEventListener: 0x0c,
	RemoveEventListener: 0x0d,
	Remove: 0x0e,
	PushRoot: 0x0f,
	RegisterTemplate: 0x10,
	RemoveAttribute: 0x11,
};

// ── MutationReader ──────────────────────────────────────────────────────────

/**
 * Reads binary-encoded mutations from an ArrayBuffer region.
 * Call `next()` repeatedly to decode one mutation at a time;
 * returns `null` when the End sentinel is reached or the buffer is exhausted.
 */
export class MutationReader {
	constructor(buffer, byteOffset, byteLength) {
		this.view = new DataView(buffer, byteOffset, byteLength);
		this.bytes = new Uint8Array(buffer, byteOffset, byteLength);
		this.offset = 0;
		this.end = byteLength;
	}

	readU8() {
		const v = this.view.getUint8(this.offset);
		this.offset += 1;
		return v;
	}

	readU16() {
		const v = this.view.getUint16(this.offset, true);
		this.offset += 2;
		return v;
	}

	readU32() {
		const v = this.view.getUint32(this.offset, true);
		this.offset += 4;
		return v;
	}

	readStr() {
		const len = this.readU32();
		if (len === 0) return "";
		const s = decoder.decode(
			this.bytes.subarray(this.offset, this.offset + len),
		);
		this.offset += len;
		return s;
	}

	readShortStr() {
		const len = this.readU16();
		if (len === 0) return "";
		const s = decoder.decode(
			this.bytes.subarray(this.offset, this.offset + len),
		);
		this.offset += len;
		return s;
	}

	readPath() {
		const len = this.readU8();
		const p = this.bytes.slice(this.offset, this.offset + len);
		this.offset += len;
		return p;
	}

	next() {
		if (this.offset >= this.end) return null;
		const op = this.readU8();
		switch (op) {
			case Op.End:
				return null;
			case Op.AppendChildren:
				return { op, id: this.readU32(), m: this.readU32() };
			case Op.AssignId:
				return { op, path: this.readPath(), id: this.readU32() };
			case Op.CreatePlaceholder:
				return { op, id: this.readU32() };
			case Op.CreateTextNode:
				return { op, id: this.readU32(), text: this.readStr() };
			case Op.LoadTemplate:
				return {
					op,
					tmplId: this.readU32(),
					index: this.readU32(),
					id: this.readU32(),
				};
			case Op.ReplaceWith:
				return { op, id: this.readU32(), m: this.readU32() };
			case Op.ReplacePlaceholder:
				return { op, path: this.readPath(), m: this.readU32() };
			case Op.InsertAfter:
				return { op, id: this.readU32(), m: this.readU32() };
			case Op.InsertBefore:
				return { op, id: this.readU32(), m: this.readU32() };
			case Op.SetAttribute: {
				const id = this.readU32();
				const ns = this.readU8();
				const name = this.readShortStr();
				const value = this.readStr();
				return { op, id, ns, name, value };
			}
			case Op.RemoveAttribute: {
				const id = this.readU32();
				const ns = this.readU8();
				const name = this.readShortStr();
				return { op, id, ns, name };
			}
			case Op.SetText:
				return { op, id: this.readU32(), text: this.readStr() };
			case Op.NewEventListener: {
				const id = this.readU32();
				const handlerId = this.readU32();
				const name = this.readShortStr();
				return { op, id, handlerId, name };
			}
			case Op.RemoveEventListener:
				return { op, id: this.readU32(), name: this.readShortStr() };
			case Op.Remove:
				return { op, id: this.readU32() };
			case Op.PushRoot:
				return { op, id: this.readU32() };
			case Op.RegisterTemplate: {
				const tmplId = this.readU32();
				const name = this.readShortStr();
				const rootCount = this.readU16();
				const nodeCount = this.readU16();
				const attrCount = this.readU16();
				const nodes = [];
				for (let i = 0; i < nodeCount; i++) {
					const kind = this.readU8();
					if (kind === 0x00) {
						// Element
						const tag = this.readU8();
						const childCount = this.readU16();
						const children = [];
						for (let c = 0; c < childCount; c++) children.push(this.readU16());
						const attrFirst = this.readU16();
						const attrNum = this.readU16();
						nodes.push({ kind, tag, children, attrFirst, attrCount: attrNum });
					} else if (kind === 0x01) {
						// Text
						nodes.push({ kind, text: this.readStr() });
					} else if (kind === 0x02) {
						// Dynamic
						nodes.push({ kind, dynamicIndex: this.readU32() });
					} else if (kind === 0x03) {
						// DynamicText
						nodes.push({ kind, dynamicIndex: this.readU32() });
					}
				}
				const attrs = [];
				for (let i = 0; i < attrCount; i++) {
					const akind = this.readU8();
					if (akind === 0x00) {
						// Static
						attrs.push({
							kind: akind,
							name: this.readShortStr(),
							value: this.readStr(),
						});
					} else if (akind === 0x01) {
						// Dynamic
						attrs.push({ kind: akind, dynamicIndex: this.readU32() });
					}
				}
				const rootIndices = [];
				for (let i = 0; i < rootCount; i++) rootIndices.push(this.readU16());
				return {
					op,
					tmplId,
					name,
					rootCount,
					nodeCount,
					attrCount,
					nodes,
					attrs,
					rootIndices,
				};
			}
			default:
				throw new Error(`Unknown opcode 0x${op.toString(16)}`);
		}
	}
}
