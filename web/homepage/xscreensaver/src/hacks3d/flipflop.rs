//! Port of `hacks/glx/flipflop.c`.
//!
//! ```text
//! flipflop, Copyright (c) 2003 Kevin Ogden <kogden1@hotmail.com>
//!                     (c) 2006 Sergio Gutiérrez "Sergut" <sergut@gmail.com>
//!                     (c) 2008 Andrew Galante <a.drew7@gmail.com>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! 2003 Kevin Odgen                  First version
//! 2006 Sergio Gutiérrez "Sergut"    Made several parameters dynamic and
//!                                   selectable from the command line
//! 2008 Andrew Galante               Added -textured option
//! ```
//!
//! Tiles on a board, flipping over each other into the empty squares.
//!
//! A tile does not slide: it turns end over end about the edge it shares with
//! the square it is moving into, which is why the board can only ever be
//! rearranged and never jumbled. With a picture on it, the picture is
//! scrambled as they go, and the tile's texture coordinates are swapped end for
//! end whenever it flips, so that the piece of picture lands the right way up.
//!
//! The board is claimed square by square as the move is decided rather than as
//! it finishes, so two tiles can never head for the same place.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Glx, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random,
    screenhack_event_helper,
};

/// Thickness and the proportion of the board covered, for the two modes.
const STICK_THICK: f32 = 54.0;
const STICK_RATIO: i32 = 80;
const TILE_THICK: f32 = 4.0;
const TILE_RATIO: i32 = 95;

/// One tile: where it is, which way it is going, and how far over it has got.
struct Sheet {
    /// Which tile is on each square, or none.
    occupied: Vec<i32>,
    xpos: Vec<i32>,
    ypos: Vec<i32>,
    /// 0 for still, then x+, y+, x-, y-.
    direction: Vec<i32>,
    angle: Vec<f32>,
    color: Vec<[f32; 3]>,
    /// The corners of this tile's piece of the picture: left, top, right,
    /// bottom.
    tex: Vec<[f32; 4]>,
}

struct FlipFlop {
    trackball: Trackball,
    board_x: i32,
    board_y: i32,
    board_avg: i32,
    numsquares: usize,
    half_thick: f32,
    spin: f32,
    textured: bool,
    sheet: Sheet,
    theta: f32,
    /// How much of a flip happens in a frame, how far back the camera is, and
    /// how many moves are attempted a frame.
    flipspeed: f32,
    reldist: f32,
    energy: i32,
    texid: u32,
    got_texture: bool,
    aspect: f32,
    wire: bool,
}

