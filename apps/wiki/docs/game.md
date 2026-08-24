# Radikal Rally (`?app=game`)

A side-scrolling campaign minigame: drive the party's electric car from the
suburbs to Christiansborg, hanging election posters on the way. Every mechanic
is a Radikale Venstre position turned into play. Sprites come from the old
[radikal-game](https://github.com/noverby-legacy/radikal-game) mockup
(`assets/game/`).

## Theme → mechanic

| Politics | Mechanic |
|-|-|
| Green transition | The car is electric and the battery drains as you drive, plus a slow idle drain so dawdling is never free. Pick up battery power-ups, and linger in a sunbeam stretch to solar-charge. Running dry strands you: plan your charging. |
| Open borders / free movement | Five boom barriers block the road. You can jump them, or grab an EU flag first, then the next barrier lifts by itself (Schengen in sprite form). Driving into one bounces you back and wastes charge. |
| LGBT+ rights | Rainbow strips painted on the road. Crossing one gives a burst of speed with a rainbow trail and confetti, at zero battery cost. |
| Campaign activity | Fourteen poster spots line the route in three flavours (see below). Hang a poster (↓ / E) once the board lights pink; the HUD counts n/14. You may double back for missed boards, if you can afford the charge. |
| Parliament | Christiansborg closes the course on the right. Arriving triggers the election result. |

## Poster spots and verticality

Reaching a spot is one rule everywhere: within `POSTER_RANGE` horizontally and
`POSTER_REACH_V` vertically of the car. Placing spots at different heights is
what turns that one rule into three challenges.

- **Roadside boards** hang from the car on the road, as before.
- **Light poles** carry a panel at ~160, only in reach near the top of a jump,
  so hanging one is a timing problem.
- **City platforms** are decks the car lands on, and a ledge spot sits up
  there, so hanging one is a landing problem. Decks are one-way: the car passes
  up through them and settles coming down, and drives underneath at road level,
  so a platform can never trap anyone. Drive off the end and you fall.

Platform tops all sit under the standing-jump apex (`JUMP_VY² / 2·GRAVITY`) so
the road can reach them. The exception is the last light pole, at 285 above a
deck at 160: the only way to it is a jump from the platform, which is the
climax of the course. Four of the ten batteries sit on decks, so the greedy
road-only line finishes but cannot afford every poster.

## Ending tiers

Thresholds are shares of the course, so they survive a change to its length.

- 85% or more of the spots (12+/14): landslide. Fireworks over Christiansborg,
  both flags up.
- 50% or more (7+/14): elected, modest cheer.
- Below that: below the threshold (spærregrænsen), grey and quiet.
- Battery dead before arriving: stranded; the overlay tells you to charge
  smarter and offers a retry.

## Controls

Keyboard: →/D drive, ←/A reverse, ↑/W/Space jump, ↓/E/Enter hang a poster.
Touch: on-screen buttons (◀ ▶ left, jump + poster right). Enter or the overlay
button starts and restarts.

## Architecture

`src/components/game.rs` holds a pure `World` (fixed course layout, physics,
platform support, battery, events) that is unit-tested on the host, and a thin wasm layer:
canvas-2D renderer, image cache, input, and the `GameApp` component. The app is
node-independent (like `?app=feedback`) so `/?app=game` works signed out, and
it is deliberately absent from the app rail, like the `cow` easter egg.

## Ideas not built yet

- Moving platforms (a bus, a delivery van) to land on, so a spot is only
  reachable in a window.
- Education pickup (books) that briefly doubles poster-hanging range: better
  argued, further reached.
- A fossil lobby truck that overtakes and drops oil slicks (skid, steering
  briefly reversed).
- Weather: headwind that raises drain, tailwind from the windmill cluster.
- A ferry/bridge segment where the EU flag also waives the toll.
- Two-player poster race on one keyboard.
