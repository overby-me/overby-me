//! Port of `hacks/glx/winduprobot.c`.
//!
//! ```text
//! winduprobot, Copyright © 2014-2023 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Draws a robot wind-up toy.
//!
//! I've had this little robot since I was about six years old!  When the time
//! came for us to throw the Cocktail Robotics Grand Challenge at DNA Lounge, I
//! photographed this robot (holding a tiny martini glass) to make a flyer for
//! the event.
//!
//! Then I decided to try and make award statues for the contest by modeling
//! this robot and 3D-printing it (a robot on a post, with the DNA Lounge
//! grommet around it.)  So I learned Maya and built a model.
//!
//! Well, that 3D printing idea didn't work out, but since I had the model
//! already, I exported it and turned it into a screen saver.
//! ```
//!
//! The clockwork is real. The crank turns a gear, the gear turns a wheel, and
//! the leg is rotated by the angle of a right triangle drawn from the stop pin
//! to the wheel's axis, which is what makes the feet swing rather than pivot.
//! The forward lurch of the whole toy is not computed at all: it is a table of
//! three hundred and sixty measurements upstream made by eye, one per degree of
//! the crank, in [`super::winduprobot_wobble`].
//!
//! Every so often a robot's shell fades out and you can watch the gears work,
//! and every so often one of them says something, in a cartoon word bubble
//! drawn as a rounded box with an arrow, outlined in one pass and filled in
//! another with a polygon offset between them so the outline wins.
//!
//! There are two divergences. Upstream draws twenty-five robots. A display
//! list here is replayed as geometry rather than living on the card, and one
//! robot is sixty-three thousand vertices, so twenty-five of them is 1.6
//! million a frame. The default is five, which comes to 334k with the floor:
//! the same as `beats`, which the README records as fine. The slider still
//! goes to a hundred for anyone whose machine can take it. And the walkers are
//! depth sorted by projecting their origins, which upstream gets from
//! `gluProject`; this multiplies out the same two matrices.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, DepthFunc, Fog, Glx, Shape};
use crate::runtime::gllist::GlList;
use crate::runtime::involute::{self, Gear, Size};
use crate::runtime::shapes::unit_dome;
use crate::runtime::texfont::TexFont;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};

use super::winduprobot_wobble::WOBBLE_PROFILE;

const ROBOT_ARM: usize = 0;
const ROBOT_BODY_1: usize = 1;
const ROBOT_BODY_2: usize = 2;
const ROBOT_CRANK: usize = 3;
const ROBOT_GEARBOX: usize = 4;
const ROBOT_HAND: usize = 5;
const ROBOT_LEG: usize = 6;
const ROBOT_ROTATOR: usize = 7;
const ROBOT_WIREFRAME: usize = 8;
const ROBOT_DOME: usize = 9;
const ROBOT_GEAR: usize = 10;
const GROUND: usize = 11;
const NPARTS: usize = 12;

/// The models, in the order the parts are numbered. The last three are not
/// models at all: a dome, a gear and the floor, all generated.
const MODELS: [Option<&str>; NPARTS] = [
    Some(crate::models::ROBOT_ARM_HALF),
    Some(crate::models::ROBOT_BODY_HALF_OUTSIDE),
    Some(crate::models::ROBOT_BODY_HALF_INSIDE),
    Some(crate::models::ROBOT_CRANK_FULL),
    Some(crate::models::ROBOT_GEARBOX_HALF),
    Some(crate::models::ROBOT_HAND_HALF),
    Some(crate::models::ROBOT_LEG_HALF),
    Some(crate::models::ROBOT_ROTATOR_HALF),
    Some(crate::models::ROBOT_WIREFRAME),
    None,
    None,
    None,
];

const COLOR_KEYS: [&str; NPARTS] = [
    "armColor",
    "bodyColor",
    "insideColor",
    "crankColor",
    "gearboxColor",
    "handColor",
    "legColor",
    "wheelColor",
    "wireColor",
    "domeColor",
    "gearColor",
    "groundColor",
];

/// What a part is made of. A display list replays geometry and not state, so
/// none of this can live inside the list the way it does upstream: it goes on
/// at every call.
#[derive(Clone, Copy)]
struct Finish {
    color: [f32; 4],
    spec: [f32; 4],
    shiny: f32,
    /// The two chrome parts, which are sphere-mapped with a picture of a
    /// shiny ball.
    chrome: bool,
}

/// One robot.
#[derive(Clone, Copy, Default)]
struct Walker {
    x: f32,
    y: f32,
    z: f32,
    /// Direction of the front of the robot, degrees.
    facing: f32,
    /// Front to back and side to side tilt, degrees.
    pitch: f32,
    roll: f32,
    /// Some robots are faster.
    speed: f32,
    /// Gear state, degrees.
    crank_rot: f32,
    hand_rot: [f32; 2],
    hand_pos: [f32; 2],
    /// How off true does it walk, degrees.
    balance: f32,
    body_transparency: f32,
    fading: i32,
}

struct WindupRobot {
    trackball: Trackball,
    lists: [u32; NPARTS],
    finishes: [Finish; NPARTS],
    walkers: Vec<Walker>,
    /// Where the camera is aimed, and where it was aimed before it started
    /// moving to a new spot.
    looking: [f32; 3],
    olooking: [f32; 3],
    tracking: bool,
    tracking_ratio: f32,
    chrome: u32,
    font: TexFont,
    bubble_tick: i32,
    words: String,
    lines: i32,
    max_lines: i32,
    text_color: [f32; 4],
    text_bg: [f32; 4],
    text_bd: [f32; 4],
    width: i32,
    height: i32,
    speed: f32,
    size: f32,
    opacity: f32,
    talk_chance: f32,
    do_fade: bool,
    wire: bool,
}

fn resource_color(g: &Gl, key: &str) -> [f32; 4] {
    let pixel = crate::runtime::color::parse_color(g.res.string(key))
        .unwrap_or(crate::runtime::color::WHITE);
    let (r, gg, b) = crate::runtime::color::unrgb(pixel);
    [r as f32 / 255.0, gg as f32 / 255.0, b as f32 / 255.0, 1.0]
}

