# Radikal Rally (`?app=game`)

A side-scrolling campaign minigame: drive the party's electric car from the
suburbs to Christiansborg, hanging election posters on the way. Every mechanic
is a Radikale Venstre position turned into play. Sprites come from the old
[radikal-game](https://github.com/noverby-legacy/radikal-game) mockup
(`assets/game/`); everything added since is drawn with canvas primitives.

## Theme → mechanic

| Politics | Mechanic |
|-|-|
| Green transition | The car is electric and the battery drains as you drive, plus a slow idle drain so dawdling is never free. Pick up battery power-ups, and linger in a sunbeam stretch to solar-charge. Running dry strands you: plan your charging. |
| Open borders / free movement | Five boom barriers block the road. You can jump them, or grab an EU flag first, then the next barrier lifts by itself (Schengen in sprite form). Driving into one bounces you back and wastes charge. |
| EU membership pays | A bridge toll near the end takes 18% of your charge, unless you arrive holding an EU flag, which waives it. Too flat to pay and without a flag, you are turned back to find one or the other. |
| LGBT+ rights | Rainbow strips painted on the road. Crossing one gives a burst of speed with a rainbow trail and confetti, at zero battery cost. |
| Education | A stack of books doubles your reach for ten seconds: better argued, further reached. A dashed box round the car shows the reach while it lasts. |
| Against the fossil lobby | A lorry sits in the road until you come up behind it, then bolts, spilling oil as it runs. An oil slick crosses your steering for 1.3s, so the way out of a skid is to steer into it, and sitting in the oil keeps you there. |
| Wind power | Four weather stretches. A tailwind out of the windmills pushes you along and cuts what the drive costs; a headwind does the reverse. Streaks show which is which before the battery does. |
| Campaign activity | Sixteen poster spots line the route in three flavours (see below). Hang a poster once the board lights pink; the HUD counts n/16. You may double back for missed boards, if you can afford the charge. |
| Parliament | Christiansborg closes the course on the right. Arriving triggers the election result. |

## Poster spots and verticality

Reaching a spot is one rule everywhere: within the car's horizontal and
vertical reach (`POSTER_RANGE` / `POSTER_REACH_V`, doubled while studying).
Placing spots at different heights is what turns that one rule into three
challenges.

- **Roadside boards** hang from the car on the road.
- **Light poles** carry a panel at ~160, only in reach near the top of a jump,
  so hanging one is a timing problem.
- **City platforms** are decks the car lands on, and a ledge spot sits up
  there, so hanging one is a landing problem. Decks are one-way: the car passes
  up through them and settles coming down, and drives underneath at road level,
  so a platform can never trap anyone. Drive off the end and you fall.

Two of the decks are vehicles rather than buildings: a bus and a delivery van
that slide back and forth along their stretch. A rider is carried by the deck's
own movement each tick, and the ledge spot above their route is fixed, so it is
only in reach while the vehicle happens to be passing under it.

Platform tops all sit under the standing-jump apex (`JUMP_VY² / 2·GRAVITY`) so
the road can reach them. The exception is the last light pole, at 285 above a
deck at 160: the only way to it is a jump from the platform, which is the
climax of the course. Four of the ten batteries sit on decks, so the greedy
road-only line finishes but cannot afford every poster.

## Ending tiers

Thresholds are shares of the course, so they survive a change to its length.

- 85% or more of the spots: landslide. Fireworks over Christiansborg, both
  flags up.
- 50% or more: elected, modest cheer.
- Below that: below the threshold (spærregrænsen), grey and quiet.
- Battery dead before arriving: stranded; the overlay tells you to charge
  smarter and offers a retry.

## Two-player race

The intro offers a race on one keyboard. Both cars share one `World`, so single
player is just a race with one entrant, and a board hung by one player is gone
for the other.

The race ends the moment the **first** car reaches Christiansborg, and the
winner is whoever hung more posters by then. Rushing therefore costs you: you
can end the race while behind on the only thing being scored. Neither player
can drive the other off screen, since a car more than `RACE_SPREAD` behind is
dragged along.

## Controls

One player: →/D drive, ←/A reverse, ↑/W/Space jump, ↓/S/Enter hang a poster.
In a race, player one takes the arrow keys and Enter, and player two takes A/D,
W and S. Touch: on-screen buttons drive player one (◀ ▶ left, jump + poster
right); a race needs two keyboards' worth of hands.

## Architecture

`src/components/game.rs` holds a pure `World` (course layout, physics, platform
support, battery, events) that is unit-tested on the host, and a thin wasm
layer: canvas-2D renderer, image cache, input, and the `GameApp` component. Per-
car state lives in `Car`, and `World::tick` takes one `Input` per car, so the
two-player mode needed no second code path. The app is node-independent (like
`?app=feedback`) so `/?app=game` works signed out, and it is deliberately absent
from the app rail, like the `cow` easter egg.

## Ideas not built yet

- Weather that changes mid-run rather than by place: rain that lengthens the
  stopping distance.
- A canvassing minigame at each board: hold the key to argue, and a meter
  decides whether the poster sticks.
- Ghost replay of your best run, to race yourself.
