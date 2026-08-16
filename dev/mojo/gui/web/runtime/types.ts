// --- Types for WASM module exports ---

export interface WasmExports extends WebAssembly.Exports {
	memory: WebAssembly.Memory;
	__heap_base: WebAssembly.Global;
	__heap_end: WebAssembly.Global;

	// Arithmetic — add
	add_int32(x: number, y: number): number;
	add_int64(x: bigint, y: bigint): bigint;
	add_float32(x: number, y: number): number;
	add_float64(x: number, y: number): number;

	// Arithmetic — subtract
	sub_int32(x: number, y: number): number;
	sub_int64(x: bigint, y: bigint): bigint;
	sub_float32(x: number, y: number): number;
	sub_float64(x: number, y: number): number;

	// Arithmetic — multiply
	mul_int32(x: number, y: number): number;
	mul_int64(x: bigint, y: bigint): bigint;
	mul_float32(x: number, y: number): number;
	mul_float64(x: number, y: number): number;

	// Arithmetic — division
	div_int32(x: number, y: number): number;
	div_int64(x: bigint, y: bigint): bigint;
	div_float32(x: number, y: number): number;
	div_float64(x: number, y: number): number;

	// Arithmetic — modulo
	mod_int32(x: number, y: number): number;
	mod_int64(x: bigint, y: bigint): bigint;

	// Arithmetic — power
	pow_int32(x: number): number;
	pow_int64(x: bigint): bigint;
	pow_float32(x: number): number;
	pow_float64(x: number): number;

	// Unary — negate
	neg_int32(x: number): number;
	neg_int64(x: bigint): bigint;
	neg_float32(x: number): number;
	neg_float64(x: number): number;

	// Unary — absolute value
	abs_int32(x: number): number;
	abs_int64(x: bigint): bigint;
	abs_float32(x: number): number;
	abs_float64(x: number): number;

	// Min / Max
	min_int32(x: number, y: number): number;
	max_int32(x: number, y: number): number;
	min_int64(x: bigint, y: bigint): bigint;
	max_int64(x: bigint, y: bigint): bigint;
	min_float64(x: number, y: number): number;
	max_float64(x: number, y: number): number;

	// Clamp
	clamp_int32(x: number, lo: number, hi: number): number;
	clamp_float64(x: number, lo: number, hi: number): number;

	// Bitwise
	bitand_int32(x: number, y: number): number;
	bitor_int32(x: number, y: number): number;
	bitxor_int32(x: number, y: number): number;
	bitnot_int32(x: number): number;
	shl_int32(x: number, y: number): number;
	shr_int32(x: number, y: number): number;

	// Comparison
	eq_int32(x: number, y: number): number;
	ne_int32(x: number, y: number): number;
	lt_int32(x: number, y: number): number;
	le_int32(x: number, y: number): number;
	gt_int32(x: number, y: number): number;
	ge_int32(x: number, y: number): number;

	// Boolean logic
	bool_and(x: number, y: number): number;
	bool_or(x: number, y: number): number;
	bool_not(x: number): number;

	// Algorithms — fibonacci
	fib_int32(n: number): number;
	fib_int64(n: bigint): bigint;

	// Algorithms — factorial
	factorial_int32(n: number): number;
	factorial_int64(n: bigint): bigint;

	// Algorithms — GCD
	gcd_int32(x: number, y: number): number;

	// Identity / passthrough
	identity_int32(x: number): number;
	identity_int64(x: bigint): bigint;
	identity_float32(x: number): number;
	identity_float64(x: number): number;

	// ── Mutation Protocol ────────────────────────────────────────────

	// Buffer management
	mutation_buf_alloc(capacity: number): bigint;
	mutation_buf_free(ptr: bigint): void;

	// Simple opcodes (no string/path payload)
	write_op_end(buf: bigint, off: number): number;
	write_op_append_children(
		buf: bigint,
		off: number,
		id: number,
		m: number,
	): number;
	write_op_create_placeholder(buf: bigint, off: number, id: number): number;
	write_op_load_template(
		buf: bigint,
		off: number,
		tmplId: number,
		index: number,
		id: number,
	): number;
	write_op_replace_with(
		buf: bigint,
		off: number,
		id: number,
		m: number,
	): number;
	write_op_insert_after(
		buf: bigint,
		off: number,
		id: number,
		m: number,
	): number;
	write_op_insert_before(
		buf: bigint,
		off: number,
		id: number,
		m: number,
	): number;
	write_op_remove(buf: bigint, off: number, id: number): number;
	write_op_push_root(buf: bigint, off: number, id: number): number;

