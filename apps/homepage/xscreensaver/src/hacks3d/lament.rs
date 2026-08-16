//! Port of `hacks/glx/lament.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 1998-2018 Jamie Zawinski <jwz@jwz.org>
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
//! ```text
//! Animates Lemarchand's Box, the Lament Configuration.  By jwz, 25-Jul-98.
//! ```
//!
//! A three-inch puzzle box in gold leaf that turns over slowly and, every so
//! often, comes apart: the faces rise and twist into a star, the corners
//! rotate away as tetrahedra, the lid opens, a pillar slides out, the whole
//! thing swells into a sphere, or it unfolds into the Leviathan and the walls
//! fold away behind it. Then it puts itself back together and goes back to
//! turning.
//!
//! The box is one model drawn in a modelling program, cut into thirty pieces
//! that each animation moves separately, and it arrives as thirty flat arrays
//! of normals and vertices with nothing in them about what is gold and what is
//! not. That is worked out here, as upstream does: the normal of a triangle
//! says which of the six walls it faces, and if the triangle also lies within
//! a hundredth of an inch of that wall then it is on the outside and gets the
//! gold leaf for that wall. Everything else is inside the box.
//!
//! Upstream then compiles each piece into a display list with a material and a
//! texture bound per triangle. Display lists here do not carry materials, and
//! a material set between two triangles would split them into two draw calls
//! anyway, so the triangles are instead sorted once at startup into the eight
//! groups the six wall textures, the interior and the black make, and each
//! group is drawn in one go. A piece is eight draw calls rather than a
//! thousand, and the picture is the same.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::gllist::GlList;
use crate::runtime::shapes::calc_normal;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
    random,
};
use std::f32::consts::PI;

/// The pieces, in the order upstream lists them.
const OBJ_BOX: usize = 0;
const OBJ_ISO_BASE_A: usize = 1;
const OBJ_ISO_BASE_B: usize = 2;
const OBJ_ISO_DEN: usize = 3;
const OBJ_ISO_DSE: usize = 4;
const OBJ_ISO_DWN: usize = 5;
const OBJ_ISO_SWD: usize = 6;
const OBJ_ISO_UNE: usize = 7;
const OBJ_ISO_UNW: usize = 8;
const OBJ_ISO_USE: usize = 9;
const OBJ_ISO_USW: usize = 10;
const OBJ_LID_A: usize = 12;
const OBJ_LID_B: usize = 13;
const OBJ_LID_BASE: usize = 14;
const OBJ_LID_C: usize = 15;
const OBJ_LID_D: usize = 16;
const OBJ_PILLAR_A: usize = 17;
const OBJ_PILLAR_B: usize = 18;
const OBJ_PILLAR_BASE: usize = 19;
const OBJ_STAR_D: usize = 20;
const OBJ_STAR_U: usize = 21;
const OBJ_TASER_A: usize = 22;
const OBJ_TASER_B: usize = 23;
const OBJ_TASER_BASE: usize = 24;
const OBJ_TETRA_BASE: usize = 25;
const OBJ_TETRA_DSE: usize = 26;
const OBJ_TETRA_DWN: usize = 27;
const OBJ_TETRA_UNE: usize = 28;
const OBJ_TETRA_USW: usize = 29;

/// The pieces themselves, in that same order. `leviathan` is in the model file
/// and is index eleven, but nothing draws it: the shape that unfolds is built
/// out of triangles here instead.
const ALL_OBJS: &[&str] = &[
    crate::models::LAMENT_MODEL_BOX,
    crate::models::LAMENT_MODEL_ISO_BASE_A,
    crate::models::LAMENT_MODEL_ISO_BASE_B,
    crate::models::LAMENT_MODEL_ISO_DEN,
    crate::models::LAMENT_MODEL_ISO_DSE,
    crate::models::LAMENT_MODEL_ISO_DWN,
    crate::models::LAMENT_MODEL_ISO_SWD,
    crate::models::LAMENT_MODEL_ISO_UNE,
    crate::models::LAMENT_MODEL_ISO_UNW,
    crate::models::LAMENT_MODEL_ISO_USE,
    crate::models::LAMENT_MODEL_ISO_USW,
    crate::models::LAMENT_MODEL_LEVIATHAN,
    crate::models::LAMENT_MODEL_LID_A,
    crate::models::LAMENT_MODEL_LID_B,
    crate::models::LAMENT_MODEL_LID_BASE,
    crate::models::LAMENT_MODEL_LID_C,
    crate::models::LAMENT_MODEL_LID_D,
    crate::models::LAMENT_MODEL_PILLAR_A,
    crate::models::LAMENT_MODEL_PILLAR_B,
    crate::models::LAMENT_MODEL_PILLAR_BASE,
    crate::models::LAMENT_MODEL_STAR_D,
    crate::models::LAMENT_MODEL_STAR_U,
    crate::models::LAMENT_MODEL_TASER_A,
    crate::models::LAMENT_MODEL_TASER_B,
    crate::models::LAMENT_MODEL_TASER_BASE,
    crate::models::LAMENT_MODEL_TETRA_BASE,
    crate::models::LAMENT_MODEL_TETRA_DSE,
    crate::models::LAMENT_MODEL_TETRA_DWN,
    crate::models::LAMENT_MODEL_TETRA_UNE,
    crate::models::LAMENT_MODEL_TETRA_USW,
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Type {
    Box,

    StarOut,
    StarRot,
    StarRotIn,
    StarRotOut,
    StarUnrot,
    StarIn,

    TetraUne,
    TetraUsw,
    TetraDwn,
    TetraDse,

    LidOpen,
    LidClose,
    LidZoom,

    TaserOut,
    TaserSlide,
    TaserSlideIn,
    TaserIn,

    PillarOut,
    PillarSpin,
    PillarIn,

    SphereOut,
    SphereIn,

    LeviathanSpin,
    LeviathanFade,
    LeviathanTwist,
    LeviathanCollapse,
    LeviathanExpand,
    LeviathanUntwist,
    LeviathanUnfade,
    LeviathanUnspin,
}

/// Ambient, diffuse, specular, shininess. Upstream's comments beside these
/// numbers name the second block specular and the third diffuse; the code that
/// reads them has it the other way round, and the code is what runs.
const EXTERIOR_COLOR: [f32; 13] = [
    0.33, 0.22, 0.03, 1.00, //
    0.78, 0.57, 0.11, 1.00, //
    0.99, 0.91, 0.81, 1.00, //
    27.80,
];
const INTERIOR_COLOR: [f32; 13] = [
    0.20, 0.20, 0.15, 1.00, //
    0.40, 0.40, 0.32, 1.00, //
    0.99, 0.99, 0.81, 1.00, //
    50.80,
];
const LEVIATHAN_COLOR: [f32; 13] = [
    0.30, 0.30, 0.30, 1.00, //
    0.85, 0.85, 0.95, 1.00, //
    0.99, 0.99, 0.99, 1.00, //
    50.80,
];
const BLACK_COLOR: [f32; 13] = [
    0.05, 0.05, 0.05, 1.00, //
    0.05, 0.05, 0.05, 1.00, //
    0.05, 0.05, 0.05, 1.00, //
    80.00,
];

/// Which skin a triangle wears, which decides both its material and its
/// texture.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Skin {
    /// The gold leaf of one of the six walls, numbered as `which_face` does.
    Outer(usize),
    /// The inside of the box.
    Inner,
    /// Unlit black, which only the two halves of the Leviathan's shell wear.
    Black,
}

