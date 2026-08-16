# Colour audit against Material 3

2026-07-31. Read against the current guidance at `m3.material.io/styles/color/roles`
and the M3 Expressive introduction, both fetched and read rather than recalled.
Every number below is measured from `assets/m3-theme.css` and `assets/style.css`,
not estimated.

## The rules being tested

M3 states them plainly, so they are checkable:

- **Containers are fills.** "Container – Roles used as a fill color for foreground
  elements like buttons. They should not be used for text or icons."
- **On-roles pair with their parent.** `on primary` on `primary`,
  `on primary container` on `primary container`. Pairs are guaranteed ≥3:1.
- **on surface / on surface variant** work on any surface or surface-container.
- **Improper pairings break under user contrast settings**, not necessarily today:
  "Improper color mappings can produce unintended visual results and break
  accessibility … as the contrast level changes."
- **outline** is for important boundaries (a text-field border, a target).
  **outline variant** is for decorative lines — dividers, and containers holding
  multiple elements. M3 gives both as explicit "Don't"s.

## What is right

**The scheme itself.** Generated from the two brand seeds by
`scripts/gen-theme.ts`, and every canonical pair clears 4.5:1 — well past the 3:1
M3 guarantees — in both themes:

| pair | light | dark |
|-|-|-|
| on-primary / primary | 6.47 | 7.70 |
| on-primary-container / primary-container | 13.28 | 7.26 |
| on-secondary-container / secondary-container | 13.28 | 7.24 |
| on-tertiary-container / tertiary-container | 13.30 | 7.25 |
| on-error-container / error-container | 13.26 | 5.51 |
| on-surface / surface | 16.78 | 13.30 |
| on-surface-variant / surface-variant | 7.23 | 5.49 |

**No container role is used as a text or icon colour anywhere.** Zero hits across
the whole stylesheet — the rule most often broken, and this app does not break it.

**State layers are exactly M3**: hover 0.08, focus 0.10, pressed 0.10, dragged
0.16, applied through one `.state-layer` mixin rather than ad-hoc opacities.

**The disabled recipe is exactly M3**: 12% on-surface fill, 38% on-surface content.

**The raw colour literals are justified.** Of the 31 the CSS gate counts, the ones
inspected are: white text over photo scrims and gradients (no role exists for
"on top of an arbitrary photograph"), `rgba` shadow tints, a decorative spectrum
gradient, and `@media print` (where the theme does not apply and paper is white).
None of them is a themed surface in disguise.

## What is wrong

### 1. The snackbar ignores its own tokens

`--md-snackbar`, `--md-on-snackbar` and `--md-snackbar-action` are defined as
`inverse-surface`, `inverse-on-surface` and `inverse-primary` — the correct M3
mapping, which is what makes a snackbar read as a transient system message
rather than as another card. They are used **zero times**. `.snackbar` paints
itself `surface-container-high` / `on-surface` instead, which is the same family
as everything it floats over.

Three dead tokens is the tell. Either use them or delete them; I would use them.

### 2. `error` used as text on `error-container`

`.status-banner.is-negative` and `.empty-state-orb.error-orb` put `error` on
`error-container`. It passes today — 5.00:1 light, 5.51:1 dark — but the paired
role gives 13.26:1 in light for the same design, and this is precisely the
mapping M3 warns collapses when a user turns contrast up. Use
`on-error-container`.

### 3. Six `outline` uses that should be `outline-variant`

Of 13 uses, six are boundaries M3 names in its "Don't" list:

| where | what it is |
|-|-|
| `.amendment-item` | a divider (`border-bottom`) |
| `.search-results` | a container of many elements |
| `.user-menu-dropdown` | a container of many elements |
| `.author-suggestions` | a container of many elements |
| `.link-popover` | a container of many elements |
| `.rich-editor blockquote` | decorative rule |

The other seven are correct: text fields, the editor surface, and a hover
boundary on a ballot option, which are exactly what `outline` is for.

### 4. Two navigation surfaces are swapped relative to M3

| component | app | M3 default |
|-|-|-|
| nav drawer | `surface` | `surface-container-low` |
| nav rail | `surface-container` | `surface` |
| top app bar | `surface-container` | `surface`, `surface-container` once scrolled |
| tools sheet | `surface-container-high` | `surface-container-low` |
| dialog | `surface-container-high` | `surface-container-high` ✓ |
| nav bar | `surface-container` | `surface-container` ✓ |

The drawer is the one that costs something: a navigation area painted the same
colour as the body it covers. The rail at `surface-container` actually follows
the roles page ("surface for a background area and surface container for a
navigation area") even though it differs from the component default, so I would
leave it. The app bar is a deliberate always-tonal choice; M3's scroll-linked
version would be an improvement rather than a correction.

### 5. Secondary is nearly unused

Counted across the stylesheet: **primary 84**, tertiary 34, error 13,
**secondary 6**. M3 assigns secondary to "less prominent components … like
filter chips" and the selected state of navigation. Here almost everything
emphatic is primary and almost everything accented is tertiary, so the middle
tier of the hierarchy is missing: there is *emphatic* and there is *quiet*, with
little in between. The filter chips, the segmented buttons and the selected rail
destination are the obvious candidates.

This is not an accessibility fault; it is a flatter hierarchy than the system is
built to give, and it is the same point M3 Expressive makes in its second tactic
("use contrast between primary, secondary, and tertiary color roles to prioritize
actions").

## Worth doing, in order

1. Snackbar onto the inverse roles it already defines (small, and the most
   clearly wrong thing here).
2. `on-error-container` for the two error surfaces (two lines).
3. The six `outline` → `outline-variant` boundaries.
4. Nav drawer to `surface-container-low`.
5. Give secondary a job: filter chips, segmented buttons, selected rail item.

## Notes from M3 Expressive, for future tasks

Not colour, but read while there and worth recording. The Expressive update adds
fourteen new or updated components; four of them map onto things this app has
hand-rolled:

- **Toolbars (new component).** The editor's formatting bar is exactly this, built
  ad hoc as a tonal pill. Adopting the spec would also settle the docked/floating
  question the sticky-toolbar bug came out of.
- **Split button (new).** "Project" on the console wants precisely this: a primary
  action plus a menu of where to send it.
- **Button groups (new).** The ballot's option rows and the console's view
  switches are groups pretending to be loose buttons.
- **Loading indicator (new).** Replaces the spinner with the expressive
  indeterminate indicator; the app currently has three spinner sizes.

Also noted:

- **Emphasized type styles.** The scale gained emphasized variants meant for
  headlines and key actions. The projector is the obvious place: it is read from
  across a hall and currently leans on size alone.
- **The 35-shape library and shape morph.** The empty-state orb already morphs;
  avatars and the projector's current-speaker chip could carry a real shape from
  the library rather than a hand-written border-radius.
- **Motion-physics springs.** Already adopted for the spatial cases. The effects
  springs (colour and opacity) are not — every colour transition in the app is
  still `easing-standard`.
- **The research claim worth knowing**: M3E reports users spotting key UI
  elements up to four times faster in expressive screens, from 46 studies. That
  is the argument for spending effort on hierarchy rather than on decoration, and
  it supports fixing the secondary-role gap above before adding anything new.