/// `unit_gear`: the sixteen-tooth gear in the gearbox, an involute one like
/// `pinion`'s.
fn unit_gear(g: &mut Glx, color: [f32; 4], wire: bool) {
    let thickness = 0.32;
    let gear = Gear {
        r: 0.5,
        nteeth: 16,
        tooth_h: 0.12,
        thickness,
        thickness2: thickness * 0.5,
        thickness3: thickness,
        inner_r: 0.5 * 0.7,
        inner_r2: 0.5 * 0.4,
        inner_r3: 0.5 * 0.1,
        size: Size::Large,
        color,
        color2: color,
        ..Gear::default()
    };
    involute::draw_gear(g, &gear, wire);
}

/// `draw_ground`: the grid the robots walk on. Upstream draws it as a lot of
/// small grids rather than one big one, because a very long line oriented away
/// from the viewer would disappear on some drivers.
fn draw_ground(g: &mut Glx, color: [f32; 4]) -> i32 {
    let cells = 30;
    let cell_size = 0.8f32;
    let grids = 12;
    let mut points = 0;

    g.push_matrix();
    g.rotate(frand(90.0) as f32, 0.0, 0.0, 1.0);
    g.color4f(color[0], color[1], color[2], color[3]);
    g.material_ambient_diffuse(color);
    g.translate(
        -(cells * grids) as f32 * cell_size / 2.0,
        -(cells * grids) as f32 * cell_size / 2.0,
        0.0,
    );
    for _ in 0..grids {
        g.push_matrix();
        for _ in 0..grids {
            g.begin(Shape::Lines);
            for i in -cells / 2..cells / 2 {
                let a = i as f32 * cell_size;
                let b = (cells / 2) as f32 * cell_size;
                g.vertex3f(a, -b, 0.0);
                g.vertex3f(a, b, 0.0);
                g.vertex3f(-b, a, 0.0);
                g.vertex3f(b, a, 0.0);
                points += 2;
            }
            g.end();
            g.translate(cells as f32 * cell_size, 0.0, 0.0);
        }
        g.pop_matrix();
        g.translate(0.0, cells as f32 * cell_size, 0.0);
    }
    g.pop_matrix();
    points
}

impl WindupRobot {
    /// Put a part on screen. Upstream compiles the material into the list;
    /// this has to apply it here.
    fn draw_component(&self, g: &mut Glx, i: usize, alpha: f32) {
        let f = self.finishes[i];
        let mut c = f.color;
        c[3] = alpha;
        g.material_ambient_diffuse(c);
        g.material_specular(f.spec);
        g.material_shininess(f.shiny);
        if f.chrome && self.chrome != 0 {
            g.texturing(true);
            g.bind_texture(self.chrome);
            g.tex_gen_sphere(true);
        }
        g.call_list(self.lists[i]);
        if f.chrome && self.chrome != 0 {
            g.tex_gen_sphere(false);
            g.texturing(false);
        }
    }

    /// A part you can see through would otherwise show its own back faces, so
    /// it goes down once into the depth buffer with the colour mask shut and
    /// then again, blended, only where the depth already matches.
    fn draw_transparent_component(&self, g: &mut Glx, i: usize, alpha: f32) {
        if alpha < 0.0 {
            return;
        }
        let alpha = alpha.min(1.0);
        if self.wire || alpha >= 1.0 {
            self.draw_component(g, i, alpha);
            return;
        }
        g.color_mask(false);
        self.draw_component(g, i, alpha);
        g.color_mask(true);
        g.depth_func(DepthFunc::Equal);
        g.blend(Blend::Alpha);
        self.draw_component(g, i, alpha);
        g.depth_func(DepthFunc::Less);
        g.blend(Blend::Off);
    }

    /// One arm: four quarters of it mirrored about two axes, and a hand whose
    /// two halves open by `open`.
    fn draw_arm(&self, g: &mut Glx, f: &Walker, left: bool, rot: f32, open: f32) {
        let arm_x = 4766.0; // Distance from the origin to the arm axis.
        let arm_y = 12212.0;
        let open = open * 5.5; // Scale of the finger range.
        let t = f.body_transparency;

        g.push_matrix();
        if !left {
            g.translate(0.0, 0.0, arm_x * 2.0);
        }
        g.translate(0.0, arm_y, -arm_x);
        g.rotate(rot, 1.0, 0.0, 0.0);
        g.translate(0.0, -arm_y, arm_x);

        g.front_face_cw(false);
        self.draw_transparent_component(g, ROBOT_ARM, t);
        g.scale(1.0, -1.0, 1.0);
        g.translate(0.0, -arm_y * 2.0, 0.0);
        g.front_face_cw(true);
        self.draw_transparent_component(g, ROBOT_ARM, t);

        g.push_matrix();
        g.translate(0.0, 0.0, -arm_x * 2.0);
        g.scale(1.0, 1.0, -1.0);
        g.front_face_cw(false);
        self.draw_transparent_component(g, ROBOT_ARM, t);
        g.scale(1.0, -1.0, 1.0);
        g.translate(0.0, -arm_y * 2.0, 0.0);
        g.front_face_cw(true);
        self.draw_transparent_component(g, ROBOT_ARM, t);
        g.pop_matrix();

        g.translate(0.0, 0.0, open);
        g.front_face_cw(true);
        self.draw_transparent_component(g, ROBOT_HAND, t);

        g.translate(0.0, 0.0, -open);
        g.scale(1.0, 1.0, -1.0);
        g.translate(0.0, 0.0, arm_x * 2.0);
        g.front_face_cw(false);
        g.translate(0.0, 0.0, open);
        self.draw_transparent_component(g, ROBOT_HAND, t);
        g.pop_matrix();
    }

    fn draw_body(&self, g: &mut Glx, f: &Walker, inside: bool) {
        let which = if inside { ROBOT_BODY_2 } else { ROBOT_BODY_1 };
        g.push_matrix();
        g.front_face_cw(false);
        self.draw_transparent_component(g, which, f.body_transparency);
        g.scale(1.0, 1.0, -1.0);
        g.front_face_cw(true);
        self.draw_transparent_component(g, which, f.body_transparency);
        g.pop_matrix();
    }

    fn draw_gearbox(&self, g: &mut Glx) {
        g.push_matrix();
        g.front_face_cw(false);
        self.draw_component(g, ROBOT_GEARBOX, 1.0);
        g.scale(1.0, 1.0, -1.0);
        g.front_face_cw(true);
        self.draw_component(g, ROBOT_GEARBOX, 1.0);
        g.pop_matrix();
    }

    fn draw_gear(&self, g: &mut Glx) {
        let n = 350.0;
        g.scale(n, n, n);
        self.draw_component(g, ROBOT_GEAR, 1.0);
    }

