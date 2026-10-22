//! Port of `hacks/scooter.c`.
//!
//! ```text
//! scooter -- a journey through space tunnel and stars
//!
//! Copyright (c) 2001 Sven Thoennissen <posse@gmx.net>
//!
//! This program is based on the original "scooter", a blanker module from the
//! Nightshift screensaver which is part of EGS (Enhanced Graphics System) on
//! the Amiga computer. EGS has been developed by VIONA Development.
//!
//!
//! (now the obligatory stuff)
//!
//! Permission to use, copy, modify, and distribute this software and its
//! documentation for any purpose and without fee is hereby granted,
//! provided that the above copyright notice appear in all copies and that
//! both that copyright notice and this permission notice appear in
//! supporting documentation.
//!
//! This file is provided AS IS with no warranties of any kind.  The author
//! shall have no liability with respect to the infringement of copyrights,
//! trade secrets or any patents by this file or any part thereof.  In no
//! event will the author be liable for any lost revenue or profits or
//! other special, indirect and consequential damages.
//!
//! Changes:
//!
//! ??/??/2001 (Sven Thoennissen <posse@gmx.net>):    Initial release
//! 05/08/2019 (EoflaOE <eoflaoevicecity@gmail.com>): Ported to XScreenSaver
//! ```
//!
//! A tunnel of rectangular doorways flying towards you, with stars outside it.
//! The tunnel is not a shape anyone models. It is a chain of several hundred
//! joints, each holding only a rotation, and the chain is built by walking from
//! the joint the viewer sits on outwards, stepping a fixed distance along
//! whatever direction the previous joint's rotation left you pointing. Bend the
//! rotations a little from one joint to the next and the chain curves; that
//! curve is the tunnel.
//!
//! Flying forwards is then just shifting the whole array of rotations down by
//! the speed each frame and rolling a fresh one onto the far end. Nothing moves
//! towards the viewer; the numbering does. A door is an index into that chain
//! and a colour, a star is an index plus an offset from the middle, and both
//! come back round to the far end when their index runs off the front.
//!
//! The new rotations at the far end are not random. They ease along a sine
//! between one random bend and the next over ten to thirty seconds, so the
//! tunnel banks into a curve and out of it rather than jittering.
//!
//! Two upstream details are worth knowing when comparing pictures. Corner
//! coordinates are truncated to a short, as `XPoint` is upstream, so a door
//! crossing the viewer's own plane briefly throws its corners around the screen
//! rather than off it. And the ramp position that eases the bends is computed in
//! 64 bits here: upstream computes it in an int that overflows once the frame
//! rate is set high enough to make an interval more than about 131072 frames
//! long, which would turn the easing into noise.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{Pixel, XColor};
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, nrand};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XPoint};

const MIN_DOORS: i32 = 4;
const MIN_SPEED: i32 = 1;
const MAX_SPEED: i32 = 10;

/// Everything in the tunnel is measured in tenths of a pixel at the reference
/// screen size.
const SPACE_XY_FACTOR: i32 = 10;

const DOOR_WIDTH: i32 = 600 * SPACE_XY_FACTOR;
const DOOR_HEIGHT: i32 = 400 * SPACE_XY_FACTOR;

/// How far out from the tunnel's centre a star may sit.
const STAR_MIN_X: i32 = 1000 * SPACE_XY_FACTOR;
const STAR_MIN_Y: i32 = 750 * SPACE_XY_FACTOR;
const STAR_MAX_X: i32 = 10000 * SPACE_XY_FACTOR;
const STAR_MAX_Y: i32 = 7500 * SPACE_XY_FACTOR;

const STAR_SIZE_MIN: i32 = 2 * SPACE_XY_FACTOR;
const STAR_SIZE_MAX: i32 = 64 * SPACE_XY_FACTOR;

/// Greater values make scooter run harder curves, smaller values produce calm
/// curves.
const DOOR_CURVEDNESS: i32 = 14;

/// 3D to 2D projection; greater values create more fish-eye effect.
const PROJECTION_DEGREE: f64 = 2.4;

/// The author's own screen, at which scooter is in its original size. Every
/// number in the hack is tuned for this, and other windows are rescaled.
const ASPECT_SCREENWIDTH: f32 = 1152.0;
const ASPECT_SCREENHEIGHT: f32 = 864.0;

/// The sine table, in the spirit of the Amiga original: an angle is an index
/// into it, so a full turn is 32768 and wrapping is a mask.
const SINUSTABLE_SIZE: usize = 0x8000;
const SINUSTABLE_MASK: i32 = 0x7fff;

