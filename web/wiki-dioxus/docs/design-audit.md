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
  confusion but costs ~37 call-site edits. **Deferred**: large, tedious, and the
  ratchet already prevents regressions; best done in browser-verified batches.
- ◻️ Self-host the two render-blocking Google Fonts `@import`s. **Deferred**: Google
  serves per-`unicode-range` woff2 subsets, so static self-hosting risks dropping the
  Danish glyphs (æ ø å) from the primary text font and breaking every Material icon —
  app-wide blast radius that needs browser verification. Do it with the full,
  unsubsetted font files and a real-browser check.

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

## Highest-risk survivor

The rich-text editor (`editor.rs` + `richtext/`, ~1,750 lines) rests on the
deprecated `document.execCommand`. It now has a debounced autosave and a
beforeunload unsaved-changes guard (so a crash/close no longer loses work), but
the `execCommand` engine itself is retained on purpose: it works in every current
browser, is isolated behind the `richtext::exec`/`query_*` seam, and the interim
Dioxus frontend will be replaced wholesale by the atproto rewrite. It carries over
on paper but is still the piece most likely to be rebuilt in that rewrite.
