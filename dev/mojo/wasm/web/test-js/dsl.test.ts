// DSL Ergonomic Builder Tests — M10.5
//
// Tests the declarative builder DSL:
//   - Node construction (text, dyn_text, dyn_node, attr, dyn_attr, elements)
//   - Tag helpers (all 40 tags)
//   - to_template conversion and template equivalence
//   - VNodeBuilder ergonomic VNode construction
//   - count_* utility functions
//
// Self-contained tests call dsl_test_* WASM exports that return 1/0.
// Orchestrated tests use dsl_node_* and dsl_vb_* exports to build
// structures from the JS side and verify properties.

import { writeStringStruct } from "../runtime/strings.ts";
import type { WasmExports } from "../runtime/types.ts";
import { assert, suite } from "./harness.ts";

const ws = writeStringStruct;

type Fns = WasmExports & Record<string, CallableFunction>;

// ── Node kind constants (matching src/vdom/dsl.mojo) ────────────────────────

const NODE_TEXT = 0;
const NODE_ELEMENT = 1;
const NODE_DYN_TEXT = 2;
const NODE_DYN_NODE = 3;
const NODE_STATIC_ATTR = 4;
const NODE_DYN_ATTR = 5;
const _NODE_EVENT = 6;
const _NODE_BIND_VALUE = 7;

// ── HTML tag constants (matching src/vdom/tags.mojo) ────────────────────────

const TAG_DIV = 0;
const TAG_SPAN = 1;
const TAG_H1 = 10;
const TAG_BUTTON = 19;

// ── Template/VNode constants ────────────────────────────────────────────────

const TNODE_ELEMENT = 0;
const TNODE_TEXT = 1;
const VNODE_TEMPLATE_REF = 0;

// ══════════════════════════════════════════════════════════════════════════════

