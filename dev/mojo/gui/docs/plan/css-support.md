# CSS Support Scope — Blitz v0.2.0

> **Pinned revision:** `2f83df96220561316611ecf857e20cd1feed8ca0`
>
> **Purpose:** Document which CSS features work in the Blitz desktop renderer so that mojo-gui app authors know what they can rely on and what to avoid.

---

## Architecture Overview

Blitz v0.2.0 processes CSS through a three-stage pipeline:

| Stage | Engine | Role |
|-------|--------|------|
| **Parsing & Style Resolution** | [Stylo](https://github.com/servo/stylo) (Firefox's CSS engine) | Parses CSS, resolves selectors, computes cascaded values |
| **Layout** | [Taffy](https://github.com/DioxusLabs/taffy) + custom block/inline layout | Flexbox, CSS Grid, block flow, inline text layout |
| **Rendering** | [Vello](https://github.com/linebender/vello) + [Parley](https://github.com/linebender/parley) | GPU-accelerated 2D painting, text shaping & rendering |

Because Stylo handles parsing, **nearly all CSS syntax parses correctly** — the gaps are in layout and rendering where Taffy or Vello don't yet implement a feature.

---

## Support Matrix

### ✅ Fully Supported

These features work reliably in mojo-gui desktop apps.

#### Box Model

| Feature | Notes |
|---------|-------|
| `margin`, `padding`, `border-width` | All sides, shorthand, `auto` margins |
| `box-sizing: border-box / content-box` | Default is `content-box` (overridden to `border-box` in mojo-gui UA stylesheet) |
| `width`, `height` | Pixels, percentages, `auto` |
| `min-width`, `min-height` | Pixels, percentages |
| `max-width`, `max-height` | Pixels, percentages |

#### Display & Visibility

| Feature | Notes |
|---------|-------|
| `display: block` | Full block flow layout |
| `display: flex` | Full Flexbox support (see below) |
| `display: grid` | CSS Grid support (see below) |
| `display: inline` | Inline layout within block formatting contexts |
| `display: inline-block` | Supported |
| `display: none` | Hides element and removes from layout |
| `visibility: hidden / visible` | Supported |

#### Flexbox (`display: flex`)

| Feature | Notes |
|---------|-------|
| `flex-direction` | `row`, `row-reverse`, `column`, `column-reverse` |
| `flex-wrap` | `nowrap`, `wrap`, `wrap-reverse` |
| `justify-content` | `flex-start`, `flex-end`, `center`, `space-between`, `space-around`, `space-evenly` |
| `align-items` | `flex-start`, `flex-end`, `center`, `stretch`, `baseline` |
| `align-self` | All values |
| `align-content` | All values |
| `flex-grow`, `flex-shrink` | Supported |
| `flex-basis` | Pixels, percentages, `auto` |
| `gap`, `row-gap`, `column-gap` | Supported |
| `order` | Supported |

#### CSS Grid (`display: grid`)

| Feature | Notes |
|---------|-------|
| `grid-template-columns`, `grid-template-rows` | `px`, `%`, `fr`, `auto`, `repeat()`, `minmax()` |
| `grid-template-areas` | Named areas |
| `grid-column`, `grid-row` | Line-based placement |
| `grid-area` | Shorthand placement |
| `gap`, `row-gap`, `column-gap` | Supported |
| `justify-items`, `align-items` | Supported |
| `justify-content`, `align-content` | Supported |
| `justify-self`, `align-self` | Supported |
| `grid-auto-flow` | `row`, `column`, `dense` |
| `grid-auto-rows`, `grid-auto-columns` | Implicit track sizing |

#### Positioning

| Feature | Notes |
|---------|-------|
| `position: relative` | Supported — offsets via `top/right/bottom/left` |
| `position: absolute` | Supported — positioned relative to nearest positioned ancestor |
| `position: static` | Parsed but treated as `relative` internally |
| `top`, `right`, `bottom`, `left` | Pixels, percentages, `auto` |
| `z-index` | Supported for paint ordering |

#### Typography

| Feature | Notes |
|---------|-------|
| `font-family` | System fonts, generic families (`sans-serif`, `serif`, `monospace`, etc.) |
| `font-size` | Pixels, `em`, `rem`, percentages, keywords (`small`, `large`, etc.) |
| `font-weight` | Numeric (100–900) and keywords (`bold`, `normal`) |
| `font-style` | `normal`, `italic`, `oblique` |
| `line-height` | Number, pixels, percentages |
| `text-align` | `left`, `right`, `center`, `justify` |
| `text-decoration` | `underline`, `line-through`, `overline`, `none` |
| `color` | Named colors, hex, `rgb()`, `rgba()`, `hsl()`, `hsla()` |
| `white-space` | `normal`, `nowrap`, `pre`, `pre-wrap`, `pre-line` |
| `word-break`, `overflow-wrap` | Basic support |
| `letter-spacing` | Supported |

#### Colors & Backgrounds

| Feature | Notes |
|---------|-------|
| `background-color` | All CSS color formats |
| `background-image: linear-gradient()` | Linear gradients supported |
| `background-image: radial-gradient()` | Radial gradients supported |
| `background-image: url()` | Image backgrounds (requires `net` feature) |
| `background-size` | `cover`, `contain`, pixels, percentages |
| `background-position` | Supported |
| `background-repeat` | Supported |
| `opacity` | Supported — elements with `opacity: 0` are hidden; partial opacity uses Vello layers |

#### Borders & Outlines

| Feature | Notes |
|---------|-------|
| `border-style` | `solid`, `none`, `hidden` (others parsed but may render as solid) |
| `border-color` | All CSS color formats |
| `border-width` | Pixels |
| `border-radius` | Pixels, percentages — all four corners |
| `outline` | Supported (style, color, width) |

#### Box Shadows

| Feature | Notes |
|---------|-------|
| `box-shadow` (outset) | Offset, blur, spread, color |
| `box-shadow` (inset) | Offset, blur, spread, color |
| Multiple shadows | Supported |

#### Overflow

| Feature | Notes |
|---------|-------|
| `overflow: hidden` | Clips content |
| `overflow: scroll` | Scroll containers with scroll offset support |
| `overflow: visible` | Default — no clipping |

#### Selectors & Cascade

| Feature | Notes |
|---------|-------|
| Type selectors (`div`, `p`) | Supported (Stylo) |
| Class selectors (`.foo`) | Supported |
| ID selectors (`#bar`) | Supported |
| Descendant combinator (`a b`) | Supported |
| Child combinator (`a > b`) | Supported |
| Adjacent sibling (`a + b`) | Supported |
| General sibling (`a ~ b`) | Supported |
| Attribute selectors (`[attr]`, `[attr=val]`) | Supported |
| Pseudo-classes (`:hover`, `:focus`, `:first-child`, etc.) | Supported |
| Pseudo-elements (`::before`, `::after`) | Supported for generated content |
| `@media` queries | Supported (screen size, `prefers-color-scheme`) |
| CSS variables (`--custom-property`) | Supported |
| `calc()` | Supported |
| `!important` | Supported |
| Cascade layers (`@layer`) | Supported (Stylo) |

#### Units

| Unit | Status |
|------|--------|
| `px` | ✅ |
| `em`, `rem` | ✅ |
| `%` | ✅ |
| `vw`, `vh`, `vmin`, `vmax` | ✅ |
| `ch`, `ex` | ✅ (via Parley) |
| `fr` (grid) | ✅ |
| `auto` | ✅ |

---

### ⚠️ Partially Supported

These features work in some cases but have known limitations.

| Feature | Status | Limitation |
|---------|--------|------------|
| `position: fixed` | ⚠️ | Treated as `absolute` — no viewport-fixed behavior |
| `position: sticky` | ⚠️ | Treated as `relative` — no scroll-based sticking |
| `overflow: auto` | ⚠️ | Treated as `scroll` — always reserves scrollbar space |
| `display: table` | ⚠️ | Falls back to `grid` layout internally; basic table rendering works but complex table features (colspan, rowspan, caption) are incomplete |
| `display: contents` | ⚠️ | Parsed but may not behave correctly in all layout modes |
| `display: inline-flex` | ⚠️ | Outer inline behavior may not be fully correct |
| `display: inline-grid` | ⚠️ | Outer inline behavior may not be fully correct |
| `opacity` (partial) | ⚠️ | Binary (0 vs >0) is reliable; intermediate values use Vello layers which may have edge cases |
| `transform` (2D) | ⚠️ | Basic 2D transforms (`translate`, `rotate`, `scale`, `matrix`) work; transform-origin is supported; nested transforms have edge cases |
| `max-content`, `min-content`, `fit-content` | ⚠️ | Parsed but fall back to `auto` in Taffy |
| `stretch` / `-webkit-fill-available` | ⚠️ | Parsed but fall back to `auto` |
| Grid `subgrid` | ⚠️ | Parsed but not implemented in Taffy |
| Grid `masonry` | ⚠️ | Parsed but not implemented in Taffy |
| `flex-basis: content` | ⚠️ | Falls back to `auto` |
| Form controls (`<input>`, `<select>`) | ⚠️ | Basic text input works; styled form controls are limited |

---

### 🔲 Not Supported

These features are parsed by Stylo but have no layout or rendering implementation in Blitz v0.2.0.

| Feature | Notes |
|---------|-------|
| `transition` | No CSS transition engine — changes apply instantly |
| `animation` / `@keyframes` | No CSS animation engine |
| `transform` (3D) | `perspective`, `rotateX/Y/Z`, `translate3d` — no 3D rendering pipeline |
| `filter` | `blur()`, `brightness()`, `contrast()`, etc. — Vello doesn't expose these |
| `backdrop-filter` | Not implemented |
| `clip-path` | Not implemented |
| `mask` / `mask-image` | Not implemented |
| `text-shadow` | Not implemented |
| `text-overflow: ellipsis` | Not implemented |
| `columns` / `column-count` | Multi-column layout not implemented in Taffy |
| `float` | Not implemented — floated elements lay out in normal flow |
| `clear` | Not implemented (depends on float) |
| `shape-outside` | Not implemented |
| `writing-mode` (vertical) | Not implemented |
| `direction: rtl` | Not implemented |
| `resize` | Not implemented |
| `cursor` | Cursor styles are not applied (Winit manages system cursor) |
| `pointer-events` | Parsed but not used in hit testing |
| `will-change` | Parsed (Stylo) but no rendering optimization |
| `content-visibility` | Not implemented |
| `container` queries (`@container`) | Not implemented |
| `scroll-snap-*` | Not implemented |
| `overscroll-behavior` | Not implemented |
| Print-related (`@page`, `break-*`) | Not applicable (no print target) |
| `object-fit` / `object-position` | Parsed but only partially used for images |

---

## Recommendations for mojo-gui Apps

### Safe to Use Everywhere

These patterns work identically on both web and desktop:

```css
/* Box model & spacing */
margin, padding, border, border-radius, box-sizing

/* Layout */
display: flex | grid | block | none
flex-direction, flex-wrap, justify-content, align-items, gap
grid-template-columns, grid-template-rows, grid-area

/* Positioning */
position: relative | absolute
top, right, bottom, left, z-index

/* Typography */
font-family, font-size, font-weight, color, text-align, line-height

/* Visual */
background-color, background-image (gradients), opacity, box-shadow
border-radius, outline

/* Responsive */
@media queries, CSS variables, calc()
```

### Avoid on Desktop (or Feature-Gate)

Use `@parameter if is_wasm_target()` in Mojo or `@media` queries to provide desktop-safe fallbacks for:

| Feature | Desktop Workaround |
|---------|-------------------|
| `transition` / `animation` | Use the Mojo reactive system for state-driven updates |
| `position: fixed` | Use `position: absolute` relative to a full-viewport container |
| `position: sticky` | Use scroll event handlers + `position: relative` with dynamic offsets |
| `float` | Use `display: flex` or `display: grid` instead |
| `columns` | Use `display: grid` with `grid-template-columns` |
| `text-overflow: ellipsis` | Truncate text in Mojo before setting it |
| `filter` / `backdrop-filter` | Not available — adjust design to avoid these effects |

### CSS Strategy for Shared Examples

The mojo-gui shared examples (Counter, Todo, Benchmark, MultiView) use only the "Safe to Use" subset above. This ensures identical behavior across web and desktop renderers.

The mojo-gui desktop UA stylesheet (`_DEFAULT_UA_CSS` in `desktop/src/desktop/launcher.mojo`) provides baseline styles that paper over Blitz's default rendering to match browser defaults more closely.

---

## Blitz Version Tracking

| Property | Value |
|----------|-------|
| Blitz version | v0.2.0 |
| Git revision | `2f83df96220561316611ecf857e20cd1feed8ca0` |
| Stylo version | Mozilla's Stylo (Firefox CSS engine) |
| Taffy version | Bundled via `stylo_taffy` (Flexbox, Grid, Block) |
| Vello version | 0.6.x |
| Parley | Bundled (text layout & shaping) |
| Winit | 0.30.x (windowing — Wayland-only in mojo-gui) |
| anyrender_vello | 0.6.x (GPU rendering backend) |

When Blitz is upgraded, re-audit this document against the new version. Key areas to recheck:

1. `position: fixed` / `position: sticky` support
2. CSS transitions & animations
3. `display: table` improvements
4. `transform` 3D support
5. `filter` / `backdrop-filter` support
6. `display: contents` correctness
7. `float` / multi-column layout

---

## Test Methodology

This audit was produced by:

1. **Source code review** of `stylo_taffy/src/convert.rs` — the Stylo → Taffy style conversion layer, which documents unsupported CSS features via `TODO` comments and fallback mappings.
2. **Source code review** of `blitz-paint/src/render.rs` — the Vello rendering pipeline, which documents rendering limitations.
3. **Source code review** of `blitz-dom/src/layout/` — the layout engine, which documents layout gaps.
4. **Runtime verification** of the 4 shared mojo-gui examples (Counter, Todo, Benchmark, MultiView) on Wayland.
5. **Cross-reference** with Blitz's own [roadmap issue](https://github.com/DioxusLabs/blitz/issues/119) and README status.