# HTML package — re-exports tag constants, DSL helpers, and DSL tests.
#
# These modules were moved from the `vdom` package during the Phase 1
# separation.  The `vdom` package now contains only renderer-agnostic
# virtual DOM primitives (template, vnode, builder, registry).
#
# Usage:
#     from html import el_div, el_button, text, dyn_text, ...
#     from html import TAG_DIV, TAG_SPAN, ...

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
    TAG_UNKNOWN,
    TAG_COUNT,
    tag_name,
)
from .dsl import (
    # Node type and kind tags
    Node,
    NODE_TEXT,
    NODE_ELEMENT,
    NODE_DYN_TEXT,
    NODE_DYN_NODE,
    NODE_STATIC_ATTR,
    NODE_DYN_ATTR,
    NODE_EVENT,
    NODE_BIND_VALUE,
    # Auto-numbering sentinel
    DYN_TEXT_AUTO,
    # Leaf constructors
    text,
    dyn_text,
    dyn_node,
    attr,
    dyn_attr,
    # Inline event handler constructors
    onclick_add,
    onclick_sub,
    onclick_set,
    onclick_toggle,
    onclick_custom,
    on_event,
    # Inline string event handler constructors (Phase 20 — M20.3)
    oninput_set_string,
    onchange_set_string,
    # Inline keydown handler (Phase 22)
    onkeydown_enter_custom,
    # Value binding constructors (Phase 20 — M20.4)
    bind_value,
    bind_attr,
    # Conditional helpers
    class_if,
    class_when,
    text_when,
    attr_if,
    attr_when,
    # Generic element constructors
    el,
    el_empty,
    # Tag helpers — Layout / Sectioning
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
    # Tag helpers — Headings
    el_h1,
    el_h2,
    el_h3,
    el_h4,
    el_h5,
    el_h6,
    # Tag helpers — Lists
    el_ul,
    el_ol,
    el_li,
    # Tag helpers — Interactive
    el_button,
    el_input,
    el_form,
    el_textarea,
    el_select,
    el_option,
    el_label,
    # Tag helpers — Links / Media
    el_a,
    el_img,
    # Tag helpers — Table
    el_table,
    el_thead,
    el_tbody,
    el_tr,
    el_td,
    el_th,
    # Tag helpers — Inline / Formatting
    el_strong,
    el_em,
    el_br,
    el_hr,
    el_pre,
    el_code,
    # Template conversion
    to_template,
    to_template_multi,
    # VNodeBuilder
    VNodeBuilder,
    # Utility helpers
    count_nodes,
    count_all_items,
    count_dynamic_text_slots,
    count_dynamic_node_slots,
    count_dynamic_attr_slots,
    count_static_attr_nodes,
)