	// String-carrying opcodes (text param is a Mojo String struct pointer)
	write_op_create_text_node(
		buf: bigint,
		off: number,
		id: number,
		text: bigint,
	): number;
	write_op_set_text(buf: bigint, off: number, id: number, text: bigint): number;
	write_op_set_attribute(
		buf: bigint,
		off: number,
		id: number,
		ns: number,
		name: bigint,
		value: bigint,
	): number;
	write_op_new_event_listener(
		buf: bigint,
		off: number,
		id: number,
		name: bigint,
	): number;
	write_op_remove_event_listener(
		buf: bigint,
		off: number,
		id: number,
		name: bigint,
	): number;

	// Path-carrying opcodes
	write_op_assign_id(
		buf: bigint,
		off: number,
		pathPtr: bigint,
		pathLen: number,
		id: number,
	): number;
	write_op_replace_placeholder(
		buf: bigint,
		off: number,
		pathPtr: bigint,
		pathLen: number,
		m: number,
	): number;

	// Composite test helper
	write_test_sequence(buf: bigint): number;

	// Print
	print_static_string(): void;
	print_int32(): void;
	print_int64(): void;
	print_float32(): void;
	print_float64(): void;
	print_input_string(structPtr: bigint): void;

	// Return string
	return_static_string(outStructPtr: bigint): void;
	return_input_string(inStructPtr: bigint, outStructPtr: bigint): void;

	// String ops
	string_length(structPtr: bigint): bigint;
	string_concat(
		xStructPtr: bigint,
		yStructPtr: bigint,
		outStructPtr: bigint,
	): void;
	string_repeat(xStructPtr: bigint, n: number, outStructPtr: bigint): void;
	string_eq(xStructPtr: bigint, yStructPtr: bigint): number;

	// ── ElementId Allocator ──────────────────────────────────────────

	eid_alloc_create(): bigint;
	eid_alloc_destroy(allocPtr: bigint): void;
	eid_alloc(allocPtr: bigint): number;
	eid_free(allocPtr: bigint, id: number): void;
	eid_is_alive(allocPtr: bigint, id: number): number;
	eid_count(allocPtr: bigint): number;
	eid_user_count(allocPtr: bigint): number;

	// ── Reactive Runtime / Signals ───────────────────────────────────

	// Runtime lifecycle
	runtime_create(): bigint;
	runtime_destroy(rtPtr: bigint): void;

	// Signal CRUD
	signal_create_i32(rtPtr: bigint, initial: number): number;
	signal_read_i32(rtPtr: bigint, key: number): number;
	signal_write_i32(rtPtr: bigint, key: number, value: number): void;
	signal_peek_i32(rtPtr: bigint, key: number): number;
	signal_destroy(rtPtr: bigint, key: number): void;

	// Signal queries
	signal_subscriber_count(rtPtr: bigint, key: number): number;
	signal_version(rtPtr: bigint, key: number): number;
	signal_count(rtPtr: bigint): number;
	signal_contains(rtPtr: bigint, key: number): number;

	// Signal arithmetic helpers
	signal_iadd_i32(rtPtr: bigint, key: number, rhs: number): void;
	signal_isub_i32(rtPtr: bigint, key: number, rhs: number): void;

	// Context management
	runtime_set_context(rtPtr: bigint, contextId: number): void;
	runtime_clear_context(rtPtr: bigint): void;
	runtime_has_context(rtPtr: bigint): number;
	runtime_dirty_count(rtPtr: bigint): number;
	runtime_has_dirty(rtPtr: bigint): number;

	// ── Scopes ───────────────────────────────────────────────────────

	// Scope lifecycle
	scope_create(rtPtr: bigint, height: number, parentId: number): number;
	scope_create_child(rtPtr: bigint, parentId: number): number;
	scope_destroy(rtPtr: bigint, id: number): void;
	scope_count(rtPtr: bigint): number;
	scope_contains(rtPtr: bigint, id: number): number;