/// One vertex of a prepared piece.
struct Vert {
    p: [f32; 3],
    n: [f32; 3],
    t: [f32; 2],
}

/// A piece of the box, sorted into the runs that can be drawn in one go.
struct Model {
    groups: Vec<(Skin, Vec<Vert>)>,
}

struct Lament {
    rot: Rotator,
    rotx: f64,
    roty: f64,
    rotz: f64,
    trackball: Trackball,
    ffwdp: bool,

    models: Vec<Model>,
    texids: [u32; 8],
    do_texture: bool,
    wireframe: bool,

    /// Which mode of the object is current.
    kind: Type,
    /// Countdown before animating again.
    anim_pause: i32,
    /// Relative position during animations.
    anim_r: f32,
    anim_y: f32,
    anim_z: f32,
    facing_p: bool,

    state: usize,
    states: Vec<Type>,
}

/// `which_face`: which of the six walls a triangle faces, and whether it is
/// close enough to that wall to be on the outside of the box.
fn which_face(n: &[f32; 3], v: &[f32; 3]) -> (usize, bool) {
    let size = 3.0; /* 3" square */
    let slack = 0.01;

    let (face, mut outer) = if n[1] < -0.5 {
        (1, v[1] < slack) /* S */
    } else if n[2] > 0.5 {
        (2, v[2] > size - slack) /* U */
    } else if n[1] > 0.5 {
        (3, v[1] > size - slack) /* N */
    } else if n[2] < -0.5 {
        (4, v[2] < slack) /* D */
    } else if n[0] < -0.5 {
        (5, v[0] < slack) /* W */
    } else {
        (6, v[0] > size - slack) /* E */
    };

    /* Faces that don't have normals parallel to the axes aren't external. */
    if outer
        && (n[0] > -0.95 && n[0] < 0.95)
        && (n[1] > -0.95 && n[1] < 0.95)
        && (n[2] > -0.95 && n[2] < 0.95)
    {
        outer = false;
    }
    (face, outer)
}

/// `texturize_vert`: the texture coordinates are surface coordinates, on the
/// plane of whichever cube wall this triangle belongs to.
fn texture_coord(which: usize, v: &[f32; 3]) -> [f32; 2] {
    let size = 3.0; /* 3" square */
    let (s, q) = match which {
        1 => (v[0], v[2]),
        2 => (v[0], v[1]),
        3 => (v[0], size - v[2]),
        4 => (v[0], size - v[1]),
        5 | 6 => (v[1], v[2]),
        _ => (0.0, 0.0),
    };
    [s / size, q / size]
}

/// Sort one piece's triangles into runs that share a skin, and fold the
/// transform upstream applies inside the display list into the positions:
/// the model is three inches across and the box is drawn a unit across,
/// centred.
fn prepare(text: &str, blackp: bool) -> Model {
    let list = GlList::parse(text);
    let s = 1.0 / 3.0; /* box is 3" square */
    let mut groups: Vec<(Skin, Vec<Vert>)> = Vec::new();

    for tri in list.data.chunks_exact(18) {
        let n0 = [tri[0], tri[1], tri[2]];
        let v0 = [tri[3], tri[4], tri[5]];
        let (face, outerp) = which_face(&n0, &v0);
        let skin = if outerp {
            Skin::Outer(face - 1)
        } else if blackp {
            Skin::Black
        } else {
            Skin::Inner
        };

        let slot = match groups.iter().position(|(k, _)| *k == skin) {
            Some(i) => i,
            None => {
                groups.push((skin, Vec::new()));
                groups.len() - 1
            }
        };
        for v in tri.chunks_exact(6) {
            let n = [v[0], v[1], v[2]];
            let p = [v[3], v[4], v[5]];
            groups[slot].1.push(Vert {
                p: [p[0] * s - 0.5, p[1] * s - 0.5, p[2] * s - 0.5],
                n,
                t: texture_coord(face, &p),
            });
        }
    }

    assert_eq!(
        list.primitive,
        Shape::Triangles,
        "lament's models are triangles"
    );
    Model { groups }
}

/// One of the eight square textures out of the tall picture they are stacked
/// in: the six walls of the box, then the inside, then the Leviathan.
///
/// Upstream's image loader hands a picture over from the bottom row up, which
/// is the row order OpenGL wants and the opposite of the order it is stored
/// in, so the tiles come out of the file back to front and each one of them
/// upside down. The picture reads as the Leviathan, then the woodgrain of the
/// inside, then the six walls.
fn tile(px: &[u8], w: usize, i: usize) -> Vec<u8> {
    let row = w * 4;
    let top = px.len() / row - (i + 1) * w;
    let mut out = Vec::with_capacity(row * w);
    for y in (0..w).rev() {
        let from = (top + y) * row;
        out.extend_from_slice(&px[from..from + row]);
    }
    out
}

/// `set_colors`.
fn set_colors(g: &mut Gl, c: &[f32; 13]) {
    g.glx.material_ambient([c[0], c[1], c[2], c[3]]);
    g.glx.material_diffuse([c[4], c[5], c[6], c[7]]);
    g.glx.material_specular([c[8], c[9], c[10], c[11]]);
    g.glx.material_shininess(c[12]);
}

/// `set_colors_alpha`.
fn set_colors_alpha(g: &mut Gl, c: &[f32; 13], a: f32) {
    let mut c = *c;
    c[3] = a;
    c[7] = a;
    c[11] = a;
    set_colors(g, &c);
}

impl Lament {
    fn bind(&self, g: &mut Gl, which: Option<usize>) {
        match which {
            Some(i) if self.do_texture => {
                g.glx.texturing(true);
                g.glx.bind_texture(self.texids[i]);
            }
            _ => g.glx.texturing(false),
        }
    }

    /// One piece of the box, in as few draw calls as it has skins.
    fn draw_model(&self, g: &mut Gl, which: usize) {
        let wire = self.wireframe;
        for (skin, verts) in &self.models[which].groups {
            match skin {
                Skin::Outer(f) => {
                    set_colors(g, &EXTERIOR_COLOR);
                    self.bind(g, Some(*f));
                }
                Skin::Inner => {
                    set_colors(g, &INTERIOR_COLOR);
                    self.bind(g, Some(6));
                }
                Skin::Black => {
                    set_colors(g, &BLACK_COLOR);
                    self.bind(g, None);
                }
            }

            if wire {
                // Upstream closes a line loop around each triangle, so the
                // shared edges are drawn twice; the same three lines a
                // triangle at a time come out identical and in one call.
                g.glx.begin(Shape::Lines);
                for t in verts.chunks_exact(3) {
                    for i in 0..3 {
                        for v in [&t[i], &t[(i + 1) % 3]] {
                            g.glx.vertex3f(v.p[0], v.p[1], v.p[2]);
                        }
                    }
                }
                g.glx.end();
                continue;
            }

            g.glx.begin(Shape::Triangles);
            for v in verts {
                g.glx.tex_coord2f(v.t[0], v.t[1]);
                g.glx.normal3f(v.n[0], v.n[1], v.n[2]);
                g.glx.vertex3f(v.p[0], v.p[1], v.p[2]);
            }
            g.glx.end();
        }
    }

