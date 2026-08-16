"""Self-contained DSL test functions extracted from main.mojo (M10.13).

Each function exercises a specific aspect of the DSL end-to-end and returns
1 (pass) or 0 (fail).  The @export wrappers in main.mojo delegate to these.

These functions are called from both JS tests (via WASM exports) and serve
as regression tests for the declarative builder DSL.
"""

from .dsl import (
    Node,
    NODE_TEXT,
    NODE_ELEMENT,
    NODE_DYN_TEXT,
    NODE_DYN_NODE,
    NODE_STATIC_ATTR,
    NODE_DYN_ATTR,
    NODE_EVENT,
    NODE_BIND_VALUE,
    text,
    dyn_text,
    dyn_node,
    attr,
    dyn_attr,
    onclick_custom,
    onkeydown_enter_custom,
    oninput_set_string,
    onchange_set_string,
    bind_value,
    bind_attr,
    el_div,
    el_span,
    el_p,
    el_section,
    el_header,
    el_footer,
    el_nav,
    el_main,
    el_article,
    el_aside,
    el_h1,
    el_h2,
    el_h3,
    el_h4,
    el_h5,
    el_h6,
    el_ul,
    el_ol,
    el_li,
    el_button,
    el_input,
    el_form,
    el_textarea,
    el_select,
    el_option,
    el_label,
    el_a,
    el_img,
    el_table,
    el_thead,
    el_tbody,
    el_tr,
    el_td,
    el_th,
    el_strong,
    el_em,
    el_br,
    el_hr,
    el_pre,
    el_code,
    to_template,
    to_template_multi,
    VNodeBuilder,
    count_nodes,
    count_all_items,
    count_dynamic_text_slots,
    count_dynamic_node_slots,
    count_dynamic_attr_slots,
    count_static_attr_nodes,
)
from .tags import (
    TAG_DIV,
    TAG_SPAN,
    TAG_P,
    TAG_SECTION,
    TAG_HEADER,
    TAG_FOOTER,
    TAG_NAV,
    TAG_MAIN,
    TAG_ARTICLE,
    TAG_ASIDE,
    TAG_H1,
    TAG_H2,
    TAG_H3,
    TAG_H4,
    TAG_H5,
    TAG_H6,
    TAG_UL,
    TAG_OL,
    TAG_LI,
    TAG_BUTTON,
    TAG_INPUT,
    TAG_FORM,
    TAG_TEXTAREA,
    TAG_SELECT,
    TAG_OPTION,
    TAG_LABEL,
    TAG_A,
    TAG_IMG,
    TAG_TABLE,
    TAG_THEAD,
    TAG_TBODY,
    TAG_TR,
    TAG_TD,
    TAG_TH,
    TAG_STRONG,
    TAG_EM,
    TAG_BR,
    TAG_HR,
    TAG_PRE,
    TAG_CODE,
)
from vdom.template import TNODE_ELEMENT, TNODE_TEXT
from vdom.vnode import VNodeStore, VNODE_TEMPLATE_REF
from vdom.builder import TemplateBuilder
from memory import alloc
from signals import create_runtime, destroy_runtime, Runtime
from signals.handle import SignalString
from events.registry import (
    ACTION_SIGNAL_SET_STRING,
    ACTION_CUSTOM,
    ACTION_KEY_ENTER_CUSTOM,
)


fn test_text_node() -> Int32:
    """Test: text() creates a NODE_TEXT with correct content."""
    var n = text(String("hello"))
    if n.kind != NODE_TEXT:
        return 0
    if n.text != String("hello"):
        return 0
    if n.is_element():
        return 0
    if not n.is_text():
        return 0
    if not n.is_child():
        return 0
    if n.is_attr():
        return 0
    return 1


fn test_dyn_text_node() -> Int32:
    """Test: dyn_text() creates a NODE_DYN_TEXT with correct index."""
    var n = dyn_text(3)
    if n.kind != NODE_DYN_TEXT:
        return 0
    if n.dynamic_index != 3:
        return 0
    if not n.is_dyn_text():
        return 0
    if not n.is_child():
        return 0
    return 1


fn test_dyn_node_slot() -> Int32:
    """Test: dyn_node() creates a NODE_DYN_NODE with correct index."""
    var n = dyn_node(5)
    if n.kind != NODE_DYN_NODE:
        return 0
    if n.dynamic_index != 5:
        return 0
    if not n.is_dyn_node():
        return 0
    return 1


fn test_static_attr() -> Int32:
    """Test: attr() creates a NODE_STATIC_ATTR with name and value."""
    var n = attr(String("class"), String("container"))
    if n.kind != NODE_STATIC_ATTR:
        return 0
    if n.text != String("class"):
        return 0
    if n.attr_value != String("container"):
        return 0
    if not n.is_attr():
        return 0
    if not n.is_static_attr():
        return 0
    if n.is_child():
        return 0
    return 1


