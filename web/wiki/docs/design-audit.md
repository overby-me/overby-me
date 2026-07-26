# Presentation-layer design audit (2026-07)

A dimension-by-dimension audit of the parts of `web/wiki-dioxus` that survive the
planned atproto/Rust backend rewrite: the design system, component library,
layout, accessibility, and screen UX. **Overall verdict: good shape — the work is
enforcement and consistency, not rescue.** Color/elevation/shape/motion are
genuinely token-driven and re-skinnable; naming is consistent; CSS specificity is
flat (13 `!important` in 5,370 lines); accessibility is a real strength.

Status keys: ✅ done, ◻️ open. Items reference the four "fundamental" findings.

## 1. Design tokens — spacing/type escape the scale (✅)

Color/motion/radius flow through tokens, but spacing + font-size mostly didn't:
a full `--md-sys-spacing-*` scale exists yet ~340 raw px drove padding/margin/gap
and ~80 raw px drove font-size. A global density/type change meant find-and-replace,
not a token edit.

- ✅ Kept spacing utilities (`.mt-1/.mt-2/.mb-1/.mb-2/.stack`) now reference tokens;
  the 9 dead utilities (`.p-*`, `.m-*`, `.w-full`, `.mobile-only`, `.text-center`,
  `.grid-2`) were removed.
- ✅ Ratchet lint gate: `scripts/check-css-spacing.nu` (in `just check`) — the raw-px
  count may only decrease. `style.css` is now at **zero** raw-px spacing/font-size.
- ✅ The gate covers the COMPONENTS too. A raw px in an rsx! `style:` attribute
  escapes the scale exactly the way one in the stylesheet does, and gate A never
  saw it; that hole is why ~110 inline styles had drifted off-scale. Both ratchets
  now sit at zero, and a third gate fails on a `var(--token)` nothing defines
  (three such references had silently been resolving to their literal fallback:
  `--md-sys-motion-duration-medium4`, `--md-sys-motion-spring-fast`,
  `--md-sys-state-disabled-opacity`).
- ✅ Dead `var(--token, <literal>)` fallbacks removed app-wide. They never fired
  (every token is defined) but they drift: two had gone stale against the theme
  they duplicated, and one baked a light-mode colour behind a themed token.
- ✅ The 19 `border-radius` literals that sat exactly on the corner scale
  (4/8/12/16/20/999px) now read their token. The five that remain are deliberately
  below `corner-extra-small` — a keycap and a code block at 6px, inline diff marks
  at 3px, one banner at 14px — a de-facto micro tier the M3 scale has no step for.
  Extend the scale before adding another one.
- ◻️ Self-host the two render-blocking Google Fonts `@import`s. **Deferred**: Google
  serves per-`unicode-range` woff2 subsets, so static self-hosting risks dropping the
  Danish glyphs (æ ø å) from the primary text font and breaking every Material icon —
  app-wide blast radius that needs browser verification. Do it with the full,
  unsubsetted font files and a real-browser check.

## 1b. Type scale — one role, one definition (✅)

The scale existed twice: `m3-tokens.css` held a correct `.md-<role>` set, and
`style.css` held a partial, hand-tuned copy under the short names the components
actually use. The copy was wrong in three places (`.title-medium` read the
*body-large* size token, `.title-small` and `.label-large` the *body-medium* one),
restated line-height and tracking as raw px, and simply **omitted** `.body-small`
(15 call sites), `.label-medium`, `.label-small` and the display roles — so those
elements silently rendered at the inherited 16px instead of their role.

- ✅ One rule per role, in style.css's TYPOGRAPHY section, driven end to end by the
  size/line/tracking tokens. Each rule answers to all three spellings: `.md-<role>`,
  the short `.<role>` alias, and (for the six heading roles) `h1`…`h6`.
  `m3-tokens.css` is back to what its name says: tokens only.
- ✅ Every role states its tracking even where the spec's value is zero. `body` sets
  the body-large tracking document-wide, so a silent role inherited 0.5px and a
  headline was not the size the scale claimed.
- ✅ Verified in a real browser: all 15 roles and h1–h6 now compute to their M3
  metrics, and each `.md-*` matches its short alias exactly.

## 2. Component library — was two systems (✅)

A shadcn-style `ui/` primitive set (11) sat beside the real M3 class system; 8 were
dead code under `#[allow(dead_code)]`, and `AlertDialog` was abandoned as
non-functional (custom `widgets::Dialog` built instead).

- ✅ Deleted the 8 dead primitives; kept `checkbox/radio_group/switch`.
- ✅ Split the 667-line `widgets.rs` into one file per component under `widgets/`
  (atoms, dialog, tool_sheet, table, segmented_button, color_picker, image + a shared
  `focus` a11y module), re-exported at `widgets::`. Domain-free → crate-extractable.
- ◻️ `ToolSheet` has one app coupling (`crate::window_size`) to decouple before a
  crate lift. Consider dropping `dx-components-theme.css` if the 3 kept primitives
  don't need it (they reference no `--dx*` tokens). **Deferred**: it is a single
  `WINDOW_SIZE().is_extra_large()` call; decoupling it now (a `docked` prop the
  caller computes) just pushes the same window-size logic onto every call site with
  no benefit until the library is actually extracted. Do it at lift time.

## 2b. Components reaching around the system (✅)

Where the system had no class for what a screen needed, the screen inlined CSS —
and once inlining is normal, it also gets used for things the system *does*
provide. The stylesheet gained the missing pieces and the call sites now use them:

