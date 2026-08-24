# Radikal Rally (`?app=game`)

A side-scrolling campaign minigame: drive the party's electric car from the
suburbs to Christiansborg, hanging election posters on the way. Every mechanic
is a Radikale Venstre position turned into play. Sprites come from the old
[radikal-game](https://github.com/noverby-legacy/radikal-game) mockup
(`assets/game/`).

## Theme → mechanic

| Politics | Mechanic |
|-|-|
| Green transition | The car is electric and the battery drains as you drive. Pick up battery power-ups, and linger in the sunbeam stretch to solar-charge. Running dry strands you: plan your charging. |
| Open borders / free movement | Boom barriers block the road. You can jump them, or grab an EU flag first, then the next barrier lifts by itself (Schengen in sprite form). Driving into one bounces you back and wastes charge. |
| LGBT+ rights | Rainbow strips painted on the road. Crossing one gives a burst of speed with a rainbow trail and confetti, at zero battery cost. |
| Campaign activity | Ten empty poster boards line the route. Hang a poster (↓ / E) while passing within range; the HUD counts n/10. You may double back for missed boards, if you can afford the charge. |
| Parliament | Christiansborg closes the course on the right. Arriving triggers the election result. |

## Ending tiers

- 8-10 posters: landslide. Fireworks over Christiansborg, both flags up.
- 4-7 posters: elected, modest cheer.
- 0-3 posters: below the threshold (spærregrænsen), grey and quiet.
- Battery dead before arriving: stranded; the overlay tells you to charge
  smarter and offers a retry.

## Controls

Keyboard: →/D drive, ←/A reverse, ↑/W/Space jump, ↓/E/Enter hang a poster.
Touch: on-screen buttons (◀ ▶ left, jump + poster right). Enter or the overlay
button starts and restarts.

## Architecture

`src/components/game.rs` holds a pure `World` (fixed course layout, physics,
battery, events) that is unit-tested on the host, and a thin wasm layer:
canvas-2D renderer, image cache, input, and the `GameApp` component. The app is
node-independent (like `?app=feedback`) so `/?app=game` works signed out, and
it is deliberately absent from the app rail, like the `cow` easter egg.

## Ideas not built yet

- Education pickup (books) that briefly doubles poster-hanging range: better
  argued, further reached.
- A fossil lobby truck that overtakes and drops oil slicks (skid, steering
  briefly reversed).
- Weather: headwind that raises drain, tailwind from the windmill cluster.
- A ferry/bridge segment where the EU flag also waives the toll.
- Two-player poster race on one keyboard.
