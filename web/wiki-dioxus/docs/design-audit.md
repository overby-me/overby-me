# Presentation-layer design audit (2026-07)

A dimension-by-dimension audit of the parts of `web/wiki-dioxus` that survive the
planned atproto/Rust backend rewrite: the design system, component library,
layout, accessibility, and screen UX. **Overall verdict: good shape — the work is
enforcement and consistency, not rescue.** Color/elevation/shape/motion are
genuinely token-driven and re-skinnable; naming is consistent; CSS specificity is
flat (13 `!important` in 5,370 lines); accessibility is a real strength.

Status keys: ✅ done, ◻️ open. Items reference the four "fundamental" findings.

## 1. Design tokens — spacing/type escape the scale (partly ✅)

Color/motion/radius flow through tokens, but spacing + font-size mostly don't:
a full `--md-sys-spacing-*` scale exists yet ~340 raw px drive padding/margin/gap
and ~80 raw px drive font-size. A global density/type change means find-and-replace,
not a token edit.

- ✅ Kept spacing utilities (`.mt-1/.mt-2/.mb-1/.mb-2/.stack`) now reference tokens;
  the 9 dead utilities (`.p-*`, `.m-*`, `.w-full`, `.mobile-only`, `.text-center`,
  `.grid-2`) were removed.
- ✅ Ratchet lint gate: `scripts/check-css-spacing.sh` (in `just check`) — the raw-px
  count may only decrease. Baseline is a large backlog by design; lower it as you migrate.
- ◻️ Migrate the ~340 remaining literals to tokens incrementally (verify in a real
  browser — ~90% already sit on the 4px grid). NB the utility numeral is a size STEP,
  not the token index (`.mt-1` == 8px == `spacing-2`); a rename would remove that
  confusion but costs ~37 call-site edits.
- ◻️ Self-host the two render-blocking Google Fonts `@import`s.

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
  don't need it (they reference no `--dx*` tokens).

## 3. Loading / Error / Empty — no single contract (◻️)

Three loading treatments plus screens that render nothing while pending;
`unwrap_or_default()` collapses loading/error/empty into one blank render, so a
failed fetch looks like "no groups" and screens flash blank→full. No optimistic UI —
every mutation is fire-then-full-refetch.

- ◻️ A shared LEE wrapper that distinguishes `None`(→skeleton)/`Err`(→error)/
  `Some(empty)`(→orb)/`Some(data)`, rolled across home/folder/comments/social/profile.
- ◻️ Optimistic insert for high-frequency low-risk actions (comments).

## 4. Accessibility — strong, with a few sharp gaps (mostly ✅)

Semantic HTML is the norm (120 real `<button>`, `nav/main/aside`, real `<table>`), a
reused focus-trap/Escape/return-focus system, snackbar live region,
`prefers-reduced-motion`, keyboard shortcuts + skip link, ARIA combobox, WCAG-AA
contrast, status never colour-only.

- ✅ Compact nav drawer (primary phone nav) now has the focus-trap/Escape/role/
  aria-label/return-focus pattern it was missing.
- ✅ `Dialog` + user popover now carry accessible names; `aria-current` on nav;
  `aria-pressed` on editor format toggles.
- ◻️ Remaining: `aria-activedescendant` on the search combobox; SVG graph text alt;
  perm-matrix icon labels; range-slider labels; a live region for the speaker queue;
  the medium-width tree overlay's focus trap.

## Highest-risk survivor

The rich-text editor (`editor.rs` + `richtext.rs`, ~1,750 lines) rests on the
deprecated `document.execCommand`, with no autosave and no unsaved-changes guard.
It carries over on paper but is the piece most likely to need replacing in the rewrite.
