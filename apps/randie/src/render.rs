//! Drawing the room, the drone and the drone's own map onto a canvas.
//!
//! This is what the C's `SdlRenderer` did, in the same colours: a white floor,
//! black walls, blue windows, green sensor beams and a black drone. Everything
//! that was drawn as text there -- the readouts down the right-hand side -- is
//! DOM here instead, which is why there is no font work in this file.

// Canvas coordinates are floats; the casts turn counts and indices into them.
#![allow(clippy::cast_precision_loss)]

use randie_firmware::laser::LASER_MAX_DISTANCE_CM;
use randie_firmware::map::{FieldState, MAP_HEIGHT, MAP_WIDTH, Map};
use randie_sim::block::{Block, BlockType};
use randie_sim::{Simulator, Vec2};
use web_sys::CanvasRenderingContext2d;

use crate::camera::Camera;

/// The room's floor.
const WHITE: &str = "#ffffff";
/// Walls, the drone, and the grid.
const BLACK: &str = "#000000";
/// Windows.
const BLUE: &str = "#0000ff";
/// Sensor beams.
const GREEN: &str = "#00b000";
/// The origin's grid lines, and the drone on the minimap.
const RED: &str = "#ff0000";
/// Fields the drone has flown over.
const VISITED: &str = "#00c853";

/// How faint the grid's ordinary lines are.
const GRID_LINE: &str = "rgba(0, 0, 0, 0.12)";
/// How faint the grid's metre lines are.
const GRID_METRE: &str = "rgba(0, 0, 0, 0.35)";

/// Below this many pixels to the block, the fine grid is not drawn: it would
/// be a solid wash.
const MIN_GRID_PIXELS: f64 = 5.0;

/// How much of the canvas's height the minimap takes.
const MINIMAP_FRACTION: f64 = 0.3;
/// How far the minimap sits from the canvas's edge, in pixels.
const MINIMAP_MARGIN: f64 = 12.0;

/// Draws one frame.
pub fn draw(
    ctx: &CanvasRenderingContext2d,
    sim: &Simulator,
    camera: &Camera,
    width: f64,
    height: f64,
) {
    ctx.set_fill_style_str(WHITE);
    ctx.fill_rect(0.0, 0.0, width, height);

    draw_grid(ctx, camera, width, height);
    draw_blocks(ctx, &sim.blocks, camera);
    draw_drone(ctx, sim, camera);
    draw_scale(ctx, camera, width, height);
    draw_minimap(ctx, &sim.drone.firmware.map, sim, width, height);
}

/// The block grid, with a heavier line every metre and a red one at the room's
/// origin.
///
/// The C stepped the grid across the window in whole blocks from the top-left
/// pixel, so the lines only lined up with the blocks when the view happened to
/// be panned to a multiple of the block size. These are drawn at the block
/// boundaries themselves.
fn draw_grid(ctx: &CanvasRenderingContext2d, camera: &Camera, width: f64, height: f64) {
    let spacing = camera.scale(Block::SIZE);
    let top_left = camera.to_world(0.0, 0.0);
    let bottom_right = camera.to_world(width, height);

    ctx.set_line_width(1.0);

    let fine = spacing >= MIN_GRID_PIXELS;
    let first_column = (top_left.x / Block::SIZE).floor() as i64;
    let last_column = (bottom_right.x / Block::SIZE).ceil() as i64;

    for column in first_column..=last_column {
        let x = column as f64 * Block::SIZE;
        let metre = column % 4 == 0;

        if !fine && !metre {
            continue;
        }

        ctx.set_stroke_style_str(match (x == 0.0, metre) {
            (true, _) => RED,
            (false, true) => GRID_METRE,
            (false, false) => GRID_LINE,
        });

        let (screen_x, _) = camera.to_screen(Vec2::new(x, 0.0));
        line(ctx, screen_x, 0.0, screen_x, height);
    }

    let first_row = (bottom_right.y / Block::SIZE).floor() as i64;
    let last_row = (top_left.y / Block::SIZE).ceil() as i64;

    for row in first_row..=last_row {
        let y = row as f64 * Block::SIZE;
        let metre = row % 4 == 0;

        if !fine && !metre {
            continue;
        }

        ctx.set_stroke_style_str(match (y == 0.0, metre) {
            (true, _) => RED,
            (false, true) => GRID_METRE,
            (false, false) => GRID_LINE,
        });

        let (_, screen_y) = camera.to_screen(Vec2::new(0.0, y));
        line(ctx, 0.0, screen_y, width, screen_y);
    }
}

/// The room itself.
fn draw_blocks(ctx: &CanvasRenderingContext2d, blocks: &[Block], camera: &Camera) {
    let size = camera.scale(Block::SIZE);

    for block in blocks {
        ctx.set_fill_style_str(match block.kind {
            BlockType::Air => continue,
            BlockType::Wall => BLACK,
            BlockType::Window => BLUE,
        });

        // The canvas's origin is the top-left corner, and the room's y grows
        // the other way, so a block is drawn from its top-left corner.
        let (x, y) = camera.to_screen(Vec2::new(block.min.x, block.max.y));
        ctx.fill_rect(x, y, size, size);
    }
}