    /// `facing_screen_p`: is a point five inches in front of the door near the
    /// middle of the screen? If it is, the box is looking at you, and the lid
    /// opening is worth zooming into.
    fn facing_screen_p(&self, g: &Gl) -> bool {
        let m = g.glx.modelview_matrix();
        let p = g.glx.projection_matrix();
        let ndc = p.mul(&m).transform([0.0, -5.0, 0.0]);
        // `gluProject` returns window coordinates, which upstream then divides
        // by the window size and shifts to the middle; that is half of the
        // normalised coordinates, whatever the viewport is.
        let (x, y, z) = (ndc[0] / 2.0, ndc[1] / 2.0, ndc[2] / 2.0 + 0.5);
        z < 0.9 && x > -0.15 && x < 0.15 && y > -0.15 && y < 0.15
    }

    /// `scale_for_window`: roughly the width of the window, but never so large
    /// that the 512-square texture starts to look blocky.
    fn scale_for_window(&self, g: &mut Gl) {
        let target_size = 1.4 * 512.0;
        let (w, h) = (g.width() as f32, g.height() as f32);
        let size = w.min(h);

        /* Make it take up roughly the full width of the window. */
        let mut scale = 20.0;

        /* But if the window is wider than tall, make it only take up the
        height of the window instead. */
        if w > h {
            scale /= w / h;
        }

        /* If the window is super wide, make it bigger. */
        if scale < 8.0 {
            scale = 8.0;
        }

        /* Constrain it to roughly life-sized on the screen, not huge. */
        let mut target_size = target_size;
        let mut max = 500.0; /* 3" on my screen... */
        if w > 2560.0 {
            /* Retina displays */
            target_size *= 2.5;
            max *= 2.5;
        }
        if target_size > max {
            target_size = max;
        }

        /* But if that would make the image larger than target_size, scale it
        back down again. */
        if size > target_size {
            scale *= target_size / size;
        }

        g.glx.scale(scale, scale, scale);
    }

    /// The shape the box unfolds into: three long triangles meeting at a
    /// point, lined up with the cube's diagonal.
    fn leviathan(&self, g: &mut Gl, ratio: f32, alpha: f32, top_p: bool) {
        let wire = self.wireframe;
        let r = 0.34;
        let z = 2.0 * ratio;

        let th = (2.0 / 6.0f32.sqrt()).acos(); /* Line up with cube's diagonal */

        g.glx.push_matrix();
        g.glx.rotate(-45.0, 0.0, 1.0, 0.0);
        g.glx.rotate(-th * 180.0 / PI, 0.0, 0.0, 1.0);
        if !top_p {
            g.glx.rotate(180.0, 0.0, 0.0, 1.0);
        }

        let p: Vec<[f32; 2]> = (0..3)
            .map(|i| {
                let th = i as f32 * PI * 2.0 / 3.0;
                [th.cos() * r, th.sin() * r]
            })
            .collect();

        g.glx.front_face_cw(false);
        for i in 0..3 {
            let j = (i + 1) % 3;
            let n = calc_normal(
                [z, 0.0, 0.0],
                [0.0, p[i][0], p[i][1]],
                [0.0, p[j][0], p[j][1]],
            );

            /* Leviathan is the final texture */
            self.bind(g, Some(self.texids.len() - 1));
            set_colors(g, &LEVIATHAN_COLOR);

            g.glx.normal3f(n[0], n[1], n[2]);
            g.glx.begin(if wire {
                Shape::LineLoop
            } else {
                Shape::Triangles
            });
            g.glx.tex_coord2f(0.5, 1.0);
            g.glx.vertex3f(z, 0.0, 0.0);
            g.glx.tex_coord2f(0.0, 0.0);
            g.glx.vertex3f(0.0, p[i][0], p[i][1]);
            g.glx.tex_coord2f(1.0, 0.0);
            g.glx.vertex3f(0.0, p[j][0], p[j][1]);
            g.glx.end();

            /* Shield for fading */
            if alpha < 0.9 && !wire {
                let a = 0.35;
                let b = 0.69;
                set_colors_alpha(g, &BLACK_COLOR, 1.0 - alpha);
                self.bind(g, None);
                g.glx.blend(Blend::Alpha);
                g.glx.begin(Shape::Quads);
                g.glx.vertex3f(z * a, p[j][0] * b, p[j][1] * b);
                g.glx.vertex3f(z * a, p[i][0] * b, p[i][1] * b);
                g.glx.vertex3f(0.0, p[i][0] * 1.01, p[i][1] * 1.01);
                g.glx.vertex3f(0.0, p[j][0] * 1.01, p[j][1] * 1.01);
                g.glx.end();
                g.glx.blend(Blend::Off);
            }
        }

        g.glx.pop_matrix();
    }

    /// The three walls that fold away behind the Leviathan, fading out as they
    /// go.
    fn folding_walls(&self, g: &mut Gl, ratio: f32, top_p: bool) {
        let wire = self.wireframe;
        let pa = [
            [-0.5, -0.215833f32],
            [0.0, 0.5],
            [0.5, 0.0],
            [-0.215833, -0.5],
        ];
        let tex = [0usize, 5, 1, 4, 2, 3];
        let top = -pa[0][1];
        let end_angle = 30.85;
        let rr = (ratio / 2.0 * PI).sin();
        let offa = 0.15 * rr;
        let offb = 0.06 * rr;

        g.glx.push_matrix();
        if top_p {
            g.glx.rotate(60.0, 1.0, -1.0, 1.0);
            g.glx.rotate(180.0, 0.0, 1.0, 0.0);
            g.glx.rotate(90.0, 1.0, 0.0, 0.0);
        } else {
            g.glx.rotate(180.0, 1.0, 0.0, 0.0);
        }

        /* Scale down the points near the axis */
        let p = [
            [pa[0][0], 0.5, pa[0][1]],
            [pa[1][0] - offb, 0.5, pa[1][1] - offa],
            [pa[2][0] - offa, 0.5, pa[2][1] - offb],
            [pa[3][0], 0.5, pa[3][1]],
        ];

        if !wire {
            g.glx.blend(Blend::Alpha);
        }

        for i in 0..3 {
            g.glx.push_matrix();

            if i == 1 {
                g.glx.rotate(-90.0, 1.0, 0.0, 0.0);
                g.glx.rotate(180.0, 1.0, 1.0, 0.0);
            } else if i == 2 {
                g.glx.rotate(-90.0, 1.0, 0.0, 0.0);
                g.glx.rotate(180.0, 0.0, 1.0, 0.0);
                g.glx.rotate(90.0, 0.0, 1.0, 0.0);
            }

            g.glx.rotate(-90.0, 0.0, 1.0, 0.0);

            g.glx
                .translate(-(top / 2.0 + 0.25), 0.5, -(top / 2.0 + 0.25));
            g.glx.rotate(-45.0, 0.0, 1.0, 0.0);
            g.glx.rotate(ratio * -end_angle, 0.0, 0.0, 1.0);
            g.glx.rotate(45.0, 0.0, 1.0, 0.0);
            g.glx.translate(top / 2.0 + 0.25, -0.5, top / 2.0 + 0.25);

            /* Get the texture coordinates right.
            This is hairy and incomprehensible. */
            let mut t = [
                [pa[0][1] + 0.5, pa[0][0] + 0.5],
                [pa[1][1] + 0.5, pa[1][0] + 0.5],
                [pa[2][1] + 0.5, pa[2][0] + 0.5],
                [pa[3][1] + 0.5, pa[3][0] + 0.5],
            ];

            if i == 0 {
                // Upstream spells both of these as a macro called SWAP, and
                // neither of them swaps anything: one turns both coordinates
                // of every corner around, the other only the first, and the
                // second writes its other half into a scratch array that is
                // then dropped.
                for c in &mut t {
                    c[0] = 1.0 - c[0];
                    if !top_p {
                        c[1] = 1.0 - c[1];
                    }
                }
            } else if i == 1 {
                for c in &mut t {
                    let f = c[0];
                    c[0] = c[1];
                    c[1] = -f;
                }
            }

            set_colors_alpha(g, &EXTERIOR_COLOR, 1.0 - ratio);
            self.bind(g, Some(tex[i + if top_p { 3 } else { 0 }]));

            let n = calc_normal(p[0], p[1], p[2]);
            g.glx.normal3f(n[0], n[1], n[2]);
            g.glx
                .begin(if wire { Shape::LineLoop } else { Shape::Quads });
            for k in 0..4 {
                g.glx.tex_coord2f(t[k][0], t[k][1]);
                g.glx.vertex3f(p[k][0], p[k][1], p[k][2]);
            }
            g.glx.end();

            // Upstream would draw two black triangles between the quads here,
            // and does not, because of a gap between them it could not find.
            // The shield in `leviathan` stands in for them.

            g.glx.pop_matrix();
        }

        if !wire {
            g.glx.blend(Blend::Off);
        }
        g.glx.pop_matrix();
    }