fn test_dyn_attr() -> Int32:
    """Test: dyn_attr() creates a NODE_DYN_ATTR with correct index."""
    var n = dyn_attr(2)
    if n.kind != NODE_DYN_ATTR:
        return 0
    if n.dynamic_index != 2:
        return 0
    if not n.is_dyn_attr():
        return 0
    if not n.is_attr():
        return 0
    return 1


fn test_empty_element() -> Int32:
    """Test: el_div() with no args creates an empty element."""
    var n = el_div()
    if n.kind != NODE_ELEMENT:
        return 0
    if n.tag != TAG_DIV:
        return 0
    if n.item_count() != 0:
        return 0
    if n.child_count() != 0:
        return 0
    if n.attr_count() != 0:
        return 0
    return 1


fn test_element_with_children() -> Int32:
    """Test: el_div with text children."""
    var n = el_div([text(String("hello")), text(String("world"))])
    if n.kind != NODE_ELEMENT:
        return 0
    if n.tag != TAG_DIV:
        return 0
    if n.item_count() != 2:
        return 0
    if n.child_count() != 2:
        return 0
    if n.attr_count() != 0:
        return 0
    return 1


fn test_element_with_attrs() -> Int32:
    """Test: el_div with attributes only."""
    var n = el_div(
        [
            attr(String("class"), String("box")),
            attr(String("id"), String("main")),
        ]
    )
    if n.item_count() != 2:
        return 0
    if n.child_count() != 0:
        return 0
    if n.attr_count() != 2:
        return 0
    if n.static_attr_count() != 2:
        return 0
    return 1


fn test_element_mixed() -> Int32:
    """Test: element with a mix of attrs, children, and dynamic slots."""
    var n = el_div(
        [
            attr(String("class"), String("counter")),
            dyn_attr(0),
            text(String("hello")),
            dyn_text(0),
            el_span([text(String("inner"))]),
        ]
    )
    if n.item_count() != 5:
        return 0
    if n.child_count() != 3:
        return 0
    if n.attr_count() != 2:
        return 0
    if n.static_attr_count() != 1:
        return 0
    if n.dynamic_attr_count() != 1:
        return 0
    return 1


fn test_nested_elements() -> Int32:
    """Test: deeply nested element tree."""
    var n = el_div(
        [
            el_h1([text(String("Title"))]),
            el_ul(
                [
                    el_li([text(String("A"))]),
                    el_li([text(String("B"))]),
                    el_li([text(String("C"))]),
                ]
            ),
        ]
    )
    if n.child_count() != 2:
        return 0
    # Total tree nodes: div + h1 + "Title" + ul + li*3 + "A" + "B" + "C" = 10
    if count_nodes(n) != 10:
        return 0
    return 1


fn test_counter_template() -> Int32:
    """Test: build counter template via DSL and verify structure.

    Builds the same template as CounterApp does manually:
        div > [ span > dynamic_text[0],
                button > text("+") + dynamic_attr[0],
                button > text("-") + dynamic_attr[1] ]
    Then registers it and verifies template properties match.
    """
    # Build using DSL
    var view = el_div(
        [
            el_span([dyn_text(0)]),
            el_button([text(String("+")), dyn_attr(0)]),
            el_button([text(String("-")), dyn_attr(1)]),
        ]
    )

    # Verify Node tree structure before template conversion
    # div(1) + span(1) + dyn_text(1) + button(1) + text("+")(1) + button(1) + text("-")(1) = 7
    # dyn_attr items are attrs, not children — count_nodes skips attrs.
    if count_nodes(view) != 7:
        return 0

    if count_dynamic_text_slots(view) != 1:
        return 0
    if count_dynamic_attr_slots(view) != 2:
        return 0

    # Convert to Template
    var rt_ptr = create_runtime()
    var template = to_template(view, String("dsl-counter"))
    var tmpl_id = rt_ptr[0].templates.register(template^)

    # Verify template properties
    # 1 root (the div)
    if rt_ptr[0].templates.root_count(tmpl_id) != 1:
        destroy_runtime(rt_ptr)
        return 0

    # 7 nodes total: div, span, dyn_text, btn1, text("+"), btn2, text("-")
    if rt_ptr[0].templates.node_count(tmpl_id) != 7:
        destroy_runtime(rt_ptr)
        return 0

    # Root node is an element (div)
    if rt_ptr[0].templates.node_kind(tmpl_id, 0) != TNODE_ELEMENT:
        destroy_runtime(rt_ptr)
        return 0

    # Root node tag is TAG_DIV
    if rt_ptr[0].templates.node_html_tag(tmpl_id, 0) != TAG_DIV:
        destroy_runtime(rt_ptr)
        return 0

    # Div has 3 children: span, button, button
    if rt_ptr[0].templates.node_child_count(tmpl_id, 0) != 3:
        destroy_runtime(rt_ptr)
        return 0

    # 1 dynamic text slot
    if rt_ptr[0].templates.dynamic_text_count(tmpl_id) != 1:
        destroy_runtime(rt_ptr)
        return 0

    # 2 dynamic attr slots
    if rt_ptr[0].templates.dynamic_attr_count(tmpl_id) != 2:
        destroy_runtime(rt_ptr)
        return 0

    destroy_runtime(rt_ptr)
    return 1