- **Sizes the system never offered.** `.spinner` was a fixed 44px page loader, so
  every inline spinner overrode it by hand (or didn't, and rendered a 44px blob
  beside a 12px label). It now takes a `--spinner-size` with `.spinner-sm` (22px,
  one icon slot) and `.spinner-xs` (18px). Likewise `.material-icons.icon-inline`
  sizes a glyph from the text it sits in (1.15em) instead of the four different
  hand-set px values that were in the tree.
- **Two components sharing one class name.** The docked search `<input>` wore
  `.breadcrumbs .search-field` — the *member table's* pill container — and then
  overrode most of it inline. It has its own `.search-box` / `.search-input` now.
- **Patterns repeated by hand.** `.list-subheader` (an M3 list subheader, faked at
  six sites with three different paddings), `.list-section-header` (the drawer's
  copy of `.card-header`), `.btn-busy` (a spinner centred over a submitting
  button), `.text-accent` / `.text-error` / `.text-preserve-breaks`,
  `.stack-wrap` / `.stack-end`, `.scroll-x`, `.chip-row-authors`, `.range-field`,
  `.upload-thumb`, `.map-embed`, `.graph-svg`, `.text-field-compact`.
- **Inline styles that restated what the class already did.** `.stack`/`.stack-h`
  supply `gap: 8px` and `align-items: center`; `.list-item` and `.crumb-link`
  supply `cursor: pointer`; `.file-upload-done` supplies its own flex row;
  `.list-item-trailing` supplies `margin-left: auto`; `* { margin: 0 }` makes a
  `margin: 0` reset a no-op. All of those were being repeated inline.
- **A duplicated affordance.** `.sort-list .sort-item::before` already draws the
  drag handle, and the row also rendered a hand-placed `drag_indicator` span — two
  handles side by side. The RSX one is gone.
- **Loading rendered three ways.** `widgets::Spinner` is the shared one; two
  screens hand-rolled `.spinner-overlay > .spinner` instead and now call it.
- ◻️ A few structural class names carry no rule (`app-rail-icon`, `home-hero`,
  `home-hero-text`, and the `comment-send` / `list-expand-toggle` hooks that
  `test-browser.nu` selects on). They are inert markup, not style deviations; left
  in place deliberately.

## 3. Loading / Error / Empty — no single contract (partly ✅)

Three loading treatments plus screens that render nothing while pending;
`unwrap_or_default()` collapses loading/error/empty into one blank render, so a
failed fetch looks like "no groups" and screens flash blank→full. No optimistic UI —
every mutation is fire-then-full-refetch.

- ✅ Reusable `widgets::EmptyState` + `widgets::ErrorState` establish the shared
  empty/error presentation; `widgets/feedback.rs` documents the four-state match
  (`None`→skeleton / `Err`→error / `Some(empty)`→orb / `Some(data)`→rows).
- ✅ `social.rs` and the page resolver (`loader.rs`) now use them and log the error
  detail instead of dumping raw `{e}` into the UI. `social.rs` is the reference LEE
  screen.
- ✅ Rolled the four-state match onto the screens with the real "error looks empty"
  bug: `comments.rs` (a failed thread load now shows an error state, not "no
  comments") and `profile.rs` memberships (a failed load shows an error, not an
  empty membership list) — both resources now return `Result<_, _>`.
- ✅ `folder.rs` and home `Recent` intentionally left as-is: the folder always has an
  `initial` server-resolved child set to fall back on (a failed live refetch shows
  stale rows, never a false "empty"), and Recent is a supplementary widget that is
  by-design hidden when it has nothing to show, where a large error card would be
  worse UX than silence.
- ◻️ Optimistic insert for high-frequency low-risk actions (comments). **Deferred**:
  needs an insert-then-reconcile path with rollback on failure; medium risk, best
  landed with browser verification.

## 4. Accessibility — strong, with a few sharp gaps (mostly ✅)

Semantic HTML is the norm (120 real `<button>`, `nav/main/aside`, real `<table>`), a
reused focus-trap/Escape/return-focus system, snackbar live region,
`prefers-reduced-motion`, keyboard shortcuts + skip link, ARIA combobox, WCAG-AA
contrast, status never colour-only.

- ✅ Compact nav drawer (primary phone nav) now has the focus-trap/Escape/role/
  aria-label/return-focus pattern it was missing.
- ✅ `Dialog` + user popover now carry accessible names; `aria-current` on nav;
  `aria-pressed` on editor format toggles.
- ✅ Closed the remaining gaps: `aria-activedescendant` on the search combobox; an
  accessible name on the SVG graph (`role=img`); granted/denied labels on the
  perm-matrix glyphs; min/max names + `aria-valuetext` on the range sliders; an
  sr-only polite live region for the speaker queue; and the medium-width tree
  overlay now has the focus-trap/Escape/aria-modal/return-focus pattern when modal.
- ✅ Two colour-contract leftovers from the old dark green app bar were still
  forcing `#fff`: `.breadcrumbs` and `.crumb-link` on today's tonal (light)
  `.top-app-bar`. Both take the bar's paired on-surface colour now — the crumb
  text was white-on-light wherever `.crumb-name` did not override it. The error
  boundary's `<pre>` likewise had a fixed `#ededed` that went white-on-white in
  dark mode; it is a tonal container. `test-contrast-audit.js` reports zero
  sub-AA elements in both themes after the change.

## Highest-risk survivor

The rich-text editor (`editor.rs` + `richtext/`, ~1,750 lines) rests on the
deprecated `document.execCommand`. It now has a debounced autosave and a
beforeunload unsaved-changes guard (so a crash/close no longer loses work), but
the `execCommand` engine itself is retained on purpose: it works in every current
browser, is isolated behind the `richtext::exec`/`query_*` seam, and the interim
Dioxus frontend will be replaced wholesale by the atproto rewrite. It carries over
on paper but is still the piece most likely to be rebuilt in that rewrite.