    /// The box swelling into a sphere: each wall is a sixteen by sixteen grid
    /// whose corners are pushed out along their own radius.
    fn lament_sphere(&self, g: &mut Gl, ratio: f32) -> usize {
        let wire = self.wireframe;
        let size = 3.0; /* 3" square */
        let mut polys = 0;
        let facets = 16; /* NxN grid on each face */
        let norms: [[f32; 3]; 6] = [
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, -1.0],
            [-1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
        ];
        let s = 1.0 / facets as f32;

        /* The ratio used for the normals: linger on the square normals. */
        let ratio2 = 1.0 - ((1.0 - ratio) / 2.0 * PI).sin();
        let r1 = 1.0 - ratio2 / 2.0;
        let r2 = ratio2 / 2.0;

        g.glx.push_matrix();
        g.glx.translate(-0.5, -0.5, -0.5);
        g.glx.scale(1.0 / size, 1.0 / size, 1.0 / size);

        set_colors(g, &EXTERIOR_COLOR);

        for (face, norm3) in norms.iter().enumerate() {
            let frontp = if norm3[0] != 0.0 {
                norm3[0] < 0.0
            } else if norm3[1] != 0.0 {
                norm3[1] > 0.0
            } else {
                norm3[2] < 0.0
            };

            self.bind(g, if wire { None } else { Some(face) });
            g.glx.front_face_cw(frontp);
            g.glx
                .begin(if wire { Shape::LineLoop } else { Shape::Quads });

            for yi in 0..facets {
                for xi in 0..facets {
                    let x0 = xi as f32 * s;
                    let y0 = yi as f32 * s;
                    let x1 = x0 + s;
                    let y1 = y0 + s;

                    /* verts of the cube */
                    let mut pa = [[0.0f32; 3]; 4];
                    let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
                    for (i, (u, v)) in corners.iter().enumerate() {
                        if norm3[0] != 0.0 {
                            pa[i] = [if frontp { 0.0 } else { 1.0 }, *u, *v];
                        } else if norm3[1] != 0.0 {
                            pa[i] = [*u, if frontp { 1.0 } else { 0.0 }, *v];
                        } else {
                            pa[i] = [*u, *v, if frontp { 0.0 } else { 1.0 }];
                        }
                        for c in &mut pa[i] {
                            *c *= size;
                        }
                    }

                    /* Convert square to sphere by treating as a normalized
                    vector */
                    let mut pb = [[0.0f32; 3]; 4];
                    for (b, a) in pb.iter_mut().zip(&pa) {
                        let x = (a[0] / size) - 0.5;
                        let y = (a[1] / size) - 0.5;
                        let z = (a[2] / size) - 0.5;
                        let d = (x * x + y * y + z * z).sqrt() / 2.0;
                        let q = [x / d + size / 2.0, y / d + size / 2.0, z / d + size / 2.0];
                        for k in 0..3 {
                            b[k] = a[k] + ((q[k] - a[k]) * ratio);
                        }
                    }

                    /* The normals of an intermediate point are the weighted
                    average of the cube's orthogonal normals, and the sphere's
                    radial normals: early in the sequence, the edges are sharp,
                    but they soften as it expands. */
                    let na = calc_normal(pa[0], pa[1], pa[2]);
                    let mut norm = [[0.0f32; 3]; 4];
                    for (n, b) in norm.iter_mut().zip(&pb) {
                        let d = (b[0] * b[0] + b[1] * b[1] + b[2] * b[2]).sqrt();
                        for k in 0..3 {
                            n[k] = (na[k] * r1) + ((b[k] / d) * r2);
                        }
                    }

                    for ((a, b), n) in pa.iter().zip(&pb).zip(&norm) {
                        let t = texture_coord(face + 1, a);
                        g.glx.tex_coord2f(t[0], t[1]);
                        g.glx.normal3f(n[0], n[1], n[2]);
                        g.glx.vertex3f(b[0], b[1], b[2]);
                    }
                    polys += 1;
                }
            }
            g.glx.end();
        }

        g.glx.pop_matrix();
        polys
    }

    /// `shuffle_states`. Rather than picking states randomly, pick an ordering
    /// randomly, do it, and then re-randomise; that way one is assured of
    /// seeing all of them in a short time. States appear in the list several
    /// times over, which is how they get their probabilities.
    fn shuffle_states(&mut self) {
        for i in 0..self.states.len() {
            let a = random() as usize % self.states.len();
            self.states.swap(a, i);
        }
    }

