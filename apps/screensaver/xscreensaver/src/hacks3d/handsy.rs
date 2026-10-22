//! Port of `hacks/glx/handsy.c`.
//!
//! ```text
//! handsy, Copyright © 2018-2025 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//! ```
//!
//! A pair of hands standing over a fog-bound grid, signing at each other: a
//! wave, applause, a thumbs-up, a walk on two fingers, and a fair amount of
//! abuse.
//!
//! There is no skeleton solver here. A hand is eight modelled bones and a list
//! of angles saying how far each joint is bent, and an animation is a list of
//! those poses with how long to take reaching each one. The poses were made by
//! hand, in the saver's own debug mode, and live in [`super::handsy_anim`].
//!
//! The two hands run separate animations, because half of what they do is a
//! conversation: one waves while the other stays hidden, one holds up the
//! scissors while the other holds up the paper. The right hand can run a beat
//! behind the left, which is what makes applause sound like applause rather
//! than one loud clap.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Fog, Shape};
use crate::runtime::gllist::GlList;
use crate::runtime::rotator::Rotator;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};
use std::f64::consts::PI;

use super::handsy_anim::{ALL_HAND_ANIMS, GOATSE_ANIM, HIDDEN_ANIM, Hand, HandAnim};

const FINGER_DISTAL: usize = 0;
const FINGER_INTERMEDIATE: usize = 1;
const FINGER_PROXIMAL: usize = 2;
const FINGER_METACARPAL: usize = 3;
const THUMB_DISTAL: usize = 4;
const THUMB_PROXIMAL: usize = 5;
const THUMB_METACARPAL: usize = 6;
const PALM: usize = 7;
const GROUND: usize = 8;

/// The eight bones, in the order the constants above give. Each is half a
/// bone: the other half is the same model mirrored in Y, which is why a
/// finger is drawn twice.
const MODELS: [&str; 8] = [
    crate::models::HANDSY_MODEL_FINGER_DISTAL,
    crate::models::HANDSY_MODEL_FINGER_INTERMEDIATE,
    crate::models::HANDSY_MODEL_FINGER_PROXIMAL,
    crate::models::HANDSY_MODEL_FINGER_METACARPAL,
    crate::models::HANDSY_MODEL_THUMB_DISTAL,
    crate::models::HANDSY_MODEL_THUMB_PROXIMAL,
    crate::models::HANDSY_MODEL_THUMB_METACARPAL,
    crate::models::HANDSY_MODEL_PALM,
];

/// How far one joint can bend, in radians.
struct Joint {
    min: f64,
    max: f64,
}

const fn j(min: f64, max: f64) -> Joint {
    Joint { min, max }
}

/// A digit: three bendable bones, a fourth that never moves, and how far the
/// whole thing can be spread from its neighbour.
struct Finger {
    bones: [Joint; 4],
    base: Joint,
}

/// `hand_geom`: the position and extent of the various joints.
struct HandGeom {
    fingers: [Finger; 5],
    /// The wrist bending up and down.
    palm: Joint,
    /// The wrist bending side to side.
    wrist1: Joint,
    /// The wrist turning over.
    wrist2: Joint,
}

/// The one hand this saver knows how to draw. A thumb bends further and
/// spreads a great deal further than a finger; a finger bends back a little
/// past straight and spreads hardly at all.
const HUMAN_HAND: HandGeom = HandGeom {
    fingers: [
        // Thumb: distal, proximal, metacarpal, none.
        Finger {
            bones: [j(0.0, 1.6), j(0.0, 1.6), j(0.0, 1.6), j(0.0, 0.0)],
            base: j(-1.70, 0.00),
        },
        // Index, middle, ring, pinky: distal, intermediate, proximal, and a
        // metacarpal that is part of the palm and does not move.
        Finger {
            bones: [j(-0.2, 1.6), j(-0.2, 1.6), j(-0.2, 1.6), j(0.0, 0.0)],
            base: j(-0.25, 0.25),
        },
        Finger {
            bones: [j(-0.2, 1.6), j(-0.2, 1.6), j(-0.2, 1.6), j(0.0, 0.0)],
            base: j(-0.25, 0.25),
        },
        Finger {
            bones: [j(-0.2, 1.6), j(-0.2, 1.6), j(-0.2, 1.6), j(0.0, 0.0)],
            base: j(-0.25, 0.25),
        },
        Finger {
            bones: [j(-0.2, 1.6), j(-0.2, 1.6), j(-0.2, 1.6), j(0.0, 0.0)],
            base: j(-0.25, 0.25),
        },
    ],
    palm: j(-0.7, 1.5),
    wrist1: j(-PI, PI),
    wrist2: j(-PI, PI),
};