	// Scope queries
	scope_height(rtPtr: bigint, id: number): number;
	scope_parent(rtPtr: bigint, id: number): number;
	scope_is_dirty(rtPtr: bigint, id: number): number;
	scope_set_dirty(rtPtr: bigint, id: number, dirty: number): void;
	scope_render_count(rtPtr: bigint, id: number): number;
	scope_hook_count(rtPtr: bigint, id: number): number;
	scope_hook_value_at(rtPtr: bigint, id: number, index: number): number;
	scope_hook_tag_at(rtPtr: bigint, id: number, index: number): number;

	// Scope rendering lifecycle
	scope_begin_render(rtPtr: bigint, scopeId: number): number;
	scope_end_render(rtPtr: bigint, prevScope: number): void;
	scope_has_scope(rtPtr: bigint): number;
	scope_get_current(rtPtr: bigint): number;
	scope_is_first_render(rtPtr: bigint, scopeId: number): number;

	// ── Hooks ────────────────────────────────────────────────────────

	hook_use_signal_i32(rtPtr: bigint, initial: number): number;
	hook_use_memo_i32(rtPtr: bigint, initial: number): number;
	hook_use_effect(rtPtr: bigint): number;

	// ── Template Builder ─────────────────────────────────────────────

	tmpl_builder_create(namePtr: bigint): bigint;
	tmpl_builder_destroy(ptr: bigint): void;
	tmpl_builder_push_element(
		ptr: bigint,
		htmlTag: number,
		parent: number,
	): number;
	tmpl_builder_push_text(ptr: bigint, text: bigint, parent: number): number;
	tmpl_builder_push_dynamic(
		ptr: bigint,
		dynamicIndex: number,
		parent: number,
	): number;
	tmpl_builder_push_dynamic_text(
		ptr: bigint,
		dynamicIndex: number,
		parent: number,
	): number;
	tmpl_builder_push_static_attr(
		ptr: bigint,
		nodeIndex: number,
		name: bigint,
		value: bigint,
	): void;
	tmpl_builder_push_dynamic_attr(
		ptr: bigint,
		nodeIndex: number,
		dynamicIndex: number,
	): void;
	tmpl_builder_node_count(ptr: bigint): number;
	tmpl_builder_root_count(ptr: bigint): number;
	tmpl_builder_attr_count(ptr: bigint): number;
	tmpl_builder_register(rtPtr: bigint, builderPtr: bigint): number;

	// ── Template Registry Queries ────────────────────────────────────

	tmpl_count(rtPtr: bigint): number;
	tmpl_root_count(rtPtr: bigint, tmplId: number): number;
	tmpl_node_count(rtPtr: bigint, tmplId: number): number;
	tmpl_node_kind(rtPtr: bigint, tmplId: number, nodeIdx: number): number;
	tmpl_node_tag(rtPtr: bigint, tmplId: number, nodeIdx: number): number;
	tmpl_node_child_count(rtPtr: bigint, tmplId: number, nodeIdx: number): number;
	tmpl_node_child_at(
		rtPtr: bigint,
		tmplId: number,
		nodeIdx: number,
		childPos: number,
	): number;
	tmpl_node_dynamic_index(
		rtPtr: bigint,
		tmplId: number,
		nodeIdx: number,
	): number;
	tmpl_node_attr_count(rtPtr: bigint, tmplId: number, nodeIdx: number): number;
	tmpl_attr_total_count(rtPtr: bigint, tmplId: number): number;
	tmpl_get_root_index(rtPtr: bigint, tmplId: number, rootPos: number): number;
	tmpl_attr_kind(rtPtr: bigint, tmplId: number, attrIdx: number): number;
	tmpl_attr_dynamic_index(
		rtPtr: bigint,
		tmplId: number,
		attrIdx: number,
	): number;
	tmpl_dynamic_node_count(rtPtr: bigint, tmplId: number): number;
	tmpl_dynamic_text_count(rtPtr: bigint, tmplId: number): number;
	tmpl_dynamic_attr_count(rtPtr: bigint, tmplId: number): number;
	tmpl_static_attr_count(rtPtr: bigint, tmplId: number): number;
	tmpl_contains_name(rtPtr: bigint, name: bigint): number;
	tmpl_find_by_name(rtPtr: bigint, name: bigint): number;
	tmpl_node_first_attr(rtPtr: bigint, tmplId: number, nodeIdx: number): number;