/// `SGN`.
fn sgn(a: i32) -> i32 {
    if a < 0 { -1 } else { 1 }
}

#[derive(Clone, Copy, Default)]
struct Vec3D {
    x: i32,
    y: i32,
    z: i32,
}

#[derive(Clone, Copy, Default)]
struct Angle3D {
    x: i32,
    y: i32,
    z: i32,
}

#[derive(Clone, Copy, Default)]
struct ColorRgb {
    r: i32,
    g: i32,
    b: i32,
}

struct Rect {
    lefttop: XPoint,
    rightbottom: XPoint,
}

#[derive(Clone, Copy, Default)]
struct Door {
    /// Left-top, right-top, right-bottom, left-bottom.
    coords: [Vec3D; 4],
    zelement: i32,
    color: Pixel,
}

#[derive(Clone, Copy, Default)]
struct Star {
    xpos: i32,
    ypos: i32,
    width: i32,
    height: i32,
    zelement: i32,
    draw: bool,
}

/// One joint of the tunnel: where it ended up, and the rotation that got it
/// there.
#[derive(Clone, Copy, Default)]
struct ZElement {
    pos: Vec3D,
    angle: Angle3D,
}

struct Scooter {
    mi: ModeInfo,
    stars: Vec<Star>,
    doors: Vec<Door>,
    zelements: Vec<ZElement>,
    doorcount: usize,
    ztotal: i32,
    speed: i32,
    zelements_per_door: i32,
    zelement_distance: i32,
    spectator_zelement: usize,
    projnorm_z: i32,
    rotation_duration: i32,
    rotation_step: i32,
    starcount: usize,
    current_rotation: Angle3D,
    rotation_delta: Angle3D,

    /// The doors cycle through colours by walking from one random colour to the
    /// next over a random number of doors.
    begincolor: ColorRgb,
    endcolor: ColorRgb,
    colorcount: i32,
    colorsteps: i32,

    /// Scales all stars and doors to the window's dimensions.
    aspect_scale: f32,
    pscale: i32,
    sintable: Vec<f32>,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // No *_COLORS define, so the default random colormap.
    let mi = ModeInfo::new(d, ColorScheme::Random);
    let mut st = Scooter {
        mi,
        stars: Vec::new(),
        doors: Vec::new(),
        zelements: Vec::new(),
        doorcount: MIN_DOORS as usize,
        ztotal: 0,
        speed: MIN_SPEED,
        zelements_per_door: 60,
        zelement_distance: 300,
        spectator_zelement: 60,
        projnorm_z: 50 * 240,
        rotation_duration: 1,
        rotation_step: 0,
        starcount: 1,
        current_rotation: Angle3D::default(),
        rotation_delta: Angle3D::default(),
        begincolor: ColorRgb::default(),
        endcolor: ColorRgb::default(),
        colorcount: 0,
        colorsteps: 0,
        aspect_scale: 1.0,
        pscale: 1,
        sintable: Vec::new(),
    };
    st.restart();
    Box::new(st)
}

/// A random colour, as 16-bit components built from one 24-bit draw.
fn randomcolor() -> ColorRgb {
    let n = nrand(0x100_0000);
    ColorRgb {
        r: (n >> 16) << 8,
        g: ((n >> 8) & 0xff) << 8,
        b: (n & 0xff) << 8,
    }
}

impl Scooter {
    fn sin(&self, a: i32) -> f32 {
        self.sintable[(a & SINUSTABLE_MASK) as usize]
    }

    fn cos(&self, a: i32) -> f32 {
        self.sintable[((a + (SINUSTABLE_SIZE as i32 / 4)) & SINUSTABLE_MASK) as usize]
    }