fn constrain_joint(v: f64, limit: &Joint) -> f64 {
    v.clamp(limit.min, limit.max)
}

/// How far every joint of one hand is bent right now, where the hand is, and
/// how solid it is. This is upstream's `hand`, and the poses in
/// [`super::handsy_anim`] are the same thing with no alpha.
#[derive(Clone, Copy)]
struct Pose {
    /// Five digits, four bones each, thumb first.
    joint: [[f64; 4]; 5],
    /// How far each digit is spread from the next.
    base: [f64; 5],
    /// Up and down, side to side, and the twist.
    wrist: [f64; 3],
    pos: [f64; 3],
    /// Whether this is a left hand. Only the flag on `current` is ever read,
    /// and nothing interpolates it, so it is fixed from the moment the hand
    /// is made.
    sinister: bool,
    alpha: f64,
}

impl Pose {
    fn of(h: &Hand) -> Self {
        Self {
            joint: h.joint,
            base: h.base,
            wrist: h.wrist,
            pos: h.pos,
            sinister: h.sinister,
            alpha: 0.0,
        }
    }
}

/// One hand: where it started this step, where it is going, and where it is.
struct Hands {
    from: Pose,
    to: Pose,
    current: Pose,
}

/// The animation one side is running, and how far into it that side is. Both
/// left hands run the same one and so do both right hands, which is what
/// keeps a crowd of them in step.
#[derive(Default)]
struct Side {
    anim: Option<&'static [HandAnim]>,
    /// Which key frame of it.
    anim_hand: usize,
    anim_start: f64,
    tick: f64,
    /// Added to the right hand's first frame, so that the two are out of step
    /// on purpose.
    delay: f64,
}

struct Handsy {
    trackball: Trackball,
    /// The wander of the hands and the turn of the ground.
    rot: Rotator,
    /// The tilt, when the hands are meant to keep facing the camera.
    rot2: Option<Rotator>,
    spin: [bool; 3],
    lists: Vec<u32>,
    nhands: usize,
    side: [Side; 2],
    hands: Vec<Hands>,
    color: [f32; 4],
    ground_color: [f32; 4],
    /// Whether the animation now running is the one that wants a ring drawn
    /// around it.
    ringp: bool,
    speed: f64,
    face_front: bool,
    wire: bool,
    aspect: f32,
}

fn resource_color(g: &Gl, key: &str) -> [f32; 4] {
    let pixel = crate::runtime::color::parse_color(g.res.string(key))
        .unwrap_or(crate::runtime::color::WHITE);
    let (r, gg, b) = crate::runtime::color::unrgb(pixel);
    [r as f32 / 255.0, gg as f32 / 255.0, b as f32 / 255.0, 1.0]
}

impl Handsy {
    /// One bone, and then the same bone mirrored: the models are half a
    /// finger split down the middle.
    fn draw_part(&self, g: &mut Gl, i: usize) {
        g.glx.front_face_cw(false);
        g.glx.call_list(self.lists[i]);
        g.glx.push_matrix();
        g.glx.scale(1.0, -1.0, 1.0);
        g.glx.front_face_cw(true);
        g.glx.call_list(self.lists[i]);
        g.glx.pop_matrix();
    }

