// HTML Tag Name Lookup — JS side
//
// Maps numeric tag IDs (emitted by the Mojo template system) to their
// corresponding HTML tag name strings.  Must stay in sync with
// src/vdom/tags.mojo.

// ── Tag constants (must match src/vdom/tags.mojo) ───────────────────────────

export const Tag = {
	// Layout / Sectioning
	DIV: 0,
	SPAN: 1,
	P: 2,
	SECTION: 3,
	HEADER: 4,
	FOOTER: 5,
	NAV: 6,
	MAIN: 7,
	ARTICLE: 8,
	ASIDE: 9,

	// Headings
	H1: 10,
	H2: 11,
	H3: 12,
	H4: 13,
	H5: 14,
	H6: 15,

	// Lists
	UL: 16,
	OL: 17,
	LI: 18,

	// Interactive
	BUTTON: 19,
	INPUT: 20,
	FORM: 21,
	TEXTAREA: 22,
	SELECT: 23,
	OPTION: 24,
	LABEL: 25,

	// Links / Media
	A: 26,
	IMG: 27,

	// Table
	TABLE: 28,
	THEAD: 29,
	TBODY: 30,
	TR: 31,
	TD: 32,
	TH: 33,

	// Inline
	STRONG: 34,
	EM: 35,
	BR: 36,
	HR: 37,
	PRE: 38,
	CODE: 39,

	// Sentinel
	UNKNOWN: 255,
} as const;

export type TagId = (typeof Tag)[keyof typeof Tag];

// ── Tag name lookup table ───────────────────────────────────────────────────

const TAG_NAMES: Record<number, string> = {
	[Tag.DIV]: "div",
	[Tag.SPAN]: "span",
	[Tag.P]: "p",
	[Tag.SECTION]: "section",
	[Tag.HEADER]: "header",
	[Tag.FOOTER]: "footer",
	[Tag.NAV]: "nav",
	[Tag.MAIN]: "main",
	[Tag.ARTICLE]: "article",
	[Tag.ASIDE]: "aside",
	[Tag.H1]: "h1",
	[Tag.H2]: "h2",
	[Tag.H3]: "h3",
	[Tag.H4]: "h4",
	[Tag.H5]: "h5",
	[Tag.H6]: "h6",
	[Tag.UL]: "ul",
	[Tag.OL]: "ol",
	[Tag.LI]: "li",
	[Tag.BUTTON]: "button",
	[Tag.INPUT]: "input",
	[Tag.FORM]: "form",
	[Tag.TEXTAREA]: "textarea",
	[Tag.SELECT]: "select",
	[Tag.OPTION]: "option",
	[Tag.LABEL]: "label",
	[Tag.A]: "a",
	[Tag.IMG]: "img",
	[Tag.TABLE]: "table",
	[Tag.THEAD]: "thead",
	[Tag.TBODY]: "tbody",
	[Tag.TR]: "tr",
	[Tag.TD]: "td",
	[Tag.TH]: "th",
	[Tag.STRONG]: "strong",
	[Tag.EM]: "em",
	[Tag.BR]: "br",
	[Tag.HR]: "hr",
	[Tag.PRE]: "pre",
	[Tag.CODE]: "code",
};

/**
 * Return the HTML tag name for a numeric tag ID.
 *
 * Returns `"unknown"` for unrecognised IDs.
 */
export function tagName(id: number): string {
	return TAG_NAMES[id] ?? "unknown";
}

/** Total number of known tag IDs (excluding UNKNOWN sentinel). */
export const TAG_COUNT = 40;
