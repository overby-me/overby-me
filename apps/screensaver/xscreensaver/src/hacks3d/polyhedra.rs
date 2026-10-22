//! Port of `hacks/glx/polyhedra-gl.c`.
//!
//! ```text
//! polyhedra, Copyright (c) 2004-2014 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Renders 160 different 3D solids, and displays some information about each.
//!  A new solid is chosen every few seconds.
//!
//! This file contains the OpenGL side; computation of the polyhedra themselves
//! is in "polyhedra.c".
//! ```
//!
//! The seventy-five uniform polyhedra and their duals, plus the five prisms
//! and antiprisms that go with them, turning one at a time with their names
//! and their counts written beside them. Zvi Har'El's construction is in
//! [`crate::runtime::kaleido`]; this is the part that draws it.
//!
//! Faces get flat shading with one normal apiece, worked out by summing the
//! normals at each corner: a star face folded round its centre has no single
//! plane, and the sum is the closest thing to one. Nothing is culled and the
//! lighting is two-sided, because several of these are hemipolyhedra whose
//! faces pass through the middle and are seen from both sides at once.
//!
//! Upstream feeds each face to the GLU tessellator, since a face that has been
//! cut down to a three-pointed star is not convex. [`crate::runtime::tess`] is
//! that tessellator. The colour goes on the vertices through
//! `GL_COLOR_MATERIAL` rather than on the material directly, which the
//! recorder would take as a reason to start a new batch, so the whole solid
//! comes out as one draw call however many faces it has.
//!
//! And, as upstream has it, the hundred and sixty-first solid is a teapot.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_random_colormap};
use crate::runtime::gl::Shape;
use crate::runtime::kaleido;
use crate::runtime::opts::SelectItem;
use crate::runtime::shapes::calc_normal;
use crate::runtime::teapot::unit_teapot;
use crate::runtime::tess::triangulate;
use crate::runtime::texfont::TexFont;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random,
    screenhack_event_helper,
};

/// The teapot goes on the end of the list of real ones.
const NPOLYHEDRA: usize = kaleido::SHAPES + 1;

/// What the label says about whichever one is being drawn.
#[derive(Default)]
struct Info {
    name: String,
    class: String,
    wythoff: String,
    config: String,
    group: String,
    faces: usize,
    edges: usize,
    vertices: usize,
    density: i32,
    chi: i32,
}

struct Polyhedra {
    rot: Rotator,
    trackball: Trackball,
    fonts: Vec<TexFont>,

    which: i32,
    change_to: i32,
    object_list: u32,
    info: Info,

    /// 0 is normal, 1 is shrinking away, 2 is growing back.
    mode: i32,
    mode_tick: i32,

    colors: Vec<XColor>,

    last_change_time: f64,
    change_tick: i32,

    do_which: i32,
    do_titles: bool,
    wireframe: bool,
    duration: i32,
    speed: f32,
}

/// `kludge_normal`: sum the normals at each corner of a face and use that for
/// the whole of it. A face that is not flat has no honest normal, and this is
/// nearer than picking one corner would be.
fn kludge_normal(points: &[[f32; 3]]) -> [f32; 3] {
    let n = points.len();
    let mut normal = [0.0f32; 3];
    let mut last = [0.0f32; 3];
    for i in 0..n {
        last = calc_normal(points[i], points[(i + 1) % n], points[(i + 2) % n]);
        for k in 0..3 {
            normal[k] += last[k];
        }
    }
    if normal == [0.0; 3] { last } else { normal }
}