    /// `draw_hand`: the palm, then five digits hung off it, each bone rotated
    /// by its own joint angle and then translated to the end of itself.
    fn draw_hand(&self, g: &mut Gl, h: &Pose) {
        let off = if h.sinister { -1.0 } else { 1.0 };

        g.glx.line_width(1.0);
        g.glx.push_matrix();

        g.glx
            .translate((off * h.pos[0]) as f32, h.pos[1] as f32, h.pos[2] as f32);
        g.glx
            .rotate((h.wrist[1] * 180.0 / PI * -off) as f32, 0.0, 1.0, 0.0);
        g.glx
            .rotate((h.wrist[2] * 180.0 / PI * -off) as f32, 0.0, 0.0, 1.0);
        g.glx
            .rotate((h.wrist[0] * 180.0 / PI) as f32, 1.0, 0.0, 0.0);

        // A display list here replays geometry and not state, so the colour
        // of the hand goes on where the lists are called.
        let mut color = self.color;
        color[3] = h.alpha as f32;
        g.glx.color4f(color[0], color[1], color[2], color[3]);
        g.glx.material_ambient_diffuse(color);

        if !self.wire {
            g.glx.blend(Blend::Alpha);
        }

        g.glx.push_matrix();
        if h.sinister {
            g.glx.scale(-1.0, 1.0, 1.0);
            g.glx.front_face_cw(true);
        } else {
            g.glx.front_face_cw(false);
        }
        g.glx.call_list(self.lists[PALM]);
        g.glx.pop_matrix();

        for finger in 0..5 {
            g.glx.push_matrix();
            if finger == 0 {
                // The thumb hangs off the side of the palm and points across
                // it, and has one bone fewer than a finger: the angles used
                // are the proximal and the distal, and the spread stands in
                // for the metacarpal.
                g.glx.translate(off as f32 * 0.113, -0.033, 0.093);
                g.glx.rotate(off as f32 * 45.0, 0.0, 1.0, 0.0);
                if h.sinister {
                    g.glx.rotate(180.0, 0.0, 0.0, 1.0);
                }
                g.glx
                    .rotate((off * h.base[finger] * -180.0 / PI) as f32, 1.0, 0.0, 0.0);
                self.draw_part(g, THUMB_METACARPAL);

                g.glx.translate(0.0, 0.0, 0.1497);
                g.glx
                    .rotate((h.joint[finger][1] * -180.0 / PI) as f32, 0.0, 1.0, 0.0);
                self.draw_part(g, THUMB_PROXIMAL);

                g.glx.translate(0.0, 0.0, 0.1212);
                g.glx
                    .rotate((h.joint[finger][0] * -180.0 / PI) as f32, 0.0, 1.0, 0.0);
                self.draw_part(g, THUMB_DISTAL);
            } else {
                // Where each finger meets the palm, how far up the palm it
                // sits, and how far out from the middle it splays.
                let (across, up, splay) = match finger {
                    1 => (0.135, 0.26835, 4.0),   // index
                    2 => (0.046, 0.27152, 1.0),   // middle
                    3 => (-0.046, 0.25577, -1.0), // ring
                    _ => (-0.135, 0.22204, -4.0), // pinky
                };
                g.glx.translate(off as f32 * across, 0.004, up);
                g.glx.rotate(off as f32 * splay, 0.0, 1.0, 0.0);

                g.glx.rotate(90.0, 0.0, 0.0, 1.0);
                self.draw_part(g, FINGER_METACARPAL);

                g.glx.translate(0.0, 0.0, 0.1155);
                g.glx
                    .rotate((off * h.base[finger] * -180.0 / PI) as f32, 1.0, 0.0, 0.0);
                g.glx
                    .rotate((h.joint[finger][2] * -180.0 / PI) as f32, 0.0, 1.0, 0.0);
                self.draw_part(g, FINGER_PROXIMAL);

                g.glx.translate(0.0, 0.0, 0.1815);
                g.glx
                    .rotate((h.joint[finger][1] * -180.0 / PI) as f32, 0.0, 1.0, 0.0);
                self.draw_part(g, FINGER_INTERMEDIATE);

                g.glx.translate(0.0, 0.0, 0.1003);
                g.glx
                    .rotate((h.joint[finger][0] * -180.0 / PI) as f32, 0.0, 1.0, 0.0);
                self.draw_part(g, FINGER_DISTAL);
            }
            g.glx.pop_matrix();
        }
        g.glx.pop_matrix();

        // One animation gets a ring drawn around the hole it makes, which
        // grows and shrinks with the hand. It is drawn outside the hand's own
        // matrix, in the pair's frame.
        if h.sinister && self.ringp {
            let color = [1.0f32, 0.4, 0.4, 1.0];
            let center = 0.4f32;
            let r = (center - h.pos[0] as f32 + 0.1).max(0.22);
            g.glx.push_matrix();
            g.glx.translate(-center, -0.28, 0.5);
            g.glx
                .rotate((h.wrist[2] * 180.0 / PI * -off) as f32, 0.0, 0.0, 1.0);
            g.glx
                .rotate((h.wrist[0] * 180.0 / PI) as f32, 1.0, 0.0, 0.0);

            g.glx.color4f(color[0], color[1], color[2], color[3]);
            g.glx.material_ambient_diffuse(color);
            g.glx.lighting(false);
            g.glx.line_width(8.0);
            g.glx.begin(Shape::LineLoop);
            let mut th = 0.0f64;
            while th < PI * 2.0 {
                g.glx
                    .vertex3f(r * th.cos() as f32, r * th.sin() as f32, 0.0);
                th += PI / 180.0;
            }
            g.glx.end();
            if !self.wire {
                g.glx.lighting(true);
            }
            g.glx.pop_matrix();
        }

        g.glx.blend(Blend::Off);
    }