	// Template string queries (Phase 5)
	tmpl_node_text(rtPtr: bigint, tmplId: number, nodeIdx: number): string;
	tmpl_attr_name(rtPtr: bigint, tmplId: number, attrIdx: number): string;
	tmpl_attr_value(rtPtr: bigint, tmplId: number, attrIdx: number): string;

	// ── VNode Store ──────────────────────────────────────────────────

	vnode_store_create(): bigint;
	vnode_store_destroy(storePtr: bigint): void;
	vnode_push_template_ref(storePtr: bigint, tmplId: number): number;
	vnode_push_template_ref_keyed(
		storePtr: bigint,
		tmplId: number,
		key: bigint,
	): number;
	vnode_push_text(storePtr: bigint, text: bigint): number;
	vnode_push_placeholder(storePtr: bigint, elementId: number): number;
	vnode_push_fragment(storePtr: bigint): number;
	vnode_count(storePtr: bigint): number;
	vnode_kind(storePtr: bigint, index: number): number;
	vnode_template_id(storePtr: bigint, index: number): number;
	vnode_element_id(storePtr: bigint, index: number): number;
	vnode_has_key(storePtr: bigint, index: number): number;
	vnode_dynamic_node_count(storePtr: bigint, index: number): number;
	vnode_dynamic_attr_count(storePtr: bigint, index: number): number;
	vnode_fragment_child_count(storePtr: bigint, index: number): number;
	vnode_fragment_child_at(
		storePtr: bigint,
		index: number,
		childPos: number,
	): number;
	vnode_push_dynamic_text_node(
		storePtr: bigint,
		vnodeIndex: number,
		text: bigint,
	): void;
	vnode_push_dynamic_placeholder(storePtr: bigint, vnodeIndex: number): void;
	vnode_push_dynamic_attr_text(
		storePtr: bigint,
		vnodeIndex: number,
		name: bigint,
		value: bigint,
		elemId: number,
	): void;
	vnode_push_dynamic_attr_int(
		storePtr: bigint,
		vnodeIndex: number,
		name: bigint,
		value: number,
		elemId: number,
	): void;
	vnode_push_dynamic_attr_bool(
		storePtr: bigint,
		vnodeIndex: number,
		name: bigint,
		value: number,
		elemId: number,
	): void;
	vnode_push_dynamic_attr_event(
		storePtr: bigint,
		vnodeIndex: number,
		name: bigint,
		handlerId: number,
		elemId: number,
	): void;
	vnode_push_dynamic_attr_none(
		storePtr: bigint,
		vnodeIndex: number,
		name: bigint,
		elemId: number,
	): void;
	vnode_push_fragment_child(
		storePtr: bigint,
		vnodeIndex: number,
		childIndex: number,
	): void;
	vnode_get_dynamic_node_kind(
		storePtr: bigint,
		vnodeIndex: number,
		dynIndex: number,
	): number;
	vnode_get_dynamic_attr_kind(
		storePtr: bigint,
		vnodeIndex: number,
		attrIndex: number,
	): number;
	vnode_get_dynamic_attr_element_id(
		storePtr: bigint,
		vnodeIndex: number,
		attrIndex: number,
	): number;
	vnode_store_clear(storePtr: bigint): void;

	// ── Phase 4: Create & Diff Engine ────────────────────────────────

	// MutationWriter lifecycle
	writer_create(bufPtr: bigint, capacity: number): bigint;
	writer_destroy(writerPtr: bigint): void;
	writer_offset(writerPtr: bigint): number;
	writer_finalize(writerPtr: bigint): number;

	// Create engine
	create_vnode(
		writerPtr: bigint,
		eidPtr: bigint,
		rtPtr: bigint,
		storePtr: bigint,
		vnodeIndex: number,
	): number;

	// Diff engine
	diff_vnodes(
		writerPtr: bigint,
		eidPtr: bigint,
		rtPtr: bigint,
		storePtr: bigint,
		oldIndex: number,
		newIndex: number,
	): number;