impl FlipFlop {
    /// One tile: two faces and four edges, or just the faces in wireframe.
    fn draw_sheet(&self, g: &mut Glx, tex: [f32; 4]) {
        let t = self.half_thick;
        g.begin(if self.wire {
            Shape::LineLoop
        } else {
            Shape::Quads
        });

        g.normal3f(0.0, -1.0, 0.0);
        g.tex_coord2f(tex[0], tex[3]);
        g.vertex3f(t, -t, t);
        g.tex_coord2f(tex[2], tex[3]);
        g.vertex3f(1.0 - t, -t, t);
        g.tex_coord2f(tex[2], tex[1]);
        g.vertex3f(1.0 - t, -t, 1.0 - t);
        g.tex_coord2f(tex[0], tex[1]);
        g.vertex3f(t, -t, 1.0 - t);

        if self.wire {
            g.end();
            g.begin(Shape::LineLoop);
        }

        // The back.
        g.normal3f(0.0, 1.0, 0.0);
        g.tex_coord2f(tex[0], tex[1]);
        g.vertex3f(t, t, 1.0 - t);
        g.tex_coord2f(tex[2], tex[1]);
        g.vertex3f(1.0 - t, t, 1.0 - t);
        g.tex_coord2f(tex[2], tex[3]);
        g.vertex3f(1.0 - t, t, t);
        g.tex_coord2f(tex[0], tex[3]);
        g.vertex3f(t, t, t);

        if self.wire {
            g.end();
            return;
        }

        // Four edges.
        g.normal3f(0.0, 0.0, -1.0);
        g.tex_coord2f(tex[0], tex[3]);
        g.vertex3f(t, t, t);
        g.tex_coord2f(tex[2], tex[3]);
        g.vertex3f(1.0 - t, t, t);
        g.tex_coord2f(tex[2], tex[3]);
        g.vertex3f(1.0 - t, -t, t);
        g.tex_coord2f(tex[0], tex[3]);
        g.vertex3f(t, -t, t);

        g.normal3f(0.0, 0.0, 1.0);
        g.tex_coord2f(tex[0], tex[1]);
        g.vertex3f(t, t, 1.0 - t);
        g.tex_coord2f(tex[0], tex[1]);
        g.vertex3f(t, -t, 1.0 - t);
        g.tex_coord2f(tex[2], tex[1]);
        g.vertex3f(1.0 - t, -t, 1.0 - t);
        g.tex_coord2f(tex[2], tex[1]);
        g.vertex3f(1.0 - t, t, 1.0 - t);

        g.normal3f(1.0, 0.0, 0.0);
        g.tex_coord2f(tex[2], tex[1]);
        g.vertex3f(1.0 - t, t, 1.0 - t);
        g.tex_coord2f(tex[2], tex[1]);
        g.vertex3f(1.0 - t, -t, 1.0 - t);
        g.tex_coord2f(tex[2], tex[3]);
        g.vertex3f(1.0 - t, -t, t);
        g.tex_coord2f(tex[2], tex[3]);
        g.vertex3f(1.0 - t, t, t);

        g.normal3f(-1.0, 0.0, 0.0);
        g.tex_coord2f(tex[0], tex[1]);
        g.vertex3f(t, t, 1.0 - t);
        g.tex_coord2f(tex[0], tex[3]);
        g.vertex3f(t, t, t);
        g.tex_coord2f(tex[0], tex[3]);
        g.vertex3f(t, -t, t);
        g.tex_coord2f(tex[0], tex[1]);
        g.vertex3f(t, -t, 1.0 - t);
        g.end();
    }

    /// Pick a tile and a direction at random and try to move it. It may not
    /// go anywhere: this is an attempt, not a move.
    fn new_move(&mut self) {
        let num = random() as usize % self.numsquares;
        let i = self.sheet.xpos[num];
        let j = self.sheet.ypos[num];
        let dir = (random() % 4) as i32 + 1;
        if self.sheet.direction[num] != 0 {
            return;
        }
        let (di, dj) = match dir {
            1 => (1, 0),
            2 => (0, 1),
            3 => (-1, 0),
            _ => (0, -1),
        };
        let (ni, nj) = (i + di, j + dj);
        if ni < 0 || ni >= self.board_x || nj < 0 || nj >= self.board_y {
            return;
        }
        let to = (ni * self.board_y + nj) as usize;
        if self.sheet.occupied[to] != -1 {
            return;
        }
        // Claim the square now rather than when it arrives, so that two of
        // them can never head for the same place.
        self.sheet.direction[num] = dir;
        self.sheet.occupied[to] = num as i32;
        self.sheet.occupied[(i * self.board_y + j) as usize] = -1;
    }

    /// One frame of everyone's motion. `rot` is how far a flip turns in a
    /// frame.
    fn move_tiles(&mut self, rot: f32) {
        let textured = self.textured;
        for index in 0..self.numsquares {
            let dir = self.sheet.direction[index];
            if dir == 0 {
                continue;
            }
            // Going up, the picture is swapped end for end at the start of the
            // flip; coming down, at the end of it. Either way the piece lands
            // the right way up.
            if textured && self.sheet.angle[index] == 0.0 && (dir == 1 || dir == 2) {
                let t = &mut self.sheet.tex[index];
                if dir == 1 {
                    t.swap(0, 2);
                } else {
                    t.swap(1, 3);
                }
            }
            self.sheet.angle[index] += rot;
            if self.sheet.angle[index] < std::f32::consts::PI {
                continue;
            }
            match dir {
                1 => self.sheet.xpos[index] += 1,
                2 => self.sheet.ypos[index] += 1,
                3 => self.sheet.xpos[index] -= 1,
                _ => self.sheet.ypos[index] -= 1,
            }
            self.sheet.direction[index] = 0;
            self.sheet.angle[index] = 0.0;
            if textured && (dir == 3 || dir == 4) {
                let t = &mut self.sheet.tex[index];
                if dir == 3 {
                    t.swap(0, 2);
                } else {
                    t.swap(1, 3);
                }
            }
        }
    }