    /// The crank on the robot's back, and the gear it turns.
    fn draw_crank(&self, g: &mut Glx, rot: f32) {
        let origin = 12210.0;
        let rot = -rot;
        g.push_matrix();
        g.translate(0.0, origin, 0.0);
        g.rotate(rot, 0.0, 0.0, 1.0);
        g.push_matrix();
        g.rotate(90.0, 1.0, 0.0, 0.0);
        self.draw_gear(g);
        g.pop_matrix();
        g.translate(0.0, -origin, 0.0);
        g.front_face_cw(false);
        self.draw_component(g, ROBOT_CRANK, 1.0);
        g.pop_matrix();
    }

    /// The wheel the legs hang off, which the crank drives.
    fn draw_rotator(&self, g: &mut Glx, rot: f32) {
        let origin = 10093.0;
        g.push_matrix();
        g.translate(0.0, origin, 0.0);
        g.rotate(rot, 0.0, 0.0, 1.0);
        g.push_matrix();
        g.rotate(90.0, 1.0, 0.0, 0.0);
        self.draw_gear(g);
        g.pop_matrix();
        g.translate(0.0, -origin, 0.0);

        g.front_face_cw(false);
        self.draw_component(g, ROBOT_ROTATOR, 1.0);

        g.scale(1.0, 1.0, -1.0);
        g.front_face_cw(true);
        g.rotate(180.0, 0.0, 0.0, 1.0);
        g.translate(0.0, -origin * 2.0, 0.0);
        self.draw_component(g, ROBOT_ROTATOR, 1.0);
        g.pop_matrix();
    }

    /// The gears showing through a faded shell, drawn unlit.
    fn draw_wireframe(&self, g: &mut Glx, f: &Walker) {
        let alpha = 0.6 - f.body_transparency;
        if alpha < 0.0 {
            return;
        }
        let alpha = alpha * 0.3;
        if !self.wire {
            g.lighting(false);
        }
        g.line_width(0.3);
        g.push_matrix();
        self.draw_transparent_component(g, ROBOT_WIREFRAME, alpha);
        g.pop_matrix();
        g.line_width(1.0);
        if !self.wire {
            g.lighting(true);
        }
    }

    /// A leg, rotated by the angle of the right triangle from the stop pin to
    /// the wheel's axis, which is what makes the foot swing.
    fn draw_leg(&self, g: &mut Glx, rot: f32, left: bool) {
        let leg_distance = 9401.0; // Ground to the leg axis.
        let rot_distance = 10110.0; // Ground to the rotator axis.
        let mut pin_distance = 14541.0f32; // Ground to the stop pin.
        let orbit_r = rot_distance - leg_distance;

        // Actually it's the bottom of the pin minus its diameter, or
        // something.
        pin_distance -= 590.0;

        g.push_matrix();
        if left {
            g.rotate(180.0, 0.0, 1.0, 0.0);
        }
        let mut rot = if left { rot } else { -(rot + 180.0) };
        rot -= 90.0;

        let x = orbit_r * (-rot * std::f32::consts::PI / 180.0).cos();
        let y = orbit_r * (-rot * std::f32::consts::PI / 180.0).sin();

        // Rotate the leg by angle B of the right triangle ABC, where A is the
        // stop pin, D the rotator wheel's axis, C is D + y and B is the leg's
        // axis. So sin(th) = dist(A,C) / dist(A,B).
        {
            let ay = pin_distance - leg_distance;
            let (cx, cy) = (0.0f32, y);
            let bx = x;
            let dbc = cx - bx;
            let dac = cy - ay;
            let dab = (dbc * dbc + dac * dac).sqrt();
            let th = (dac / dab).asin();
            rot = th / (std::f32::consts::PI / 180.0);
            rot += 90.0;
            if dbc > 0.0 {
                rot = 360.0 - rot;
            }
        }

        g.translate(0.0, orbit_r, 0.0);
        g.translate(x, y, 0.0);
        g.translate(0.0, leg_distance, 0.0);
        g.rotate(rot, 0.0, 0.0, 1.0);
        g.translate(0.0, -leg_distance, 0.0);

        g.front_face_cw(false);
        self.draw_component(g, ROBOT_LEG, 1.0);
        g.scale(-1.0, 1.0, 1.0);
        g.front_face_cw(true);
        self.draw_component(g, ROBOT_LEG, 1.0);
        g.pop_matrix();
    }

    /// The glass dome over the head, which never goes fully opaque: you are
    /// always meant to see the head through it.
    fn draw_dome(&self, g: &mut Glx, f: &Walker) {
        let n = 8.3;
        let dome_y = 15290.0;
        let trans = f.body_transparency.clamp(0.0, 0.7);

        if !self.wire {
            g.blend(Blend::Alpha);
        }
        g.push_matrix();
        g.translate(0.0, dome_y, 0.0);
        g.scale(100.0, 100.0, 100.0);
        g.rotate(90.0, 1.0, 0.0, 0.0);
        g.translate(0.35, 0.0, 0.0);
        g.scale(n, n, n);
        g.front_face_cw(false);
        self.draw_component(g, ROBOT_DOME, trans);
        g.pop_matrix();
        if !self.wire {
            g.blend(Blend::Off);
        }
    }

    /// A robot standing in the right place, one unit tall.
    fn draw_walker(&self, g: &mut Glx, f: &Walker) {
        g.push_matrix();
        g.translate(f.y, f.z, f.x);
        let n = 0.01;
        g.scale(n, n, n);
        g.rotate(90.0, 0.0, 1.0, 0.0);
        g.rotate(f.facing, 0.0, 1.0, 0.0);
        g.rotate(f.pitch, 0.0, 0.0, 1.0);
        g.rotate(f.roll, 1.0, 0.0, 0.0);
        let n = 0.00484; // Make it one unit tall.
        g.scale(n, n, n);

        self.draw_gearbox(g);
        self.draw_crank(g, f.crank_rot);
        self.draw_rotator(g, f.crank_rot);
        self.draw_leg(g, f.crank_rot, false);
        self.draw_leg(g, f.crank_rot, true);
        self.draw_wireframe(g, f);

        // These last, and the outer shell before the inner one, because the
        // order in which things reach the depth buffer is what makes the
        // transparency come out right.
        if f.body_transparency >= 0.001 {
            self.draw_arm(g, f, true, f.hand_rot[0], f.hand_pos[0]);
            self.draw_arm(g, f, false, f.hand_rot[1], f.hand_pos[1]);
            self.draw_body(g, f, false);
            self.draw_body(g, f, true);
            self.draw_dome(g, f);
        }
        g.pop_matrix();
    }