fn test_to_template_simple() -> Int32:
    """Test: simple div with static text converts to valid template."""
    var view = el_div([text(String("hello"))])
    var rt_ptr = create_runtime()
    var template = to_template(view, String("dsl-simple"))
    var tmpl_id = rt_ptr[0].templates.register(template^)

    # 2 nodes: div + text
    if rt_ptr[0].templates.node_count(tmpl_id) != 2:
        destroy_runtime(rt_ptr)
        return 0

    # 1 root
    if rt_ptr[0].templates.root_count(tmpl_id) != 1:
        destroy_runtime(rt_ptr)
        return 0

    # Root is element
    if rt_ptr[0].templates.node_kind(tmpl_id, 0) != TNODE_ELEMENT:
        destroy_runtime(rt_ptr)
        return 0

    # Child is text
    if rt_ptr[0].templates.node_kind(tmpl_id, 1) != TNODE_TEXT:
        destroy_runtime(rt_ptr)
        return 0

    destroy_runtime(rt_ptr)
    return 1


fn test_to_template_attrs() -> Int32:
    """Test: element with static and dynamic attrs converts correctly."""
    var view = el_div(
        [
            attr(String("class"), String("box")),
            dyn_attr(0),
            text(String("content")),
        ]
    )
    var rt_ptr = create_runtime()
    var template = to_template(view, String("dsl-attrs"))
    var tmpl_id = rt_ptr[0].templates.register(template^)

    # 2 nodes: div + text("content")
    if rt_ptr[0].templates.node_count(tmpl_id) != 2:
        destroy_runtime(rt_ptr)
        return 0

    # 1 static attr + 1 dynamic attr = 2 total attrs
    if rt_ptr[0].templates.attr_total_count(tmpl_id) != 2:
        destroy_runtime(rt_ptr)
        return 0

    if rt_ptr[0].templates.static_attr_count(tmpl_id) != 1:
        destroy_runtime(rt_ptr)
        return 0

    if rt_ptr[0].templates.dynamic_attr_count(tmpl_id) != 1:
        destroy_runtime(rt_ptr)
        return 0

    destroy_runtime(rt_ptr)
    return 1


fn test_to_template_multi_root() -> Int32:
    """Test: multiple root nodes via to_template_multi."""
    var roots: List[Node] = [
        el_h1([text(String("Title"))]),
        el_p([text(String("Body"))]),
    ]
    var rt_ptr = create_runtime()
    var template = to_template_multi(roots, String("dsl-multi"))
    var tmpl_id = rt_ptr[0].templates.register(template^)

    # 2 roots
    if rt_ptr[0].templates.root_count(tmpl_id) != 2:
        destroy_runtime(rt_ptr)
        return 0

    # 4 nodes: h1 + "Title" + p + "Body"
    if rt_ptr[0].templates.node_count(tmpl_id) != 4:
        destroy_runtime(rt_ptr)
        return 0

    destroy_runtime(rt_ptr)
    return 1


fn test_vnode_builder() -> Int32:
    """Test: VNodeBuilder creates a VNode with correct dynamic content."""
    var rt_ptr = create_runtime()
    var store_ptr = alloc[VNodeStore](1)
    store_ptr.init_pointee_move(VNodeStore())

    # Register a template (we just need an ID)
    var view = el_div([dyn_text(0), dyn_attr(0), dyn_attr(1)])
    var template = to_template(view, String("dsl-vb-test"))
    var tmpl_id = rt_ptr[0].templates.register(template^)

    # Build VNode using VNodeBuilder
    var vb = VNodeBuilder(tmpl_id, store_ptr)
    vb.add_dyn_text(String("Count: 42"))
    vb.add_dyn_event(String("click"), UInt32(10))
    vb.add_dyn_text_attr(String("class"), String("active"))
    var idx = vb.index()

    # Verify VNode
    if store_ptr[0].kind(idx) != VNODE_TEMPLATE_REF:
        store_ptr.destroy_pointee()
        store_ptr.free()
        destroy_runtime(rt_ptr)
        return 0

    if store_ptr[0].template_id(idx) != tmpl_id:
        store_ptr.destroy_pointee()
        store_ptr.free()
        destroy_runtime(rt_ptr)
        return 0

    # 1 dynamic text node
    if store_ptr[0].dynamic_node_count(idx) != 1:
        store_ptr.destroy_pointee()
        store_ptr.free()
        destroy_runtime(rt_ptr)
        return 0

    # 2 dynamic attrs (event + text attr)
    if store_ptr[0].dynamic_attr_count(idx) != 2:
        store_ptr.destroy_pointee()
        store_ptr.free()
        destroy_runtime(rt_ptr)
        return 0

    store_ptr.destroy_pointee()
    store_ptr.free()
    destroy_runtime(rt_ptr)
    return 1