	// VNode mount state queries
	vnode_root_id_count(storePtr: bigint, index: number): number;
	vnode_get_root_id(storePtr: bigint, index: number, pos: number): number;
	vnode_dyn_node_id_count(storePtr: bigint, index: number): number;
	vnode_get_dyn_node_id(storePtr: bigint, index: number, pos: number): number;
	vnode_dyn_attr_id_count(storePtr: bigint, index: number): number;
	vnode_get_dyn_attr_id(storePtr: bigint, index: number, pos: number): number;
	vnode_is_mounted(storePtr: bigint, index: number): number;

	// ── Phase 6: Event Handler Registry ──────────────────────────────

	// Handler registration
	handler_register_signal_add(
		rtPtr: bigint,
		scopeId: number,
		signalKey: number,
		delta: number,
		eventName: bigint,
	): number;
	handler_register_signal_sub(
		rtPtr: bigint,
		scopeId: number,
		signalKey: number,
		delta: number,
		eventName: bigint,
	): number;
	handler_register_signal_set(
		rtPtr: bigint,
		scopeId: number,
		signalKey: number,
		value: number,
		eventName: bigint,
	): number;
	handler_register_signal_toggle(
		rtPtr: bigint,
		scopeId: number,
		signalKey: number,
		eventName: bigint,
	): number;
	handler_register_signal_set_input(
		rtPtr: bigint,
		scopeId: number,
		signalKey: number,
		eventName: bigint,
	): number;
	handler_register_signal_set_string(
		rtPtr: bigint,
		scopeId: number,
		stringKey: number,
		versionKey: number,
		eventName: bigint,
	): number;
	handler_register_custom(
		rtPtr: bigint,
		scopeId: number,
		eventName: bigint,
	): number;
	handler_register_noop(
		rtPtr: bigint,
		scopeId: number,
		eventName: bigint,
	): number;

	// Handler management
	handler_remove(rtPtr: bigint, handlerId: number): void;
	handler_count(rtPtr: bigint): number;
	handler_contains(rtPtr: bigint, handlerId: number): number;

	// Handler queries
	handler_scope_id(rtPtr: bigint, handlerId: number): number;
	handler_action(rtPtr: bigint, handlerId: number): number;
	handler_signal_key(rtPtr: bigint, handlerId: number): number;
	handler_operand(rtPtr: bigint, handlerId: number): number;

	// Event dispatch
	dispatch_event(rtPtr: bigint, handlerId: number, eventType: number): number;
	dispatch_event_with_i32(
		rtPtr: bigint,
		handlerId: number,
		eventType: number,
		value: number,
	): number;
	dispatch_event_with_string(
		rtPtr: bigint,
		handlerId: number,
		eventType: number,
		value: bigint,
	): number;

	// String signal queries (Phase 20)
	signal_create_string(rtPtr: bigint, initial: bigint): bigint;
	signal_string_key(packed: bigint): number;
	signal_version_key(packed: bigint): number;
	signal_peek_string(rtPtr: bigint, stringKey: number, outPtr: bigint): void;
	signal_write_string(
		rtPtr: bigint,
		stringKey: number,
		versionKey: number,
		value: bigint,
	): void;
	signal_string_count(rtPtr: bigint): number;

	// Dirty scope management
	runtime_drain_dirty(rtPtr: bigint): number;

	// ── Phase 13.2: Memo (Computed/Derived Signals) ──────────────────

	memo_create_i32(rtPtr: bigint, scopeId: number, initial: number): number;
	memo_begin_compute(rtPtr: bigint, memoId: number): void;
	memo_end_compute_i32(rtPtr: bigint, memoId: number, value: number): void;
	memo_read_i32(rtPtr: bigint, memoId: number): number;
	memo_is_dirty(rtPtr: bigint, memoId: number): number;
	memo_destroy(rtPtr: bigint, memoId: number): void;
	memo_count(rtPtr: bigint): number;
	memo_output_key(rtPtr: bigint, memoId: number): number;
	memo_context_id(rtPtr: bigint, memoId: number): number;

	// ── Phase 14: Effects (Reactive Side Effects) ────────────────────