impl Polyhedra {
    /// `new_polyhedron`: pick the next one, work it out, and compile it.
    fn new_polyhedron(&mut self, g: &mut Gl) {
        self.colors = make_random_colormap(128, true);

        if self.do_which >= NPOLYHEDRA as i32 {
            self.do_which = -1;
        }
        self.which = if self.change_to >= 0 {
            self.change_to
        } else if self.do_which >= 0 {
            self.do_which
        } else {
            (random() as usize % NPOLYHEDRA) as i32
        };
        self.change_to = -1;

        let wire = self.wireframe;
        g.glx.new_list(self.object_list);
        g.glx.push_matrix();
        if self.which as usize == NPOLYHEDRA - 1 {
            self.info = Info {
                name: "Teapot".into(),
                class: "Utah Teapotahedron".into(),
                wythoff: "X00398|1984".into(),
                config: "Melitta".into(),
                group: "Teapotahedral (Newell[1975])".into(),
                ..Info::default()
            };
            let c = self.colors[0];
            if wire {
                g.glx.color3f(0.0, 1.0, 0.0);
            } else {
                g.glx.color3f(
                    c.red as f32 / 65536.0,
                    c.green as f32 / 65536.0,
                    c.blue as f32 / 65536.0,
                );
            }
            g.glx.scale(0.8, 0.8, 0.8);
            let faces = unit_teapot(&mut g.glx, 6, wire);
            self.info.faces = faces;
            self.info.edges = faces * 3 / 2;
            self.info.vertices = faces * 3;
        } else {
            let s = kaleido::shape(self.which as usize).expect("the table is only eighty long");
            self.info = Info {
                name: s.name.clone(),
                class: s.class.clone(),
                wythoff: s.wythoff.clone(),
                config: s.config.clone(),
                group: s.group.clone(),
                faces: s.logical_faces,
                edges: s.nedges,
                vertices: s.logical_vertices,
                density: s.density,
                chi: s.chi,
            };

            let point = |i: usize| {
                let p = s.points[i];
                [p.x as f32, p.y as f32, p.z as f32]
            };
            g.glx.front_face_cw(false);
            // One block for the lot: the colour rides on the vertices, so the
            // faces do not each need their own.
            g.glx
                .begin(if wire { Shape::Lines } else { Shape::Triangles });
            for f in &s.faces {
                if wire {
                    g.glx.color3f(0.0, 1.0, 0.0);
                } else {
                    let c = self.colors[f.color % self.colors.len()];
                    g.glx.color3f(
                        c.red as f32 / 65536.0,
                        c.green as f32 / 65536.0,
                        c.blue as f32 / 65536.0,
                    );
                }
                let corners: Vec<[f32; 3]> = f.points.iter().map(|&i| point(i)).collect();

                if wire {
                    for i in 0..corners.len() {
                        let (a, b) = (corners[i], corners[(i + 1) % corners.len()]);
                        g.glx.vertex3f(a[0], a[1], a[2]);
                        g.glx.vertex3f(b[0], b[1], b[2]);
                    }
                    continue;
                }

                let n = kludge_normal(&corners);
                g.glx.normal3f(n[0], n[1], n[2]);
                for t in triangulate(&corners) {
                    for c in t {
                        let p = corners[c];
                        g.glx.vertex3f(p[0], p[1], p[2]);
                    }
                }
            }
            g.glx.end();
        }
        g.glx.pop_matrix();
        g.glx.end_list();
    }

    /// `draw_label`: everything known about the current solid, up the left.
    fn draw_label(&self, g: &mut Gl) {
        if !self.do_titles || self.fonts.is_empty() {
            return;
        }
        let p = &self.info;
        let mut name2 = p.name.clone();
        if !p.class.is_empty() {
            name2.push_str("  (");
            name2.push_str(&p.class);
            name2.push(')');
        }
        let label = format!(
            "Polyhedron {}:   \t{}\n\n\
             Wythoff Symbol:\t{}\n\
             Vertex Configuration:\t{}\n\
             Symmetry Group:\t{}\n\
             \n\
             Faces:\t  {}\n\
             Edges:\t  {}\n\
             Vertices:\t  {}\n\
             Density:\t  {}\n\
             Euler:\t{}{}\n",
            self.which,
            name2,
            p.wythoff,
            p.config,
            p.group,
            p.faces,
            p.edges,
            p.vertices,
            p.density,
            if p.chi < 0 { "" } else { "  " },
            p.chi,
        );

        let (w, h) = (g.width(), g.height());
        let f = if w >= 500 && h >= 375 {
            0
        } else if w >= 350 && h >= 260 {
            1
        } else {
            2
        };
        let font = &self.fonts[f.min(self.fonts.len() - 1)];
        font.print_label(&mut g.glx, &label, w, h, 1, [0.8, 0.8, 0.8, 1.0]);
    }
}