    /// `draw_ground`: the grid the hands stand over.
    ///
    /// Upstream draws a dozen dozen small grids rather than one big one,
    /// because iOS loses long fogged lines that lean away from the viewer.
    /// The fog, the blending and the material go on at the call site instead
    /// of here: this runs once, into a display list, and a list replays
    /// geometry and not state.
    fn draw_ground(&self, g: &mut Gl) {
        let cells = 30;
        let cell_size = 0.8f32;
        let grids = 12;

        g.glx.line_width(1.0);
        g.glx.push_matrix();
        g.glx.scale(0.2, 0.2, 0.2);
        g.glx.rotate(frand(90.0) as f32, 0.0, 0.0, 1.0);
        if !self.wire {
            g.glx.line_width(4.0);
        }

        let c = self.ground_color;
        g.glx.color4f(c[0], c[1], c[2], c[3]);

        let span = cells as f32 * grids as f32 * cell_size / 2.0;
        g.glx.translate(-span, -span, 0.0);

        for _ in 0..grids {
            g.glx.push_matrix();
            for _ in 0..grids {
                g.glx.begin(Shape::Lines);
                for i in -(cells / 2)..(cells / 2) {
                    let a = i as f32 * cell_size;
                    let b = (cells / 2) as f32 * cell_size;
                    g.glx.vertex3f(a, -b, 0.0);
                    g.glx.vertex3f(a, b, 0.0);
                    g.glx.vertex3f(-b, a, 0.0);
                    g.glx.vertex3f(b, a, 0.0);
                }
                g.glx.end();
                g.glx.translate(cells as f32 * cell_size, 0.0, 0.0);
            }
            g.glx.pop_matrix();
            g.glx.translate(0.0, cells as f32 * cell_size, 0.0);
        }
        g.glx.pop_matrix();
    }