fn test_vnode_builder_keyed() -> Int32:
    """Test: keyed VNodeBuilder creates a keyed VNode."""
    var rt_ptr = create_runtime()
    var store_ptr = alloc[VNodeStore](1)
    store_ptr.init_pointee_move(VNodeStore())

    var view = el_div([text(String("item"))])
    var template = to_template(view, String("dsl-keyed"))
    var tmpl_id = rt_ptr[0].templates.register(template^)

    var vb = VNodeBuilder(tmpl_id, String("item-42"), store_ptr)
    var idx = vb.index()

    if not store_ptr[0].has_key(idx):
        store_ptr.destroy_pointee()
        store_ptr.free()
        destroy_runtime(rt_ptr)
        return 0

    store_ptr.destroy_pointee()
    store_ptr.free()
    destroy_runtime(rt_ptr)
    return 1


fn test_all_tag_helpers() -> Int32:
    """Test: every tag helper produces the correct tag constant."""
    # Layout / Sectioning
    if el_div().tag != TAG_DIV:
        return 0
    if el_span().tag != TAG_SPAN:
        return 0
    if el_p().tag != TAG_P:
        return 0
    if el_section().tag != TAG_SECTION:
        return 0
    if el_header().tag != TAG_HEADER:
        return 0
    if el_footer().tag != TAG_FOOTER:
        return 0
    if el_nav().tag != TAG_NAV:
        return 0
    if el_main().tag != TAG_MAIN:
        return 0
    if el_article().tag != TAG_ARTICLE:
        return 0
    if el_aside().tag != TAG_ASIDE:
        return 0
    # Headings
    if el_h1().tag != TAG_H1:
        return 0
    if el_h2().tag != TAG_H2:
        return 0
    if el_h3().tag != TAG_H3:
        return 0
    if el_h4().tag != TAG_H4:
        return 0
    if el_h5().tag != TAG_H5:
        return 0
    if el_h6().tag != TAG_H6:
        return 0
    # Lists
    if el_ul().tag != TAG_UL:
        return 0
    if el_ol().tag != TAG_OL:
        return 0
    if el_li().tag != TAG_LI:
        return 0
    # Interactive
    if el_button().tag != TAG_BUTTON:
        return 0
    if el_input().tag != TAG_INPUT:
        return 0
    if el_form().tag != TAG_FORM:
        return 0
    if el_textarea().tag != TAG_TEXTAREA:
        return 0
    if el_select().tag != TAG_SELECT:
        return 0
    if el_option().tag != TAG_OPTION:
        return 0
    if el_label().tag != TAG_LABEL:
        return 0
    # Links / Media
    if el_a().tag != TAG_A:
        return 0
    if el_img().tag != TAG_IMG:
        return 0
    # Table
    if el_table().tag != TAG_TABLE:
        return 0
    if el_thead().tag != TAG_THEAD:
        return 0
    if el_tbody().tag != TAG_TBODY:
        return 0
    if el_tr().tag != TAG_TR:
        return 0
    if el_td().tag != TAG_TD:
        return 0
    if el_th().tag != TAG_TH:
        return 0
    # Inline / Formatting
    if el_strong().tag != TAG_STRONG:
        return 0
    if el_em().tag != TAG_EM:
        return 0
    if el_br().tag != TAG_BR:
        return 0
    if el_hr().tag != TAG_HR:
        return 0
    if el_pre().tag != TAG_PRE:
        return 0
    if el_code().tag != TAG_CODE:
        return 0
    return 1


fn test_count_utilities() -> Int32:
    """Test: count_* utility functions on a non-trivial tree."""
    var tree = el_div(
        [
            attr(String("class"), String("app")),
            dyn_attr(0),
            el_h1([dyn_text(0)]),
            el_ul(
                [
                    el_li([text(String("A")), dyn_attr(1)]),
                    el_li([dyn_text(1), dyn_node(0)]),
                ]
            ),
        ]
    )

    # Tree nodes (excluding attrs): div + h1 + dyn_text(0) + ul + li + "A" + li + dyn_text(1) + dyn_node(0) = 9
    if count_nodes(tree) != 9:
        return 0

    # DYN_TEXT slots: 2 (index 0 inside h1, index 1 inside second li)
    if count_dynamic_text_slots(tree) != 2:
        return 0

    # DYN_NODE slots: 1 (index 0 inside second li)
    if count_dynamic_node_slots(tree) != 1:
        return 0

    # DYN_ATTR slots: 2 (index 0 on div, index 1 on first li)
    if count_dynamic_attr_slots(tree) != 2:
        return 0

    # STATIC_ATTR: 1 (class on div)
    if count_static_attr_nodes(tree) != 1:
        return 0

    return 1


