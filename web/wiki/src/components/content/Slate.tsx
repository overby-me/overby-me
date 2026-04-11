/* eslint-disable functional/immutable-data */

import {
	FormatAlignCenter,
	FormatAlignJustify,
	FormatAlignLeft,
	FormatAlignRight,
	FormatBold,
	FormatItalic,
	FormatListBulleted,
	FormatListNumbered,
	FormatUnderlined,
	Link,
	LinkOff,
	Redo,
	Undo,
} from "@mui/icons-material";
import {
	Button,
	ButtonGroup,
	FormControl,
	IconButton,
	InputBase,
	InputLabel,
	MenuItem,
	Link as MuiLink,
	Paper,
	Popper,
	Select,
	Stack,
	ToggleButton,
	ToggleButtonGroup,
} from "@mui/material";
import { Box } from "@mui/system";
import { Link as NextLink, SlateErrorBoundary } from "comps";
import {
	type CustomEditor,
	type CustomElement,
	type CustomText,
	type ElementType,
	LIST_TYPES,
	STYLE_TYPES,
	type StyleType,
	TEXT_ALIGN_TYPES,
	type TextAlignType,
} from "core/types/slate";
import isHotkey from "is-hotkey";
import React, { type ChangeEventHandler, useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import {
	createEditor,
	type Descendant,
	Editor,
	Element as SlateElement,
	Transforms,
} from "slate";
import { type HistoryEditor, withHistory } from "slate-history";
import { jsx } from "slate-hyperscript";
import {
	Editable,
	type RenderElementProps,
	type RenderLeafProps,
	Slate as SlateEditor,
	useSlate,
	withReact,
} from "slate-react";

const HOTKEYS = {
	"mod+b": "bold",
	"mod+i": "italic",
	"mod+u": "underline",
	"mod+`": "code",
};
const STYLE_NAME_KEYS: Record<StyleType, string> = {
	paragraph: "editor.paragraph",
	"heading-one": "editor.headingOne",
	"heading-two": "editor.headingTwo",
	"heading-three": "editor.headingThree",
	"heading-four": "editor.headingFour",
	"heading-five": "editor.headingFive",
	"heading-six": "editor.headingSix",
	"block-quote": "editor.blockQuote",
	"block-pre": "editor.blockPre",
};

const ELEMENT_TAGS: Record<
	string,
	(el?: Element) => Omit<CustomElement, "children">
> = {
	BLOCKQUOTE: () => ({ type: "block-quote" }),
	H1: () => ({ type: "heading-one" }),
	H2: () => ({ type: "heading-two" }),
	H3: () => ({ type: "heading-three" }),
	H4: () => ({ type: "heading-four" }),
	H5: () => ({ type: "heading-five" }),
	H6: () => ({ type: "heading-six" }),
	IMG: (el) => ({ type: "image", url: el?.getAttribute("src") }),
	LI: () => ({ type: "list-item" }),
	OL: () => ({ type: "numbered-list" }),
	P: () => ({ type: "paragraph" }),
	PRE: () => ({ type: "block-pre" }),
	UL: () => ({ type: "bulleted-list" }),
};

// COMPAT: `B` is omitted here because Google Docs uses `<b>` in weird ways.
const TEXT_TAGS: Record<string, (el?: Element) => Omit<CustomText, "text">> = {
	CODE: () => ({ code: true }),
	DEL: () => ({ strikethrough: true }),
	EM: () => ({ italic: true }),
	I: () => ({ italic: true }),
	S: () => ({ strikethrough: true }),
	STRONG: () => ({ bold: true }),
	U: () => ({ underline: true }),
	A: (el) => ({ link: el?.getAttribute("href") ?? "" }),
};

const deserialize = (
	el: HTMLElement | ChildNode,
): CustomElement | string | null | (string | null | Descendant)[] => {
	if (el.nodeType === 3) {
		return el.textContent;
	} else if (el.nodeType !== 1) {
		return null;
	} else if (el.nodeName === "BR") {
		return "\n";
	}

	const { nodeName } = el;
	const parent =
		nodeName === "PRE" &&
		el.childNodes[0] &&
		el.childNodes[0].nodeName === "CODE"
			? el.childNodes[0]
			: el;

	const parentChildren = Array.from(parent.childNodes).flatMap(deserialize);
	const children =
		parentChildren.length === 0 ? [{ text: "" }] : parentChildren;

	if (el.nodeName === "BODY") {
		return jsx("fragment", {}, children);
	}

	if (ELEMENT_TAGS[nodeName]) {
		const attrs = ELEMENT_TAGS[nodeName](el as Element);
		return jsx("element", attrs, children);
	}

	if (TEXT_TAGS[nodeName]) {
		const attrs = TEXT_TAGS[nodeName](el as Element);
		return children.map((child) => jsx("text", attrs, child));
	}

	return children;
};

const withHtml = (editor: CustomEditor) => {
	const { insertData, isInline, isVoid } = editor;

	editor.isInline = (element) => {
		return element.type === "link" ? true : isInline(element);
	};

	editor.isVoid = (element) => {
		return element.type === "image" ? true : isVoid(element);
	};

	editor.insertData = (data) => {
		const html = data.getData("text/html");

		if (html) {
			const parsed = new DOMParser().parseFromString(html, "text/html");
			const fragment = deserialize(parsed.body) as Descendant[];
			try {
				Transforms.insertFragment(editor, fragment);
			} catch (error) {
				console.log(error);
			}
			return;
		}

		insertData(data);
	};

	return editor;
};

const validate = (value?: Descendant[]): Descendant[] => {
	if (Array.isArray(value) && value.length > 0) return value;
	else return [{ type: "paragraph", children: [{ text: "" }] }];
};

const Slate = ({
	value,
	onChange,
	readOnly = false,
}: {
	value?: Descendant[];
	onChange?: (value: Descendant[]) => void;
	readOnly: boolean;
}) => {
	const editorRef = useRef<CustomEditor | null>(null);
	if (!editorRef.current)
		editorRef.current = withHtml(withHistory(withReact(createEditor())));
	const editor = editorRef.current;
	const renderElement = useCallback(
		(props: RenderElementProps) => <Element {...props} />,
		[],
	);
	const renderLeaf = useCallback(
		(props: RenderLeafProps) => <Leaf {...props} />,
		[],
	);
	const validated = validate(value);
	if (editor) {
		editor.children = validated;
		if (editor.selection) {
			try {
				Editor.node(editor, editor.selection.anchor.path);
				Editor.node(editor, editor.selection.focus.path);
			} catch {
				editor.selection = {
					anchor: { path: [0, 0], offset: 0 },
					focus: { path: [0, 0], offset: 0 },
				};
			}
		}
	} else {
		return null;
	}

	return (
		<SlateEditor editor={editor!} value={validated} onChange={onChange}>
			{!readOnly && (
				<Box
					sx={{
						// Disable scroll (Firefox)
						scrollbarWidth: "none",
						// Disable scroll (Webkit)
						"::-webkit-scrollbar": {
							display: "none",
						},
						overflowX: "auto",
						WebkitOverflowScrolling: "touch",
						width: "100%",
					}}
				>
					<Stack direction="row" spacing={1} sx={{ mt: 1, mb: 1, ml: 2 }}>
						<BlockSelect />
						<HistoryButtons />
						<MarkButtons />
						<ListButtons />
						<AlignButtons />
						<LinkButton />
					</Stack>
				</Box>
			)}
			{(!readOnly ||
				(validated.length !== 0 &&
					(validated as CustomElement[])[0].children?.[0].text !== "")) && (
				<Box sx={{ m: 2 }}>
					<SlateErrorBoundary editor={editor}>
						<Editable
							renderElement={renderElement}
							renderLeaf={renderLeaf}
							spellCheck
							readOnly={readOnly}
							onKeyDown={(event) => {
								Object.entries(HOTKEYS).forEach(([hotkey, mark]) => {
									if (isHotkey(hotkey, event)) {
										event.preventDefault();
										toggleMark(editor, mark);
									}
								});
							}}
						/>
					</SlateErrorBoundary>
				</Box>
			)}
		</SlateEditor>
	);
};

const toggleMark = (editor: CustomEditor, format: string) => {
	const isActive = isMarkActive(editor, format);

	if (isActive) {
		Editor.removeMark(editor, format);
	} else {
		Editor.addMark(editor, format, true);
	}
};

const currentBlocks = (
	editor: CustomEditor,
	validBlocks: Readonly<ElementType[]>,
	blockType = "type",
	prop?: string,
) => {
	const { selection } = editor;
	if (!selection) return "paragraph";

	const blocks = Array.from(
		Editor.nodes(editor, {
			at: Editor.unhangRange(editor, selection),
			match: (n) =>
				!Editor.isEditor(n) &&
				SlateElement.isElement(n) &&
				validBlocks.includes(n[blockType] as ElementType),
		}),
	)?.[0] as unknown as CustomElement[] | undefined;

	if (!blocks) return undefined;

	return blocks.length > 0
		? blocks
				.filter((block) => block[blockType])
				?.map((block) => (prop ? block[prop] : block[blockType]))
		: undefined;
};

const isBlockActive = (
	editor: CustomEditor,
	format: string,
	blockType = "type",
) => {
	const { selection } = editor;
	if (!selection) return false;

	const [match] = Array.from(
		Editor.nodes(editor, {
			at: Editor.unhangRange(editor, selection),
			match: (n) =>
				!Editor.isEditor(n) &&
				SlateElement.isElement(n) &&
				n[blockType] === format,
		}),
	);

	return !!match;
};

const isMarkActive = (editor: CustomEditor, format: string) => {
	const marks = Editor.marks(editor);
	return marks ? marks[format] === true : false;
};

const Element = ({ attributes, children, element }: RenderElementProps) => {
	const style = { textAlign: element.align };
	switch (element.type) {
		case "block-quote":
			return (
				<blockquote style={style} {...attributes}>
					{children}
				</blockquote>
			);
		case "block-pre":
			return (
				<pre style={style} {...attributes}>
					{children}
				</pre>
			);
		case "bulleted-list":
			return (
				<ul style={style} {...attributes}>
					{children}
				</ul>
			);
		case "heading-one":
			return (
				<h1 style={style} {...attributes}>
					{children}
				</h1>
			);
		case "heading-two":
			return (
				<h2 style={style} {...attributes}>
					{children}
				</h2>
			);
		case "heading-three":
			return (
				<h3 style={style} {...attributes}>
					{children}
				</h3>
			);
		case "heading-four":
			return (
				<h4 style={style} {...attributes}>
					{children}
				</h4>
			);
		case "heading-five":
			return (
				<h5 style={style} {...attributes}>
					{children}
				</h5>
			);
		case "heading-six":
			return (
				<h6 style={style} {...attributes}>
					{children}
				</h6>
			);
		case "list-item":
			return (
				<li style={style} {...attributes}>
					{children}
				</li>
			);
		case "numbered-list":
			return (
				<ol style={style} {...attributes}>
					{children}
				</ol>
			);

		default:
			return (
				<p style={{ ...style, hyphens: "auto" }} {...attributes}>
					{children}
				</p>
			);
	}
};

const Leaf = ({ attributes, children, leaf }: RenderLeafProps) => {
	const bold = leaf.bold ? <strong>{children}</strong> : children;
	const code = leaf.code ? <em>{bold}</em> : bold;
	const underline = leaf.underline ? <u>{code}</u> : code;
	const italic = leaf.italic ? <i>{underline}</i> : underline;
	const link = leaf.link ? (
		<MuiLink component={NextLink} href={leaf.link}>
			{children}
		</MuiLink>
	) : (
		italic
	);

	return <span {...attributes}>{link}</span>;
};

const BlockSelect = () => {
	const editor = useSlate();
	const { t } = useTranslation();
	return (
		<FormControl title={t("editor.style")} sx={{ minWidth: "140px" }}>
			<InputLabel>{t("editor.style")}</InputLabel>
			<Select
				label={t("editor.style")}
				sx={{ width: 145 }}
				value={currentBlocks(editor, STYLE_TYPES) ?? "paragraph"}
				onChange={(e) =>
					Transforms.setNodes<CustomElement>(editor, {
						type: e.target.value as ElementType,
					})
				}
			>
				{STYLE_TYPES.map((style) => (
					<MenuItem key={style} value={style}>
						{t(STYLE_NAME_KEYS[style])}
					</MenuItem>
				))}
			</Select>
		</FormControl>
	);
};

const HistoryButtons = () => {
	const editor = useSlate() as HistoryEditor;

	return (
		<ButtonGroup color="inherit">
			<Button
				disabled={editor.history.undos.length === 0}
				onClick={() => editor.undo()}
			>
				<Undo />
			</Button>
			<Button
				disabled={editor.history.redos.length === 0}
				onClick={() => editor.redo()}
			>
				<Redo />
			</Button>
		</ButtonGroup>
	);
};

const ListButtons = () => {
	const editor = useSlate();
	const handleBlock = (_: unknown, format: ElementType) => {
		const isActive = isBlockActive(editor, format, "type");

		Transforms.unwrapNodes<CustomElement>(editor, {
			match: (n) =>
				!Editor.isEditor(n) &&
				SlateElement.isElement(n) &&
				(LIST_TYPES as unknown as string[]).includes(n.type),
			split: true,
		});
		const newProperties: CustomElement = {
			type: isActive ? "paragraph" : "list-item",
		};

		Transforms.setNodes<CustomElement>(editor, newProperties);

		if (!isActive) {
			const block = { type: format, children: [] };
			Transforms.wrapNodes(editor, block);
		}
	};

	return (
		<ToggleButtonGroup
			exclusive
			value={currentBlocks(editor, LIST_TYPES) ?? []}
			onChange={handleBlock}
		>
			<ToggleButton value="numbered-list">
				<FormatListNumbered />
			</ToggleButton>
			<ToggleButton value="bulleted-list">
				<FormatListBulleted />
			</ToggleButton>
		</ToggleButtonGroup>
	);
};

const AlignButtons = () => {
	const editor = useSlate();
	const handleBlock = (_: unknown, block: TextAlignType) => {
		const isActive = isBlockActive(editor, block, "align");
		const newProperties = {
			align: isActive ? undefined : block,
		};
		Transforms.setNodes<SlateElement>(editor, newProperties);
	};

	return (
		<ToggleButtonGroup
			exclusive
			value={currentBlocks(editor, TEXT_ALIGN_TYPES, "align") ?? []}
			onChange={handleBlock}
		>
			<ToggleButton value="left">
				<FormatAlignLeft />
			</ToggleButton>
			<ToggleButton value="center">
				<FormatAlignCenter />
			</ToggleButton>
			<ToggleButton value="right">
				<FormatAlignRight />
			</ToggleButton>
			<ToggleButton value="justify">
				<FormatAlignJustify />
			</ToggleButton>
		</ToggleButtonGroup>
	);
};

const MarkButtons = () => {
	const editor = useSlate();
	const marks = Editor.marks(editor);
	const handleMarks = (
		_event: React.MouseEvent<HTMLElement>,
		newMarks: string[],
	) => {
		// Symmetrical Difference
		newMarks
			.filter((mark) => !getMarks(marks).includes(mark))
			.concat(getMarks(marks).filter((x) => !newMarks.includes(x)))
			.map((mark) => toggleMark(editor, mark));
	};

	const getMarks = (marks: Omit<CustomText, "text"> | null) => {
		if (!marks) return [];
		return (marks.bold ? ["bold"] : [])
			.concat(marks.italic ? ["italic"] : [])
			.concat(marks.underline ? ["underline"] : []);
	};
	return (
		<ToggleButtonGroup value={getMarks(marks)} onChange={handleMarks}>
			<ToggleButton value="bold">
				<FormatBold />
			</ToggleButton>
			<ToggleButton value="italic">
				<FormatItalic />
			</ToggleButton>
			<ToggleButton value="underline">
				<FormatUnderlined />
			</ToggleButton>
		</ToggleButtonGroup>
	);
};

const LinkButton = () => {
	const editor = useSlate();
	const marks = Editor.marks(editor);
	const [anchorEl, setAnchorEl] = React.useState<null | HTMLElement>(null);
	const open = Boolean(anchorEl);

	const handleClick = (e: React.MouseEvent<HTMLElement>) => {
		e.preventDefault();
		setAnchorEl(anchorEl ? null : e.currentTarget);
	};

	const handleChange: ChangeEventHandler<HTMLInputElement> = (event) => {
		Editor.addMark(editor, "link", event.target.value);
	};

	return (
		<>
			<Popper
				open={open}
				anchorEl={anchorEl}
				placement="bottom"
				disablePortal={false}
				modifiers={[
					{
						name: "flip",
						enabled: true,
						options: {
							altBoundary: true,
							rootBoundary: "document",
							padding: 8,
						},
					},
					{
						name: "preventOverflow",
						enabled: true,
						options: {
							altAxis: true,
							altBoundary: true,
							tether: true,
							rootBoundary: "document",
							padding: 8,
						},
					},
				]}
			>
				<Paper
					sx={{
						p: "2px 4px",
						display: "flex",
						alignItems: "center",
					}}
				>
					<InputBase
						sx={{ ml: 1, flex: 1 }}
						value={marks?.link ?? ""}
						onChange={handleChange}
					/>
					<IconButton
						sx={{ p: "10px" }}
						color="inherit"
						onClick={() => {
							setAnchorEl(null);
							Editor.removeMark(editor, "link");
						}}
					>
						<LinkOff />
					</IconButton>
				</Paper>
			</Popper>
			<ToggleButton value="link" onClick={handleClick} selected={!!marks?.link}>
				<Link />
			</ToggleButton>
		</>
	);
};

export default Slate;