/// Resolve the object knob: `random`, an index, or a name with underscores
/// where upstream's list has spaces.
fn resolve_which(want: &str) -> i32 {
    if want.is_empty() || want.eq_ignore_ascii_case("random") {
        return -1;
    }
    if let Ok(n) = want.trim().parse::<i32>() {
        return if (0..NPOLYHEDRA as i32).contains(&n) {
            n
        } else {
            -1
        };
    }
    let want: String = want
        .chars()
        .map(|c| if c == '-' || c == '_' { ' ' } else { c })
        .collect();
    if want.eq_ignore_ascii_case("utah teapotahedron") {
        return NPOLYHEDRA as i32 - 1;
    }
    for n in 0..kaleido::SHAPES {
        let (name, class) = kaleido::shape_names(n);
        if want.eq_ignore_ascii_case(name) || want.eq_ignore_ascii_case(class) {
            return n as i32;
        }
    }
    -1
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let do_spin = g.res.bool("spin");
    let do_wander = g.res.bool("wander");
    let spin_speed = 2.0;
    let wander_speed = 0.05;
    let spin_accel = 0.2;

    let mut this = Polyhedra {
        rot: Rotator::new(
            if do_spin { spin_speed } else { 0.0 },
            if do_spin { spin_speed } else { 0.0 },
            if do_spin { spin_speed } else { 0.0 },
            spin_accel,
            if do_wander { wander_speed } else { 0.0 },
            true,
        ),
        trackball: Trackball::new(),
        fonts: ["sans-serif 14", "sans-serif 10", "sans-serif 8"]
            .iter()
            .map(|f| TexFont::load(&mut g.glx, f))
            .collect(),
        which: -1,
        change_to: -1,
        object_list: 0,
        info: Info::default(),
        mode: 0,
        mode_tick: 0,
        colors: Vec::new(),
        last_change_time: 0.0,
        change_tick: 0,
        do_which: resolve_which(g.res.string("which")),
        do_titles: g.res.bool("titles"),
        wireframe: g.res.bool("wireframe"),
        duration: g.res.int("duration").max(1),
        speed: (g.res.float("speed") as f32).max(0.01),
    };

    this.object_list = g.glx.gen_lists(1);
    this.new_polyhedron(g);
    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Polyhedra {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let (mut height, mut y) = (height, 0);
        let mut h = height as f32 / width as f32;
        if width > height * 5 {
            // Tiny window: show the middle.
            height = width * 9 / 16;
            y = -height / 2;
            h = height as f32 / width as f32;
        }
        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, 1.0 / h, 1.0, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        let n = NPOLYHEDRA as i32;
        if let XEvent::KeyPress { key } = event {
            self.change_to = -1;
            match key {
                ' ' | '\t' | '\r' | '\n' => {
                    self.change_to = (random() as usize % NPOLYHEDRA) as i32
                }
                '>' | '.' | '+' | '=' => self.change_to = (self.which + 1) % n,
                '<' | ',' | '-' | '_' => self.change_to = (self.which + n - 1) % n,
                _ => {}
            }
            if self.change_to != -1 {
                return true;
            }
        }
        if screenhack_event_helper(event) {
            self.change_to = (random() as usize % NPOLYHEDRA) as i32;
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        let down = self.trackball.button_down();
        let ticks = (20.0 / self.speed) as i32;

        if self.mode == 0 && self.do_which >= 0 && self.change_to < 0 {
            // Pinned to one solid: never change.
        } else if self.mode == 0 {
            if self.change_to >= 0 {
                self.change_tick = 999;
                self.last_change_time = 1.0;
            }
            self.change_tick += 1;
            if self.change_tick > 10 {
                // Upstream reads the wall clock, which is only being used as a
                // second counter. Seconds since the saver started is the same
                // counter and does not turn over at midnight.
                let now = g.time;
                if self.last_change_time == 0.0 {
                    self.last_change_time = now;
                }
                self.change_tick = 0;
                if !down && now - self.last_change_time >= self.duration as f64 {
                    self.mode = 1; // Go out.
                    self.mode_tick = ticks;
                    self.last_change_time = now;
                }
            }
        } else if self.mode == 1 {
            self.mode_tick -= 1;
            if self.mode_tick <= 0 {
                self.new_polyhedron(g);
                self.mode_tick = ticks;
                self.mode = 2; // Go in.
            }
        } else {
            self.mode_tick -= 1;
            if self.mode_tick <= 0 {
                self.mode = 0;
            }
        }

        g.glx.clear();
        g.glx.depth_test(true);
        // No culling: several of these are seen from inside as well as out.
        g.glx.cull_face(false);
        if !self.wireframe {
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
            g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(0, [0.0, 1.0, 1.0, 1.0]);
            g.glx.color_material(true);
        }

        g.glx.push_matrix();

        let s = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);
        g.glx.scale(1.1, 1.1, 1.1);

        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 8.0,
            (y as f32 - 0.5) * 8.0,
            (z as f32 - 0.5) * 15.0,
        );
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        let (x, y, z) = self.rot.rotation(!down);
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);

        g.glx.scale(2.0, 2.0, 2.0);
        g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_shininess(128.0);

        if self.mode != 0 {
            let t = 20.0 / self.speed;
            let s = if self.mode == 1 {
                self.mode_tick as f32 / t
            } else {
                (t - self.mode_tick as f32 + 1.0) / t
            };
            g.glx.scale(s, s, s);
        }

        g.glx.scale(2.0, 2.0, 2.0);
        g.glx.call_list(self.object_list);
        if self.mode == 0 && !down {
            // The label cannot go inside the display list.
            self.draw_label(g);
        }

        g.glx.pop_matrix();
        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*titleFont:    sans-serif 14",
    "*titleFont2:   sans-serif 10",
    "*titleFont3:   sans-serif 8",
    "*suppressRotationAnimation: True",
    "*spin:         True",
    "*wander:       True",
    "*speed:        1.0",
    "*titles:       True",
    "*duration:     12",
    "*which:        random",
];