    /// `tick_hands`: pick a new animation when both sides have run out, step
    /// each side along its key frames, and slide every joint from where it
    /// was to where it is going.
    fn tick_hands(&mut self, now: f64) {
        if self.side[0].anim.is_none() && self.side[1].anim.is_none() {
            let i = random() as usize % ALL_HAND_ANIMS.len();
            let chosen = &ALL_HAND_ANIMS[i];
            for s in 0..2 {
                self.side[s] = Side {
                    anim: Some(chosen.pair[s]),
                    anim_hand: 0,
                    anim_start: now,
                    tick: 0.0,
                    delay: if s == 1 { chosen.delay } else { 0.0 },
                };
            }
            // Upstream compares the pointers, and so does this.
            self.ringp = std::ptr::eq(chosen.pair[0], GOATSE_ANIM);

            let side = &self.side;
            for h in &mut self.hands {
                let s = usize::from(h.from.sinister);
                h.from = h.current;
                let Some(anim) = side[s].anim else { continue };
                h.to = Pose::of(anim[0].dest);
                h.to.sinister = h.from.sinister;
                h.to.alpha = if std::ptr::eq(anim, HIDDEN_ANIM) {
                    0.0
                } else {
                    1.0
                };
                // A key frame can shift and turn the whole hand as well.
                for k in 0..3 {
                    h.to.wrist[k] += anim[0].rot[k];
                    h.to.pos[k] += anim[0].pos[k];
                }
            }
        }

        for s in 0..2 {
            // Done with this hand, but not with the other.
            let Some(anim) = self.side[s].anim else {
                continue;
            };
            let frame = &anim[self.side[s].anim_hand];

            let elapsed = now - self.side[s].anim_start;
            let duration = frame.duration / self.speed;
            let duration2 = duration + (self.side[s].delay + frame.pause) / self.speed;

            if elapsed > duration2 && self.side[s].tick >= 1.0 {
                // Done animating and pausing, and the last frame is painted.
                self.side[s].anim_hand += 1;
                self.side[s].tick = 1.0;
                if self.side[s].anim_hand >= anim.len() {
                    self.side[s].anim = None;
                    for h in &mut self.hands {
                        if usize::from(h.from.sinister) == s {
                            h.to = h.current;
                            h.from = h.current;
                        }
                    }
                } else {
                    let jf = &anim[self.side[s].anim_hand];
                    let hidden = std::ptr::eq(anim, HIDDEN_ANIM);
                    for h in &mut self.hands {
                        if usize::from(h.current.sinister) != s {
                            continue;
                        }
                        h.from = h.current;
                        h.to = Pose::of(jf.dest);
                        h.to.alpha = if hidden { 0.0 } else { 1.0 };
                        for k in 0..3 {
                            h.to.wrist[k] += jf.rot[k];
                            h.to.pos[k] += jf.pos[k];
                        }
                    }
                    self.side[s].anim_start = now;
                    self.side[s].tick = 0.0;
                    self.side[s].delay = 0.0;
                }
            } else if elapsed > duration {
                // Done animating, still pausing.
                self.side[s].tick = 1.0;
            } else {
                self.side[s].tick = elapsed / duration;
            }
            self.side[s].tick = self.side[s].tick.min(1.0);

            // Move the joints into position: `current` sits between `from`
            // and `to` by the ratio `tick`.
            let tick = self.side[s].tick;
            let geom = &HUMAN_HAND;
            for h in &mut self.hands {
                if usize::from(h.current.sinister) != s {
                    continue;
                }
                for (jj, finger) in geom.fingers.iter().enumerate() {
                    for (k, bone) in finger.bones.iter().enumerate() {
                        h.current.joint[jj][k] = constrain_joint(
                            h.from.joint[jj][k] + tick * (h.to.joint[jj][k] - h.from.joint[jj][k]),
                            bone,
                        );
                    }
                    h.current.base[jj] = constrain_joint(
                        h.from.base[jj] + tick * (h.to.base[jj] - h.from.base[jj]),
                        &finger.base,
                    );
                }
                for (k, limit) in [&geom.palm, &geom.wrist1, &geom.wrist2]
                    .into_iter()
                    .enumerate()
                {
                    h.current.wrist[k] = constrain_joint(
                        h.from.wrist[k] + tick * (h.to.wrist[k] - h.from.wrist[k]),
                        limit,
                    );
                }
                for k in 0..3 {
                    h.current.pos[k] = h.from.pos[k] + tick * (h.to.pos[k] - h.from.pos[k]);
                }
                h.current.alpha = h.from.alpha + tick * (h.to.alpha - h.from.alpha);
            }
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let speed = g.res.float("speed");
    let spin = g.res.string("spin").to_string();
    let axis = |c: char, d: char| spin.contains(c) || spin.contains(d);
    let spin = [axis('x', 'X'), axis('y', 'Y'), axis('z', 'Z')];

    let mut nhands = g.res.int("count").max(1) as usize;
    if nhands % 2 == 1 {
        // An even number: they come in pairs.
        nhands += 1;
    }

    let spin_speed = 0.5 * speed;
    let wander_speed = 0.005 * speed;
    let tilt_speed = 0.001 * speed;
    let face_front = g.res.bool("faceFront");

    let mut this = Handsy {
        trackball: Trackball::new(),
        rot: Rotator::new(
            if spin[0] { spin_speed } else { 0.0 },
            if spin[1] { spin_speed } else { 0.0 },
            if spin[2] { spin_speed } else { 0.0 },
            0.5,
            if g.res.bool("wander") {
                wander_speed
            } else {
                0.0
            },
            false,
        ),
        rot2: face_front.then(|| Rotator::new(0.0, 0.0, 0.0, 0.0, tilt_speed, true)),
        spin,
        lists: Vec::new(),
        nhands,
        side: [
            Side {
                tick: 1.0,
                ..Side::default()
            },
            Side {
                tick: 1.0,
                ..Side::default()
            },
        ],
        hands: Vec::new(),
        color: [0.0; 4],
        ground_color: [0.0; 4],
        ringp: false,
        speed,
        face_front,
        wire,
        aspect: 1.0,
    };
    this.color = resource_color(g, "foreground");
    this.ground_color = resource_color(g, "groundColor");

    // The pose each side starts in is the last animation's first frame, held
    // off screen and invisible until the first animation calls for it.
    let mut def = [Pose::of(&super::handsy_anim::OPEN_PALM); 2];
    if let Some(last) = ALL_HAND_ANIMS.last() {
        for (i, d) in def.iter_mut().enumerate() {
            if let Some(f) = last.pair[i].first() {
                *d = Pose::of(f.dest);
            }
        }
    }
    for d in &mut def {
        d.pos[1] = 5.0;
        d.pos[2] = 5.0;
    }
    for i in 0..nhands {
        // Right, left, right, left: the grid draws them in pairs.
        let mut pose = def[i % 2];
        pose.sinister = i % 2 == 1;
        this.hands.push(Hands {
            from: pose,
            to: pose,
            current: pose,
        });
    }

    for src in MODELS {
        let model = GlList::parse(src);
        let list = g.glx.gen_lists(1);
        g.glx.new_list(list);
        g.glx.push_matrix();
        g.glx.scale(0.1, 0.1, 0.1);
        model.render(&mut g.glx, wire);
        g.glx.pop_matrix();
        g.glx.end_list();
        this.lists.push(list);
    }
    let ground = g.glx.gen_lists(1);
    g.glx.new_list(ground);
    this.draw_ground(g);
    g.glx.end_list();
    this.lists.push(ground);

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Handsy {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        g.glx.viewport(0, 0, width, height);
        self.aspect = width as f32 / height as f32;
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, self.aspect, 1.0, 100.0);
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
        g.glx.scale(10.0, 10.0, 10.0);

        let turning = !self.trackball.button_down();
        g.glx.mult_matrix(self.trackball.matrix());

        let (mut x, mut y, mut z);
        if self.face_front {
            // Sway a little rather than turning all the way over, so that
            // whatever the hands are saying stays legible.
            let (maxx, maxy, maxz) = (120.0 / 10.0, 55.0 / 10.0, 40.0 / 10.0);
            (x, y, z) = match &mut self.rot2 {
                Some(rot2) => rot2.position(turning),
                None => (0.0, 0.0, 0.0),
            };
            if self.spin[0] {
                g.glx.rotate((maxx / 2.0 - x * maxx) as f32, 0.0, 1.0, 0.0);
            }
            if self.spin[1] {
                g.glx.rotate((maxy / 2.0 - y * maxy) as f32, 1.0, 0.0, 0.0);
            }
            if self.spin[2] {
                g.glx.rotate((maxz / 2.0 - z * maxz) as f32, 0.0, 0.0, 1.0);
            }
        } else {
            (x, y, z) = self.rot.rotation(turning);
            g.glx.rotate((x * 360.0) as f32, 1.0, 0.0, 0.0);
            g.glx.rotate((y * 360.0) as f32, 0.0, 1.0, 0.0);
            g.glx.rotate((z * 360.0) as f32, 0.0, 0.0, 1.0);
        }

        g.glx.rotate(-70.0, 1.0, 0.0, 0.0);
        g.glx.translate(0.0, 0.0, -0.5);

        // The ground: its fog and its blending are set here rather than in
        // the list, which holds only geometry.
        g.glx.push_matrix();
        let turn = if self.spin[1] {
            y
        } else if self.spin[0] {
            x
        } else {
            z
        };
        g.glx.rotate((turn * 90.0) as f32, 0.0, 0.0, 1.0);
        g.glx.material_ambient_diffuse(self.ground_color);
        if !self.wire {
            g.glx.blend(Blend::Alpha);
            g.glx.fog(Some(Fog::Exp2 {
                density: 0.015,
                color: [0.0, 0.0, 0.0, 1.0],
            }));
        }
        g.glx.call_list(self.lists[GROUND]);
        if !self.wire {
            g.glx.blend(Blend::Off);
            g.glx.fog(None);
        }
        g.glx.pop_matrix();

        (x, y, z) = self.rot.position(turning);
        // The origin of the hands is 1.0 above the floor.
        z += 1.0;
        g.glx.translate(
            ((x - 0.5) * 0.8) as f32,
            ((y - 0.5) * 1.1) as f32,
            ((z - 0.5) * 0.2) as f32,
        );

        g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_shininess(128.0);
        g.glx.material_ambient_diffuse([0.7, 0.7, 1.0, 1.0]);

        // Lay the pairs out in a square-ish grid, keeping pairs together.
        // There are always at least two hands and always an even number, so
        // upstream's lone-hand case cannot arise.
        let (rows, cols) = if self.nhands <= 2 {
            (1, 1)
        } else {
            let rows = ((self.nhands / 2) as f64).sqrt() as usize;
            let rows = rows.max(1);
            (
                rows,
                (self.nhands as f64 / 2.0 / rows as f64).ceil() as usize,
            )
        };
        if g.width() < g.height() {
            g.glx.scale(0.5, 0.5, 0.5);
        }
        if cols > 1 {
            let s = 1.0 / rows as f32;
            g.glx.scale(s, s, s);
        }
        let s = 0.8f32;
        g.glx
            .translate(-s * rows as f32 * 1.5, -s * cols as f32, 0.0);
        g.glx.translate(s, s, 0.0);

        let mut i = 0;
        for y in 0..cols {
            for x in 0..rows {
                g.glx.push_matrix();
                g.glx
                    .translate(x as f32 * s * 3.0, y as f32 * s * 2.0, y as f32 * s);
                if let Some(h) = self.hands.get(i) {
                    let pose = h.current;
                    self.draw_hand(g, &pose);
                    i += 1;
                }
                g.glx.translate(s, 0.0, 0.0);
                if let Some(h) = self.hands.get(i) {
                    let pose = h.current;
                    self.draw_hand(g, &pose);
                    i += 1;
                }
                g.glx.pop_matrix();
            }
        }
        g.glx.pop_matrix();

        self.tick_hands(g.time);
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:       30000",
    "*count:       2",
    "*foreground:  #8888CC",
    "*groundColor: #0000FF",
    "*showFPS:     False",
    "*wireframe:   False",
    "*speed:       1.0",
    "*spin:        XY",
    "*wander:      True",
    "*faceFront:   True",
];

const SPINS: &[crate::runtime::opts::SelectItem] = &[
    crate::runtime::opts::SelectItem {
        value: "XY",
        label: "Rotate around X and Y axes",
    },
    crate::runtime::opts::SelectItem {
        value: "0",
        label: "Don't rotate",
    },
    crate::runtime::opts::SelectItem {
        value: "X",
        label: "Rotate around X axis",
    },
    crate::runtime::opts::SelectItem {
        value: "Y",
        label: "Rotate around Y axis",
    },
    crate::runtime::opts::SelectItem {
        value: "Z",
        label: "Rotate around Z axis",
    },
    crate::runtime::opts::SelectItem {
        value: "XZ",
        label: "Rotate around X and Z axes",
    },
    crate::runtime::opts::SelectItem {
        value: "YZ",
        label: "Rotate around Y and Z axes",
    },
    crate::runtime::opts::SelectItem {
        value: "XYZ",
        label: "Rotate around all three axes",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.05, 2.0, 0.05, 2, "1.0"),
    Opt::slider("count", "Number of hands", 2.0, 32.0, 2.0, 0, "2"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::select("spin", "Rotation", SPINS, "XY"),
    Opt::boolean("faceFront", "Always face front", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "handsy",
    label: "Handsy",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2018",
        video: Some("https://www.youtube.com/watch?v=awI8EawYTdE"),
        blurb: "A set of robotic hands communicate non-verbally.",
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
    use crate::runtime::gl::Primitive;

    /// Eight bones, each a solid half-model rather than a stand-in.
    #[test]
    fn a_hand_is_eight_bones() {
        for (i, src) in MODELS.iter().enumerate() {
            let m = GlList::parse(src);
            assert_eq!(m.primitive, Shape::Triangles, "part {i} is not triangles");
            assert!(m.points > 100, "part {i} is only {} vertices", m.points);
        }
    }

    /// Every animation names at least one pose and takes time to reach it,
    /// which is what stops the stepping from dividing by zero or running off
    /// the end of a frame list.
    #[test]
    fn every_animation_has_frames() {
        assert_eq!(ALL_HAND_ANIMS.len(), 30);
        for (i, p) in ALL_HAND_ANIMS.iter().enumerate() {
            for anim in p.pair {
                assert!(!anim.is_empty(), "animation {i} has no frames");
                for f in anim {
                    assert!(f.duration > 0.0, "a frame of {i} takes no time");
                    assert!(f.pause >= 0.0, "a frame of {i} pauses backwards");
                }
            }
        }
        // The default pose comes off the end of the table, so it has to have
        // one.
        assert!(ALL_HAND_ANIMS.last().is_some());
    }

    /// The ground is a dozen dozen grids of thirty cells, two lines each.
    #[test]
    fn the_ground_is_twelve_grids_of_twelve() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let f = r.frame();
        let lines: usize = f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::Lines)
            .map(|b| b.count)
            .sum();
        assert_eq!(lines, 12 * 12 * 30 * 4, "{lines} is not the grid");
    }

    /// The hands are drawn out of the models: two of them, thirty-nine list
    /// calls each, and every call under its own matrix.
    #[test]
    fn two_hands_are_drawn() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let f = r.frame();
        let solid = f
            .batches
            .iter()
            .filter(|b| b.primitive == Primitive::Triangles)
            .count();
        // A palm, six thumb halves and thirty-two finger halves, twice.
        assert_eq!(solid, 2 * (1 + 6 + 32), "{solid} pieces is not two hands");
    }

    /// The hands start off screen and invisible and come in as the first
    /// animation runs, so the pose has to actually move.
    #[test]
    fn the_hands_come_in() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let first: Vec<_> = r.frame().vertices.to_vec();
        for _ in 0..120 {
            r.step();
        }
        let later = &r.frame().vertices;
        assert_ne!(first.len(), 0);
        assert!(
            first
                .iter()
                .zip(later.iter())
                .any(|(a, b)| a.pos != b.pos || a.color[3] != b.color[3]),
            "the hands never moved"
        );
    }
}
