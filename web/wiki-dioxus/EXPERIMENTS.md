# M3 Expressive redesign experiments

This branch (`wiki-dioxus-complete-material-design-3-experiments`) is a
free-exploration playground: each commit is one self-contained, bold Material
Design 3 (2025 "Expressive") redesign idea for a different part of the app. They
are meant to be reviewed, kept, tweaked, or dropped individually. Nothing here
was written with the test suite in mind.

Every experiment compiles (`cargo check` + `clippy` clean) and each is its own
commit, so you can `jj` cherry-pick or drop them one at a time. Most are
CSS-only (in `assets/style.css`); a handful touch RSX.

## The invariant that shaped everything

`.card` must never get `transform` / `opacity` / `filter` / `will-change` /
`contain`: those establish a containing block for `position: fixed`, and the FAB,
tool sheets, dialogs and their scrims render *inside* cards. So card-level
"motion" is done with `box-shadow` / `border-radius` / pseudo-elements only.
Child elements of cards may transform freely.

A reusable `--card-accent` custom property was introduced on `.card` (default
primary) with `.accent-tertiary` / `.accent-secondary` / `.accent-error`
retints; several experiments build on it.

## The experiments

- **Accent-spine cards** — a tonal accent spine down each card's leading edge, a
  faint accent header wash (no divider), extra-large corners, hover elevation.
- **Morphing loader** — the spinner is now a filled primary blob that spins while
  morphing its border-radius between organic shapes.
- **Morphing nav-rail indicator** — the active rail indicator grows and morphs
  from a pill toward a rounded square with a soft primary glow.
- **Tactile ballot** — poll result bars grow in with a green→magenta gradient;
  open-poll options are bordered cards that fill on selection.
- **Spatial breadcrumb rail** — crumbs are tonal pills that spring up on hover;
  current location filled, app crumb in tertiary.
- **Expressive reading** — airier long-form type, gradient heading underlines,
  tonal blockquotes, fill-in content links.
- **Button morphs** — filled buttons lift on hover and squish to a squarer corner
  on press; a `.btn-tonal` variant.
- **Void-portal empty state** — the not-found screen gets a big floating, morphing
  tonal orb, bold title, and actions.
- **Format-aware file card** — the file card retints its accent by format and
  shows tonal metadata chips.
- **Conversational comment thread** — tertiary-tinted composer, larger glowing
  rounded-square avatars, bolder authors, hover lift.
- **Home hero** — a time-aware greeting (from the local hour) in a tonal hero
  header with an animated waving hand.
- **Unified bottom dock (compact)** — the search/breadcrumb bar and the navigation
  bar merge into ONE elevated rounded bottom surface, reclaiming the second bar's
  vertical space. (Requested.)
- **Expressive list rows + chips** — folder rows grow a leading accent bar and
  nudge on hover; chips are springy pills; the count badge is a tonal pill.
- **Expressive dialogs** — tonal circular icon badge, rise-from-below entrance,
  soft scrim backdrop blur.
- **Expressive carousel** — a wider hero lead tile, spring hover-grow, Ken-Burns
  image zoom.
- **Springy segmented control** — hover wash, press squish, bolder tonal selection.
- **Expressive text-field focus** — corner morph, 2px outline, faint primary wash.
- **Auth hero backdrop** — a soft radial brand wash behind the login/register card.
- **Tonal scrollbars** — slim theme-tinted scrollbars app-wide.
- **M3 snackbar + tooltip** — snackbar becomes a tonal surface with an accent
  spine; tooltips move to M3 tokens as an elevated bubble.
- **Framed map viewport** — the embedded map sits in a rounded tonal frame.
- **Expressive member roster** — tonal row hover, actions fade in on hover, tonal
  avatars.
- **Floating editor toolbar** — the rich-text controls sit on their own tonal bar.
- **Profile identity hero** — a large tonal avatar hero with a membership chip.
- **Bolder projector stage** — the current speaker becomes a gradient stage card
  with a glowing avatar and a display-scale name.

## Functional experiments (not just aesthetic)

These change behaviour / add capability rather than only looks:

- **Unified bottom dock (compact)** — merges two bottom bars into one, reclaiming
  vertical space (also listed above; it is as much layout as looks).
- **Back-to-top button** — a scroll-aware button that returns to the top of long
  pages; adds app-wide smooth scrolling (also smooths TOC anchor jumps).
- **Reading progress bar** — a thin top bar showing scroll progress through the
  page, sharing the back-to-top scroll listener.
- **Copy-link action** — a "Copy link" item in the document tools sheet copies the
  page URL to the clipboard and confirms with a snackbar (adds the web-sys
  `Clipboard` feature).
- **Skip-to-content link** — an a11y skip link (first focusable element, hidden
  until focused) that jumps keyboard users to `#main-content`.
- **`/` search shortcut** — bare `/` opens search (alongside Ctrl+K), suppressed
  while typing in a field or the editor.
- **Scroll-to-top on navigation** — moving to a different node returns to the top,
  keyed on the path so switching apps in place keeps your scroll position.
- **Skeleton content loading** — a content-shaped shimmer placeholder while a node
  loads, instead of a bare spinner.
- **Density toggle** — a compact/comfortable UI-density preference in the account
  menu, persisted to localStorage and reflected as `data-density` on the shell;
  compact tightens card / list / folder-row spacing.
- **Bottom-sheet drag handle** — an M3 drag handle on the compact tools bottom
  sheet (mostly cosmetic, but signals the swipe affordance).
- **TOC scroll-spy** — highlights the table-of-contents entry for the section
  currently at the top of the viewport as you scroll a document. Completes the
  long-document reading set (typography + progress bar + back-to-top + spy).

## Notes / follow-ups

- The graph SVG could get the same `.viewport-frame` as the map (left out to avoid
  a risky wrap of a long SVG block).
- `.btn-tonal` is defined but not yet applied anywhere; it's ready to use.
- The unified bottom dock is the most structural change — worth a real-device look
  at the tier heights / content padding.