	effect_create(rtPtr: bigint, scopeId: number): number;
	effect_begin_run(rtPtr: bigint, effectId: number): void;
	effect_end_run(rtPtr: bigint, effectId: number): void;
	effect_is_pending(rtPtr: bigint, effectId: number): number;
	effect_destroy(rtPtr: bigint, effectId: number): void;
	effect_count(rtPtr: bigint): number;
	effect_context_id(rtPtr: bigint, effectId: number): number;
	effect_drain_pending(rtPtr: bigint): number;
	effect_pending_at(rtPtr: bigint, index: number): number;

	// ── Phase 13.5: AppShell Memo Helpers ────────────────────────────

	shell_create(): bigint;
	shell_destroy(shellPtr: bigint): void;
	shell_is_alive(shellPtr: bigint): number;
	shell_create_root_scope(shellPtr: bigint): number;
	shell_create_child_scope(shellPtr: bigint, parentId: number): number;
	shell_create_signal_i32(shellPtr: bigint, initial: number): number;
	shell_read_signal_i32(shellPtr: bigint, key: number): number;
	shell_peek_signal_i32(shellPtr: bigint, key: number): number;
	shell_write_signal_i32(shellPtr: bigint, key: number, value: number): void;
	shell_begin_render(shellPtr: bigint, scopeId: number): number;
	shell_end_render(shellPtr: bigint, prevScope: number): void;
	shell_has_dirty(shellPtr: bigint): number;
	shell_collect_dirty(shellPtr: bigint): void;
	shell_next_dirty(shellPtr: bigint): number;
	shell_scheduler_empty(shellPtr: bigint): number;
	shell_dispatch_event(
		shellPtr: bigint,
		handlerId: number,
		eventType: number,
	): number;
	shell_dispatch_event_with_string(
		shellPtr: bigint,
		handlerId: number,
		eventType: number,
		value: bigint,
	): number;
	shell_rt_ptr(shellPtr: bigint): bigint;
	shell_store_ptr(shellPtr: bigint): bigint;
	shell_eid_ptr(shellPtr: bigint): bigint;
	shell_memo_create_i32(
		shellPtr: bigint,
		scopeId: number,
		initial: number,
	): number;
	shell_memo_begin_compute(shellPtr: bigint, memoId: number): void;
	shell_memo_end_compute_i32(
		shellPtr: bigint,
		memoId: number,
		value: number,
	): void;
	shell_memo_read_i32(shellPtr: bigint, memoId: number): number;
	shell_memo_is_dirty(shellPtr: bigint, memoId: number): number;
	shell_use_memo_i32(shellPtr: bigint, initial: number): number;

	// ── Phase 14.4: AppShell Effect Helpers ──────────────────────────

	shell_effect_create(shellPtr: bigint, scopeId: number): number;
	shell_effect_begin_run(shellPtr: bigint, effectId: number): void;
	shell_effect_end_run(shellPtr: bigint, effectId: number): void;
	shell_effect_is_pending(shellPtr: bigint, effectId: number): number;
	shell_use_effect(shellPtr: bigint): number;
	shell_effect_drain_pending(shellPtr: bigint): number;
	shell_effect_pending_at(shellPtr: bigint, index: number): number;

	// ── Phase 7: Counter App ─────────────────────────────────────────

	// App lifecycle
	counter_init(): bigint;
	counter_destroy(appPtr: bigint): void;
	counter_rebuild(appPtr: bigint, bufPtr: bigint, capacity: number): number;
	counter_handle_event(
		appPtr: bigint,
		handlerId: number,
		eventType: number,
	): number;
	counter_flush(appPtr: bigint, bufPtr: bigint, capacity: number): number;

	// App queries
	counter_rt_ptr(appPtr: bigint): bigint;
	counter_tmpl_id(appPtr: bigint): number;
	counter_incr_handler(appPtr: bigint): number;
	counter_decr_handler(appPtr: bigint): number;
	counter_count_value(appPtr: bigint): number;
	counter_has_dirty(appPtr: bigint): number;
	counter_scope_id(appPtr: bigint): number;
	counter_count_signal(appPtr: bigint): number;
	counter_doubled_value(appPtr: bigint): number;
	counter_doubled_memo(appPtr: bigint): number;
}
