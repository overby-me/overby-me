# Material 3 audit

An audit of the Dioxus wiki against the current Material 3 (and M3 Expressive)
guidance, 2026-07-30.

**How this was produced.** `m3.material.io` renders client-side, so a plain fetch
returns nothing but a page title. The guidance quoted here was read by rendering
each page in headless chromium and extracting the text. Pages read: window size
classes, canonical layouts (overview + supporting pane), large-screen adaptive
design, navigation rail, navigation bar, navigation drawer, tabs, FAB, FAB menu,
buttons, cards, lists, dialogs, colour roles, applying type, shape, motion, and
the M3 Expressive announcement.

Findings are ordered by what they would change for someone using the app, not by
how easy they are.

---

## What is already right

Worth stating first, because most of the system is in good shape and the
findings below are refinements rather than a rebuild.

- **Breakpoints match exactly.** `window_size.rs` splits at 600 / 840 / 1200 /
  1600, which is M3's compact / medium / expanded / large / extra-large.
- **Navigation components match their breakpoints.** A bottom bar below 600, a
  collapsed rail from 600, an expanded rail (rail + tree pane) from 1200. That is
  what the rail and bar pages ask for, including the bar's five-destination limit
  with an overflow sheet beyond it.
- **Tokens are real tokens.** 38 typescale tokens, 23 distinct colour roles in
  use, motion durations plus Expressive spring tokens split into spatial (which
  overshoot) and effects (which do not). A CSS lint fails the build on raw px
  spacing or font sizes and on undefined custom properties.
- **Accessibility fundamentals are present**: skip link, focus traps with
  return-focus on every overlay, `aria-current` on nav items, live regions for
  turn changes, and arrow-key tab navigation.

---

## 1. One pane where the guidance asks for two or three

**What M3 says.** From large-screen adaptive design: *"A product's layout should
adjust to fit each breakpoint. For example, a large window can have two panes,
while an extra-large window can have three."* The supporting-pane canonical
layout puts the supporting pane **beside** the focus pane at expanded and wider,
at a fixed 360dp, and below it at compact and medium.

**What the app does.** Every view is a single column: `.content-measure` caps the
reading column at 60rem and `margin-inline: 0 auto` pins it to the **left** of
the content pane. On a 1600px screen that leaves several hundred pixels of empty
surface to its right, used by nothing except the docked tools sheet, which only
appears at extra-large and only on views that mount one.

**Why it matters.** This is the difference between a phone layout stretched wide
and a desktop layout. Three places where the second pane is not a nicety:

- **The console.** It now uses tabs at every size, so a chair on a laptop sees
  the agenda *or* the speaker list. Mid-meeting they want both. Tabs are the
  right answer at compact; at expanded the agenda should be the focus pane with
  speakers/polls/feed in a 360dp supporting pane beside it.
- **Content and its comments.** Comments sit below a motion, so a long motion
  pushes the discussion off-screen. Comments are the textbook supporting pane:
  meaningful only in relation to the thing they are about.
- **Folder listings.** A folder and the item you picked from it is the
  list-detail layout, which the guidance treats as a separate canonical case.

**Fix.** Introduce a pane scaffold in the shell keyed on the existing size class,
and adopt it view by view. The console is the best first case: the state is
already there (the tab index becomes "which supporting pane"), and it is where
the payoff is most obvious.

**Effort.** Scaffold plus the console: a day. Each further view: hours.

## 2. The empty gutter is a symptom, not a style

`.content-measure`'s `margin-inline: 0 auto` is what created the FAB bug fixed in
`3c96ebe3`: anything anchored to the window's right edge floats in dead space far
from the content it acts on. The tools-sheet FAB still has this problem.

Either the column should be centred (so the dead space is split and no anchor is
badly wrong), or the space should be given to a supporting pane per finding 1.
The second is the better answer; the first is a two-line stopgap.

## 3. The compact drawer is the superseded pattern

**What M3 says.** The navigation drawer page now opens by steering readers away
from it: *"use an expanded navigation rail, which has mostly the same
functionality of the navigation drawer and adapts better across breakpoints"*,
and it reserves standard drawers for expanded, large and extra-large.

**What the app does.** `NavigationDrawer` is a modal drawer on compact, holding
the place tree and the account menu.

**Why it matters.** Less than it sounds. The modal expanded rail and the modal
drawer are nearly the same surface; what differs is that the rail keeps the app
destinations visible alongside the tree, which suits this app, where the rail's
apps and the drawer's tree are two different axes. It is worth reframing when the
navigation is next touched, not before.

## 4. Buttons are pre-Expressive

**What M3 says.** Expressive defines five button sizes (extra-small through
extra-large) and two shapes, with the shape morphing on press.

**What the app does.** One size at 40px min-height, plus a `.btn-sm`, all at
`corner-full`. The morph-on-press exists for the FAB and the segmented button but
not for ordinary buttons.

**Why it matters.** Cosmetic, but it is the most visible single difference
between "M3" and "M3 Expressive" at a glance, and this app has already opted into
Expressive elsewhere (spring tokens, extra-large card corners, the tab indicator).

**Fix.** Add the size scale as modifier classes and the press morph to `.btn`.
Half a day, mostly deciding which existing buttons change size.

## 5. Dialogs are narrower than spec and never go full-screen

The basic dialog is capped at `max-width: 420px`; M3's basic dialog is 560dp. More
substantially, M3 uses **full-screen dialogs on compact** for anything with a
non-trivial form, and the app has none: the editor, the invite flow and the
new-context dialogs all stay basic dialogs on a phone.

**Fix.** Raise the cap to 560px, and add a full-screen variant for the compact
breakpoint, starting with the editor.

## 6. Small conformance gaps

- **FAB margins.** M3: 16dp at compact, rising to 24dp at large and extra-large.
  The app uses 24px everywhere, so the FAB is slightly further from the corner
  than it should be on a phone.
- **Cards** use the extra-large corner where M3's default is medium. This is a
  deliberate Expressive choice and is documented in the CSS; keep it, but it is a
  divergence worth knowing about.
- **The CSS lint has no colour ratchet.** It fails the build on raw px spacing
  and font sizes, but a hard-coded `#hex` or `rgb()` passes. There are **31 raw
  colour literals** outside comments today, led by `#fff` (6) and
  `rgba(0, 0, 0, 0.15)` (5), plus a `#f44336` red that is not the error role.
  Some are legitimate (scrim and shadow alphas that no role covers); the point is
  that nothing distinguishes those from drift, and colour is exactly where drift
  hurts, since a literal cannot follow the theme into dark mode.

---

## Recommended order

1. **Pane scaffold + the console** (finding 1). The only structural item, and the
   one a chair would feel during a meeting.
2. **Colour ratchet in the CSS lint** (finding 6). Cheap, and it protects
   everything else from drifting.
3. **Dialogs** (finding 5): the 560px cap immediately, full-screen on compact
   when the editor is next touched.
4. **Button sizes and press morph** (finding 4), as a single visual pass.
5. **Content + comments as a supporting pane** (finding 1), once the scaffold
   exists.
6. **The drawer as a modal expanded rail** (finding 3), whenever navigation is
   next opened up.