/// The drone, and where its beams reach.
///
/// The C drew all sixty beams in the same solid green, which at the sonar's
/// fifty-seven rays is a green wedge with the three laser beams lost in it.
/// The wedge is drawn faint here and the laser beams solid, so it is possible
/// to see which is which.
fn draw_drone(ctx: &CanvasRenderingContext2d, sim: &Simulator, camera: &Camera) {
    let drone = &sim.drone;

    ctx.set_line_width(1.0);
    ctx.set_stroke_style_str("rgba(0, 176, 0, 0.18)");
    for ray in &drone.sonar.rays {
        let (x1, y1) = camera.to_screen(ray.origin);
        let (x2, y2) = camera.to_screen(ray.end());
        line(ctx, x1, y1, x2, y2);
    }

    // Each laser beam is drawn as far as the firmware was told it reached, so
    // what the drone can see is what is on the screen. The C drew all sixty at
    // full length whatever came back, which puts a solid green line through
    // every wall the beam is pointed at.
    let readings = [
        drone.firmware.rep.laser.val_left,
        drone.firmware.rep.laser.val_front,
        drone.firmware.rep.laser.val_right,
    ];

    for (ray, reading) in drone.laser.rays.iter().zip(readings) {
        let hit = reading < LASER_MAX_DISTANCE_CM;

        let end = if hit {
            ray.origin + Vec2::from_polar(f64::from(reading), ray.angle)
        } else {
            ray.end()
        };

        ctx.set_stroke_style_str(if hit { GREEN } else { "rgba(0, 176, 0, 0.35)" });
        let (x1, y1) = camera.to_screen(ray.origin);
        let (x2, y2) = camera.to_screen(end);
        line(ctx, x1, y1, x2, y2);
    }

    let (x, y) = camera.to_screen(drone.pos);
    let radius = camera.scale(drone.size / 2.0);

    ctx.set_fill_style_str(BLACK);
    ctx.begin_path();
    let _ = ctx.arc(x, y, radius.max(2.0), 0.0, std::f64::consts::TAU);
    ctx.fill();

    // Which way it is pointing. The C left this to the beams.
    let nose = drone.pos + Vec2::from_polar(drone.size / 2.0, drone.angle);
    let (nose_x, nose_y) = camera.to_screen(nose);
    ctx.set_stroke_style_str(WHITE);
    ctx.set_line_width(2.0);
    line(ctx, x, y, nose_x, nose_y);
}

/// A bar showing how long a metre is at the current zoom. Down in the
/// bottom-right, which is the one corner nothing else uses.
fn draw_scale(ctx: &CanvasRenderingContext2d, camera: &Camera, width: f64, height: f64) {
    let length = camera.scale(100.0);
    let left = width - length - 24.0;
    let bottom = height - 24.0;

    ctx.set_stroke_style_str(BLACK);
    ctx.set_line_width(2.0);
    line(ctx, left, bottom, left + length, bottom);
    line(ctx, left, bottom - 5.0, left, bottom + 5.0);
    line(
        ctx,
        left + length,
        bottom - 5.0,
        left + length,
        bottom + 5.0,
    );

    ctx.set_fill_style_str(BLACK);
    ctx.set_font("12px ui-monospace, monospace");
    let _ = ctx.fill_text("1 m", left, bottom - 10.0);
}

/// The map the drone has built, in the canvas's top-right corner, with where
/// it believes itself to be marked on it.
fn draw_minimap(
    ctx: &CanvasRenderingContext2d,
    map: &Map,
    sim: &Simulator,
    width: f64,
    height: f64,
) {
    let size = height * MINIMAP_FRACTION;
    let left = width - size - MINIMAP_MARGIN;
    let top = MINIMAP_MARGIN;
    let cell = size / f64::from(MAP_WIDTH);

    ctx.set_fill_style_str("rgba(255, 255, 255, 0.92)");
    ctx.fill_rect(left, top, size, size);

    for row in 0..MAP_HEIGHT {
        for column in 0..MAP_WIDTH {
            // The map's rows run south to north and the canvas's run the other
            // way, so the highest row is drawn first.
            let field = map.read(column, MAP_HEIGHT - 1 - row);

            ctx.set_fill_style_str(match field {
                FieldState::Unvisited => continue,
                FieldState::Visited => VISITED,
                FieldState::Wall => BLACK,
                FieldState::Window => BLUE,
            });

            ctx.fill_rect(
                left + f64::from(column) * cell,
                top + f64::from(row) * cell,
                cell.ceil(),
                cell.ceil(),
            );
        }
    }

    // Where the drone believes it is, which is not where it is. A belief that
    // has drifted off the edge of the map is drawn at the edge rather than
    // outside the frame.
    let believed = sim.drone.firmware.nav.position();
    let column = believed.x.min(MAP_WIDTH - 1);
    let row = MAP_HEIGHT - 1 - believed.y.min(MAP_HEIGHT - 1);

    ctx.set_fill_style_str(RED);
    ctx.fill_rect(
        left + f64::from(column) * cell,
        top + f64::from(row) * cell,
        cell.ceil(),
        cell.ceil(),
    );

    ctx.set_stroke_style_str("rgba(0, 0, 0, 0.5)");
    ctx.set_line_width(1.0);
    ctx.stroke_rect(left, top, size, size);
}

fn line(ctx: &CanvasRenderingContext2d, x1: f64, y1: f64, x2: f64, y2: f64) {
    ctx.begin_path();
    ctx.move_to(x1, y1);
    ctx.line_to(x2, y2);
    ctx.stroke();
}