    /// `init_scooter`.
    fn restart(&mut self) {
        self.doorcount = self.mi.count.max(MIN_DOORS) as usize;
        self.speed = self.mi.cycles.clamp(MIN_SPEED, MAX_SPEED);
        self.starcount = self.mi.size.max(1) as usize;
        self.zelements_per_door = 60;
        self.zelement_distance = 300;
        self.ztotal = self.doorcount as i32 * self.zelements_per_door;
        self.starcount = self.starcount.min(self.ztotal as usize);

        // Prepare initial values for next_door_color.
        self.endcolor = randomcolor();
        self.colorcount = 0;
        self.colorsteps = 0;

        if self.sintable.is_empty() {
            self.sintable = (0..SINUSTABLE_SIZE)
                .map(|i| ((std::f64::consts::TAU / SINUSTABLE_SIZE as f64 * i as f64) as f32).sin())
                .collect();
        }

        self.doors = vec![Door::default(); self.doorcount];
        self.zelements = vec![ZElement::default(); self.ztotal as usize];
        for i in 0..self.doorcount {
            self.doors[i].zelement = self.zelements_per_door * (i as i32 + 1) - 1;
            self.next_door_color(i);
        }

        self.stars = vec![Star::default(); self.starcount];
        for i in 0..self.starcount {
            self.stars[i].zelement = self.ztotal * i as i32 / self.starcount as i32;
        }

        self.projnorm_z = 50 * 240;
        self.spectator_zelement = self.zelements_per_door as usize;

        self.current_rotation = Angle3D::default();
        self.rotation_delta = Angle3D::default();
        self.rotation_duration = 1;
        self.rotation_step = 0;

        self.pscale = if self.mi.width > 2560 || self.mi.height > 2560 {
            2 // Retina displays.
        } else {
            1
        };
    }

    /// The next colour along the current ramp, rolling a new ramp when this one
    /// runs out.
    fn next_door_color(&mut self, k: usize) {
        if self.mi.npixels() <= 2 {
            self.doors[k].color = self.mi.white;
            return;
        }

        if self.colorcount >= self.colorsteps {
            self.colorcount = 0;
            self.colorsteps = 8 + nrand(32);
            self.begincolor = self.endcolor;
            self.endcolor = randomcolor();
        }

        let lerp = |a: i32, b: i32| a + ((b - a) * self.colorcount / self.colorsteps);
        let mut xcol = XColor {
            pixel: 0,
            red: lerp(self.begincolor.r, self.endcolor.r) as u16,
            green: lerp(self.begincolor.g, self.endcolor.g) as u16,
            blue: lerp(self.begincolor.b, self.endcolor.b) as u16,
        };
        self.colorcount += 1;
        xcol.alloc();
        self.doors[k].color = xcol.pixel;
    }

    /// ```text
    ///  y
    ///
    ///  ^
    ///  |      z
    ///  |    .
    ///  |   /
    ///  |  /
    ///  | /
    ///  |/
    /// -+------------> x
    /// /|
    /// ```
    ///
    /// Rotation angles: a = alpha (x-rotation), b = beta (y-rotation),
    /// c = gamma (z-rotation).
    fn rotate_3d(&self, src: Vec3D, angle: Angle3D) -> Vec3D {
        let (cosa, cosb, cosc) = (self.cos(angle.x), self.cos(angle.y), self.cos(angle.z));
        let (sina, sinb, sinc) = (self.sin(angle.x), self.sin(angle.y), self.sin(angle.z));
        let mut dest = Vec3D::default();

        // X axis.
        let (tz, ty) = (src.z as f32, src.y as f32);
        dest.z = (tz * cosa - ty * sina) as i32;
        dest.y = (tz * sina + ty * cosa) as i32;

        // Y axis.
        let (tz, tx) = (dest.z as f32, src.x as f32);
        dest.z = (tz * cosb - tx * sinb) as i32;
        dest.x = (tz * sinb + tx * cosb) as i32;

        // Z axis.
        let (tx, ty) = (dest.x as f32, dest.y as f32);
        dest.x = (tx * cosc - ty * sinc) as i32;
        dest.y = (tx * sinc + ty * cosc) as i32;

        dest
    }

    /// Advance the rotation that the far end of the tunnel is being built with,
    /// easing along a sine from one random bend to the next.
    fn calc_new_element(&mut self) {
        // Upstream computes this index in an int, which overflows once a frame
        // rate is set high enough to make an interval very long.
        let idx = (SINUSTABLE_SIZE as i64 / 2) * self.rotation_step as i64
            / self.rotation_duration as i64;
        let rot = self.sin(idx as i32);

        let step = self.rotation_step;
        self.rotation_step += 1;
        if step >= self.rotation_duration {
            // Frames per second as a timebase. Upstream divides by the delay
            // without checking it, and the panel's frame-rate slider goes all
            // the way to a delay of zero.
            let fps = 1_000_000 / self.mi.delay.max(1) as i32;

            // One rotation interval takes 10-30 seconds at speed 1.
            self.rotation_duration = 10 * fps + nrand(20 * fps);

            self.rotation_delta.x = nrand(DOOR_CURVEDNESS * 2 + 1) - DOOR_CURVEDNESS;
            self.rotation_delta.y = nrand(DOOR_CURVEDNESS * 2 + 1) - DOOR_CURVEDNESS;
            self.rotation_delta.z = nrand(DOOR_CURVEDNESS * 2 + 1) - DOOR_CURVEDNESS;

            self.rotation_step = 0;
        }

        self.current_rotation.x += (rot * self.rotation_delta.x as f32) as i32;
        self.current_rotation.y += (rot * self.rotation_delta.y as f32) as i32;
        self.current_rotation.z += (rot * self.rotation_delta.z as f32) as i32;

        self.current_rotation.x &= SINUSTABLE_MASK;
        self.current_rotation.y &= SINUSTABLE_MASK;
        self.current_rotation.z &= SINUSTABLE_MASK;
    }