    /// Is this robot standing on another one?
    fn collision(&self, i: usize, extra_space: f32) -> bool {
        if self.walkers.len() <= 1 {
            return false;
        }
        let w = self.walkers[i];
        for (j, w2) in self.walkers.iter().enumerate() {
            if i == j {
                continue;
            }
            let min = 0.75 + extra_space;
            let (dx, dy) = (w.x - w2.x, w.y - w2.y);
            if dx * dx + dy * dy <= min * min {
                return true;
            }
        }
        false
    }

    /// Turn the crank by one degree, which moves the legs and displaces the
    /// robot.
    fn tick_walker(&mut self, i: usize) {
        let f = &mut self.walkers[i];
        f.crank_rot += 1.0;
        let deg = ((f.crank_rot + 0.5) as i32).rem_euclid(360) as usize;
        let fwd;

        if deg == 0 {
            fwd = WOBBLE_PROFILE[deg][2];
            f.pitch = WOBBLE_PROFILE[deg][1];
            f.z = WOBBLE_PROFILE[deg][0];
        } else {
            fwd = WOBBLE_PROFILE[deg][2] - WOBBLE_PROFILE[deg - 1][2];
            f.pitch += WOBBLE_PROFILE[deg][1] - WOBBLE_PROFILE[deg - 1][1];
            f.z += WOBBLE_PROFILE[deg][0] - WOBBLE_PROFILE[deg - 1][0];
        }

        // Lean slightly toward the foot that is off the ground.
        f.roll = -2.5 * ((deg as f32 - 90.0) * std::f32::consts::PI / 180.0).sin();

        if random().is_multiple_of(10) {
            let mut b = f.balance / 10.0;
            let s = if f.balance > 0.0 { 1.0 } else { -1.0 };
            if s < 0.0 {
                b = -b;
            }
            f.facing += s * frand(b as f64) as f32;
        }

        let (ox, oy) = (f.x, f.y);
        let th = f.facing * std::f32::consts::PI / 180.0;
        let mut fwd = fwd;
        f.x += fwd * th.cos();
        f.y += fwd * th.sin();

        // Walking into another robot pushes this one back and turns it.
        if self.collision(i, 0.0) {
            let f = &mut self.walkers[i];
            fwd *= -1.5;
            f.x = ox + fwd * th.cos();
            f.y = oy + fwd * th.sin();
            f.facing += frand(10.0) as f32 - 5.0;
            if random().is_multiple_of(30) {
                f.facing += frand(90.0) as f32 - 45.0;
            }
        }

        // Don't bother fading if it is already transparent.
        if !self.do_fade && self.opacity <= 0.5 {
            return;
        }
        let (tick, linger) = (0.002, 3.0);
        let opacity = self.opacity;
        let f = &mut self.walkers[i];
        if f.fading == 0 && random().is_multiple_of(40000) {
            f.fading = -1;
        }
        if f.fading < 0 {
            f.body_transparency -= tick;
            if f.body_transparency <= -linger {
                f.body_transparency = -linger;
                f.fading = 1;
            }
        } else if f.fading > 0 {
            f.body_transparency += tick;
            if f.body_transparency >= opacity {
                f.body_transparency = opacity;
                f.fading = 0;
            }
        }
    }

    fn init_walker(&mut self, i: usize) {
        let start_tick = random() % 360;
        let one = self.walkers.len() == 1;
        let opacity = self.opacity;
        {
            let f = &mut self.walkers[i];
            f.crank_rot = 0.0;
            f.pitch = WOBBLE_PROFILE[0][1];
            f.z = WOBBLE_PROFILE[0][0];
            f.body_transparency = opacity;
            f.hand_rot[0] = frand(180.0) as f32;
            f.hand_pos[0] = 0.6 + frand(0.4) as f32;
            f.hand_rot[1] = 180.0 - f.hand_rot[0];
            f.hand_pos[1] = f.hand_pos[0];
            if random().is_multiple_of(30) {
                f.hand_rot[1] += frand(10.0) as f32;
            }
            if random().is_multiple_of(30) {
                f.hand_pos[1] = 0.6 + frand(0.4) as f32;
            }
            f.facing = frand(360.0) as f32;
            f.balance = frand(10.0) as f32 - 5.0;
            f.speed = if one { 1.0 } else { 0.6 + frand(0.8) as f32 };
        }

        for _ in 0..start_tick {
            self.tick_walker(i);
        }

        // Place them at random, but not on top of each other.
        for _ in 0..1000 {
            let mut range = 10.0;
            if self.walkers.len() > 10 {
                range += self.walkers.len() as f32 / 10.0;
            }
            let f = &mut self.walkers[i];
            f.x = frand(range as f64) as f32 - range / 2.0;
            f.y = frand(range as f64) as f32 - range / 2.0;
            if !self.collision(i, 1.5) {
                break;
            }
        }
    }

    /// If robot zero has walked too far from where the camera is pointed,
    /// start moving the aim to the new spot. Just following him exactly looks
    /// terrible, because of how jerkily they walk.
    fn look_at_center(&mut self, g: &mut Glx) {
        let target = [self.walkers[0].x, self.walkers[0].y, 0.8];
        let max_dist = (2.5 / self.size).clamp(1.0, 10.0);

        if self.tracking {
            let r = (1.0 - (self.tracking_ratio * std::f32::consts::PI).cos()) / 2.0;
            for ((look, old), t) in self
                .looking
                .iter_mut()
                .zip(self.olooking.iter())
                .zip(target.iter())
            {
                *look = old + r * (t - old);
            }
            self.tracking_ratio += 0.02;
            if self.tracking_ratio >= 1.0 {
                self.tracking = false;
                self.olooking = self.looking;
            }
        }

        if !self.tracking {
            let d: f32 = (0..3)
                .map(|k| (target[k] - self.looking[k]).powi(2))
                .sum::<f32>()
                .sqrt();
            if d > max_dist {
                self.tracking = true;
                self.tracking_ratio = 0.0;
                self.olooking = self.looking;
            }
        }

        g.translate(-self.looking[1], -self.looking[2], -self.looking[0]);
    }