    fn animate(&mut self) {
        let pause = 10;
        let pause2 = 120.0;
        let speed = if self.ffwdp { 20.0 } else { 1.0 };

        match self.kind {
            Type::Box => {
                self.state += 1;
                if self.state >= self.states.len() {
                    self.shuffle_states();
                    self.state = 0;
                }
                self.kind = self.states[self.state];
                if self.kind == Type::Box {
                    self.anim_pause = pause2 as i32;
                }
                self.anim_r = 0.0;
                self.anim_y = 0.0;
                self.anim_z = 0.0;
            }

            /* ---------------------------------------------------------- */
            Type::StarOut => {
                self.anim_z += 0.01 * speed;
                if self.anim_z >= 1.0 {
                    self.anim_z = 1.0;
                    self.kind = Type::StarRot;
                    self.anim_pause = pause;
                }
            }
            Type::StarRot => {
                self.anim_r += 1.0 * speed;
                if self.anim_r >= 45.0 {
                    self.anim_r = 45.0;
                    self.kind = Type::StarRotIn;
                    self.anim_pause = pause;
                }
            }
            Type::StarRotIn => {
                self.anim_z -= 0.01 * speed;
                if self.anim_z <= 0.0 {
                    self.anim_z = 0.0;
                    self.kind = Type::StarRotOut;
                    self.anim_pause =
                        (pause2 * (1.0 + frand(2.0) as f32 + frand(2.0) as f32)) as i32;
                }
            }
            Type::StarRotOut => {
                self.anim_z += 0.01 * speed;
                if self.anim_z >= 1.0 {
                    self.anim_z = 1.0;
                    self.kind = Type::StarUnrot;
                    self.anim_pause = pause;
                }
            }
            Type::StarUnrot => {
                self.anim_r -= 1.0 * speed;
                if self.anim_r <= 0.0 {
                    self.anim_r = 0.0;
                    self.kind = Type::StarIn;
                    self.anim_pause = pause;
                }
            }
            Type::StarIn => {
                self.anim_z -= 0.01 * speed;
                if self.anim_z <= 0.0 {
                    self.anim_z = 0.0;
                    self.kind = Type::Box;
                    self.anim_pause = pause2 as i32;
                }
            }

            /* ---------------------------------------------------------- */
            Type::TetraUne | Type::TetraUsw | Type::TetraDwn | Type::TetraDse => {
                self.anim_r += 1.0 * speed;
                if self.anim_r >= 360.0 {
                    self.anim_r = 0.0;
                    self.kind = Type::Box;
                    self.anim_pause = pause2 as i32;
                } else if self.anim_r > 119.0 && self.anim_r <= 120.0 {
                    self.anim_r = 120.0;
                    self.anim_pause = pause;
                } else if self.anim_r > 239.0 && self.anim_r <= 240.0 {
                    self.anim_r = 240.0;
                    self.anim_pause = pause;
                }
            }

            /* ---------------------------------------------------------- */
            Type::LidOpen => {
                self.anim_r += 1.0 * speed;
                if self.anim_r >= 112.0 {
                    self.anim_r = 112.0;
                    self.anim_z = 0.0;
                    self.anim_pause = pause2 as i32;
                    self.kind = if self.facing_p {
                        Type::LidZoom
                    } else {
                        Type::LidClose
                    };
                }
            }
            Type::LidClose => {
                self.anim_r -= 1.0 * speed;
                if self.anim_r <= 0.0 {
                    self.anim_r = 0.0;
                    self.kind = Type::Box;
                    self.anim_pause = pause2 as i32;
                }
            }
            Type::LidZoom => {
                self.anim_z += 0.01 * speed;
                if self.anim_z > 1.0 {
                    self.anim_r = 0.0;
                    self.anim_z = 0.0;
                    self.kind = Type::Box;
                }
            }

            /* ---------------------------------------------------------- */
            Type::TaserOut => {
                self.anim_z += 0.005 * speed;
                if self.anim_z >= 0.5 {
                    self.anim_z = 0.5;
                    self.kind = Type::TaserSlide;
                    self.anim_pause =
                        (pause as f32 * (1.0 + frand(5.0) as f32 + frand(5.0) as f32)) as i32;
                }
            }
            Type::TaserSlide => {
                self.anim_y += 0.005 * speed;
                if self.anim_y >= 0.255 {
                    self.anim_y = 0.255;
                    self.kind = Type::TaserSlideIn;
                    self.anim_pause =
                        (pause2 * (1.0 + frand(5.0) as f32 + frand(5.0) as f32)) as i32;
                }
            }
            Type::TaserSlideIn => {
                self.anim_y -= 0.0025 * speed;
                if self.anim_y <= 0.0 {
                    self.anim_y = 0.0;
                    self.kind = Type::TaserIn;
                    self.anim_pause = pause;
                }
            }
            Type::TaserIn => {
                self.anim_z -= 0.0025 * speed;
                if self.anim_z <= 0.0 {
                    self.anim_z = 0.0;
                    self.kind = Type::Box;
                    self.anim_pause = pause2 as i32;
                }
            }

            /* ---------------------------------------------------------- */
            Type::PillarOut => {
                if self.anim_y == 0.0 {
                    /* mostly in */
                    self.anim_y += 0.005
                        * if random().is_multiple_of(5) {
                            1.0
                        } else {
                            -1.0
                        }
                        * speed;
                } else if self.anim_y > 0.0 {
                    self.anim_y += 0.005 * speed;
                } else {
                    self.anim_y -= 0.001 * speed;
                }

                if self.anim_z == 0.0 {
                    let i = random() % 7; /* A, B or both */
                    self.anim_z = if i == 0 {
                        3.0
                    } else if i < 5 {
                        2.0
                    } else {
                        1.0
                    };
                    /* We can do quarter turns, because it's radially
                    symmetrical. */
                    self.anim_r = 90.0
                        * (1.0 + frand(6.0) as f32)
                        * if random() & 1 != 0 { 1.0 } else { -1.0 };
                }
                if self.anim_y > 0.4 {
                    self.anim_y = 0.4;
                    self.kind = Type::PillarSpin;
                    self.anim_pause = pause;
                } else if self.anim_y < -0.03 {
                    self.anim_y = -0.03;
                    self.kind = Type::PillarSpin;
                    self.anim_pause = pause;
                }
            }
            Type::PillarSpin => {
                let negp = self.anim_r < 0.0;
                self.anim_r += if negp { 1.0 } else { -1.0 } * speed;
                if if negp {
                    self.anim_r > 0.0
                } else {
                    self.anim_r < 0.0
                } {
                    self.anim_r = 0.0;
                    self.kind = Type::PillarIn;
                }
            }
            Type::PillarIn => {
                let negp = self.anim_y < 0.0;
                self.anim_y += if negp { 1.0 } else { -1.0 } * 0.005 * speed;
                if if negp {
                    self.anim_y > 0.0
                } else {
                    self.anim_y < 0.0
                } {
                    self.anim_y = 0.0;
                    self.anim_z = 0.0;
                    self.kind = Type::Box;
                    self.anim_pause = pause;
                }
            }

            /* ---------------------------------------------------------- */
            Type::SphereOut => {
                self.anim_y += 0.01 * speed;
                if self.anim_y >= 1.0 {
                    self.anim_y = 1.0;
                    self.kind = Type::SphereIn;
                    self.anim_pause =
                        (pause2 * (1.0 + frand(1.0) as f32 + frand(1.0) as f32)) as i32;
                }
            }
            Type::SphereIn => {
                self.anim_y -= 0.01 * speed;
                if self.anim_y <= 0.0 {
                    self.anim_y = 0.0;
                    self.kind = Type::Box;
                    self.anim_pause = pause;
                }
            }

            /* ---------------------------------------------------------- */
            Type::LeviathanSpin => {
                self.anim_r += 3.5 * speed;
                if self.anim_r >= 360.0 * 3.0 {
                    self.anim_r = 0.0;
                    self.kind = Type::LeviathanFade;
                    self.anim_pause = 0;
                }
            }
            Type::LeviathanFade => {
                self.anim_z += 0.01 * speed;
                if self.anim_z >= 1.0 {
                    self.anim_z = 1.0;
                    self.kind = Type::LeviathanTwist;
                    self.anim_pause = 0;
                }
            }
            Type::LeviathanTwist => {
                self.anim_y += 2.0 * speed;
                self.anim_z = 1.0;
                if self.anim_y >= 180.0 {
                    self.anim_y = 0.0;
                    self.kind = Type::LeviathanCollapse;
                    self.anim_pause = 0;
                }
            }
            Type::LeviathanCollapse => {
                self.anim_y += 0.01 * speed;
                if self.anim_y >= 1.0 {
                    self.anim_y = 1.0;
                    self.kind = Type::LeviathanExpand;
                    self.anim_pause = pause2 as i32 * 4;
                }
            }
            Type::LeviathanExpand => {
                self.anim_y -= 0.005 * speed;
                if self.anim_y <= 0.0 {
                    self.anim_y = 180.0;
                    self.kind = Type::LeviathanUntwist;
                }
            }
            Type::LeviathanUntwist => {
                self.anim_y -= 2.0 * speed;
                self.anim_z = 1.0;
                if self.anim_y <= 0.0 {
                    self.anim_y = 0.0;
                    self.kind = Type::LeviathanUnfade;
                    self.anim_pause = 0;
                }
            }
            Type::LeviathanUnfade => {
                self.anim_z -= 0.1 * speed;
                if self.anim_z <= 0.0 {
                    self.anim_z = 0.0;
                    self.kind = Type::LeviathanUnspin;
                    self.anim_pause = 0;
                }
            }
            Type::LeviathanUnspin => {
                self.anim_r += 3.5 * speed;
                if self.anim_r >= 360.0 * 2.0 {
                    self.anim_r = 0.0;
                    self.kind = Type::Box;
                    self.anim_pause = pause2 as i32;
                }
            }
        }

        if self.ffwdp && self.kind == Type::Box {
            self.ffwdp = false;
            while self.kind == Type::Box {
                self.animate();
            }
            self.anim_pause = 0;
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wireframe = g.res.bool("wireframe");
    let rot_speed = 0.5;

    let mut this = Lament {
        rot: Rotator::new(rot_speed, rot_speed, rot_speed, 1.0, 0.0, true),
        rotx: 0.0,
        roty: 0.0,
        rotz: 0.0,
        trackball: Trackball::new(),
        ffwdp: false,
        models: Vec::new(),
        texids: [0; 8],
        do_texture: g.res.bool("texture") && !wireframe,
        wireframe,
        kind: Type::Box,
        anim_pause: 300 + (random() % 100) as i32,
        anim_r: 0.0,
        anim_y: 0.0,
        anim_z: 0.0,
        facing_p: false,
        state: 0,
        states: Vec::new(),
    };

    if this.do_texture {
        // One 512-wide picture of eight square tiles stacked up.
        if let Some((w, h, px)) = crate::runtime::png::decode_rgba(crate::images::LAMENT512)
            && h == w * this.texids.len() as i32
        {
            for (i, id) in this.texids.iter_mut().enumerate() {
                *id = g.glx.gen_texture();
                g.glx.bind_texture(*id);
                g.glx.tex_image_2d(w, w, tile(&px, w as usize, i));
                g.glx.tex_clamp(false);
                g.glx.tex_nearest(true);
            }
        }
        if this.texids[0] == 0 {
            this.do_texture = false;
        }
    }

    this.models = ALL_OBJS
        .iter()
        .enumerate()
        .map(|(i, text)| prepare(text, i == OBJ_ISO_BASE_A || i == OBJ_ISO_BASE_B))
        .collect();

    let mut push = |n: usize, which: Type| {
        for _ in 0..n {
            this.states.push(which);
        }
    };
    push(4, Type::TetraUne); /* most common */
    push(4, Type::TetraUsw);
    push(4, Type::TetraDwn);
    push(4, Type::TetraDse);

    push(8, Type::StarOut); /* pretty common */
    push(8, Type::TaserOut);
    push(8, Type::PillarOut);

    push(4, Type::LidOpen); /* rare */
    push(2, Type::SphereOut); /* rare */
    push(1, Type::LeviathanSpin); /* very rare */

    push(35, Type::Box); /* rest state */
    this.shuffle_states();

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Lament {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let h = height as f32 / width as f32;
        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.frustum(-1.0, 1.0, -h, h, 5.0, 60.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx.translate(0.0, 0.0, -40.0);
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if let XEvent::KeyPress { key } = event
            && (*key == ' ' || *key == '\t')
        {
            self.ffwdp = true;
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        let wire = self.wireframe;
        g.glx.clear();
        if !wire {
            g.glx.depth_test(true);
            g.glx.cull_face(true);
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
            g.glx.light_position(0, -4.0, 2.0, 5.0, 1.0);
            g.glx.light_ambient(0, [0.7, 0.7, 0.7, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        }
        g.glx.front_face_cw(false);

        g.glx.push_matrix();

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        /* Make into the screen be +Y, right be +X, and up be +Z. */
        g.glx.rotate(-90.0, 1.0, 0.0, 0.0);

        self.scale_for_window(g);

        /* Apply rotation to the object. */
        if self.kind != Type::LidZoom {
            let (x, y, z) = self.rot.rotation(!self.trackball.button_down());
            self.rotx = x;
            self.roty = y;
            self.rotz = z;
        }
        g.glx.rotate(self.rotx as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(self.roty as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(self.rotz as f32 * 360.0, 0.0, 0.0, 1.0);

        g.glx.scale(0.5, 0.5, 0.5);

        match self.kind {
            Type::Box => self.draw_model(g, OBJ_BOX),

            Type::StarOut
            | Type::StarRot
            | Type::StarRotIn
            | Type::StarRotOut
            | Type::StarUnrot
            | Type::StarIn => {
                g.glx.translate(0.0, 0.0, self.anim_z / 2.0);
                g.glx.rotate(self.anim_r / 2.0, 0.0, 0.0, 1.0);
                self.draw_model(g, OBJ_STAR_U);

                g.glx.translate(0.0, 0.0, -self.anim_z);
                g.glx.rotate(-self.anim_r, 0.0, 0.0, 1.0);
                self.draw_model(g, OBJ_STAR_D);
            }

            Type::TetraUne | Type::TetraUsw | Type::TetraDwn | Type::TetraDse => {
                let (magic, x, y, z) = match self.kind {
                    Type::TetraUne => (OBJ_TETRA_UNE, 1.0, 1.0, 1.0),
                    Type::TetraUsw => (OBJ_TETRA_USW, 1.0, 1.0, -1.0),
                    Type::TetraDwn => (OBJ_TETRA_DWN, 1.0, -1.0, 1.0),
                    _ => (OBJ_TETRA_DSE, -1.0, 1.0, 1.0),
                };
                self.draw_model(g, OBJ_TETRA_BASE);
                for o in [OBJ_TETRA_UNE, OBJ_TETRA_USW, OBJ_TETRA_DWN, OBJ_TETRA_DSE] {
                    if o != magic {
                        self.draw_model(g, o);
                    }
                }
                g.glx.rotate(self.anim_r, x, y, z);
                self.draw_model(g, magic);
            }

            Type::LidOpen | Type::LidClose | Type::LidZoom => {
                let d = 0.21582;
                let lists = [OBJ_LID_A, OBJ_LID_B, OBJ_LID_C, OBJ_LID_D];

                self.facing_p = self.facing_screen_p(g);

                if self.anim_z < 0.5 {
                    g.glx.translate(0.0, -30.0 * self.anim_z, 0.0); /* zoom */
                } else {
                    g.glx.translate(8.0 * (0.5 - (self.anim_z - 0.5)), 0.0, 0.0);
                }

                self.draw_model(g, OBJ_LID_BASE);
                for (i, o) in lists.iter().enumerate() {
                    g.glx.push_matrix();
                    g.glx.rotate(90.0 * i as f32, 0.0, 1.0, 0.0);
                    g.glx.translate(-d, -0.5, d);
                    g.glx.rotate(-45.0, 0.0, 1.0, 0.0);
                    g.glx.rotate(-self.anim_r, 1.0, 0.0, 0.0);
                    g.glx.rotate(45.0, 0.0, 1.0, 0.0);
                    g.glx.translate(d, 0.5, -d);
                    g.glx.rotate(-90.0 * i as f32, 0.0, 1.0, 0.0);
                    self.draw_model(g, *o);
                    g.glx.pop_matrix();
                }
            }

            Type::TaserOut | Type::TaserSlide | Type::TaserSlideIn | Type::TaserIn => {
                g.glx.translate(0.0, -self.anim_z / 2.0, 0.0);
                self.draw_model(g, OBJ_TASER_BASE);

                g.glx.translate(0.0, self.anim_z, 0.0);
                self.draw_model(g, OBJ_TASER_A);

                g.glx.translate(self.anim_y, 0.0, 0.0);
                self.draw_model(g, OBJ_TASER_B);
            }

            Type::PillarOut | Type::PillarSpin | Type::PillarIn => {
                self.draw_model(g, OBJ_PILLAR_BASE);

                g.glx.push_matrix();
                if self.anim_z == 1.0 || self.anim_z == 3.0 {
                    g.glx.rotate(self.anim_r, 0.0, 0.0, 1.0);
                    g.glx.translate(0.0, 0.0, self.anim_y);
                }
                self.draw_model(g, OBJ_PILLAR_A);
                g.glx.pop_matrix();

                g.glx.push_matrix();
                if self.anim_z == 2.0 || self.anim_z == 3.0 {
                    g.glx.rotate(self.anim_r, 0.0, 0.0, 1.0);
                    g.glx.translate(0.0, 0.0, -self.anim_y);
                }
                self.draw_model(g, OBJ_PILLAR_B);
                g.glx.pop_matrix();
            }

            Type::SphereOut | Type::SphereIn => {
                self.lament_sphere(g, self.anim_y);
            }

            Type::LeviathanSpin
            | Type::LeviathanUnspin
            | Type::LeviathanFade
            | Type::LeviathanUnfade
            | Type::LeviathanTwist
            | Type::LeviathanUntwist => {
                /* These normals are hard to compute, so I pulled them from the
                model. */
                let axes: [(usize, f32, f32, f32); 6] = [
                    (OBJ_ISO_UNE, 0.633994, 0.442836, 0.633994),
                    (OBJ_ISO_USW, 0.442836, 0.633994, -0.633994),
                    (OBJ_ISO_DSE, -0.633994, 0.633994, 0.442836),
                    (OBJ_ISO_SWD, -0.633994, -0.442836, -0.633994),
                    (OBJ_ISO_DEN, -0.442836, -0.633994, 0.633994),
                    (OBJ_ISO_UNW, 0.633994, -0.633994, -0.442836),
                ];

                let mut s = 1.0 - self.anim_z;
                let mut s2 = (360.0 - self.anim_r).max(0.0) / 360.0;
                match self.kind {
                    Type::LeviathanSpin => {}
                    Type::LeviathanUnspin => s2 = 1.0 - s2,
                    // The rest of them fade rather than spin. Upstream then
                    // blends with GL_CONSTANT_ALPHA, which its own OpenGL ES
                    // build has no glBlendColor for and leaves out; so does
                    // this, and the pieces cut away rather than dissolve.
                    _ => s2 = 0.0,
                }
                s = (s * 0.6) + 0.4;

                self.leviathan(g, 1.0 - s2, 1.0, true);
                self.draw_model(g, OBJ_ISO_BASE_A);

                g.glx.push_matrix();
                g.glx.scale(s2, s2, s2);
                self.draw_model(g, OBJ_ISO_USE);
                g.glx.pop_matrix();

                g.glx.push_matrix();
                g.glx.rotate(self.anim_y, 1.0, -1.0, 1.0);
                self.draw_model(g, OBJ_ISO_BASE_B);
                self.leviathan(g, 1.0 - s2, 1.0, false);
                g.glx.pop_matrix();

                for (i, (obj, x, y, z)) in axes.iter().enumerate() {
                    g.glx.push_matrix();
                    g.glx.rotate(self.anim_r, *x, *y, *z);
                    g.glx.scale(s, s, s);
                    self.draw_model(g, *obj);
                    g.glx.pop_matrix();
                    if i == 2 {
                        g.glx.rotate(self.anim_y, 1.0, -1.0, 1.0);
                    }
                }

                g.glx.push_matrix();
                g.glx.scale(s2, s2, s2);
                self.draw_model(g, OBJ_ISO_DWN);
                g.glx.pop_matrix();
            }

            Type::LeviathanCollapse | Type::LeviathanExpand => {
                g.glx.push_matrix();
                self.leviathan(g, 1.0, self.anim_y, true);
                g.glx.rotate(180.0, 1.0, -1.0, 1.0);
                self.leviathan(g, 1.0, self.anim_y, false);
                g.glx.pop_matrix();
                self.folding_walls(g, self.anim_y, true);
                self.folding_walls(g, self.anim_y, false);
            }
        }

        g.glx.pop_matrix();

        if !self.ffwdp && self.anim_pause > 0 {
            self.anim_pause -= 1;
        } else {
            self.animate();
        }

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        20000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*suppressRotationAnimation: True",
    "*texture:      True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::boolean("texture", "Textured", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "lament",
    label: "Lament",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "1998",
        video: Some("https://www.youtube.com/watch?v=-TBqI4YKOKI"),
        blurb: "Lemarchand's Box, the Lament Configuration. \
                Warning: occasionally opens doors.",
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

    #[test]
    fn the_outside_of_the_box_is_gold_and_the_inside_is_not() {
        // The box piece is the whole closed cube, so it has all six walls in
        // gold leaf and an inside as well.
        let m = prepare(crate::models::LAMENT_MODEL_BOX, false);
        let mut walls = Vec::new();
        let mut inner = 0;
        for (skin, verts) in &m.groups {
            match skin {
                Skin::Outer(f) => walls.push(*f),
                Skin::Inner => inner += verts.len(),
                Skin::Black => panic!("the box is not black anywhere"),
            }
        }
        walls.sort_unstable();
        assert_eq!(walls, [0, 1, 2, 3, 4, 5]);
        assert!(inner > 0, "the box has no inside");

        // And every triangle is somewhere in one of the groups.
        let total: usize = m.groups.iter().map(|(_, v)| v.len()).sum();
        assert_eq!(total, 1584);
    }

    #[test]
    fn the_box_is_a_unit_across_and_centred() {
        // The model is three inches square with a corner at the origin; the
        // saver draws it a unit across, centred, so that the rotation is about
        // the middle of it.
        let m = prepare(crate::models::LAMENT_MODEL_BOX, false);
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for (_, verts) in &m.groups {
            for v in verts {
                for k in 0..3 {
                    lo[k] = lo[k].min(v.p[k]);
                    hi[k] = hi[k].max(v.p[k]);
                }
            }
        }
        for k in 0..3 {
            assert!((lo[k] + 0.5).abs() < 1e-5, "{lo:?}");
            assert!((hi[k] - 0.5).abs() < 1e-5, "{hi:?}");
        }
    }

    #[test]
    fn the_shells_of_the_leviathan_are_black_inside() {
        let m = prepare(crate::models::LAMENT_MODEL_ISO_BASE_A, true);
        assert!(m.groups.iter().any(|(s, _)| *s == Skin::Black));
        assert!(m.groups.iter().all(|(s, _)| *s != Skin::Inner));
    }

    #[test]
    fn a_piece_is_a_handful_of_draw_calls_not_a_thousand() {
        let mut r = start(StartArgs::new(640, 480, "", 20260812));
        r.step();
        let f = r.frame();
        assert!(f.batches.len() <= 8, "{} calls", f.batches.len());
        assert!(!f.vertices.is_empty());
    }

    #[test]
    fn it_gets_through_every_state_it_has() {
        // Every animation has to run to its end and hand over to the next: a
        // state that never leaves would freeze the box in a half-open shape.
        let mut this = bare();
        this.states = states();
        let mut seen = std::collections::BTreeSet::new();
        for i in 0..200_000 {
            // The lid only zooms in when the box happens to be facing you.
            this.facing_p = i % 3 == 0;
            this.anim_pause = 0;
            this.animate();
            seen.insert(format!("{:?}", this.kind));
        }
        // Thirty-one states in all, and none of them may be skipped over.
        assert_eq!(seen.len(), 31, "{seen:?}");
    }

    #[test]
    fn it_keeps_drawing_however_far_it_gets() {
        let mut r = start(StartArgs::new(640, 480, "", 20260812));
        for i in 0..3000 {
            if i % 100 == 0 {
                // Fast-forward is what the space bar does: skip the resting
                // box and run the animations at twenty times the speed.
                r.event(XEvent::KeyPress { key: ' ' });
            }
            r.step();
            let f = r.frame();
            assert!(!f.batches.is_empty(), "nothing drawn on frame {i}");
            assert!(
                f.batches.len() < 200,
                "frame {i} took {} calls",
                f.batches.len()
            );
        }
    }

    #[test]
    fn the_sphere_starts_as_the_box_and_ends_as_a_ball() {
        // At ratio zero every corner is still on the cube, and at ratio one
        // every corner is the same distance from the middle.
        let mut g = Gl::for_test(640, 480);
        for (ratio, want_round) in [(0.0f32, false), (1.0, true)] {
            g.glx.start_frame(640, 480);
            let this = bare();
            let polys = this.lament_sphere(&mut g, ratio);
            assert_eq!(polys, 6 * 16 * 16);
            let f = g.glx.frame();
            let mut lo = f32::MAX;
            let mut hi = 0.0f32;
            for b in &f.batches {
                for v in &f.vertices[b.first..b.first + b.count] {
                    let p = b.mvp.transform(v.pos);
                    let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
                    lo = lo.min(r);
                    hi = hi.max(r);
                }
            }
            if want_round {
                assert!(hi - lo < 1e-3, "not a ball: {lo} to {hi}");
            } else {
                // The corners of a cube are further out than the middles of
                // its faces by root three.
                assert!(hi / lo > 1.7, "not a cube: {lo} to {hi}");
            }
        }
    }

    /// The list of animations `init` builds, with the same weights.
    fn states() -> Vec<Type> {
        let mut out = Vec::new();
        for (n, which) in [
            (4, Type::TetraUne),
            (4, Type::TetraUsw),
            (4, Type::TetraDwn),
            (4, Type::TetraDse),
            (8, Type::StarOut),
            (8, Type::TaserOut),
            (8, Type::PillarOut),
            (4, Type::LidOpen),
            (2, Type::SphereOut),
            (1, Type::LeviathanSpin),
            (35, Type::Box),
        ] {
            for _ in 0..n {
                out.push(which);
            }
        }
        out
    }

    /// A saver with no models and no textures, for exercising the parts that
    /// do not need them.
    fn bare() -> Lament {
        Lament {
            rot: Rotator::new(0.0, 0.0, 0.0, 1.0, 0.0, false),
            rotx: 0.0,
            roty: 0.0,
            rotz: 0.0,
            trackball: Trackball::new(),
            ffwdp: false,
            models: Vec::new(),
            texids: [0; 8],
            do_texture: false,
            wireframe: false,
            kind: Type::Box,
            anim_pause: 0,
            anim_r: 0.0,
            anim_y: 0.0,
            anim_z: 0.0,
            facing_p: false,
            state: 0,
            states: Vec::new(),
        }
    }

    #[test]
    fn the_eight_textures_come_out_in_the_order_the_box_wants_them() {
        // The six walls are white line-work on near-black and so have a lot of
        // contrast; the inside of the box is low-contrast brown woodgrain; the
        // Leviathan is a pale symbol on neutral grey. Getting the order wrong
        // papers two of the walls with the wrong thing and lines the inside
        // with gold leaf, which is what happens if the tiles are taken from
        // the file the way round they are stored.
        let (w, h, px) = crate::runtime::png::decode_rgba(crate::images::LAMENT512).unwrap();
        assert_eq!((w, h), (512, 512 * 8));

        let look = |i: usize| {
            let t = tile(&px, w as usize, i);
            let n = (t.len() / 4) as f64;
            let mut sum = [0.0f64; 3];
            for p in t.chunks_exact(4) {
                for k in 0..3 {
                    sum[k] += f64::from(p[k]);
                }
            }
            let mean: Vec<f64> = sum.iter().map(|s| s / n).collect();
            let grey = (mean[0] + mean[1] + mean[2]) / 3.0;
            let sd = (t
                .chunks_exact(4)
                .map(|p| (f64::from(p[0]) - mean[0]).powi(2))
                .sum::<f64>()
                / n)
                .sqrt();
            (mean[0] - mean[2], grey, sd)
        };

        for i in 0..6 {
            let (_, _, sd) = look(i);
            assert!(sd > 80.0, "wall {i} is not line-work: spread {sd}");
        }
        let (warm, grey, sd) = look(6);
        assert!(
            sd < 20.0 && warm > 30.0,
            "the inside is not wood: {warm} {grey} {sd}"
        );
        let (warm, _, sd) = look(7);
        assert!(
            sd > 20.0 && sd < 80.0 && warm.abs() < 2.0,
            "the Leviathan is not a symbol on grey: {warm} {sd}"
        );
    }

    #[test]
    fn the_wireframe_has_no_textures_and_no_fill() {
        let mut r = start(StartArgs::new(640, 480, "wireframe=true", 20260812));
        r.step();
        let f = r.frame();
        for b in &f.batches {
            assert_eq!(b.primitive, crate::runtime::gl::Primitive::Lines);
            assert!(b.texture.is_none());
        }
    }
}