const OBJECTS: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Display random polyhedron",
    },
    SelectItem {
        value: "pentagonal_prism",
        label: "Pentagonal prism",
    },
    SelectItem {
        value: "pentagonal_dipyramid",
        label: "Pentagonal dipyramid",
    },
    SelectItem {
        value: "pentagonal_antiprism",
        label: "Pentagonal antiprism",
    },
    SelectItem {
        value: "pentagonal_deltohedron",
        label: "Pentagonal deltohedron",
    },
    SelectItem {
        value: "pentagrammic_prism",
        label: "Pentagrammic prism",
    },
    SelectItem {
        value: "pentagrammic_dipyramid",
        label: "Pentagrammic dipyramid",
    },
    SelectItem {
        value: "pentagrammic_antiprism",
        label: "Pentagrammic antiprism",
    },
    SelectItem {
        value: "pentagrammic_deltohedron",
        label: "Pentagrammic deltohedron",
    },
    SelectItem {
        value: "pentagrammic_crossed_antiprism",
        label: "Pentagrammic crossed antiprism",
    },
    SelectItem {
        value: "pentagrammic_concave_deltohedron",
        label: "Pentagrammic concave deltohedron",
    },
    SelectItem {
        value: "tetrahedron",
        label: "Tetrahedron",
    },
    SelectItem {
        value: "truncated_tetrahedron",
        label: "Truncated tetrahedron",
    },
    SelectItem {
        value: "triakistetrahedron",
        label: "Triakistetrahedron",
    },
    SelectItem {
        value: "octahemioctahedron",
        label: "Octahemioctahedron",
    },
    SelectItem {
        value: "octahemioctacron",
        label: "Octahemioctacron",
    },
    SelectItem {
        value: "tetrahemihexahedron",
        label: "Tetrahemihexahedron",
    },
    SelectItem {
        value: "tetrahemihexacron",
        label: "Tetrahemihexacron",
    },
    SelectItem {
        value: "octahedron",
        label: "Octahedron",
    },
    SelectItem {
        value: "cube",
        label: "Cube",
    },
    SelectItem {
        value: "cuboctahedron",
        label: "Cuboctahedron",
    },
    SelectItem {
        value: "rhombic_dodecahedron",
        label: "Rhombic dodecahedron",
    },
    SelectItem {
        value: "truncated_octahedron",
        label: "Truncated octahedron",
    },
    SelectItem {
        value: "tetrakishexahedron",
        label: "Tetrakishexahedron",
    },
    SelectItem {
        value: "truncated_cube",
        label: "Truncated cube",
    },
    SelectItem {
        value: "triakisoctahedron",
        label: "Triakisoctahedron",
    },
    SelectItem {
        value: "rhombicuboctahedron",
        label: "Rhombicuboctahedron",
    },
    SelectItem {
        value: "deltoidal_icositetrahedron",
        label: "Deltoidal icositetrahedron",
    },
    SelectItem {
        value: "truncated_cuboctahedron",
        label: "Truncated cuboctahedron",
    },
    SelectItem {
        value: "disdyakisdodecahedron",
        label: "Disdyakisdodecahedron",
    },
    SelectItem {
        value: "snub_cube",
        label: "Snub cube",
    },
    SelectItem {
        value: "pentagonal_icositetrahedron",
        label: "Pentagonal icositetrahedron",
    },
    SelectItem {
        value: "small_cubicuboctahedron",
        label: "Small cubicuboctahedron",
    },
    SelectItem {
        value: "small_hexacronic_icositetrahedron",
        label: "Small hexacronic icositetrahedron",
    },
    SelectItem {
        value: "great_cubicuboctahedron",
        label: "Great cubicuboctahedron",
    },
    SelectItem {
        value: "great_hexacronic_icositetrahedron",
        label: "Great hexacronic icositetrahedron",
    },
    SelectItem {
        value: "cubohemioctahedron",
        label: "Cubohemioctahedron",
    },
    SelectItem {
        value: "hexahemioctacron",
        label: "Hexahemioctacron",
    },
    SelectItem {
        value: "cubitruncated_cuboctahedron",
        label: "Cubitruncated cuboctahedron",
    },
    SelectItem {
        value: "tetradyakishexahedron",
        label: "Tetradyakishexahedron",
    },
    SelectItem {
        value: "great_rhombicuboctahedron",
        label: "Great rhombicuboctahedron",
    },
    SelectItem {
        value: "great_deltoidal_icositetrahedron",
        label: "Great deltoidal icositetrahedron",
    },
    SelectItem {
        value: "small_rhombihexahedron",
        label: "Small rhombihexahedron",
    },
    SelectItem {
        value: "small_rhombihexacron",
        label: "Small rhombihexacron",
    },
    SelectItem {
        value: "stellated_truncated_hexahedron",
        label: "Stellated truncated hexahedron",
    },
    SelectItem {
        value: "great_triakisoctahedron",
        label: "Great triakisoctahedron",
    },
    SelectItem {
        value: "great_truncated_cuboctahedron",
        label: "Great truncated cuboctahedron",
    },
    SelectItem {
        value: "great_disdyakisdodecahedron",
        label: "Great disdyakisdodecahedron",
    },
    SelectItem {
        value: "great_rhombihexahedron",
        label: "Great rhombihexahedron",
    },
    SelectItem {
        value: "great_rhombihexacron",
        label: "Great rhombihexacron",
    },
    SelectItem {
        value: "icosahedron",
        label: "Icosahedron",
    },
    SelectItem {
        value: "dodecahedron",
        label: "Dodecahedron",
    },
    SelectItem {
        value: "icosidodecahedron",
        label: "Icosidodecahedron",
    },
    SelectItem {
        value: "rhombic_triacontahedron",
        label: "Rhombic triacontahedron",
    },
    SelectItem {
        value: "truncated_icosahedron",
        label: "Truncated icosahedron",
    },
    SelectItem {
        value: "pentakisdodecahedron",
        label: "Pentakisdodecahedron",
    },
    SelectItem {
        value: "truncated_dodecahedron",
        label: "Truncated dodecahedron",
    },
    SelectItem {
        value: "triakisicosahedron",
        label: "Triakisicosahedron",
    },
    SelectItem {
        value: "rhombicosidodecahedron",
        label: "Rhombicosidodecahedron",
    },
    SelectItem {
        value: "deltoidal_hexecontahedron",
        label: "Deltoidal hexecontahedron",
    },
    SelectItem {
        value: "truncated_icosidodecahedron",
        label: "Truncated icosidodecahedron",
    },
    SelectItem {
        value: "disdyakistriacontahedron",
        label: "Disdyakistriacontahedron",
    },
    SelectItem {
        value: "snub_dodecahedron",
        label: "Snub dodecahedron",
    },
    SelectItem {
        value: "pentagonal_hexecontahedron",
        label: "Pentagonal hexecontahedron",
    },
    SelectItem {
        value: "small_ditrigonal_icosidodecahedron",
        label: "Small ditrigonal icosidodecahedron",
    },
    SelectItem {
        value: "small_triambic_icosahedron",
        label: "Small triambic icosahedron",
    },
    SelectItem {
        value: "small_icosicosidodecahedron",
        label: "Small icosicosidodecahedron",
    },
    SelectItem {
        value: "small_icosacronic_hexecontahedron",
        label: "Small icosacronic hexecontahedron",
    },
    SelectItem {
        value: "small_snub_icosicosidodecahedron",
        label: "Small snub icosicosidodecahedron",
    },
    SelectItem {
        value: "small_hexagonal_hexecontahedron",
        label: "Small hexagonal hexecontahedron",
    },
    SelectItem {
        value: "small_dodecicosidodecahedron",
        label: "Small dodecicosidodecahedron",
    },
    SelectItem {
        value: "small_dodecacronic_hexecontahedron",
        label: "Small dodecacronic hexecontahedron",
    },
    SelectItem {
        value: "small_stellated_dodecahedron",
        label: "Small stellated dodecahedron",
    },
    SelectItem {
        value: "great_dodecahedron",
        label: "Great dodecahedron",
    },
    SelectItem {
        value: "great_dodecadodecahedron",
        label: "Great dodecadodecahedron",
    },
    SelectItem {
        value: "medial_rhombic_triacontahedron",
        label: "Medial rhombic triacontahedron",
    },
    SelectItem {
        value: "truncated_great_dodecahedron",
        label: "Truncated great dodecahedron",
    },
    SelectItem {
        value: "small_stellapentakisdodecahedron",
        label: "Small stellapentakisdodecahedron",
    },
    SelectItem {
        value: "rhombidodecadodecahedron",
        label: "Rhombidodecadodecahedron",
    },
    SelectItem {
        value: "medial_deltoidal_hexecontahedron",
        label: "Medial deltoidal hexecontahedron",
    },
    SelectItem {
        value: "small_rhombidodecahedron",
        label: "Small rhombidodecahedron",
    },
    SelectItem {
        value: "small_rhombidodecacron",
        label: "Small rhombidodecacron",
    },
    SelectItem {
        value: "snub_dodecadodecahedron",
        label: "Snub dodecadodecahedron",
    },
    SelectItem {
        value: "medial_pentagonal_hexecontahedron",
        label: "Medial pentagonal hexecontahedron",
    },
    SelectItem {
        value: "ditrigonal_dodecadodecahedron",
        label: "Ditrigonal dodecadodecahedron",
    },
    SelectItem {
        value: "medial_triambic_icosahedron",
        label: "Medial triambic icosahedron",
    },
    SelectItem {
        value: "great_ditrigonal_dodecicosidodecahedron",
        label: "Great ditrigonal dodecicosidodecahedron",
    },
    SelectItem {
        value: "great_ditrigonal_dodecacronic_hexecontahedron",
        label: "Great ditrigonal dodecacronic hexecontahedron",
    },
    SelectItem {
        value: "small_ditrigonal_dodecicosidodecahedron",
        label: "Small ditrigonal dodecicosidodecahedron",
    },
    SelectItem {
        value: "small_ditrigonal_dodecacronic_hexecontahedron",
        label: "Small ditrigonal dodecacronic hexecontahedron",
    },
    SelectItem {
        value: "icosidodecadodecahedron",
        label: "Icosidodecadodecahedron",
    },
    SelectItem {
        value: "medial_icosacronic_hexecontahedron",
        label: "Medial icosacronic hexecontahedron",
    },
    SelectItem {
        value: "icositruncated_dodecadodecahedron",
        label: "Icositruncated dodecadodecahedron",
    },
    SelectItem {
        value: "tridyakisicosahedron",
        label: "Tridyakisicosahedron",
    },
    SelectItem {
        value: "snub_icosidodecadodecahedron",
        label: "Snub icosidodecadodecahedron",
    },
    SelectItem {
        value: "medial_hexagonal_hexecontahedron",
        label: "Medial hexagonal hexecontahedron",
    },
    SelectItem {
        value: "great_ditrigonal_icosidodecahedron",
        label: "Great ditrigonal icosidodecahedron",
    },
    SelectItem {
        value: "great_triambic_icosahedron",
        label: "Great triambic icosahedron",
    },
    SelectItem {
        value: "great_icosicosidodecahedron",
        label: "Great icosicosidodecahedron",
    },
    SelectItem {
        value: "great_icosacronic_hexecontahedron",
        label: "Great icosacronic hexecontahedron",
    },
    SelectItem {
        value: "small_icosihemidodecahedron",
        label: "Small icosihemidodecahedron",
    },
    SelectItem {
        value: "small_icosihemidodecacron",
        label: "Small icosihemidodecacron",
    },
    SelectItem {
        value: "small_dodecicosahedron",
        label: "Small dodecicosahedron",
    },
    SelectItem {
        value: "small_dodecicosacron",
        label: "Small dodecicosacron",
    },
    SelectItem {
        value: "small_dodecahemidodecahedron",
        label: "Small dodecahemidodecahedron",
    },
    SelectItem {
        value: "small_dodecahemidodecacron",
        label: "Small dodecahemidodecacron",
    },
    SelectItem {
        value: "great_stellated_dodecahedron",
        label: "Great stellated dodecahedron",
    },
    SelectItem {
        value: "great_icosahedron",
        label: "Great icosahedron",
    },
    SelectItem {
        value: "great_icosidodecahedron",
        label: "Great icosidodecahedron",
    },
    SelectItem {
        value: "great_rhombic_triacontahedron",
        label: "Great rhombic triacontahedron",
    },
    SelectItem {
        value: "great_truncated_icosahedron",
        label: "Great truncated icosahedron",
    },
    SelectItem {
        value: "great_stellapentakisdodecahedron",
        label: "Great stellapentakisdodecahedron",
    },
    SelectItem {
        value: "rhombicosahedron",
        label: "Rhombicosahedron",
    },
    SelectItem {
        value: "rhombicosacron",
        label: "Rhombicosacron",
    },
    SelectItem {
        value: "great_snub_icosidodecahedron",
        label: "Great snub icosidodecahedron",
    },
    SelectItem {
        value: "great_pentagonal_hexecontahedron",
        label: "Great pentagonal hexecontahedron",
    },
    SelectItem {
        value: "small_stellated_truncated_dodecahedron",
        label: "Small stellated truncated dodecahedron",
    },
    SelectItem {
        value: "great_pentakisdodecahedron",
        label: "Great pentakisdodecahedron",
    },
    SelectItem {
        value: "truncated_dodecadodecahedron",
        label: "Truncated dodecadodecahedron",
    },
    SelectItem {
        value: "medial_disdyakistriacontahedron",
        label: "Medial disdyakistriacontahedron",
    },
    SelectItem {
        value: "inverted_snub_dodecadodecahedron",
        label: "Inverted snub dodecadodecahedron",
    },
    SelectItem {
        value: "medial_inverted_pentagonal_hexecontahedron",
        label: "Medial inverted pentagonal hexecontahedron",
    },
    SelectItem {
        value: "great_dodecicosidodecahedron",
        label: "Great dodecicosidodecahedron",
    },
    SelectItem {
        value: "great_dodecacronic_hexecontahedron",
        label: "Great dodecacronic hexecontahedron",
    },
    SelectItem {
        value: "small_dodecahemicosahedron",
        label: "Small dodecahemicosahedron",
    },
    SelectItem {
        value: "small_dodecahemicosacron",
        label: "Small dodecahemicosacron",
    },
    SelectItem {
        value: "great_dodecicosahedron",
        label: "Great dodecicosahedron",
    },
    SelectItem {
        value: "great_dodecicosacron",
        label: "Great dodecicosacron",
    },
    SelectItem {
        value: "great_snub_dodecicosidodecahedron",
        label: "Great snub dodecicosidodecahedron",
    },
    SelectItem {
        value: "great_hexagonal_hexecontahedron",
        label: "Great hexagonal hexecontahedron",
    },
    SelectItem {
        value: "great_dodecahemicosahedron",
        label: "Great dodecahemicosahedron",
    },
    SelectItem {
        value: "great_dodecahemicosacron",
        label: "Great dodecahemicosacron",
    },
    SelectItem {
        value: "great_stellated_truncated_dodecahedron",
        label: "Great stellated truncated dodecahedron",
    },
    SelectItem {
        value: "great_triakisicosahedron",
        label: "Great triakisicosahedron",
    },
    SelectItem {
        value: "great_rhombicosidodecahedron",
        label: "Great rhombicosidodecahedron",
    },
    SelectItem {
        value: "great_deltoidal_hexecontahedron",
        label: "Great deltoidal hexecontahedron",
    },
    SelectItem {
        value: "great_truncated_icosidodecahedron",
        label: "Great truncated icosidodecahedron",
    },
    SelectItem {
        value: "great_disdyakistriacontahedron",
        label: "Great disdyakistriacontahedron",
    },
    SelectItem {
        value: "great_inverted_snub_icosidodecahedron",
        label: "Great inverted snub icosidodecahedron",
    },
    SelectItem {
        value: "great_inverted_pentagonal_hexecontahedron",
        label: "Great inverted pentagonal hexecontahedron",
    },
    SelectItem {
        value: "great_dodecahemidodecahedron",
        label: "Great dodecahemidodecahedron",
    },
    SelectItem {
        value: "great_dodecahemidodecacron",
        label: "Great dodecahemidodecacron",
    },
    SelectItem {
        value: "great_icosihemidodecahedron",
        label: "Great icosihemidodecahedron",
    },
    SelectItem {
        value: "great_icosihemidodecacron",
        label: "Great icosihemidodecacron",
    },
    SelectItem {
        value: "small_retrosnub_icosicosidodecahedron",
        label: "Small retrosnub icosicosidodecahedron",
    },
    SelectItem {
        value: "small_hexagrammic_hexecontahedron",
        label: "Small hexagrammic hexecontahedron",
    },
    SelectItem {
        value: "great_rhombidodecahedron",
        label: "Great rhombidodecahedron",
    },
    SelectItem {
        value: "great_rhombidodecacron",
        label: "Great rhombidodecacron",
    },
    SelectItem {
        value: "great_retrosnub_icosidodecahedron",
        label: "Great retrosnub icosidodecahedron",
    },
    SelectItem {
        value: "great_pentagrammic_hexecontahedron",
        label: "Great pentagrammic hexecontahedron",
    },
    SelectItem {
        value: "great_dirhombicosidodecahedron",
        label: "Great dirhombicosidodecahedron",
    },
    SelectItem {
        value: "great_dirhombicosidodecacron",
        label: "Great dirhombicosidodecacron",
    },
    SelectItem {
        value: "utah_teapotahedron",
        label: "Utah teapotahedron",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.01, 5.0, 0.01, 2, "1.0"),
    Opt::slider("duration", "Duration", 1.0, 30.0, 1.0, 0, "12"),
    Opt::select("which", "Object", OBJECTS, "random"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("titles", "Show description", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "polyhedra",
    label: "Polyhedra",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Dr. Zvi Har'El and Jamie Zawinski",
        year: "2004",
        video: Some("https://www.youtube.com/watch?v=gYb-3EErLJE"),
        blurb: "The 75 uniform polyhedra and their duals, plus 5 prisms \
                and antiprisms, and some information about each.",
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
    fn the_panel_names_the_same_solids_the_table_does() {
        // Every name on the panel has to reach a solid, or picking it from the
        // menu would silently fall back to a random one.
        for o in &OBJECTS[1..] {
            let n = resolve_which(o.value);
            assert!(n >= 0, "{} does not resolve", o.value);
            let want = o.label.to_ascii_lowercase();
            let got = if n as usize == NPOLYHEDRA - 1 {
                "utah teapotahedron".to_string()
            } else {
                kaleido::shape_names(n as usize).0.to_ascii_lowercase()
            };
            // Most of them match on the name; a few of the prisms are named on
            // the panel by their class instead.
            if got != want {
                let class = kaleido::shape_names(n as usize).1.to_ascii_lowercase();
                assert_eq!(class, want, "{} resolved to {got}", o.value);
            }
        }
        // And an index, and a name nothing has.
        assert_eq!(resolve_which("19"), 19);
        assert_eq!(resolve_which("marzipan"), -1);
        assert_eq!(resolve_which("random"), -1);
        assert_eq!(resolve_which("1000"), -1);
    }

    #[test]
    fn every_solid_draws_as_one_call() {
        // The colour goes on the vertices, so however many faces of however
        // many kinds a solid has, it is one draw call.
        for n in [0, 19, 79, 120, 158, 159] {
            let mut r = start(StartArgs::new(
                640,
                480,
                &format!("which={n}&titles=false"),
                20260812,
            ));
            r.step();
            let f = r.frame();
            assert_eq!(
                f.batches.len(),
                1,
                "solid {n} took {} calls",
                f.batches.len()
            );
            assert!(f.batches[0].count > 6, "solid {n} is empty");
        }
    }

    #[test]
    fn a_hemi_solid_reaches_through_the_middle() {
        // The tetrahemihexahedron, number 16, has four faces that pass through
        // the centre, which is why nothing may be culled: its own far side is
        // visible through it.
        let mut r = start(StartArgs::new(640, 480, "which=16&titles=false", 20260812));
        r.step();
        let f = r.frame();
        assert!(!f.batches[0].cull_face);
    }

    #[test]
    fn the_teapot_is_the_last_one() {
        let n = NPOLYHEDRA - 1;
        assert_eq!(resolve_which("utah_teapotahedron"), n as i32);
        let mut r = start(StartArgs::new(
            640,
            480,
            &format!("which={n}&titles=false"),
            20260812,
        ));
        r.step();
        let f = r.frame();
        // Thirty-two patches of six by six quads, all in one call.
        let tris: usize = f.batches.iter().map(|b| b.count).sum();
        assert_eq!(tris, 32 * 6 * 6 * 6);
    }

    #[test]
    fn it_shrinks_away_and_grows_back_between_solids() {
        // The change of solid is hidden by scaling the old one down to nothing
        // and the new one back up.
        let mut r = start(StartArgs::new(
            640,
            480,
            "duration=1&spin=false&wander=false&titles=false",
            20260812,
        ));
        let mut smallest = f32::MAX;
        let mut largest = 0.0f32;
        for _ in 0..400 {
            r.step();
            let f = r.frame();
            let mut e = 0.0f32;
            for b in &f.batches {
                for v in &f.vertices[b.first..b.first + b.count] {
                    let p = b.mvp.transform(v.pos);
                    e = e.max(p[0].abs()).max(p[1].abs());
                }
            }
            smallest = smallest.min(e);
            largest = largest.max(e);
        }
        assert!(smallest < 0.05, "it never shrank below {smallest}");
        assert!(largest > 0.3, "it never got bigger than {largest}");
    }

    #[test]
    fn the_keys_step_through_the_list() {
        let mut r = start(StartArgs::new(640, 480, "titles=false", 20260812));
        r.step();
        r.event(XEvent::KeyPress { key: '.' });
        r.step();
        r.event(XEvent::KeyPress { key: ',' });
        r.step();
        assert!(!r.frame().batches.is_empty());
    }
}