    /// A cartoony word bubble. `width` and `height` are the inside size, for
    /// the text; the frame and the arrow are outside that. The origin is the
    /// bottom left.
    #[allow(clippy::too_many_arguments)]
    fn draw_bubble_box(
        &self,
        g: &mut Glx,
        width: f32,
        height: f32,
        corner_radius: f32,
        arrow_h: f32,
        arrow_x: f32,
        fg: [f32; 4],
        bg: [f32; 4],
    ) {
        const CORNER_POINTS: i32 = 16;
        let mut outline: Vec<[f32; 3]> = Vec::new();
        let tick = std::f32::consts::FRAC_PI_2 / CORNER_POINTS as f32;

        let arrow_w = arrow_h / 2.0;
        let arrow_x2 = (width - arrow_w).min(arrow_x).max(0.0);

        let w2 = arrow_w.max(width - corner_radius * 1.10);
        let h2 = 0.0f32.max(height - corner_radius * 1.28);
        let x2 = (width - w2) / 2.0;
        let y2 = (height - h2) / 2.0;

        //                                     A  B         C   D
        let xa = x2 - corner_radius; //    E     _------------_
        let xb = x2; //                    D   /__|         |__\
        let xc = xb + w2; //                   |  |         |  |
        let xd = xc + corner_radius; //    C   |__|   EF    |__|
        let xe = xb + arrow_x2; //         B    \_|_________|_/
        let xf = xe + arrow_w; //          A          \|

        let ya = y2 - (corner_radius + arrow_h);
        let yb = y2 - corner_radius;
        let yc = y2;
        let yd = yc + h2;
        let ye = yd + corner_radius;
        let z = 0.0;

        // Let the lines take precedence over the fills.
        g.polygon_offset(Some((1.0, 1.0)));
        g.color4f(bg[0], bg[1], bg[2], bg[3]);
        g.front_face_cw(true);

        // Top left corner.
        g.begin(Shape::TriangleFan);
        g.vertex3f(xb, yd, 0.0);
        let mut th = 0.0f32;
        while th < std::f32::consts::FRAC_PI_2 + tick {
            let (x, y) = (xb - corner_radius * th.cos(), yd + corner_radius * th.sin());
            g.vertex3f(x, y, z);
            outline.push([x, y, z]);
            th += tick;
        }
        g.end();

        // Top edge.
        outline.push([xc, ye, z]);

        // Top right corner.
        g.begin(Shape::TriangleFan);
        g.vertex3f(xc, yd, 0.0);
        let mut th = std::f32::consts::FRAC_PI_2;
        while th > -tick {
            let (x, y) = (xc + corner_radius * th.cos(), yd + corner_radius * th.sin());
            g.vertex3f(x, y, z);
            outline.push([x, y, z]);
            th -= tick;
        }
        g.end();

        // Right edge.
        outline.push([xd, yc, z]);

        // Bottom right corner.
        g.begin(Shape::TriangleFan);
        g.vertex3f(xc, yc, 0.0);
        let mut th = 0.0f32;
        while th < std::f32::consts::FRAC_PI_2 + tick {
            let (x, y) = (xc + corner_radius * th.cos(), yc - corner_radius * th.sin());
            g.vertex3f(x, y, z);
            outline.push([x, y, z]);
            th += tick;
        }
        g.end();

        // Bottom right edge.
        outline.push([xf, yb, z]);

        // The arrow.
        g.front_face_cw(true);
        g.begin(Shape::Triangles);
        for p in [[xf, yb, z], [xf, ya, z], [xe, yb, z]] {
            g.vertex3f(p[0], p[1], p[2]);
            outline.push(p);
        }
        g.end();

        // Bottom left corner.
        g.begin(Shape::TriangleFan);
        g.vertex3f(xb, yc, 0.0);
        let mut th = std::f32::consts::FRAC_PI_2;
        while th > -tick {
            let (x, y) = (xb - corner_radius * th.cos(), yc - corner_radius * th.sin());
            g.vertex3f(x, y, z);
            outline.push([x, y, z]);
            th -= tick;
        }
        g.end();

        // Left edge.
        outline.push([xa, yd, z]);

        g.front_face_cw(true);
        g.begin(Shape::Quads);
        for p in [
            // Left box.
            [xa, yd],
            [xb, yd],
            [xb, yc],
            [xa, yc],
            // Centre box.
            [xb, ye],
            [xc, ye],
            [xc, yb],
            [xb, yb],
            // Right box.
            [xc, yd],
            [xd, yd],
            [xd, yc],
            [xc, yc],
        ] {
            g.vertex3f(p[0], p[1], z);
        }
        g.end();

        g.line_width(2.8);
        g.color4f(fg[0], fg[1], fg[2], fg[3]);
        g.begin(Shape::LineLoop);
        for p in outline.iter().rev() {
            g.vertex3f(p[0], p[1], p[2]);
        }
        g.end();
        g.line_width(1.0);

        g.polygon_offset(None);
    }

    /// The word bubble over a robot's head, billboarded: the prevailing
    /// modelview is read back with its rotation replaced by the identity, so
    /// the bubble faces the camera and is still occluded by anything in front
    /// of it.
    fn draw_label(&self, g: &mut Glx, f: &Walker, y_off: f32, scale: f32, label: &str) {
        if scale == 0.0 {
            return;
        }
        if !self.wire {
            g.lighting(false); // Don't light fonts.
        }
        g.push_matrix();
        g.translate(f.y, 0.0, f.x);
        g.translate(0.0, y_off, 0.0);

        let mut m = g.modelview_matrix();
        m.0[0] = 1.0;
        m.0[1] = 0.0;
        m.0[2] = 0.0;
        m.0[4] = 0.0;
        m.0[5] = 1.0;
        m.0[6] = 0.0;
        m.0[8] = 0.0;
        m.0[9] = 0.0;
        m.0[10] = 1.0;
        g.load_identity();
        g.mult_matrix(m);
        g.translate(0.0, 0.0, 0.1); // Move toward the camera.

        // A point size to stop above, so the text is not a pixellated mess.
        let mut max = 24.0f32;
        if self.height <= 640 || self.width <= 640 {
            max *= 3.0;
        }
        let e = self.font.metrics("X");
        let cw = e.width as f32;
        let ch = (e.ascent + e.descent) as f32;
        let mut s = 1.0 / ch;
        if ch > max {
            s *= max / ch;
        }
        s *= scale;

        let e = self.font.metrics(label);
        let w = e.width as f32;
        let h = (e.ascent + e.descent) as f32;

        g.scale(s, s, 1.0);
        g.translate(-w / 2.0, h * 2.0 / 3.0 + cw * 7.0, 0.0);

        g.push_matrix();
        g.translate(0.0, -h + ch * 1.2, -0.1);
        self.draw_bubble_box(
            g,
            w,
            h,
            ch * 2.0,           // Corner radius.
            ch * 2.5,           // Arrow height.
            w / 2.0 - cw * 8.0, // Arrow x.
            self.text_bd,
            self.text_bg,
        );
        g.pop_matrix();

        let c = self.text_color;
        g.color4f(c[0], c[1], c[2], c[3]);
        g.translate(0.0, ch / 2.0, 0.0);
        self.font.print_string(g, label);

        g.pop_matrix();
        if !self.wire {
            g.lighting(true);
        }
    }