fn test_template_equivalence() -> Int32:
    """Test: DSL-built template matches manually-built template.

    Builds the counter template both ways and verifies they have
    identical structure (node counts, kinds, tags, child counts,
    dynamic slot counts, attribute counts).
    """
    # ── Method 1: Manual builder (same as CounterApp) ────────────────
    var rt1 = create_runtime()
    var b = TemplateBuilder(String("manual-counter"))
    var div_idx = b.push_element(TAG_DIV, -1)
    var span_idx = b.push_element(TAG_SPAN, Int(div_idx))
    _ = b.push_dynamic_text(0, Int(span_idx))
    var btn1_idx = b.push_element(TAG_BUTTON, Int(div_idx))
    _ = b.push_text(String("+"), Int(btn1_idx))
    b.push_dynamic_attr(Int(btn1_idx), 0)
    var btn2_idx = b.push_element(TAG_BUTTON, Int(div_idx))
    _ = b.push_text(String("-"), Int(btn2_idx))
    b.push_dynamic_attr(Int(btn2_idx), 1)
    var manual_tmpl = b.build()
    var m_id = rt1[0].templates.register(manual_tmpl^)

    # ── Method 2: DSL builder ────────────────────────────────────────
    var rt2 = create_runtime()
    var view = el_div(
        [
            el_span([dyn_text(0)]),
            el_button([text(String("+")), dyn_attr(0)]),
            el_button([text(String("-")), dyn_attr(1)]),
        ]
    )
    var dsl_tmpl = to_template(view, String("dsl-counter"))
    var d_id = rt2[0].templates.register(dsl_tmpl^)

    # ── Compare ──────────────────────────────────────────────────────

    # Node counts must match
    if rt1[0].templates.node_count(m_id) != rt2[0].templates.node_count(d_id):
        destroy_runtime(rt1)
        destroy_runtime(rt2)
        return 0

    # Root counts must match
    if rt1[0].templates.root_count(m_id) != rt2[0].templates.root_count(d_id):
        destroy_runtime(rt1)
        destroy_runtime(rt2)
        return 0

    # Dynamic text slot counts must match
    if rt1[0].templates.dynamic_text_count(m_id) != rt2[
        0
    ].templates.dynamic_text_count(d_id):
        destroy_runtime(rt1)
        destroy_runtime(rt2)
        return 0

    # Dynamic attr slot counts must match
    if rt1[0].templates.dynamic_attr_count(m_id) != rt2[
        0
    ].templates.dynamic_attr_count(d_id):
        destroy_runtime(rt1)
        destroy_runtime(rt2)
        return 0

    # Attr total counts must match
    if rt1[0].templates.attr_total_count(m_id) != rt2[
        0
    ].templates.attr_total_count(d_id):
        destroy_runtime(rt1)
        destroy_runtime(rt2)
        return 0

    # Compare each node kind and tag
    var node_count = rt1[0].templates.node_count(m_id)
    for i in range(node_count):
        if rt1[0].templates.node_kind(m_id, i) != rt2[0].templates.node_kind(
            d_id, i
        ):
            destroy_runtime(rt1)
            destroy_runtime(rt2)
            return 0
        if rt1[0].templates.node_html_tag(m_id, i) != rt2[
            0
        ].templates.node_html_tag(d_id, i):
            destroy_runtime(rt1)
            destroy_runtime(rt2)
            return 0
        if rt1[0].templates.node_child_count(m_id, i) != rt2[
            0
        ].templates.node_child_count(d_id, i):
            destroy_runtime(rt1)
            destroy_runtime(rt2)
            return 0

    destroy_runtime(rt1)
    destroy_runtime(rt2)
    return 1


# ══════════════════════════════════════════════════════════════════════════════
# Phase 20 — M20.5: onclick_custom tests
# ══════════════════════════════════════════════════════════════════════════════


fn test_onclick_custom_node() -> Int32:
    """Test: onclick_custom creates a NODE_EVENT with ACTION_CUSTOM."""
    var n = onclick_custom()

    # Verify node kind
    if n.kind != NODE_EVENT:
        return 0

    # Verify event name
    if n.text != String("click"):
        return 0

    # Verify action tag = ACTION_CUSTOM (255)
    if n.tag != ACTION_CUSTOM:
        return 0

    # Verify signal_key = 0 (no signal)
    if n.dynamic_index != 0:
        return 0

    # Verify operand = 0
    if n.operand != 0:
        return 0

    return 1


