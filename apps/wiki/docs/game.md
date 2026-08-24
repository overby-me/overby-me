# Radikal Rally (`?app=game`)

A side-scrolling campaign minigame: drive the party's electric car from the
suburbs to Christiansborg, hanging election posters on the way. Every mechanic
is a Radikale Venstre position turned into play. Sprites come from the old
[radikal-game](https://github.com/noverby-legacy/radikal-game) mockup
(`assets/game/`); everything added since is drawn with canvas primitives.

A campaign is five levels, each generated when you press start, so no two
campaigns drive the same road.

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
| Campaign activity | Poster spots line the route in three flavours (see below). Hang a poster once the board lights pink; the HUD counts the level's boards and which level you are on. You may double back for missed boards, if you can afford the charge. |
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

From level four, some decks are vehicles rather than buildings: a bus that
slides back and forth along its stretch. A rider is carried by the deck's own
movement each tick, and the ledge spot above its route is fixed, so it is only
in reach while the bus happens to be passing under it.

Platform tops always sit under the standing-jump apex (`JUMP_VY² / 2·GRAVITY`)
so the road can reach them, and the generator sometimes puts a pole above a
deck, out of reach from the road: the only way to it is a jump from up there.
`generated_courses_stay_within_reach` asserts both across many seeds.

## Levels and generation

`World::generate(level, seed, players)` builds a course from a `Recipe` for
that level index and a seeded `Rng`. Nothing in the generator may touch the
clock or the platform RNG: the same seed has to replay the same five courses,
which is what makes a retry fair and the tests deterministic.

Each level index adds one kind of trouble, so the campaign teaches the course
rather than dropping everything at once. Levels also lengthen by 2100 units
each, from 8200 to 16600.

| Level | Adds |
|-|-|
| 1 | boards, barriers, static decks, sun, rainbows |
| 2 | light poles (and the odd pole stacked above a deck) |
| 3 | the fossil lobby's lorry and its oil, wind |
| 4 | moving decks, the bridge toll |
| 5 | the longest road, everything at once |

Generation runs in two passes. The **spine** walks left to right picking a
segment (`Seg`) at a time, each reporting where the next may start, which is
what keeps a random course spaced rather than piled up. The **overlay** pass
then lays resources and weather across the finished spine: batteries at
intervals (on a deck when one covers the spot, otherwise on the road), books,
wind and sun bands, and rainbow strips placed clear of any barrier, since a
boost into a closed barrier is a crash rather than a reward.

Two rules keep a random course honest:

- **Drivable.** No stretch goes further than `GROUND_REACH` without a battery
  reachable from the road, so skipping every rooftop still finishes the level.
  `every_generated_level_can_be_driven_to_the_end` proves it by running an
  autopilot over 60 generated levels.
- **Distinct.** If the dice have not rolled a feature the recipe promises by
  the time most of the course is laid, it is placed outright. Otherwise a seed
  could hand out a level four with no bus in it, indistinguishable from a
  level three.

## Ending tiers

A cleared level banks its posters and offers the next. The last level is judged
on every board of every level, as a share, so the thresholds survive the
generator changing the course length.

- 85% or more of all spots: landslide. Fireworks over Christiansborg, both
  flags up.
- 50% or more: elected, modest cheer.
- Below that: below the threshold (spærregrænsen), grey and quiet.
- Battery dead before arriving: stranded. The level restarts from the same
  seed, so it is the same road again, and the levels already cleared still
  count. A flat battery costs the level, not the campaign.

## Two-player race

The intro offers a race on one keyboard. Both cars share one `World`, so single
player is just a race with one entrant, and a board hung by one player is gone
for the other.

The race ends the moment the **first** car reaches Christiansborg, and the
winner is whoever hung more posters by then. Rushing therefore costs you: you
can end the race while behind on the only thing being scored. Neither player
can drive the other off screen, since a car more than `RACE_SPREAD` behind is
dragged along. A race is one generated level rather than a campaign.

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