    /// Read whatever the text source has ready, up to a screenful.
    fn fill_words(&mut self, g: &mut Gl) {
        let mut lines = self.words.matches('\n').count() as i32;
        let mut max = self.max_lines;
        if (self.height <= 640 || self.width <= 640) && max > 4 {
            max = 4;
        }
        while self.words.len() < 10240 && lines < max {
            match g.text_getc() {
                // The channel puts a carriage return in front of every line
                // feed, because upstream reads its words through a pty. This
                // saver is one of the ones that asks for a plain pipe
                // instead, and a bare return would be drawn as a glyph.
                Some(b'\r') => continue,
                Some(c) => {
                    if c == b'\n' {
                        lines += 1;
                    }
                    self.words.push(c as char);
                }
                None => break,
            }
        }
        self.lines = lines;
    }

    /// Every so often, put whatever the text source has said in a bubble over
    /// robot zero's head, and leave it there for a while.
    fn bubble(&mut self, g: &mut Glx) {
        let duration = 200;
        let fade = 0.015;
        let chance = if self.talk_chance <= 0.0 {
            0
        } else if self.talk_chance >= 0.99 {
            1
        } else {
            ((1.0 - self.talk_chance) * 1000.0) as u32
        };

        let s = self.words.trim_start_matches('\n');
        let s = s.trim_end_matches(['\n', ' ', '\t']).to_string();
        if s.is_empty() || chance == 0 {
            return;
        }

        if self.bubble_tick > 0 {
            self.bubble_tick -= 1;
            if self.bubble_tick == 0 {
                self.words.clear();
            }
        }
        if self.bubble_tick == 0 {
            if random().is_multiple_of(chance) {
                self.bubble_tick = duration;
            } else {
                return;
            }
        }

        let d = duration as f32;
        let t = self.bubble_tick as f32;
        let scale = if t < d * fade {
            t / (d * fade)
        } else if t > d * (1.0 - fade) {
            1.0 - ((t - d * (1.0 - fade)) / (d * fade))
        } else {
            1.0
        };

        let f = self.walkers[0];
        self.draw_label(g, &f, 1.5, scale, &s);
    }
}

