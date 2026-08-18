# Virtual DOM package — re-exports template, vnode, builder, registry.
#
# Note: tags, dsl, and dsl_tests have moved to the `html` package.
# Use `from html import ...` for HTML tag constants and DSL helpers.

from .template import (
    Template,
    TemplateNode,
    TemplateAttribute,
    TNODE_ELEMENT,
    TNODE_TEXT,
    TNODE_DYNAMIC,
    TNODE_DYNAMIC_TEXT,
    TATTR_STATIC,
    TATTR_DYNAMIC,
)
from .registry import TemplateRegistry
from .builder import TemplateBuilder, create_builder, destroy_builder
from .vnode import (
    VNode,
    VNodeStore,
    DynamicNode,
    DynamicAttr,
    AttributeValue,
    VNODE_TEMPLATE_REF,
    VNODE_TEXT,
    VNODE_PLACEHOLDER,
    VNODE_FRAGMENT,
    AVAL_TEXT,
    AVAL_INT,
    AVAL_FLOAT,
    AVAL_BOOL,
    AVAL_EVENT,
    AVAL_NONE,
    DNODE_TEXT,
    DNODE_PLACEHOLDER,
)