fn test_onclick_custom_in_element() -> Int32:
    """Test: onclick_custom inside an element counts as a dynamic attr."""
    var n = el_button(
        text(String("Add")),
        onclick_custom(),
    )

    # Should have 2 items: text + event
    if n.item_count() != 2:
        return 0

    # 1 child (text)
    if n.child_count() != 1:
        return 0

    # 1 dynamic attr (the event handler)
    if n.dynamic_attr_count() != 1:
        return 0

    return 1


fn test_onclick_custom_with_binding() -> Int32:
    """Test: onclick_custom + bind_value + oninput_set_string in sibling elements.
    """
    var rt_ptr = create_runtime()
    var keys = rt_ptr[0].create_signal_string(String(""))
    var sig = SignalString(keys[0], keys[1], rt_ptr)

    # Simulates the M20.5 TodoApp pattern:
    #   el_div(
    #       el_input(bind_value(sig), oninput_set_string(sig)),
    #       el_button(text("Add"), onclick_custom()),
    #   )
    var n = el_div(
        el_input(
            attr(String("type"), String("text")),
            bind_value(sig),
            oninput_set_string(sig),
        ),
        el_button(text(String("Add")), onclick_custom()),
    )

    # div has 2 children: input + button
    if n.child_count() != 2:
        destroy_runtime(rt_ptr)
        return 0

    # input has 3 attrs: static type + bind_value + oninput
    # 2 dynamic attrs (bind_value + event)
    if n.items[0].dynamic_attr_count() != 2:
        destroy_runtime(rt_ptr)
        return 0

    # button has 1 child (text) + 1 dynamic attr (onclick_custom)
    if n.items[1].child_count() != 1:
        destroy_runtime(rt_ptr)
        return 0
    if n.items[1].dynamic_attr_count() != 1:
        destroy_runtime(rt_ptr)
        return 0

    destroy_runtime(rt_ptr)
    return 1


# ══════════════════════════════════════════════════════════════════════════════
# Phase 20 — M20.3: oninput_set_string / onchange_set_string tests
# ══════════════════════════════════════════════════════════════════════════════


fn test_oninput_set_string_node() -> Int32:
    """Test: oninput_set_string creates a NODE_EVENT with correct fields."""
    var rt_ptr = create_runtime()
    var keys = rt_ptr[0].create_signal_string(String("hello"))
    var sig = SignalString(keys[0], keys[1], rt_ptr)

    var n = oninput_set_string(sig)

    # Verify node kind
    if n.kind != NODE_EVENT:
        destroy_runtime(rt_ptr)
        return 0
    # Verify event name
    if n.text != String("input"):
        destroy_runtime(rt_ptr)
        return 0
    # Verify action tag is ACTION_SIGNAL_SET_STRING (6)
    if n.tag != ACTION_SIGNAL_SET_STRING:
        destroy_runtime(rt_ptr)
        return 0
    # Verify signal_key = string_key
    if n.dynamic_index != keys[0]:
        destroy_runtime(rt_ptr)
        return 0
    # Verify operand = Int32(version_key)
    if n.operand != Int32(keys[1]):
        destroy_runtime(rt_ptr)
        return 0
    # Verify it counts as an event node
    if not n.is_event():
        destroy_runtime(rt_ptr)
        return 0
    # Verify it counts as an attr
    if not n.is_attr():
        destroy_runtime(rt_ptr)
        return 0

    destroy_runtime(rt_ptr)
    return 1


fn test_onchange_set_string_node() -> Int32:
    """Test: onchange_set_string creates a NODE_EVENT with correct fields."""
    var rt_ptr = create_runtime()
    var keys = rt_ptr[0].create_signal_string(String("world"))
    var sig = SignalString(keys[0], keys[1], rt_ptr)

    var n = onchange_set_string(sig)

    # Verify node kind
    if n.kind != NODE_EVENT:
        destroy_runtime(rt_ptr)
        return 0
    # Verify event name is "change" (not "input")
    if n.text != String("change"):
        destroy_runtime(rt_ptr)
        return 0
    # Verify action tag
    if n.tag != ACTION_SIGNAL_SET_STRING:
        destroy_runtime(rt_ptr)
        return 0
    # Verify signal_key = string_key
    if n.dynamic_index != keys[0]:
        destroy_runtime(rt_ptr)
        return 0
    # Verify operand = Int32(version_key)
    if n.operand != Int32(keys[1]):
        destroy_runtime(rt_ptr)
        return 0

    destroy_runtime(rt_ptr)
    return 1


fn test_oninput_in_element() -> Int32:
    """Test: oninput_set_string inside an element counts as a dynamic attr."""
    var rt_ptr = create_runtime()
    var keys = rt_ptr[0].create_signal_string(String(""))
    var sig = SignalString(keys[0], keys[1], rt_ptr)

    var n = el_input(
        attr(String("type"), String("text")),
        oninput_set_string(sig),
    )

    # 2 items: 1 static attr + 1 event
    if n.item_count() != 2:
        destroy_runtime(rt_ptr)
        return 0
    if n.static_attr_count() != 1:
        destroy_runtime(rt_ptr)
        return 0
    if n.event_count() != 1:
        destroy_runtime(rt_ptr)
        return 0
    if n.dynamic_attr_count() != 1:
        destroy_runtime(rt_ptr)
        return 0

    destroy_runtime(rt_ptr)
    return 1