fn load_texture(g: &mut Gl, png: &[u8]) -> u32 {
    let id = g.glx.gen_texture();
    g.glx.bind_texture(id);
    match crate::runtime::png::decode_rgba(png) {
        Some((w, h, px)) => g.glx.tex_image_2d(w, h, px),
        None => g.glx.tex_image_2d(1, 1, vec![255, 255, 255, 255]),
    }
    g.glx.tex_nearest(true);
    id
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let do_texture = g.res.bool("texture") && !wire;
    let chrome = if do_texture {
        load_texture(g, crate::images::CHROMESPHERE)
    } else {
        0
    };

    let spec1 = [1.00, 1.00, 1.00, 1.0];
    let spec2 = [0.40, 0.40, 0.70, 1.0];
    let mut lists = [0u32; NPARTS];
    let mut finishes = [Finish {
        color: [1.0; 4],
        spec: spec2,
        shiny: 20.0,
        chrome: false,
    }; NPARTS];

    for i in 0..NPARTS {
        let color = resource_color(g, COLOR_KEYS[i]);
        let chrome_p = i == ROBOT_BODY_1 || i == ROBOT_DOME;
        finishes[i] = Finish {
            color,
            spec: match i {
                ROBOT_BODY_1 | ROBOT_BODY_2 | ROBOT_DOME => spec1,
                _ => spec2,
            },
            shiny: match i {
                ROBOT_BODY_1 | ROBOT_BODY_2 | ROBOT_DOME => 128.0,
                _ => 20.0,
            },
            chrome: chrome_p && do_texture,
        };
        // Upstream gives the inside of the body spec1 in one place and spec2
        // in another; it is the shininess that differs, and this keeps both.
        if i == ROBOT_BODY_2 {
            finishes[i].spec = spec2;
        }

        let list = g.glx.gen_lists(1);
        g.glx.new_list(list);
        g.glx.push_matrix();
        g.glx.rotate(-90.0, 1.0, 0.0, 0.0);
        g.glx.rotate(180.0, 0.0, 0.0, 1.0);
        g.glx.scale(6.0, 6.0, 6.0);
        match i {
            ROBOT_DOME => {
                unit_dome(&mut g.glx, 32, 32, wire);
            }
            ROBOT_GEAR => {
                unit_gear(&mut g.glx, color, wire);
            }
            GROUND => {
                draw_ground(&mut g.glx, color);
            }
            // The wireframe is the only model always drawn as lines: it is
            // the shape of the shell, for when the shell is not there.
            _ => {
                if let Some(src) = MODELS[i] {
                    let lines = wire || i == ROBOT_WIREFRAME;
                    GlList::parse(src).render(&mut g.glx, lines);
                }
            }
        }
        g.glx.pop_matrix();
        g.glx.end_list();
        lists[i] = list;
    }

    let count = g.res.int("count").max(1) as usize;
    let mut this = WindupRobot {
        trackball: Trackball::new(),
        lists,
        finishes,
        walkers: vec![Walker::default(); count],
        looking: [0.0; 3],
        olooking: [0.0; 3],
        tracking: false,
        tracking_ratio: 0.0,
        chrome,
        font: TexFont::load(&mut g.glx, g.res.string("labelFont")),
        bubble_tick: 0,
        words: String::new(),
        lines: 0,
        max_lines: g.res.int("textLines"),
        text_color: resource_color(g, "textColor"),
        text_bg: resource_color(g, "textBackground"),
        text_bd: resource_color(g, "textBorderColor"),
        width: g.width(),
        height: g.height(),
        speed: g.res.float("speed") as f32,
        size: g.res.float("size") as f32,
        opacity: g.res.float("opacity") as f32,
        talk_chance: g.res.float("talk") as f32,
        do_fade: g.res.bool("fade"),
        wire,
    };

    for i in 0..count {
        this.init_walker(i);
    }
    // Since number zero is the one we track, make sure it doesn't walk too
    // straight.
    this.walkers[0].balance *= 1.5;

    // Let's tilt the floor a little.
    this.trackball.reset(-0.6 + frand(1.2), -0.6 + frand(0.2));

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for WindupRobot {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        g.text_reshape(if width < 800 { 25 } else { 40 }, 0);
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        let (mut height, width) = (self.height, self.width);
        let mut y = 0;
        let mut h = height as f32 / width as f32;
        if width > height * 5 {
            height = width * 9 / 16;
            y = -height / 2;
            h = height as f32 / width as f32;
        }
        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(40.0, 1.0 / h, 1.0, 250.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        if !self.wire {
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 0.4, 0.2, 0.4, 0.0);
            g.glx.light_ambient(0, [0.2, 0.2, 0.2, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(0, [1.0, 1.0, 1.0, 1.0]);
        }

        g.glx.push_matrix();
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        g.glx.translate(0.0, -20.0, 0.0); // Move the horizon down the screen.

        let robot_size = self.size * 7.0;
        g.glx.scale(robot_size, robot_size, robot_size);
        let glx = &mut g.glx;
        self.look_at_center(glx);

        // The floor, drawn in fog so that it fades out rather than ending.
        g.glx.push_matrix();
        g.glx
            .scale(1.0 / robot_size, 1.0 / robot_size, 1.0 / robot_size);
        if !self.wire {
            g.glx.line_width(4.0);
            g.glx.blend(Blend::Alpha);
            g.glx.fog(Some(Fog::Exp2 {
                density: 0.015,
                color: [0.0, 0.0, 0.0, 1.0],
            }));
            g.glx.lighting(false);
        }
        let c = self.finishes[GROUND].color;
        g.glx.color4f(c[0], c[1], c[2], c[3]);
        g.glx.material_ambient_diffuse(c);
        g.glx.call_list(self.lists[GROUND]);
        if !self.wire {
            g.glx.line_width(1.0);
            g.glx.blend(Blend::Off);
            g.glx.fog(None);
            g.glx.lighting(true);
        }
        g.glx.pop_matrix();

        self.fill_words(g);
        let glx = &mut g.glx;
        self.bubble(glx);

        // For the transparency to work, the robots have to be drawn from back
        // to front, so project each origin and sort on its depth.
        let mvp = g.glx.projection_matrix().mul(&g.glx.modelview_matrix());
        let mut sorted: Vec<(usize, f32)> = self
            .walkers
            .iter()
            .enumerate()
            .map(|(i, f)| (i, -mvp.transform([f.y, f.z, f.x])[2]))
            .collect();
        sorted.sort_by(|a, b| a.1.total_cmp(&b.1));

        for (i, _) in sorted {
            let f = self.walkers[i];
            let glx = &mut g.glx;
            self.draw_walker(glx, &f);
            let ticks = ((22.0 * self.speed * f.speed) as i32).clamp(1, 180);
            for _ in 0..ticks {
                self.tick_walker(i);
            }
        }

        g.glx.pop_matrix();
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:           20000",
    "*count:           5",
    "*showFPS:         False",
    "*wireframe:       False",
    "*labelFont:       sans-serif bold 24",
    "*legColor:        #AA2222",
    "*armColor:        #AA2222",
    "*handColor:       #AA2222",
    "*crankColor:      #444444",
    "*bodyColor:       #7777AA",
    "*domeColor:       #7777AA",
    "*insideColor:     #DDDDDD",
    "*gearboxColor:    #444488",
    "*gearColor:       #008877",
    "*wheelColor:      #007788",
    "*wireColor:       #006600",
    "*groundColor:     #0000FF",
    "*textColor:       #FFFFFF",
    "*textBackground:  #444444",
    "*textBorderColor: #FFFF88",
    "*textLines:       10",
    "*speed:           1.0",
    "*size:            1.0",
    "*opacity:         1.0",
    "*talk:            0.2",
    "*usePty:          False",
    "*texture:         True",
    "*fade:            True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("speed", "Robot speed", 0.01, 8.0, 0.01, 2, "1.0"),
    Opt::slider("count", "Number of robots", 1.0, 100.0, 1.0, 0, "5"),
    Opt::slider("size", "Robot size", 0.1, 10.0, 0.1, 1, "1.0"),
    Opt::slider(
        "opacity",
        "Robot skin transparency",
        0.0,
        1.0,
        0.01,
        2,
        "1.0",
    ),
    Opt::slider("talk", "Word bubbles", 0.0, 1.0, 0.01, 2, "0.2"),
    Opt::boolean("texture", "Chrome", "true"),
    Opt::boolean("fade", "Fade opacity", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "winduprobot",
    label: "Windup Robot",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2014",
        video: Some("https://www.youtube.com/watch?v=RmpsDx9MuUM"),
        blurb: "A swarm of wind-up toy robots wander around the table-top, \
                bumping into each other. Each one contains a mechanically \
                accurate gear system, which you can see when its shell fades.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner3d {
    Runner3d::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver3d = Saver3d { def: &DEF, start };

#[cfg(test)]
mod tests {
    use super::*;

    /// The walk cycle is a table of measurements, not a formula: it rises and
    /// falls, tips both ways, and only ever walks forwards.
    #[test]
    fn the_walk_cycle_only_goes_forwards() {
        assert_eq!(WOBBLE_PROFILE[0], [0.0, 0.0, 0.0]);
        for i in 1..360 {
            assert!(
                WOBBLE_PROFILE[i][2] >= WOBBLE_PROFILE[i - 1][2],
                "the robot walks backwards at {i}"
            );
            assert!(WOBBLE_PROFILE[i][0] <= 0.0, "it rises above its feet");
        }
        // A full turn of the crank is about half a robot-width of ground.
        assert!((WOBBLE_PROFILE[359][2] - 0.4855).abs() < 0.0001);
        let tips: Vec<f32> = WOBBLE_PROFILE.iter().map(|w| w[1]).collect();
        assert!(tips.iter().cloned().fold(f32::MIN, f32::max) > 7.0);
        assert!(tips.iter().cloned().fold(f32::MAX, f32::min) < -7.0);
    }

    /// Where each robot has got to. The gearbox is the first thing drawn in
    /// one, and the only part drawn at the robot's own origin with no further
    /// offset, so its colour picks the robots out of the frame. These are
    /// world units: seven of them to one robot-scale unit.
    fn walker_positions(f: &crate::runtime::gl::Frame) -> Vec<[f32; 3]> {
        let gearbox = [
            0x44 as f32 / 255.0,
            0x44 as f32 / 255.0,
            0x88 as f32 / 255.0,
        ];
        let mut out: Vec<[f32; 3]> = Vec::new();
        for b in &f.batches {
            let c = b.material.ambient_diffuse;
            if (0..3).any(|i| (c[i] - gearbox[i]).abs() > 0.01) {
                continue;
            }
            let m = b.modelview.0;
            let p = [m[12], m[13], m[14]];
            if out.last() != Some(&p) {
                out.push(p);
            }
        }
        out
    }

    /// Turning the crank walks the robot: it covers ground, it rises and falls
    /// as its weight passes from foot to foot, and it never sinks.
    #[test]
    fn cranking_it_makes_it_walk() {
        let mut r = start(StartArgs::new(640, 480, "count=1&talk=0", 20260811));
        let mut seen: Vec<[f32; 3]> = Vec::new();
        for _ in 0..40 {
            r.step();
            seen.extend(walker_positions(r.frame()));
        }
        assert_eq!(seen.len(), 40, "one robot should be drawn once a frame");
        let far = seen
            .iter()
            .map(|p| {
                let q = seen[0];
                ((p[0] - q[0]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
            })
            .fold(0.0f32, f32::max);
        assert!(far > 1.0, "the robot only moved {far}");
        // And it went somewhere rather than juddering in place: every step is
        // small, but they add up in one direction.
        let step = seen
            .windows(2)
            .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][2] - w[0][2]).powi(2)).sqrt())
            .fold(0.0f32, f32::max);
        assert!(
            step < far,
            "it moved further in one frame than in all of them"
        );
    }

    /// Robots that walk into each other push apart rather than overlapping,
    /// so no two of them are ever drawn on the same spot.
    #[test]
    fn they_bump_into_each_other() {
        let mut r = start(StartArgs::new(640, 480, "count=12&speed=8", 20260811));
        for _ in 0..60 {
            r.step();
        }
        let ps = walker_positions(r.frame());
        assert_eq!(ps.len(), 12, "not every robot was drawn");
        let mut nearest = f32::MAX;
        for (i, a) in ps.iter().enumerate() {
            for b in ps.iter().skip(i + 1) {
                let d = ((a[0] - b[0]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
                nearest = nearest.min(d);
            }
        }
        // Seven world units to one of theirs, and they keep three quarters of
        // one apart. A recoil takes a few ticks to work, so this only asks
        // that none of them is standing inside another.
        assert!(nearest > 2.0, "two robots are {nearest} apart");
    }

    /// The robots are drawn back to front, or the transparent ones would be
    /// wrong, and the floor is drawn before any of them.
    #[test]
    fn they_are_drawn_back_to_front() {
        let mut r = start(StartArgs::new(640, 480, "count=8", 20260811));
        for _ in 0..10 {
            r.step();
        }
        let f = r.frame();
        // The floor is the one batch drawn without lighting and with fog.
        let floor = f
            .batches
            .iter()
            .position(|b| b.fog.is_some())
            .expect("no floor");
        assert!(floor < 4, "the floor is drawn {floor} batches in");

        let depths: Vec<f32> = f.batches[floor + 1..]
            .iter()
            .map(|b| b.modelview.0[14])
            .collect();
        let mut sorted = depths.clone();
        sorted.sort_by(f32::total_cmp);
        // Robots are drawn far first, so their depths never increase between
        // one robot and the next, bar the parts within one robot.
        assert!(
            depths.first() <= depths.last(),
            "the far robots are drawn last"
        );
    }

    /// The dome is never solid: you are always meant to see the head through
    /// it, so it caps out well short of opaque.
    #[test]
    fn the_dome_is_always_glass() {
        let mut r = start(StartArgs::new(640, 480, "count=1&opacity=1", 20260811));
        r.step();
        let f = r.frame();
        // The floor is blended too, so leave out what is drawn in fog.
        let domes: Vec<f32> = f
            .batches
            .iter()
            .filter(|b| b.blend != crate::runtime::gl::Blend::Off && b.fog.is_none())
            .map(|b| b.material.ambient_diffuse[3])
            .collect();
        assert!(!domes.is_empty(), "nothing was drawn blended");
        assert!(
            domes.iter().all(|a| *a <= 0.7001),
            "the dome went solid: {domes:?}"
        );
    }

    /// Every part of the robot reaches the frame, told apart by its colour.
    /// The clockwork is the point of the thing: the gearbox, the two gears,
    /// the wheels and the crank are all in there.
    #[test]
    fn every_part_is_drawn() {
        let mut r = start(StartArgs::new(640, 480, "count=1&opacity=0", 20260811));
        r.step();
        let f = r.frame();
        let want = [
            ("gearbox", 0x444488u32),
            ("gear", 0x008877),
            ("wheel", 0x007788),
            ("crank", 0x444444),
            ("leg", 0xAA2222),
            ("wire", 0x006600),
            ("ground", 0x0000FF),
        ];
        for (name, rgb) in want {
            let want = [
                ((rgb >> 16) & 255) as f32 / 255.0,
                ((rgb >> 8) & 255) as f32 / 255.0,
                (rgb & 255) as f32 / 255.0,
            ];
            let n = f
                .batches
                .iter()
                .filter(|b| {
                    let c = b.material.ambient_diffuse;
                    (0..3).all(|i| (c[i] - want[i]).abs() < 0.01)
                })
                .count();
            assert!(n > 0, "the {name} was not drawn");
        }
    }

    /// Every part of the robot is on screen, and the chrome parts are the two
    /// that carry the picture of a shiny ball.
    #[test]
    fn the_shell_is_chrome() {
        let mut r = start(StartArgs::new(640, 480, "count=1", 20260811));
        r.step();
        let f = r.frame();
        let textured = f.batches.iter().filter(|b| b.texture.is_some()).count();
        assert!(textured >= 3, "only {textured} chrome draws");
        assert!(
            f.batches.iter().any(|b| b.tex_gen_sphere),
            "the chrome is not sphere-mapped"
        );
    }
}