export function testDsl(fns: Fns): void {
	// ═════════════════════════════════════════════════════════════════════
	// Section 1: Self-contained DSL tests (Mojo-side, return 1/0)
	// ═════════════════════════════════════════════════════════════════════

	suite("DSL — text node");
	assert(fns.dsl_test_text_node(), 1, "text() creates correct NODE_TEXT");

	suite("DSL — dynamic text node");
	assert(
		fns.dsl_test_dyn_text_node(),
		1,
		"dyn_text() creates correct NODE_DYN_TEXT",
	);

	suite("DSL — dynamic node slot");
	assert(
		fns.dsl_test_dyn_node_slot(),
		1,
		"dyn_node() creates correct NODE_DYN_NODE",
	);

	suite("DSL — static attribute");
	assert(
		fns.dsl_test_static_attr(),
		1,
		"attr() creates correct NODE_STATIC_ATTR",
	);

	suite("DSL — dynamic attribute");
	assert(
		fns.dsl_test_dyn_attr(),
		1,
		"dyn_attr() creates correct NODE_DYN_ATTR",
	);

	suite("DSL — empty element");
	assert(
		fns.dsl_test_empty_element(),
		1,
		"el_div() with no args creates empty element",
	);

	suite("DSL — element with children");
	assert(fns.dsl_test_element_with_children(), 1, "el_div with text children");

	suite("DSL — element with attributes");
	assert(fns.dsl_test_element_with_attrs(), 1, "el_div with static attributes");

	suite("DSL — element with mixed children and attrs");
	assert(
		fns.dsl_test_element_mixed(),
		1,
		"el_div with mixed attrs + children + dynamic slots",
	);

	suite("DSL — nested elements");
	assert(fns.dsl_test_nested_elements(), 1, "deeply nested element tree");

	suite("DSL — all tag helpers");
	assert(
		fns.dsl_test_all_tag_helpers(),
		1,
		"every tag helper produces correct tag constant",
	);

	suite("DSL — count utilities");
	assert(
		fns.dsl_test_count_utilities(),
		1,
		"count_* utility functions on non-trivial tree",
	);

	// ═════════════════════════════════════════════════════════════════════
	// Section 2: Template conversion tests
	// ═════════════════════════════════════════════════════════════════════

	suite("DSL — to_template simple");
	assert(
		fns.dsl_test_to_template_simple(),
		1,
		"simple div+text converts to valid template",
	);

	suite("DSL — to_template with attributes");
	assert(
		fns.dsl_test_to_template_attrs(),
		1,
		"element with static+dynamic attrs converts correctly",
	);

	suite("DSL — to_template multi-root");
	assert(
		fns.dsl_test_to_template_multi_root(),
		1,
		"multiple roots via to_template_multi",
	);

	suite("DSL — counter template via DSL");
	assert(
		fns.dsl_test_counter_template(),
		1,
		"counter template structure matches expected",
	);

	suite("DSL — template equivalence (DSL vs manual builder)");
	assert(
		fns.dsl_test_template_equivalence(),
		1,
		"DSL-built template matches manually-built template",
	);

	// ═════════════════════════════════════════════════════════════════════
	// Section 3: VNodeBuilder tests
	// ═════════════════════════════════════════════════════════════════════

	suite("DSL — VNodeBuilder");
	assert(
		fns.dsl_test_vnode_builder(),
		1,
		"VNodeBuilder creates VNode with correct dynamic content",
	);

	suite("DSL — VNodeBuilder keyed");
	assert(
		fns.dsl_test_vnode_builder_keyed(),
		1,
		"keyed VNodeBuilder creates keyed VNode",
	);

	// ═════════════════════════════════════════════════════════════════════
	// Section 4: JS-side orchestrated Node construction
	// ═════════════════════════════════════════════════════════════════════

	suite("DSL JS — create text node via WASM export");
	{
		const n = fns.dsl_node_text(ws("hello"));
		assert(fns.dsl_node_kind(n), NODE_TEXT, "kind is NODE_TEXT");
		fns.dsl_node_destroy(n);
	}

	suite("DSL JS — create dyn_text node via WASM export");
	{
		const n = fns.dsl_node_dyn_text(2);
		assert(fns.dsl_node_kind(n), NODE_DYN_TEXT, "kind is NODE_DYN_TEXT");
		assert(fns.dsl_node_dynamic_index(n), 2, "dynamic_index is 2");
		fns.dsl_node_destroy(n);
	}

	suite("DSL JS — create dyn_node via WASM export");
	{
		const n = fns.dsl_node_dyn_node(7);
		assert(fns.dsl_node_kind(n), NODE_DYN_NODE, "kind is NODE_DYN_NODE");
		assert(fns.dsl_node_dynamic_index(n), 7, "dynamic_index is 7");
		fns.dsl_node_destroy(n);
	}

	suite("DSL JS — create static attr via WASM export");
	{
		const n = fns.dsl_node_attr(ws("class"), ws("active"));
		assert(fns.dsl_node_kind(n), NODE_STATIC_ATTR, "kind is NODE_STATIC_ATTR");
		fns.dsl_node_destroy(n);
	}

	suite("DSL JS — create dyn_attr via WASM export");
	{
		const n = fns.dsl_node_dyn_attr(3);
		assert(fns.dsl_node_kind(n), NODE_DYN_ATTR, "kind is NODE_DYN_ATTR");
		assert(fns.dsl_node_dynamic_index(n), 3, "dynamic_index is 3");
		fns.dsl_node_destroy(n);
	}

	suite("DSL JS — create empty element via WASM export");
	{
		const n = fns.dsl_node_element(TAG_DIV);
		assert(fns.dsl_node_kind(n), NODE_ELEMENT, "kind is NODE_ELEMENT");
		assert(fns.dsl_node_tag(n), TAG_DIV, "tag is TAG_DIV");
		assert(fns.dsl_node_item_count(n), 0, "0 items");
		assert(fns.dsl_node_child_count(n), 0, "0 children");
		assert(fns.dsl_node_attr_count(n), 0, "0 attrs");
		fns.dsl_node_destroy(n);
	}

	suite("DSL JS — add items to element");
	{
		const div = fns.dsl_node_element(TAG_DIV);
		const txt = fns.dsl_node_text(ws("hello"));
		const a = fns.dsl_node_attr(ws("class"), ws("box"));
		const dt = fns.dsl_node_dyn_text(0);

		// Add items (child pointers are consumed)
		fns.dsl_node_add_item(div, a);
		fns.dsl_node_add_item(div, txt);
		fns.dsl_node_add_item(div, dt);

		assert(fns.dsl_node_item_count(div), 3, "3 items total");
		assert(fns.dsl_node_child_count(div), 2, "2 children (text + dyn_text)");
		assert(fns.dsl_node_attr_count(div), 1, "1 attr");

		fns.dsl_node_destroy(div);
	}

	suite("DSL JS — nested tree via add_item");
	{
		// Build: div > [ span > text("inner"), button > dyn_text(0) + dyn_attr(0) ]
		const span = fns.dsl_node_element(TAG_SPAN);
		fns.dsl_node_add_item(span, fns.dsl_node_text(ws("inner")));

		const btn = fns.dsl_node_element(TAG_BUTTON);
		fns.dsl_node_add_item(btn, fns.dsl_node_dyn_text(0));
		fns.dsl_node_add_item(btn, fns.dsl_node_dyn_attr(0));

		const div = fns.dsl_node_element(TAG_DIV);
		fns.dsl_node_add_item(div, span);
		fns.dsl_node_add_item(div, btn);

		assert(fns.dsl_node_child_count(div), 2, "div has 2 children");
		// Tree nodes: div + span + "inner" + button + dyn_text(0) = 5
		assert(fns.dsl_node_count_nodes(div), 5, "5 tree nodes");
		assert(fns.dsl_node_count_dyn_text(div), 1, "1 dyn_text slot");
		assert(fns.dsl_node_count_dyn_attr(div), 1, "1 dyn_attr slot");

		fns.dsl_node_destroy(div);
	}

	suite("DSL JS — to_template via WASM export");
	{
		const rt = fns.runtime_create();

		// Build: h1 > text("Title")
		const h1 = fns.dsl_node_element(TAG_H1);
		fns.dsl_node_add_item(h1, fns.dsl_node_text(ws("Title")));

		// Convert to template (consumes the node)
		const tmplId = fns.dsl_to_template(h1, ws("js-h1-test"), rt);

		assert(tmplId >= 0, true, "template ID is non-negative");
		assert(fns.tmpl_node_count(rt, tmplId), 2, "template has 2 nodes");
		assert(fns.tmpl_root_count(rt, tmplId), 1, "template has 1 root");
		assert(fns.tmpl_node_kind(rt, tmplId, 0), TNODE_ELEMENT, "root is element");
		assert(fns.tmpl_node_tag(rt, tmplId, 0), TAG_H1, "root tag is H1");
		assert(fns.tmpl_node_kind(rt, tmplId, 1), TNODE_TEXT, "child is text");
		assert(fns.tmpl_node_child_count(rt, tmplId, 0), 1, "root has 1 child");

		fns.runtime_destroy(rt);
	}

	suite("DSL JS — to_template with dynamic slots");
	{
		const rt = fns.runtime_create();

		// Build: div > [ attr("class","box"), dyn_attr(0), dyn_text(0), text("static") ]
		const div = fns.dsl_node_element(TAG_DIV);
		fns.dsl_node_add_item(div, fns.dsl_node_attr(ws("class"), ws("box")));
		fns.dsl_node_add_item(div, fns.dsl_node_dyn_attr(0));
		fns.dsl_node_add_item(div, fns.dsl_node_dyn_text(0));
		fns.dsl_node_add_item(div, fns.dsl_node_text(ws("static")));

		const tmplId = fns.dsl_to_template(div, ws("js-dynamic-test"), rt);

		assert(
			fns.tmpl_node_count(rt, tmplId),
			3,
			"3 nodes: div + dyn_text + text",
		);
		assert(fns.tmpl_dynamic_text_count(rt, tmplId), 1, "1 dynamic text slot");
		assert(fns.tmpl_dynamic_attr_count(rt, tmplId), 1, "1 dynamic attr slot");
		assert(fns.tmpl_static_attr_count(rt, tmplId), 1, "1 static attr");
		assert(fns.tmpl_attr_total_count(rt, tmplId), 2, "2 total attrs");

		fns.runtime_destroy(rt);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 5: JS-side orchestrated VNodeBuilder
	// ═════════════════════════════════════════════════════════════════════

	suite("DSL JS — VNodeBuilder via WASM exports");
	{
		const rt = fns.runtime_create();
		const store = fns.vnode_store_create();

		// Register a simple template (div > dyn_text(0))
		const div = fns.dsl_node_element(TAG_DIV);
		fns.dsl_node_add_item(div, fns.dsl_node_dyn_text(0));
		fns.dsl_node_add_item(div, fns.dsl_node_dyn_attr(0));
		const tmplId = fns.dsl_to_template(div, ws("js-vb-test"), rt);

		// Create VNodeBuilder
		const vb = fns.dsl_vb_create(tmplId, store);

		// Add dynamic content
		fns.dsl_vb_add_dyn_text(vb, ws("Count: 0"));
		fns.dsl_vb_add_dyn_event(vb, ws("click"), 42);

		const idx = fns.dsl_vb_index(vb);
		assert(idx >= 0, true, "VNode index is non-negative");

		// Verify VNode properties via store
		assert(
			fns.vnode_kind(store, idx),
			VNODE_TEMPLATE_REF,
			"VNode is TemplateRef",
		);
		assert(
			fns.vnode_template_id(store, idx),
			tmplId,
			"VNode has correct template ID",
		);
		assert(fns.vnode_dynamic_node_count(store, idx), 1, "1 dynamic node");
		assert(fns.vnode_dynamic_attr_count(store, idx), 1, "1 dynamic attr");

		fns.dsl_vb_destroy(vb);
		fns.vnode_store_destroy(store);
		fns.runtime_destroy(rt);
	}

	suite("DSL JS — VNodeBuilder with multiple dynamic attrs");
	{
		const rt = fns.runtime_create();
		const store = fns.vnode_store_create();

		// Template: div > dyn_attr(0) + dyn_attr(1) + dyn_attr(2)
		const div = fns.dsl_node_element(TAG_DIV);
		fns.dsl_node_add_item(div, fns.dsl_node_dyn_attr(0));
		fns.dsl_node_add_item(div, fns.dsl_node_dyn_attr(1));
		fns.dsl_node_add_item(div, fns.dsl_node_dyn_attr(2));
		const tmplId = fns.dsl_to_template(div, ws("js-vb-multi-attr"), rt);

		const vb = fns.dsl_vb_create(tmplId, store);
		fns.dsl_vb_add_dyn_text_attr(vb, ws("class"), ws("active"));
		fns.dsl_vb_add_dyn_int_attr(vb, ws("tabindex"), BigInt(5));
		fns.dsl_vb_add_dyn_bool_attr(vb, ws("disabled"), 1);

		const idx = fns.dsl_vb_index(vb);
		assert(fns.vnode_dynamic_attr_count(store, idx), 3, "3 dynamic attrs");

		fns.dsl_vb_destroy(vb);
		fns.vnode_store_destroy(store);
		fns.runtime_destroy(rt);
	}

	suite("DSL JS — VNodeBuilder keyed via WASM export");
	{
		const rt = fns.runtime_create();
		const store = fns.vnode_store_create();

		const div = fns.dsl_node_element(TAG_DIV);
		fns.dsl_node_add_item(div, fns.dsl_node_text(ws("item")));
		const tmplId = fns.dsl_to_template(div, ws("js-vb-keyed"), rt);

		const vb = fns.dsl_vb_create_keyed(tmplId, ws("key-99"), store);

		const idx = fns.dsl_vb_index(vb);
		assert(fns.vnode_has_key(store, idx), 1, "VNode has key");

		fns.dsl_vb_destroy(vb);
		fns.vnode_store_destroy(store);
		fns.runtime_destroy(rt);
	}

	suite("DSL JS — VNodeBuilder with placeholder");
	{
		const rt = fns.runtime_create();
		const store = fns.vnode_store_create();

		const div = fns.dsl_node_element(TAG_DIV);
		fns.dsl_node_add_item(div, fns.dsl_node_dyn_node(0));
		const tmplId = fns.dsl_to_template(div, ws("js-vb-placeholder"), rt);

		const vb = fns.dsl_vb_create(tmplId, store);
		fns.dsl_vb_add_dyn_placeholder(vb);

		const idx = fns.dsl_vb_index(vb);
		assert(
			fns.vnode_dynamic_node_count(store, idx),
			1,
			"1 dynamic node (placeholder)",
		);

		fns.dsl_vb_destroy(vb);
		fns.vnode_store_destroy(store);
		fns.runtime_destroy(rt);
	}

	suite("DSL JS — VNodeBuilder with none attr (removal)");
	{
		const rt = fns.runtime_create();
		const store = fns.vnode_store_create();

		const div = fns.dsl_node_element(TAG_DIV);
		fns.dsl_node_add_item(div, fns.dsl_node_dyn_attr(0));
		const tmplId = fns.dsl_to_template(div, ws("js-vb-none-attr"), rt);

		const vb = fns.dsl_vb_create(tmplId, store);
		fns.dsl_vb_add_dyn_none_attr(vb, ws("class"));

		const idx = fns.dsl_vb_index(vb);
		assert(
			fns.vnode_dynamic_attr_count(store, idx),
			1,
			"1 dynamic attr (none/removal)",
		);

		fns.dsl_vb_destroy(vb);
		fns.vnode_store_destroy(store);
		fns.runtime_destroy(rt);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 6: Full round-trip — DSL build → template → VNode → verify
	// ═════════════════════════════════════════════════════════════════════

	suite("DSL JS — full round-trip: counter pattern");
	{
		const rt = fns.runtime_create();
		const store = fns.vnode_store_create();

		// Build counter template via DSL (same structure as CounterApp):
		//   div > [ span > dyn_text(0), button > text("+") + dyn_attr(0), button > text("-") + dyn_attr(1) ]
		const span = fns.dsl_node_element(TAG_SPAN);
		fns.dsl_node_add_item(span, fns.dsl_node_dyn_text(0));

		const btn1 = fns.dsl_node_element(TAG_BUTTON);
		fns.dsl_node_add_item(btn1, fns.dsl_node_text(ws("+")));
		fns.dsl_node_add_item(btn1, fns.dsl_node_dyn_attr(0));

		const btn2 = fns.dsl_node_element(TAG_BUTTON);
		fns.dsl_node_add_item(btn2, fns.dsl_node_text(ws("-")));
		fns.dsl_node_add_item(btn2, fns.dsl_node_dyn_attr(1));

		const div = fns.dsl_node_element(TAG_DIV);
		fns.dsl_node_add_item(div, span);
		fns.dsl_node_add_item(div, btn1);
		fns.dsl_node_add_item(div, btn2);

		const tmplId = fns.dsl_to_template(div, ws("js-counter-roundtrip"), rt);

		// Verify template structure
		assert(fns.tmpl_node_count(rt, tmplId), 7, "7 template nodes");
		assert(fns.tmpl_root_count(rt, tmplId), 1, "1 root");
		assert(fns.tmpl_dynamic_text_count(rt, tmplId), 1, "1 dyn text slot");
		assert(fns.tmpl_dynamic_attr_count(rt, tmplId), 2, "2 dyn attr slots");
		assert(fns.tmpl_node_child_count(rt, tmplId, 0), 3, "div has 3 children");

		// Build VNode using VNodeBuilder
		const vb = fns.dsl_vb_create(tmplId, store);
		fns.dsl_vb_add_dyn_text(vb, ws("Count: 0"));
		fns.dsl_vb_add_dyn_event(vb, ws("click"), 100);
		fns.dsl_vb_add_dyn_event(vb, ws("click"), 101);

		const vnodeIdx = fns.dsl_vb_index(vb);
		assert(
			fns.vnode_kind(store, vnodeIdx),
			VNODE_TEMPLATE_REF,
			"VNode is TemplateRef",
		);
		assert(
			fns.vnode_template_id(store, vnodeIdx),
			tmplId,
			"correct template ID",
		);
		assert(fns.vnode_dynamic_node_count(store, vnodeIdx), 1, "1 dynamic node");
		assert(fns.vnode_dynamic_attr_count(store, vnodeIdx), 2, "2 dynamic attrs");

		fns.dsl_vb_destroy(vb);
		fns.vnode_store_destroy(store);
		fns.runtime_destroy(rt);
	}

	// ═════════════════════════════════════════════════════════════════════
	// Section 6: Phase 20 — M20.3: oninput_set_string / onchange_set_string
	// ═════════════════════════════════════════════════════════════════════

	suite("DSL — oninput_set_string node");
	assert(
		fns.dsl_test_oninput_set_string_node(),
		1,
		"oninput_set_string creates NODE_EVENT with correct fields",
	);

	suite("DSL — onchange_set_string node");
	assert(
		fns.dsl_test_onchange_set_string_node(),
		1,
		"onchange_set_string creates NODE_EVENT with correct fields",
	);

	suite("DSL — oninput_set_string in element");
	assert(
		fns.dsl_test_oninput_in_element(),
		1,
		"oninput_set_string counts as dynamic attr inside element",
	);

	// ═════════════════════════════════════════════════════════════════════
	// Section 7: Phase 20 — M20.4: bind_value / bind_attr
	// ═════════════════════════════════════════════════════════════════════

	suite("DSL — bind_value node");
	assert(
		fns.dsl_test_bind_value_node(),
		1,
		"bind_value creates NODE_BIND_VALUE with attr_name='value'",
	);

	suite("DSL — bind_attr node");
	assert(
		fns.dsl_test_bind_attr_node(),
		1,
		"bind_attr creates NODE_BIND_VALUE with custom attr name",
	);

	suite("DSL — bind_value in element");
	assert(
		fns.dsl_test_bind_value_in_element(),
		1,
		"bind_value counts as dynamic attr inside element",
	);

	suite("DSL — two-way binding element");
	assert(
		fns.dsl_test_two_way_binding_element(),
		1,
		"bind_value + oninput_set_string together produce 2 dynamic attrs",
	);

	suite("DSL — bind_value to_template");
	assert(
		fns.dsl_test_bind_value_to_template(),
		1,
		"bind_value converts to TATTR_DYNAMIC in template",
	);

	suite("DSL — two-way binding to_template");
	assert(
		fns.dsl_test_two_way_to_template(),
		1,
		"bind_value + oninput_set_string converts to 2 TATTR_DYNAMICs",
	);

	// ═════════════════════════════════════════════════════════════════════
	// Section 8: Phase 20 — M20.5: onclick_custom
	// ═════════════════════════════════════════════════════════════════════

	suite("DSL — onclick_custom node");
	assert(
		fns.dsl_test_onclick_custom_node(),
		1,
		"onclick_custom creates NODE_EVENT with ACTION_CUSTOM",
	);

	suite("DSL — onclick_custom in element");
	assert(
		fns.dsl_test_onclick_custom_in_element(),
		1,
		"onclick_custom counts as dynamic attr inside button element",
	);

	suite("DSL — onclick_custom with binding pattern");
	assert(
		fns.dsl_test_onclick_custom_with_binding(),
		1,
		"onclick_custom + bind_value + oninput_set_string in sibling elements",
	);

	// ═════════════════════════════════════════════════════════════════════
	// Section 9: Phase 22 — onkeydown_enter_custom
	// ═════════════════════════════════════════════════════════════════════

	suite("DSL — onkeydown_enter_custom node");
	assert(
		fns.dsl_test_onkeydown_enter_custom_node(),
		1,
		"onkeydown_enter_custom creates NODE_EVENT with ACTION_KEY_ENTER_CUSTOM",
	);

	suite("DSL — onkeydown_enter_custom in element");
	assert(
		fns.dsl_test_onkeydown_enter_custom_in_element(),
		1,
		"onkeydown_enter_custom counts as dynamic attr inside input element",
	);

	suite("DSL — onkeydown_enter_custom with binding pattern");
	assert(
		fns.dsl_test_onkeydown_enter_custom_with_binding(),
		1,
		"onkeydown_enter_custom + bind_value + oninput + onclick_custom (Phase 22 TodoApp pattern)",
	);
}
