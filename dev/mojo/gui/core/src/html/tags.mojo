# HTML Tag Constants — Numeric identifiers for common HTML element tags.
#
# Using UInt8 constants instead of string comparisons for efficient
# tag identification in the template and builder systems.
#
# These constants are used by TemplateNode.html_tag to identify which
# HTML element a template node represents.  The JS interpreter maps
# these back to actual tag name strings when creating DOM elements.

# ── Layout / Sectioning ─────────────────────────────────────────────────────

comptime TAG_DIV: UInt8 = 0
comptime TAG_SPAN: UInt8 = 1
comptime TAG_P: UInt8 = 2
comptime TAG_SECTION: UInt8 = 3
comptime TAG_HEADER: UInt8 = 4
comptime TAG_FOOTER: UInt8 = 5
comptime TAG_NAV: UInt8 = 6
comptime TAG_MAIN: UInt8 = 7
comptime TAG_ARTICLE: UInt8 = 8
comptime TAG_ASIDE: UInt8 = 9

# ── Headings ─────────────────────────────────────────────────────────────────

comptime TAG_H1: UInt8 = 10
comptime TAG_H2: UInt8 = 11
comptime TAG_H3: UInt8 = 12
comptime TAG_H4: UInt8 = 13
comptime TAG_H5: UInt8 = 14
comptime TAG_H6: UInt8 = 15

# ── Lists ────────────────────────────────────────────────────────────────────

comptime TAG_UL: UInt8 = 16
comptime TAG_OL: UInt8 = 17
comptime TAG_LI: UInt8 = 18

# ── Interactive ──────────────────────────────────────────────────────────────

comptime TAG_BUTTON: UInt8 = 19
comptime TAG_INPUT: UInt8 = 20
comptime TAG_FORM: UInt8 = 21
comptime TAG_TEXTAREA: UInt8 = 22
comptime TAG_SELECT: UInt8 = 23
comptime TAG_OPTION: UInt8 = 24
comptime TAG_LABEL: UInt8 = 25

# ── Links / Media ───────────────────────────────────────────────────────────

comptime TAG_A: UInt8 = 26
comptime TAG_IMG: UInt8 = 27

# ── Table ────────────────────────────────────────────────────────────────────

comptime TAG_TABLE: UInt8 = 28
comptime TAG_THEAD: UInt8 = 29
comptime TAG_TBODY: UInt8 = 30
comptime TAG_TR: UInt8 = 31
comptime TAG_TD: UInt8 = 32
comptime TAG_TH: UInt8 = 33

# ── Inline ───────────────────────────────────────────────────────────────────

comptime TAG_STRONG: UInt8 = 34
comptime TAG_EM: UInt8 = 35
comptime TAG_BR: UInt8 = 36
comptime TAG_HR: UInt8 = 37
comptime TAG_PRE: UInt8 = 38
comptime TAG_CODE: UInt8 = 39

# ── Sentinel ─────────────────────────────────────────────────────────────────

comptime TAG_UNKNOWN: UInt8 = 255
comptime TAG_COUNT: Int = 40  # total number of known tags


# ── Tag name lookup ──────────────────────────────────────────────────────────


fn tag_name(tag: UInt8) -> String:
    """Return the HTML tag name string for a given tag constant.

    Used by the JS interpreter bridge to create the correct DOM elements.
    Returns "unknown" for unrecognised tag IDs.
    """
    if tag == TAG_DIV:
        return "div"
    if tag == TAG_SPAN:
        return "span"
    if tag == TAG_P:
        return "p"
    if tag == TAG_SECTION:
        return "section"
    if tag == TAG_HEADER:
        return "header"
    if tag == TAG_FOOTER:
        return "footer"
    if tag == TAG_NAV:
        return "nav"
    if tag == TAG_MAIN:
        return "main"
    if tag == TAG_ARTICLE:
        return "article"
    if tag == TAG_ASIDE:
        return "aside"
    if tag == TAG_H1:
        return "h1"
    if tag == TAG_H2:
        return "h2"
    if tag == TAG_H3:
        return "h3"
    if tag == TAG_H4:
        return "h4"
    if tag == TAG_H5:
        return "h5"
    if tag == TAG_H6:
        return "h6"
    if tag == TAG_UL:
        return "ul"
    if tag == TAG_OL:
        return "ol"
    if tag == TAG_LI:
        return "li"
    if tag == TAG_BUTTON:
        return "button"
    if tag == TAG_INPUT:
        return "input"
    if tag == TAG_FORM:
        return "form"
    if tag == TAG_TEXTAREA:
        return "textarea"
    if tag == TAG_SELECT:
        return "select"
    if tag == TAG_OPTION:
        return "option"
    if tag == TAG_LABEL:
        return "label"
    if tag == TAG_A:
        return "a"
    if tag == TAG_IMG:
        return "img"
    if tag == TAG_TABLE:
        return "table"
    if tag == TAG_THEAD:
        return "thead"
    if tag == TAG_TBODY:
        return "tbody"
    if tag == TAG_TR:
        return "tr"
    if tag == TAG_TD:
        return "td"
    if tag == TAG_TH:
        return "th"
    if tag == TAG_STRONG:
        return "strong"
    if tag == TAG_EM:
        return "em"
    if tag == TAG_BR:
        return "br"
    if tag == TAG_HR:
        return "hr"
    if tag == TAG_PRE:
        return "pre"
    if tag == TAG_CODE:
        return "code"
    return "unknown"