# ══════════════════════════════════════════════════════════════════════════════
# Phase 20 — M20.4: bind_value / bind_attr tests
# ══════════════════════════════════════════════════════════════════════════════


fn test_bind_value_node() -> Int32:
    """Test: bind_value creates a NODE_BIND_VALUE with attr_name='value'."""
    var rt_ptr = create_runtime()
    var keys = rt_ptr[0].create_signal_string(String("initial"))
    var sig = SignalString(keys[0], keys[1], rt_ptr)

    var n = bind_value(sig)

    # Verify node kind
    if n.kind != NODE_BIND_VALUE:
        destroy_runtime(rt_ptr)
        return 0
    # Verify attr_name is "value"
    if n.text != String("value"):
        destroy_runtime(rt_ptr)
        return 0
    # Verify string_key stored in dynamic_index
    if n.dynamic_index != keys[0]:
        destroy_runtime(rt_ptr)
        return 0
    # Verify version_key stored in operand
    if n.operand != Int32(keys[1]):
        destroy_runtime(rt_ptr)
        return 0
    # Verify it counts as an attr
    if not n.is_attr():
        destroy_runtime(rt_ptr)
        return 0
    # Verify is_bind_value
    if not n.is_bind_value():
        destroy_runtime(rt_ptr)
        return 0
    # Verify it's NOT a child
    if n.is_child():
        destroy_runtime(rt_ptr)
        return 0

    destroy_runtime(rt_ptr)
    return 1


fn test_bind_attr_node() -> Int32:
    """Test: bind_attr creates a NODE_BIND_VALUE with custom attr name."""
    var rt_ptr = create_runtime()
    var keys = rt_ptr[0].create_signal_string(String("hint"))
    var sig = SignalString(keys[0], keys[1], rt_ptr)

    var n = bind_attr(String("placeholder"), sig)

    # Verify node kind
    if n.kind != NODE_BIND_VALUE:
        destroy_runtime(rt_ptr)
        return 0
    # Verify attr_name is "placeholder"
    if n.text != String("placeholder"):
        destroy_runtime(rt_ptr)
        return 0
    # Verify string_key
    if n.dynamic_index != keys[0]:
        destroy_runtime(rt_ptr)
        return 0
    # Verify version_key
    if n.operand != Int32(keys[1]):
        destroy_runtime(rt_ptr)
        return 0

    destroy_runtime(rt_ptr)
    return 1


fn test_bind_value_in_element() -> Int32:
    """Test: bind_value inside an element counts as a dynamic attr."""
    var rt_ptr = create_runtime()
    var keys = rt_ptr[0].create_signal_string(String("text"))
    var sig = SignalString(keys[0], keys[1], rt_ptr)

    var n = el_input(
        attr(String("type"), String("text")),
        bind_value(sig),
    )

    if n.item_count() != 2:
        destroy_runtime(rt_ptr)
        return 0
    if n.static_attr_count() != 1:
        destroy_runtime(rt_ptr)
        return 0
    if n.bind_value_count() != 1:
        destroy_runtime(rt_ptr)
        return 0
    # bind_value counts as a dynamic attr
    if n.dynamic_attr_count() != 1:
        destroy_runtime(rt_ptr)
        return 0

    destroy_runtime(rt_ptr)
    return 1


fn test_two_way_binding_element() -> Int32:
    """Test: bind_value + oninput_set_string together in an element."""
    var rt_ptr = create_runtime()
    var keys = rt_ptr[0].create_signal_string(String(""))
    var sig = SignalString(keys[0], keys[1], rt_ptr)

    var n = el_input(
        attr(String("type"), String("text")),
        bind_value(sig),
        oninput_set_string(sig),
    )

    # 3 items: 1 static attr + 1 bind_value + 1 event
    if n.item_count() != 3:
        destroy_runtime(rt_ptr)
        return 0
    if n.static_attr_count() != 1:
        destroy_runtime(rt_ptr)
        return 0
    if n.bind_value_count() != 1:
        destroy_runtime(rt_ptr)
        return 0
    if n.event_count() != 1:
        destroy_runtime(rt_ptr)
        return 0
    # Both bind_value and event count as dynamic attrs
    if n.dynamic_attr_count() != 2:
        destroy_runtime(rt_ptr)
        return 0
    # count_dynamic_attr_slots on the tree should also be 2
    if count_dynamic_attr_slots(n) != 2:
        destroy_runtime(rt_ptr)
        return 0

    destroy_runtime(rt_ptr)
    return 1