    /// The angle of a joint relative to the one the viewer sits on.
    fn relative_angle(&self, i: usize) -> Angle3D {
        let spec = self.zelements[self.spectator_zelement].angle;
        Angle3D {
            x: self.zelements[i].angle.x - spec.x,
            y: self.zelements[i].angle.y - spec.y,
            z: self.zelements[i].angle.z - spec.z,
        }
    }

    /// Fly forwards by shifting every joint's rotation down the array, then
    /// rebuild the chain of positions out from the viewer in both directions.
    fn shift_elements(&mut self) {
        let ztotal = self.ztotal as usize;
        let speed = self.speed as usize;

        for i in speed..ztotal {
            self.zelements[i - speed].angle = self.zelements[i].angle;
        }
        for i in (ztotal - speed)..ztotal {
            self.calc_new_element();
            self.zelements[i].angle = self.current_rotation;
        }

        let spec = self.spectator_zelement;
        self.zelements[spec].pos = Vec3D {
            x: 0,
            y: 0,
            z: self.zelement_distance * spec as i32,
        };

        for i in (0..spec).rev() {
            let step = Vec3D {
                x: 0,
                y: 0,
                z: -self.zelement_distance,
            };
            let pos = self.rotate_3d(step, self.relative_angle(i));
            let prev = self.zelements[i + 1].pos;
            self.zelements[i].pos = Vec3D {
                x: pos.x + prev.x,
                y: pos.y + prev.y,
                z: pos.z + prev.z,
            };
        }

        for i in (spec + 1)..ztotal {
            let step = Vec3D {
                x: 0,
                y: 0,
                z: self.zelement_distance,
            };
            let pos = self.rotate_3d(step, self.relative_angle(i));
            let prev = self.zelements[i - 1].pos;
            self.zelements[i].pos = Vec3D {
                x: pos.x + prev.x,
                y: pos.y + prev.y,
                z: pos.z + prev.z,
            };
        }

        // Shift doors and wrap around.
        for i in 0..self.doorcount {
            self.doors[i].zelement -= self.speed;
            if self.doors[i].zelement < 0 {
                self.doors[i].zelement += self.ztotal;
                self.next_door_color(i);
            }
        }

        // Shift stars.
        for i in 0..self.starcount {
            self.stars[i].zelement -= self.speed;
            if self.stars[i].zelement < 0 {
                self.stars[i].zelement += self.ztotal;
                self.stars[i].draw = true;

                // Make sure new stars are outside doors.
                let rnd = nrand(2 * (STAR_MAX_X - STAR_MIN_X)) - (STAR_MAX_X - STAR_MIN_X);
                self.stars[i].xpos = rnd + (STAR_MIN_X * sgn(rnd));

                let rnd = nrand(2 * (STAR_MAX_Y - STAR_MIN_Y)) - (STAR_MAX_Y - STAR_MIN_Y);
                self.stars[i].ypos = rnd + (STAR_MIN_Y * sgn(rnd));

                let rnd = nrand(STAR_SIZE_MAX - STAR_SIZE_MIN) + STAR_SIZE_MIN;
                self.stars[i].width = rnd;
                self.stars[i].height = rnd * 3 / 4;
            }
        }
    }