    /// All the tiles, each turned about the edge it is flipping over.
    fn draw_tiles(&self, g: &mut Glx) {
        for index in 0..self.numsquares {
            let c = self.sheet.color[index];
            g.color4f(c[0], c[1], c[2], 1.0);
            let i = self.sheet.xpos[index] as f32;
            let j = self.sheet.ypos[index] as f32;
            let a = self.sheet.angle[index] * 180.0 / std::f32::consts::PI;
            g.push_matrix();
            match self.sheet.direction[index] {
                0 => g.translate(i, 0.0, j),
                1 => {
                    g.translate(i + 1.0, 0.0, j);
                    g.rotate(180.0 - a, 0.0, 0.0, 1.0);
                }
                2 => {
                    g.translate(i, 0.0, j + 1.0);
                    g.rotate(180.0 - a, -1.0, 0.0, 0.0);
                }
                3 => {
                    g.translate(i, 0.0, j);
                    g.rotate(a, 0.0, 0.0, 1.0);
                }
                _ => {
                    g.translate(i, 0.0, j);
                    g.rotate(a, -1.0, 0.0, 0.0);
                }
            }
            self.draw_sheet(g, self.sheet.tex[index]);
            g.pop_matrix();
        }
    }

    /// Lay the picture over the board, a piece to a tile.
    fn take_texture(&mut self, g: &mut Gl) {
        let Some(img) = g.load_image(512, 512) else {
            return;
        };
        let geom = img.geometry;
        let (tw, th) = (img.width as f32, img.height as f32);
        let (tx, ty) = (geom.x as f32 / tw, geom.y as f32 / th);
        let (w, h) = (geom.width as f32 / tw, geom.height as f32 / th);

        let mut index = 0;
        for i in 0..self.board_x {
            for j in 0..self.board_y {
                if index >= self.numsquares {
                    break;
                }
                self.sheet.tex[index] = [
                    tx + w / self.board_x as f32 * i as f32,
                    ty + h / self.board_y as f32 * (j + 1) as f32,
                    tx + w / self.board_x as f32 * (i + 1) as f32,
                    ty + h / self.board_y as f32 * j as f32,
                ];
                self.sheet.color[index] = [1.0, 1.0, 1.0];
                index += 1;
            }
        }

        g.glx.bind_texture(self.texid);
        g.glx.tex_image_2d(img.width, img.height, img.pixels);
        g.glx.tex_clamp(true);
        self.got_texture = true;
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let mut textured = g.res.bool("textured") && !wire;
    let mode = g.res.string("mode").to_string();

    let mut board_x = g.res.int("sizex").max(1);
    let mut board_y = g.res.int("sizey").max(1);
    let avg = g.res.int("size");
    let board_avg;
    if avg != 0 {
        board_x = avg;
        board_y = avg;
        board_avg = avg;
    } else {
        board_avg = (board_x + board_y) / 2;
    }

    let mut numsquares = g.res.int("numsquares");
    let freesquares = g.res.int("freesquares");
    if numsquares == 0 && freesquares != 0 {
        numsquares = board_x * board_y - freesquares;
    }
    let half_thick;
    if mode != "tiles" {
        // A picture on sticks looks silly.
        textured = false;
        half_thick = STICK_THICK / 100.0;
        if numsquares == 0 {
            numsquares = board_x * board_y * STICK_RATIO / 100;
        }
    } else {
        half_thick = TILE_THICK / 100.0;
        if numsquares == 0 {
            numsquares = board_x * board_y * TILE_RATIO / 100;
        }
    }
    let numsquares = numsquares.clamp(1, board_x * board_y) as usize;

    // The tiles start in a corner of the board, coloured in a pattern.
    let mut occupied = vec![-1; (board_x * board_y) as usize];
    let mut xpos = Vec::new();
    let mut ypos = Vec::new();
    let mut color = Vec::new();
    let mut index = 0;
    for i in 0..board_x {
        for j in 0..board_y {
            if index < numsquares {
                occupied[(i * board_y + j) as usize] = index as i32;
                xpos.push(i);
                ypos.push(j);
                let t = |n: i32| if (i + j + n) % 3 == 0 { 1.0 } else { 0.0 };
                color.push([f32::max(t(0), t(1)), t(1), t(2)]);
                index += 1;
            }
        }
    }

    let texid = if textured { g.glx.gen_texture() } else { 0 };
    let mut this = FlipFlop {
        trackball: Trackball::new(),
        board_x,
        board_y,
        board_avg,
        numsquares,
        half_thick,
        spin: g.res.float("spin") as f32,
        textured,
        sheet: Sheet {
            occupied,
            xpos,
            ypos,
            direction: vec![0; numsquares],
            angle: vec![0.0; numsquares],
            color,
            tex: vec![[0.0, 0.0, 1.0, 1.0]; numsquares],
        },
        theta: 0.0,
        flipspeed: 0.03,
        reldist: 1.0,
        energy: 40,
        texid,
        got_texture: !textured,
        aspect: 1.0,
        wire,
    };
    if textured {
        this.take_texture(g);
    }
    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for FlipFlop {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let mut height = height;
        let mut y = 0;
        // A tiny window shows the middle rather than a letterbox.
        if width > height * 5 {
            height = width * 9 / 16;
            y = -height / 2;
        }
        g.glx.viewport(0, y, width, height);
        self.aspect = width as f32 / height as f32;
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if screenhack_event_helper(event) && self.got_texture {
            // A new picture, on demand. It also turns texturing on if it was
            // off, which is what upstream does.
            if self.texid == 0 {
                self.texid = g.glx.gen_texture();
            }
            self.textured = true;
            self.got_texture = false;
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        if self.textured && !self.got_texture {
            self.take_texture(g);
            if !self.got_texture {
                // Nothing to draw until the picture is there.
                g.glx.clear();
                return g.res.int("delay") as u32;
            }
        }

        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(45.0, self.aspect, 1.0, 300.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        if !self.wire {
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
            g.glx.light_ambient(0, [0.8, 0.8, 0.8, 1.0]);
            g.glx
                .light_position(0, 0.0, self.board_avg as f32 * 0.3, 0.0, 1.0);
            // The colour is the material: upstream turns on colour tracking
            // and then only ever sets colours.
            g.glx.color_material(true);
        } else {
            g.glx.lighting(false);
        }

        g.glx
            .translate(0.0, 0.0, -self.reldist * self.board_avg as f32);
        g.glx.rotate(22.5, 1.0, 0.0, 0.0);
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        g.glx.rotate(self.theta * 100.0, 0.0, 1.0, 0.0);
        // Centre the board.
        g.glx
            .translate(-0.5 * self.board_x as f32, 0.0, -0.5 * self.board_y as f32);

        if self.textured {
            g.glx.texturing(true);
            g.glx.bind_texture(self.texid);
            g.glx.blend(Blend::Alpha);
        }

        for _ in 0..self.energy {
            self.new_move();
        }
        self.move_tiles(self.flipspeed * std::f32::consts::PI);
        let glx = &mut g.glx;
        self.draw_tiles(glx);

        if !self.trackball.button_down() {
            self.theta += 0.01 * self.spin;
        }

        g.glx.texturing(false);
        g.glx.blend(Blend::Off);
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        20000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*mode:         tiles",
    "*size:         0",
    "*sizex:        9",
    "*sizey:        9",
    "*numsquares:   0",
    "*freesquares:  0",
    "*spin:         0.1",
    "*textured:     False",
];

const MODES: &[crate::runtime::opts::SelectItem] = &[
    crate::runtime::opts::SelectItem {
        value: "tiles",
        label: "Draw tiles",
    },
    crate::runtime::opts::SelectItem {
        value: "sticks",
        label: "Draw sticks",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("spin", "Spin", 0.0, 3.0, 0.05, 2, "0.1"),
    Opt::select("mode", "Mode", MODES, "tiles"),
    Opt::slider("sizex", "Width", 3.0, 20.0, 1.0, 0, "9"),
    Opt::slider("sizey", "Depth", 3.0, 20.0, 1.0, 0, "9"),
    Opt::boolean("textured", "Load image", "false"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "flipflop",
    label: "Flip Flop",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Kevin Ogden and Sergio Gutierrez",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=RzWRoAMFtnw"),
        blurb: "Coloured tiles swap with each other, flipping end over end.",
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

    /// The board is only ever rearranged. Every tile is always drawn, and
    /// none of them wanders off the board.
    #[test]
    fn no_tile_leaves_the_board() {
        // Nine by nine at ninety-five percent is seventy-six tiles.
        let want = 9 * 9 * 95 / 100;
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        for _ in 0..500 {
            r.step();
            let f = r.frame();
            assert_eq!(f.batches.len(), want, "not every tile was drawn");
            for v in &f.vertices {
                // The board is nine across, drawn about the middle.
                assert!(
                    v.pos[0] >= -0.1 && v.pos[0] <= 10.0,
                    "a tile is at {}",
                    v.pos[0]
                );
                assert!(v.pos[1].abs() <= 1.1, "a tile is {} high", v.pos[1]);
            }
        }
    }

    /// A tile turns end over end about the edge it shares with the square it
    /// is going to, and a full flip is half a turn.
    #[test]
    fn a_tile_flips_rather_than_slides() {
        let mut r = start(StartArgs::new(640, 480, "sizex=3&sizey=3", 20260811));
        let mut turned = false;
        for _ in 0..400 {
            r.step();
            for b in &r.frame().batches {
                // A turned tile has an off-diagonal in the top-left of its
                // matrix; a still one does not.
                if b.modelview.0[1].abs() > 0.2 || b.modelview.0[6].abs() > 0.2 {
                    turned = true;
                }
            }
        }
        assert!(turned, "nothing ever flipped");
    }

    /// With a picture on it, the pieces are laid out over the board and each
    /// tile takes the piece that belongs to its square.
    #[test]
    fn the_picture_is_cut_up_over_the_board() {
        let mut r = start(StartArgs::new(640, 480, "textured=true", 20260811));
        r.step();
        let f = r.frame();
        assert!(
            f.batches.iter().all(|b| b.texture.is_some()),
            "the picture is not on the tiles"
        );
        // Every tile has its own piece: the coordinates differ.
        let uvs: std::collections::HashSet<String> = f
            .batches
            .iter()
            .map(|b| format!("{:?}", f.vertices[b.first].uv))
            .collect();
        assert!(uvs.len() > 40, "only {} pieces of picture", uvs.len());
    }

    /// Sticks are the same board with thicker tiles, and they are never
    /// textured, because a picture on them looks silly.
    #[test]
    fn sticks_are_thick_and_bare() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "mode=sticks&textured=true",
            20260811,
        ));
        r.step();
        let f = r.frame();
        assert!(
            f.batches.iter().all(|b| b.texture.is_none()),
            "the sticks are textured"
        );
        // A stick is more than half as thick as it is wide.
        let ys: Vec<f32> = f.vertices.iter().map(|v| v.pos[1]).collect();
        let hi = ys.iter().copied().fold(f32::MIN, f32::max);
        assert!(hi > 0.5, "the sticks are only {hi} thick");
    }
}