fn test_bind_value_to_template() -> Int32:
    """Test: bind_value converts to a TATTR_DYNAMIC in the template."""
    var rt_ptr = create_runtime()
    var keys = rt_ptr[0].create_signal_string(String("test"))
    var sig = SignalString(keys[0], keys[1], rt_ptr)

    # Build a template with bind_value
    # Note: bind_value gets a dynamic_index from the node, but
    # to_template treats it like a dyn_attr.
    var view = el_input(
        attr(String("type"), String("text")),
        bind_value(sig),
    )
    var template = to_template(view, String("bind-test"))
    var tmpl_id = rt_ptr[0].templates.register(template^)

    # Template should have: input element with 1 static attr + 1 dynamic attr
    if rt_ptr[0].templates.node_count(tmpl_id) != 1:
        destroy_runtime(rt_ptr)
        return 0
    if rt_ptr[0].templates.dynamic_attr_count(tmpl_id) != 1:
        destroy_runtime(rt_ptr)
        return 0

    destroy_runtime(rt_ptr)
    return 1


fn test_two_way_to_template() -> Int32:
    """Test: bind_value + oninput_set_string converts to 2 TATTR_DYNAMICs."""
    var rt_ptr = create_runtime()
    var keys = rt_ptr[0].create_signal_string(String(""))
    var sig = SignalString(keys[0], keys[1], rt_ptr)

    var view = el_input(
        attr(String("type"), String("text")),
        bind_value(sig),
        oninput_set_string(sig),
    )
    var template = to_template(view, String("two-way-test"))
    var tmpl_id = rt_ptr[0].templates.register(template^)

    # 1 node (input), 2 dynamic attrs (bind_value + event)
    if rt_ptr[0].templates.node_count(tmpl_id) != 1:
        destroy_runtime(rt_ptr)
        return 0
    if rt_ptr[0].templates.dynamic_attr_count(tmpl_id) != 2:
        destroy_runtime(rt_ptr)
        return 0

    destroy_runtime(rt_ptr)
    return 1


# ══════════════════════════════════════════════════════════════════════════════
# Phase 22: onkeydown_enter_custom tests
# ══════════════════════════════════════════════════════════════════════════════


fn test_onkeydown_enter_custom_node() -> Int32:
    """Test: onkeydown_enter_custom creates a NODE_EVENT with ACTION_KEY_ENTER_CUSTOM.
    """
    var n = onkeydown_enter_custom()

    # Verify node kind
    if n.kind != NODE_EVENT:
        return 0

    # Verify event name
    if n.text != String("keydown"):
        return 0

    # Verify action tag
    if n.tag != ACTION_KEY_ENTER_CUSTOM:
        return 0

    # Verify signal_key and operand are 0 (no signal binding)
    if n.dynamic_index != 0:
        return 0
    if n.operand != 0:
        return 0

    return 1


fn test_onkeydown_enter_custom_in_element() -> Int32:
    """Test: onkeydown_enter_custom inside an input counts as a dynamic attr."""
    var n = el_input(
        attr(String("type"), String("text")),
        onkeydown_enter_custom(),
    )

    # Should have 2 items: static attr + event
    if n.item_count() != 2:
        return 0

    # 1 dynamic attr (the keydown event)
    if n.dynamic_attr_count() != 1:
        return 0

    return 1


fn test_onkeydown_enter_custom_with_binding() -> Int32:
    """Test: onkeydown_enter_custom + bind_value + oninput + onclick_custom (Phase 22 TodoApp pattern).
    """
    var rt_ptr = create_runtime()
    var keys = rt_ptr[0].create_signal_string(String(""))
    var sig = SignalString(keys[0], keys[1], rt_ptr)

    # Simulates the Phase 22 TodoApp pattern:
    #   el_div(
    #       el_input(bind_value(sig), oninput_set_string(sig), onkeydown_enter_custom()),
    #       el_button(text("Add"), onclick_custom()),
    #   )
    var n = el_div(
        el_input(
            attr(String("type"), String("text")),
            bind_value(sig),
            oninput_set_string(sig),
            onkeydown_enter_custom(),
        ),
        el_button(text(String("Add")), onclick_custom()),
    )

    # div has 2 children: input + button
    if n.child_count() != 2:
        destroy_runtime(rt_ptr)
        return 0

    # input has 4 attrs: static type + bind_value + oninput + onkeydown_enter
    # 3 dynamic attrs (bind_value + oninput event + keydown event)
    if n.items[0].dynamic_attr_count() != 3:
        destroy_runtime(rt_ptr)
        return 0

    # button has 1 child (text) + 1 dynamic attr (onclick_custom)
    if n.items[1].child_count() != 1:
        destroy_runtime(rt_ptr)
        return 0
    if n.items[1].dynamic_attr_count() != 1:
        destroy_runtime(rt_ptr)
        return 0

    destroy_runtime(rt_ptr)
    return 1