    /// The four corners of a door in space, from its joint's rotation.
    fn door_3d(&mut self, k: usize) {
        let ze_pos = self.zelements[self.doors[k].zelement as usize].pos;
        let angle = self.relative_angle(self.doors[k].zelement as usize);

        let corners = [
            (-DOOR_WIDTH / 2, DOOR_HEIGHT / 2),
            (DOOR_WIDTH / 2, DOOR_HEIGHT / 2),
            (DOOR_WIDTH / 2, -DOOR_HEIGHT / 2),
            (-DOOR_WIDTH / 2, -DOOR_HEIGHT / 2),
        ];
        for (j, (x, y)) in corners.into_iter().enumerate() {
            let c = self.rotate_3d(Vec3D { x, y, z: 0 }, angle);
            self.doors[k].coords[j] = Vec3D {
                x: c.x + ze_pos.x,
                y: c.y + ze_pos.y,
                z: c.z + ze_pos.z,
            };
        }
    }

    fn projection(&self, zval: i32) -> f32 {
        (self.projnorm_z as f64 / (PROJECTION_DEGREE * zval as f64)) as f32
    }

    fn drawdoors(&mut self, d: &mut Dpy) {
        let (width, height) = (self.mi.width, self.mi.height);
        let (midx, midy) = (width / 2, height / 2);
        let rect = Rect {
            lefttop: XPoint { x: 0, y: 0 },
            rightbottom: XPoint {
                x: width - 1,
                y: height - 1,
            },
        };
        self.mi.gc.set_line_width(2 * self.pscale);

        for i in 0..self.doorcount {
            self.door_3d(i);

            let mut lines = [XPoint::default(); 4];
            let mut visible = true;
            for (line, c) in lines.iter_mut().zip(self.doors[i].coords) {
                if c.z <= 0 {
                    visible = false;
                    break;
                }
                let proj = self.projection(c.z) * self.aspect_scale;
                *line = XPoint {
                    x: short(midx + (c.x as f32 * proj / SPACE_XY_FACTOR as f32) as i32),
                    y: short(midy - (c.y as f32 * proj / SPACE_XY_FACTOR as f32) as i32),
                };
            }
            if !visible {
                continue;
            }

            self.mi.gc.set_foreground(self.doors[i].color);
            for j in 0..4 {
                let mut clip1 = lines[j];
                let mut clip2 = lines[(j + 1) % 4];
                if clipline(&mut clip1, &mut clip2, &rect) {
                    d.win()
                        .draw_line(&self.mi.gc, clip1.x, clip1.y, clip2.x, clip2.y);
                }
            }
        }
    }

    fn drawstars(&mut self, d: &mut Dpy) {
        let (width, height) = (self.mi.width, self.mi.height);
        let (midx, midy) = (width / 2, height / 2);

        for i in 0..self.starcount {
            if !self.stars[i].draw {
                continue;
            }

            // Rotate the star around its joint, then add the joint's position.
            let ze = self.zelements[self.stars[i].zelement as usize];
            let angle = self.relative_angle(self.stars[i].zelement as usize);
            let c = self.rotate_3d(
                Vec3D {
                    x: self.stars[i].xpos,
                    y: self.stars[i].ypos,
                    z: 0,
                },
                angle,
            );
            let coords = Vec3D {
                x: c.x + ze.pos.x,
                y: c.y + ze.pos.y,
                z: c.z + ze.pos.z,
            };
            if coords.z <= 0 {
                continue;
            }

            // Projection and clipping, trivial for a rectangle.
            let proj = self.projection(coords.z) * self.aspect_scale;
            let sc = |v: i32| (v as f32 * proj / SPACE_XY_FACTOR as f32) as i32;

            let mut lefttop = XPoint {
                x: short(midx + sc(coords.x - self.stars[i].width / 2)),
                y: short(midy - sc(coords.y + self.stars[i].height / 2)),
            };
            if lefttop.x < 0 {
                lefttop.x = 0;
            } else if lefttop.x >= width {
                continue;
            }
            if lefttop.y < 0 {
                lefttop.y = 0;
            } else if lefttop.y >= height {
                continue;
            }

            let mut rightbottom = XPoint {
                x: short(midx + sc(coords.x + self.stars[i].width / 2)),
                y: short(midy - sc(coords.y - self.stars[i].height / 2)),
            };
            if rightbottom.x < 0 {
                continue;
            } else if rightbottom.x >= width {
                rightbottom.x = width - 1;
            }
            if rightbottom.y < 0 {
                continue;
            } else if rightbottom.y >= height {
                rightbottom.y = height - 1;
            }

            // In white, small stars look darker than big stars.
            self.mi.gc.set_foreground(self.mi.white);
            d.win().fill_arc(
                &self.mi.gc,
                lefttop.x,
                lefttop.y,
                rightbottom.x - lefttop.x,
                rightbottom.y - lefttop.y,
                0,
                360 * 64,
            );
        }
    }
}

/// Truncate to a short, as upstream's `XPoint` fields are.
fn short(v: i32) -> i32 {
    v as i16 as i32
}

/// Clip the line p1-p2 at the given rectangle.
///
/// Ported as it stands, including the last test, which checks the y coordinate
/// against the bottom edge where the surrounding code is clipping x. It rarely
/// matters: the first test lets a line through unclipped if either end is
/// within either dimension's range, which is most of them, and the framebuffer
/// clips the rest anyway.
fn clipline(p1: &mut XPoint, p2: &mut XPoint, rect: &Rect) -> bool {
    let mut new1 = *p1;
    let mut new2 = *p2;

    // The entire line may not need clipping.
    if (new1.x >= rect.lefttop.x && new1.x <= rect.rightbottom.x)
        || (new1.y >= rect.lefttop.y && new1.y <= rect.rightbottom.y)
        || (new2.x >= rect.lefttop.x && new2.x <= rect.rightbottom.x)
        || (new2.y >= rect.lefttop.y && new2.y <= rect.rightbottom.y)
    {
        return true;
    }

    // First: clip the y dimension, with p1 above p2.
    if new1.y > new2.y {
        std::mem::swap(&mut new1, &mut new2);
    }
    if new2.y < rect.lefttop.y || new1.y > rect.rightbottom.y {
        return false; // Totally out of view.
    }

    let m = if new2.x == new1.x {
        0.0
    } else {
        (new2.y - new1.y) as f32 / (new2.x - new1.x) as f32
    };

    if new1.y < rect.lefttop.y {
        if m != 0.0 {
            new1.x += ((rect.lefttop.y - new1.y) as f32 / m) as i32;
        }
        new1.y = rect.lefttop.y;
    }
    if new2.y > rect.rightbottom.y {
        if m != 0.0 {
            new2.x -= ((new2.y - rect.rightbottom.y) as f32 / m) as i32;
        }
        new2.y = rect.rightbottom.y;
    }

    // Then the x dimension, with p1 left of p2.
    if new1.x > new2.x {
        std::mem::swap(&mut new1, &mut new2);
    }
    if new2.x < rect.lefttop.x || new1.x > rect.rightbottom.x {
        return false;
    }

    let m = if new2.x == new1.x {
        0.0
    } else {
        (new2.y - new1.y) as f32 / (new2.x - new1.x) as f32
    };

    if new1.x < rect.lefttop.x {
        new1.y += ((rect.lefttop.y - new1.y) as f32 * m) as i32;
        new1.x = rect.lefttop.x;
    }
    if new2.y > rect.rightbottom.y {
        new2.y -= ((new2.y - rect.rightbottom.y) as f32 * m) as i32;
        new2.x = rect.rightbottom.x;
    }

    *p1 = new1;
    *p2 = new2;
    true
}

impl Screenhack for Scooter {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        d.clear_window();

        self.shift_elements();

        // With these scale factors, all doors are sized correctly for any
        // window. If the aspect ratio is not 4:3 the smaller part of the window
        // is used, so a 1000x600 window is scaled like an 800x600 one rather
        // than a 1000x750 one.
        let (w, h) = (self.mi.width as f32, self.mi.height as f32);
        self.aspect_scale = if w / h >= ASPECT_SCREENWIDTH / ASPECT_SCREENHEIGHT {
            h / ASPECT_SCREENHEIGHT
        } else {
            w / ASPECT_SCREENWIDTH
        };

        self.drawstars(d);
        self.drawdoors(d);

        self.mi.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        // Upstream has no reshape hook, so xlockmore re-runs init.
        self.mi.reshape(width, height);
        self.restart();
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*delay: 20000",
    "*count: 24",
    "*cycles: 5",
    "*size: 100",
    "*ncolors: 200",
    "*fullrandom: True",
    "*verbose: False",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("cycles", "Boat speed", 0.0, 1000.0, 1.0, 0, "5"),
    Opt::slider("count", "Number of doors", 4.0, 40.0, 1.0, 0, "24"),
    Opt::slider("size", "Number of stars", 0.0, 200.0, 1.0, 0, "100"),
    Opt::slider("ncolors", "Number of colors", 0.0, 200.0, 1.0, 0, "200"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "scooter",
    label: "Scooter",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Sven Thoennissen",
        year: "2001",
        video: Some("https://www.youtube.com/watch?v=Qqzk1BldlXg"),
        blurb: "Zooming down a tunnel in a star field. Originally an Amiga hack.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
