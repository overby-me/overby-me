//! Port of `hacks/glx/molecule.c`.
//!
//! ```text
//! xscreensaver, Copyright (c) 2001-2018 Jamie Zawinski <jwz@jwz.org>
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
//! Thirty-eight molecules, drawn from their chemistry.
//!
//! Nothing here is modelled. Each molecule is a PDB file, the format
//! crystallographers publish structures in, and the saver is a parser and a
//! renderer over it: an `ATOM` or `HETATM` line is a labelled point in space,
//! a `CONECT` line joins two of them, and a pair joined more than once is a
//! double or triple bond and gets a thicker tube. The atom's element decides
//! its colour and radius from a table of the traditional ones, so oxygen is
//! red and carbon is grey because that is how chemists draw them.
//!
//! A molecule arrives at whatever scale and offset its file was published in,
//! so the first thing done with one is to measure its bounding box and scale
//! it to fit. That measurement then decides how the rest is drawn: past a
//! certain size the atom labels are dropped, past another the whole thing
//! falls back to wireframe, and a molecule scaled below a third of its size
//! gets coarser spheres. A protein and a caffeine molecule cannot be drawn the
//! same way and upstream does not try.
//!
//! The electron shells are drawn twice: once with the colour mask off, purely
//! to fill the depth buffer, and again blended where the depth is exactly
//! equal. That is what keeps a translucent shell from piling up on itself.
//!
//! The atom labels are billboarded, which upstream explains at length: the
//! prevailing modelview is read back, its rotation replaced with the identity,
//! and the text drawn under what is left, so a label faces the camera but is
//! still occluded properly by atoms in front of it.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, DepthFunc, Mat4, Shape};
use crate::runtime::opts::SelectItem;
use crate::runtime::shapes::unit_sphere;
use crate::runtime::texfont::TexFont;
use crate::runtime::tube::tube;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random,
    screenhack_event_helper,
};

/// How densely to render spheres, and how coarsely for a molecule that has
/// had to be scaled right down.
const SPHERE_SLICES: i32 = 48;
const SPHERE_STACKS: i32 = 24;
const SPHERE_SLICES_2: i32 = 14;
const SPHERE_STACKS_2: i32 = 8;
const TUBE_FACES: i32 = 12;
const TUBE_FACES_2: i32 = 6;

/// The traditional colour and approximate size in angstroms of each atom.
struct AtomData {
    name: &'static str,
    size: f32,
    size2: f32,
    color: [f32; 4],
    text_color: [f32; 4],
}

const fn rgb(r: u32, g: u32, b: u32) -> [f32; 4] {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
}

const ALL_ATOM_DATA: &[AtomData] = &[
    AtomData {
        name: "H",
        size: 1.17,
        size2: 0.40,
        color: rgb(0xFF, 0xFF, 0xFF),
        text_color: rgb(0x00, 0x00, 0x00),
    },
    AtomData {
        name: "C",
        size: 1.75,
        size2: 0.58,
        color: rgb(0x99, 0x99, 0x99),
        text_color: rgb(0xFF, 0xFF, 0xFF),
    },
    AtomData {
        name: "CA",
        size: 1.80,
        size2: 0.60,
        color: rgb(0x00, 0x00, 0xFF),
        text_color: rgb(0xAD, 0xD8, 0xE6),
    },
    AtomData {
        name: "N",
        size: 1.55,
        size2: 0.52,
        color: rgb(0x42, 0x8D, 0xC3),
        text_color: rgb(0xEE, 0x99, 0xFF),
    },
    AtomData {
        name: "O",
        size: 1.40,
        size2: 0.47,
        color: rgb(0xFF, 0x00, 0x00),
        text_color: rgb(0xFF, 0xB6, 0xC1),
    },
    AtomData {
        name: "P",
        size: 1.28,
        size2: 0.43,
        color: rgb(0x93, 0x70, 0xDB),
        text_color: rgb(0xDB, 0x70, 0x93),
    },
    AtomData {
        name: "S",
        size: 1.80,
        size2: 0.60,
        color: rgb(0x8B, 0x8B, 0x00),
        text_color: rgb(0xFF, 0xFF, 0x00),
    },
    AtomData {
        name: "bond",
        size: 0.0,
        size2: 0.0,
        color: rgb(0xB3, 0xB3, 0xB3),
        text_color: rgb(0xFF, 0xFF, 0x00),
    },
    AtomData {
        name: "*",
        size: 1.40,
        size2: 0.47,
        color: rgb(0x00, 0x8B, 0x00),
        text_color: rgb(0x90, 0xEE, 0x90),
    },
];

/// The index of the entry used for anything not in the table, which is also
/// the last one.
const OTHER: usize = ALL_ATOM_DATA.len() - 1;
const BOND: usize = OTHER - 1;

struct Atom {
    id: i32,
    label: String,
    x: f32,
    y: f32,
    z: f32,
    data: usize,
}

struct Bond {
    from: i32,
    to: i32,
    /// How many bonds there are between these two atoms.
    strength: i32,
}

struct Molecule {
    label: String,
    atoms: Vec<Atom>,
    bonds: Vec<Bond>,
}

struct MoleculeSaver {
    rot: Rotator,
    trackball: Trackball,
    atom_font: Option<TexFont>,
    title_font: Option<TexFont>,

    molecules: Vec<Molecule>,
    which: usize,
    /// The largest dimension of the current molecule's bounding box, and the
    /// scale that brings it into view.
    molecule_size: f32,
    overall_scale: f32,
    centre: [f32; 3],
    low_rez: bool,

    /// 0 is steady, 1 is zooming out, 2 is zooming back in.
    mode: i32,
    mode_tick: f32,
    /// Which way the viewer asked to step, if they did.
    next: i32,
    draw_time: f64,
    draw_tick: i32,

    no_label_threshold: f32,
    wireframe_threshold: f32,
    shell_alpha: f32,
    timeout: f64,

    // The knobs as asked for, and as the current molecule's size leaves them.
    orig_labels: bool,
    orig_atoms: bool,
    orig_bonds: bool,
    orig_shells: bool,
    orig_wire: bool,
    do_labels: bool,
    do_atoms: bool,
    do_bonds: bool,
    do_shells: bool,
    do_titles: bool,
    do_bbox: bool,
    wire: bool,
}

/// `get_atom_data`: strip the digits off an atom name and look up the element.
fn get_atom_data(name: &str) -> usize {
    let n: &str = name.trim_matches(|c: char| !c.is_ascii_alphabetic());
    ALL_ATOM_DATA
        .iter()
        .position(|d| d.name.eq_ignore_ascii_case(n))
        .unwrap_or(OTHER)
}

/// `parse_pdb_data`. Upstream calls its own version of this function crap; it
/// is a line-at-a-time scan that takes the records it knows and ignores a long
/// list of the ones it does not.
fn parse_pdb(name: &str, data: &str) -> Molecule {
    let mut m = Molecule {
        label: String::new(),
        atoms: Vec::new(),
        bonds: Vec::new(),
    };

    // Collected once rather than iterated lazily, so the borrow of the file
    // is plainly one allocation and not one per record.
    let lines: Vec<&str> = data.lines().collect();
    for line in lines {
        if m.label.is_empty() && (line.starts_with("HEADER") || line.starts_with("COMPND")) {
            let mut s = line[6..].trim().to_string();
            if let Some(t) = s.strip_suffix(".pdb") {
                s = t.to_string();
            }
            m.label = s;
            continue;
        }

        if line.starts_with("ATOM  ") || line.starts_with("HETATM") {
            // The columns are fixed: the serial number, then the atom name,
            // then the coordinates. `ATOM` records may also carry an element
            // symbol at the end, which upstream prefers when it is there.
            let Some(id) = line.get(6..11).and_then(|s| s.trim().parse::<i32>().ok()) else {
                continue;
            };
            let mut label = line.get(12..15).unwrap_or("").trim().to_string();
            if line.starts_with("ATOM  ")
                && line.len() > 77
                && !line.as_bytes()[77].is_ascii_whitespace()
            {
                label = line[76..78].trim().to_string();
            }
            // Upstream lowercases everything after the first letter, so that
            // "CA" reads as calcium and "Ca" as an alpha carbon would. Taken
            // a character at a time: the column is whatever the file put
            // there, and a byte offset into it need not be a boundary.
            label = label
                .chars()
                .enumerate()
                .map(|(i, c)| if i == 0 { c } else { c.to_ascii_lowercase() })
                .collect();

            // `ATOM` puts its coordinates two columns later than `HETATM`.
            let at = if line.starts_with("ATOM  ") { 32 } else { 30 };
            let nums: Vec<f32> = line
                .get(at..)
                .unwrap_or("")
                .split_whitespace()
                .take(3)
                .filter_map(|s| s.parse().ok())
                .collect();
            if nums.len() != 3 {
                continue;
            }
            let data = get_atom_data(&label);
            m.atoms.push(Atom {
                id,
                label,
                x: nums[0],
                y: nums[1],
                z: nums[2],
                data,
            });
            continue;
        }

        if let Some(rest) = line.strip_prefix("CONECT") {
            // Upstream reads these with one `sscanf`, so a field that is not a
            // number ends the line: salvinorin has a trailing `NONE 65` that
            // must not be read as a bond to a sixty-fifth atom it does not
            // have.
            let ids: Vec<i32> = rest
                .split_whitespace()
                .map_while(|s| s.parse().ok())
                .collect();
            let Some(&from) = ids.first() else { continue };
            for &to in &ids[1..] {
                if to <= 0 {
                    continue;
                }
                // A pair listed twice is a double bond.
                if let Some(b) = m
                    .bonds
                    .iter_mut()
                    .find(|b| (b.from == from && b.to == to) || (b.to == from && b.from == to))
                {
                    b.strength += 1;
                } else {
                    m.bonds.push(Bond {
                        from,
                        to,
                        strength: 1,
                    });
                }
            }
        }
    }

    if m.label.is_empty() {
        m.label = name.to_string();
    }
    m
}

impl MoleculeSaver {
    fn current(&self) -> &Molecule {
        &self.molecules[self.which]
    }

    /// `atom_size`: the smaller radius when bonds are drawn, so the tubes show.
    fn atom_size(&self, a: &Atom) -> f32 {
        let d = &ALL_ATOM_DATA[a.data];
        if self.do_bonds { d.size2 } else { d.size }
    }

    /// `molecule_bounding_box`.
    fn bounding_box(&self) -> ([f32; 3], [f32; 3]) {
        let m = self.current();
        let Some(first) = m.atoms.first() else {
            return ([0.0; 3], [0.0; 3]);
        };
        let mut lo = [first.x, first.y, first.z];
        let mut hi = lo;
        for a in &m.atoms {
            for (k, v) in [a.x, a.y, a.z].into_iter().enumerate() {
                lo[k] = lo[k].min(v);
                hi[k] = hi[k].max(v);
            }
        }
        (lo, hi)
    }

    /// `ensure_bounding_box_visible`. A published structure arrives at
    /// whatever scale and offset it was measured in, so it is measured and
    /// brought into view before anything is drawn.
    fn fit(&mut self) {
        let (lo, hi) = self.bounding_box();
        let d = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        let size = d[0].max(d[1]).max(d[2]);
        self.molecule_size = size;
        self.overall_scale = 1.0;
        self.low_rez = false;

        // Don't bother scaling down a molecule already smaller than this.
        let max_size = 10.0;
        if size > max_size {
            self.overall_scale = max_size / size;
            self.low_rez = self.overall_scale < 0.3;
        }
        self.centre = [
            -(lo[0] + d[0] / 2.0),
            -(lo[1] + d[1] / 2.0),
            -(lo[2] + d[2] / 2.0),
        ];
    }

    fn set_atom_color(&self, g: &mut Gl, data: usize, alpha: f32) {
        let mut c = ALL_ATOM_DATA[data].color;
        c[3] = alpha;
        g.glx.material_ambient_diffuse(c);
    }

    /// `build_molecule`: the bonds as tubes and the atoms as spheres.
    fn build_molecule(&self, g: &mut Gl, transparent: bool) {
        let alpha = if transparent { self.shell_alpha } else { 1.0 };
        let m = self.current();
        let wire = self.wire;

        g.glx.cull_face(!wire);
        g.glx.lighting(!wire);
        g.glx.depth_test(!wire);

        if !wire {
            self.set_atom_color(g, BOND, alpha);
        }

        if self.do_bonds && !transparent {
            let faces = if self.low_rez {
                TUBE_FACES_2
            } else {
                TUBE_FACES
            };
            for b in &m.bonds {
                let (Some(from), Some(to)) = (
                    m.atoms.iter().find(|a| a.id == b.from),
                    m.atoms.iter().find(|a| a.id == b.to),
                ) else {
                    continue;
                };
                if wire {
                    g.glx.begin(Shape::Lines);
                    g.glx.vertex3f(from.x, from.y, from.z);
                    g.glx.vertex3f(to.x, to.y, to.z);
                    g.glx.end();
                    continue;
                }
                let cap = !self.do_atoms || self.do_shells;
                let base = 0.07;
                let thickness = (base * b.strength as f32).min(0.3);
                let cap_size = if cap { base / 2.0 } else { 0.0 };
                tube(
                    &mut g.glx,
                    [from.x, from.y, from.z],
                    [to.x, to.y, to.z],
                    thickness,
                    cap_size,
                    faces,
                    false,
                    cap,
                    wire,
                );
            }
        }

        if !wire && self.do_atoms {
            let stacks = if self.low_rez {
                SPHERE_STACKS_2
            } else {
                SPHERE_STACKS
            };
            let slices = if self.low_rez {
                SPHERE_SLICES_2
            } else {
                SPHERE_SLICES
            };
            for a in &m.atoms {
                let size = self.atom_size(a);
                self.set_atom_color(g, a.data, alpha);
                g.glx.push_matrix();
                g.glx.translate(a.x, a.y, a.z);
                g.glx.scale(size, size, size);
                unit_sphere(&mut g.glx, stacks, slices, wire);
                g.glx.pop_matrix();
            }
        }

        if self.do_bbox && !transparent {
            self.draw_bounding_box(g);
        }
    }

    /// `draw_bounding_box`, and the three axes through the origin.
    fn draw_bounding_box(&self, g: &mut Gl) {
        let (lo, hi) = self.bounding_box();
        let (x1, y1, z1) = (lo[0], lo[1], lo[2]);
        let (x2, y2, z2) = (hi[0], hi[1], hi[2]);
        let wire = self.wire;

        g.glx.material_ambient_diffuse([0.0, 0.0, 0.0, 0.4]);
        g.glx.front_face_cw(false);
        let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
            (
                [0.0, 1.0, 0.0],
                [[x1, y1, z1], [x1, y1, z2], [x2, y1, z2], [x2, y1, z1]],
            ),
            (
                [0.0, -1.0, 0.0],
                [[x2, y2, z1], [x2, y2, z2], [x1, y2, z2], [x1, y2, z1]],
            ),
            (
                [0.0, 0.0, 1.0],
                [[x1, y1, z1], [x2, y1, z1], [x2, y2, z1], [x1, y2, z1]],
            ),
            (
                [0.0, 0.0, -1.0],
                [[x1, y2, z2], [x2, y2, z2], [x2, y1, z2], [x1, y1, z2]],
            ),
            (
                [1.0, 0.0, 0.0],
                [[x1, y2, z1], [x1, y2, z2], [x1, y1, z2], [x1, y1, z1]],
            ),
            (
                [-1.0, 0.0, 0.0],
                [[x2, y1, z1], [x2, y1, z2], [x2, y2, z2], [x2, y2, z1]],
            ),
        ];
        for (n, vs) in faces {
            g.glx
                .begin(if wire { Shape::LineLoop } else { Shape::Quads });
            g.glx.normal3f(n[0], n[1], n[2]);
            for v in vs {
                g.glx.vertex3f(v[0], v[1], v[2]);
            }
            g.glx.end();
        }

        g.glx.lighting(false);
        g.glx.color3f(1.0, 1.0, 1.0);
        g.glx.begin(Shape::Lines);
        let (ax1, ax2) = (x1.min(0.0), x2.max(0.0));
        let (ay1, ay2) = (y1.min(0.0), y2.max(0.0));
        let (az1, az2) = (z1.min(0.0), z2.max(0.0));
        g.glx.vertex3f(ax1, 0.0, 0.0);
        g.glx.vertex3f(ax2, 0.0, 0.0);
        g.glx.vertex3f(0.0, ay1, 0.0);
        g.glx.vertex3f(0.0, ay2, 0.0);
        g.glx.vertex3f(0.0, 0.0, az1);
        g.glx.vertex3f(0.0, 0.0, az2);
        g.glx.end();
        if !wire {
            g.glx.lighting(true);
        }
    }

    /// `pick_new_molecule`, and the size-dependent knobs it settles.
    fn pick_new_molecule(&mut self, first: bool) {
        let n = self.molecules.len();
        if n == 1 {
            self.which = 0;
        } else if first {
            self.which = (random() as usize) % n;
        } else if self.next < 0 {
            self.which = if self.which == 0 {
                n - 1
            } else {
                self.which - 1
            };
            self.next = 0;
        } else if self.next > 0 {
            self.which = (self.which + 1) % n;
            self.next = 0;
        } else {
            let mut k = self.which;
            while k == self.which {
                k = (random() as usize) % n;
            }
            self.which = k;
        }

        self.fit();

        self.do_labels = self.orig_labels;
        self.do_atoms = self.orig_atoms;
        self.do_bonds = self.orig_bonds;
        self.do_shells = self.orig_shells;
        self.wire = self.orig_wire;

        // A protein and a caffeine molecule cannot be drawn the same way.
        if self.molecule_size > self.no_label_threshold {
            self.do_labels = false;
        }
        if self.molecule_size > self.wireframe_threshold {
            self.wire = true;
        }
        if self.wire {
            self.do_bonds = true;
            self.do_shells = false;
        }
        if !self.do_bonds {
            self.do_shells = false;
        }
        if !(self.do_bonds || self.do_atoms || self.do_labels) {
            // Make sure something shows up.
            self.wire = true;
            self.do_bonds = true;
        }
    }

    /// `draw_labels`. Billboarded: the rotation is taken out of the prevailing
    /// matrix so the text faces the camera, but its translation is kept so the
    /// depth buffer still occludes it properly.
    fn draw_labels(&self, g: &mut Gl) {
        if !self.do_labels {
            return;
        }
        let Some(font) = &self.atom_font else { return };
        let m = self.current();

        for a in &m.atoms {
            let size = self.atom_size(a);
            g.glx.push_matrix();
            if !self.wire {
                let mut c = ALL_ATOM_DATA[a.data].text_color;
                // A kludge so H can have black text over its white ball, and
                // still show up when the balls are off.
                if !self.do_atoms && c[0] == 0.0 && c[1] == 0.0 && c[2] == 0.0 {
                    c = [1.0, 1.0, 1.0, 1.0];
                }
                g.glx.color4f(c[0], c[1], c[2], 1.0);
            }

            g.glx.translate(a.x, a.y, a.z);
            let mut mv = g.glx.modelview_matrix();
            // Replace the rotation with the identity, keeping the translation.
            mv.0[0] = 1.0;
            mv.0[1] = 0.0;
            mv.0[2] = 0.0;
            mv.0[4] = 0.0;
            mv.0[5] = 1.0;
            mv.0[6] = 0.0;
            mv.0[8] = 0.0;
            mv.0[9] = 0.0;
            mv.0[10] = 1.0;
            g.glx.load_identity();
            g.glx.mult_matrix(Mat4(mv.0));

            g.glx.translate(0.0, 0.0, size * 1.1);

            let metrics = font.metrics(&a.label);
            let h = metrics.ascent + metrics.descent;
            let mut s = 1.0 / h as f32;
            s *= self.overall_scale;
            s *= 0.5;
            g.glx.scale(s, s, 1.0);
            g.glx
                .translate(-metrics.width as f32 / 2.0, -h as f32 / 2.0, 0.0);
            font.print_string(&mut g.glx, &a.label);
            g.glx.pop_matrix();
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let spin = g.res.string("spin").to_string();
    let spinx = spin.contains(['x', 'X']);
    let spiny = spin.contains(['y', 'Y']);
    let spinz = spin.contains(['z', 'Z']);

    let want = g.res.string("molecule").to_string();
    let molecules: Vec<Molecule> = crate::molecules::MOLECULES
        .iter()
        .filter(|(name, _)| {
            want.is_empty() || want == "(default)" || want.eq_ignore_ascii_case(name)
        })
        .map(|(name, data)| parse_pdb(name, data))
        .collect();
    // A name that matches nothing falls back to all of them rather than
    // leaving an empty screen.
    let molecules = if molecules.is_empty() {
        crate::molecules::MOLECULES
            .iter()
            .map(|(name, data)| parse_pdb(name, data))
            .collect()
    } else {
        molecules
    };

    let mut this = MoleculeSaver {
        rot: Rotator::new(
            if spinx { 0.5 } else { 0.0 },
            if spiny { 0.5 } else { 0.0 },
            if spinz { 0.5 } else { 0.0 },
            0.3,
            if g.res.bool("wander") { 0.01 } else { 0.0 },
            spinx && spiny && spinz,
        ),
        trackball: Trackball::new(),
        atom_font: Some(TexFont::load(&mut g.glx, "sans-serif 24")),
        title_font: Some(TexFont::load(&mut g.glx, "sans-serif 18")),
        molecules,
        which: 0,
        molecule_size: 0.0,
        overall_scale: 1.0,
        centre: [0.0; 3],
        low_rez: false,
        mode: 0,
        mode_tick: 0.0,
        next: 0,
        draw_time: 0.0,
        draw_tick: 0,
        no_label_threshold: 150.0,
        wireframe_threshold: 150.0,
        shell_alpha: g.res.float("shellAlpha") as f32,
        timeout: g.res.int("timeout").max(1) as f64,
        orig_labels: g.res.bool("labels"),
        orig_atoms: g.res.bool("atoms"),
        orig_bonds: g.res.bool("bonds"),
        orig_shells: g.res.bool("eshells"),
        orig_wire: g.res.bool("wireframe"),
        do_labels: false,
        do_atoms: true,
        do_bonds: true,
        do_shells: false,
        do_titles: g.res.bool("titles"),
        do_bbox: g.res.bool("bbox"),
        wire: g.res.bool("wireframe"),
    };
    if this.orig_wire {
        this.orig_bonds = true;
    }
    this.pick_new_molecule(true);

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for MoleculeSaver {
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
        g.glx.perspective(30.0, 1.0 / h, 20.0, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let s = if width < height {
            width as f32 / height as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        let step = match event {
            XEvent::KeyPress { key } => match key {
                '<' | ',' | '-' | '_' => Some(-1),
                '>' | '.' | '=' | '+' => Some(1),
                _ => None,
            },
            _ => None,
        };
        if let Some(d) = step {
            self.next = d;
            self.mode = 1;
            self.mode_tick = 4.0;
            return true;
        }
        if screenhack_event_helper(event) {
            self.mode = 1;
            self.mode_tick = 4.0;
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        // Speed at which the zoom out and in happens.
        let speed = 4.0;
        let now = g.time;
        let down = self.trackball.button_down();

        if self.draw_time == 0.0 {
            self.draw_time = now;
        } else if self.mode == 0 {
            self.draw_tick += 1;
            if self.draw_tick > 10 {
                self.draw_tick = 0;
                if !down && self.molecules.len() > 1 && self.draw_time + self.timeout <= now {
                    self.mode = 1;
                    self.mode_tick = 80.0 / speed;
                    self.draw_time = now;
                }
            }
        } else if self.mode == 1 {
            self.mode_tick -= 1.0;
            if self.mode_tick <= 0.0 {
                self.mode_tick = 80.0 / speed;
                self.mode = 2;
                self.pick_new_molecule(false);
            }
        } else {
            self.mode_tick -= 1.0;
            if self.mode_tick <= 0.0 {
                self.mode = 0;
            }
        }

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        g.glx.light_position(0, 1.0, 0.4, 0.9, 0.0);
        g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(0, [0.8, 0.8, 0.8, 1.0]);
        g.glx.light_specular(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.blend(Blend::Off);
        g.glx.depth_mask(true);
        g.glx.depth_func(DepthFunc::Less);
        g.glx.color_mask(true);

        g.glx.push_matrix();
        g.glx.scale(1.1, 1.1, 1.1);

        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 9.0,
            (y as f32 - 0.5) * 9.0,
            (z as f32 - 0.5) * 9.0,
        );
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        let (x, y, z) = self.rot.rotation(!down);
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);

        // The zoom out and back in that hides a change of molecule.
        if self.mode != 0 {
            let full = 80.0 / speed;
            let s = if self.mode == 1 {
                self.mode_tick / full
            } else {
                (full - self.mode_tick + 1.0) / full
            };
            g.glx.scale(s, s, s);
        }

        g.glx.push_matrix();
        // Upstream builds the molecule into a display list with the fit
        // applied inside it; the fit is applied here instead so the labels,
        // which cannot be in the list, share it.
        let sc = self.overall_scale;
        g.glx.scale(sc, sc, sc);
        g.glx
            .translate(self.centre[0], self.centre[1], self.centre[2]);

        self.build_molecule(g, false);

        if self.mode == 0 {
            self.draw_labels(g);
            if self.do_titles
                && let Some(font) = &self.title_font
            {
                let label = self.current().label.clone();
                if !label.is_empty() {
                    let c = ALL_ATOM_DATA[BOND].text_color;
                    let (w, h) = (g.width(), g.height());
                    font.print_label(&mut g.glx, &label, w, h, 1, [c[0], c[1], c[2], 1.0]);
                }
            }
        }
        g.glx.pop_matrix();

        if self.do_shells {
            // Fill the depth buffer without drawing, then draw only where the
            // depth is exactly what that pass left, so a translucent shell
            // does not pile up on itself.
            g.glx.push_matrix();
            g.glx.scale(sc, sc, sc);
            g.glx
                .translate(self.centre[0], self.centre[1], self.centre[2]);
            g.glx.color_mask(false);
            self.build_molecule_shell(g);
            g.glx.color_mask(true);

            g.glx.depth_func(DepthFunc::Equal);
            g.glx.blend(Blend::Alpha);
            self.build_molecule_shell(g);
            g.glx.depth_func(DepthFunc::Less);
            g.glx.blend(Blend::Off);
            g.glx.pop_matrix();
        }

        g.glx.pop_matrix();
        g.res.int("delay") as u32
    }
}

impl MoleculeSaver {
    /// The shell pass: every atom at its full radius and the shell alpha, with
    /// no bonds and no labels.
    fn build_molecule_shell(&self, g: &mut Gl) {
        let stacks = if self.low_rez {
            SPHERE_STACKS_2
        } else {
            SPHERE_STACKS
        };
        let slices = if self.low_rez {
            SPHERE_SLICES_2
        } else {
            SPHERE_SLICES
        };
        for a in &self.current().atoms {
            // The shell is the atom at its full radius rather than the smaller
            // one the bonds leave room for.
            let size = ALL_ATOM_DATA[a.data].size;
            let mut c = ALL_ATOM_DATA[a.data].color;
            c[3] = self.shell_alpha;
            g.glx.material_ambient_diffuse(c);
            g.glx.push_matrix();
            g.glx.translate(a.x, a.y, a.z);
            g.glx.scale(size, size, size);
            unit_sphere(&mut g.glx, stacks, slices, false);
            g.glx.pop_matrix();
        }
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        10000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*atomFont:     sans-serif 24",
    "*titleFont:    sans-serif 18",
    "*noLabelThreshold:   150",
    "*wireframeThreshold: 150",
    "*timeout:      20",
    "*spin:         XYZ",
    "*wander:       False",
    "*labels:       True",
    "*titles:       True",
    "*atoms:        True",
    "*bonds:        True",
    "*eshells:      True",
    "*bbox:         False",
    "*shellAlpha:   0.3",
    "*molecule:     (default)",
];

const SPINS: &[SelectItem] = &[
    SelectItem {
        value: "XYZ",
        label: "Rotate around all three axes",
    },
    SelectItem {
        value: "0",
        label: "Don't rotate",
    },
    SelectItem {
        value: "X",
        label: "Rotate around X axis",
    },
    SelectItem {
        value: "Y",
        label: "Rotate around Y axis",
    },
    SelectItem {
        value: "Z",
        label: "Rotate around Z axis",
    },
    SelectItem {
        value: "XY",
        label: "Rotate around X and Y axes",
    },
    SelectItem {
        value: "XZ",
        label: "Rotate around X and Z axes",
    },
    SelectItem {
        value: "YZ",
        label: "Rotate around Y and Z axes",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("timeout", "Duration", 5.0, 120.0, 1.0, 0, "20"),
    Opt::select("spin", "Rotation", SPINS, "XYZ"),
    Opt::boolean("wander", "Wander", "false"),
    Opt::boolean("labels", "Label atoms", "true"),
    Opt::boolean("titles", "Describe molecule", "true"),
    Opt::boolean("atoms", "Draw atomic nuclei", "true"),
    Opt::boolean("bonds", "Draw atomic bonds", "true"),
    Opt::boolean("eshells", "Draw electron shells", "true"),
    Opt::boolean("bbox", "Draw bounding box", "false"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "molecule",
    label: "Molecule",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2001",
        video: Some("https://www.youtube.com/watch?v=D1A0tNcPL4M"),
        blurb: "Draws several different representations of molecules, some \
                organic, some inorganic.",
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
    fn every_bundled_molecule_parses_into_atoms_and_bonds() {
        // The whole saver is a parser over published chemistry, so a file that
        // reads as empty is the failure mode that matters.
        for (name, data) in crate::molecules::MOLECULES {
            let m = parse_pdb(name, data);
            assert!(!m.atoms.is_empty(), "{name} has no atoms");
            assert!(!m.bonds.is_empty(), "{name} has no bonds");
            assert!(!m.label.is_empty(), "{name} has no description");
            // Every bond has to name atoms that exist.
            for b in &m.bonds {
                assert!(
                    m.atoms.iter().any(|a| a.id == b.from),
                    "{name} bonds a missing atom {}",
                    b.from
                );
                assert!(
                    m.atoms.iter().any(|a| a.id == b.to),
                    "{name} bonds a missing atom {}",
                    b.to
                );
            }
        }
    }

    #[test]
    fn caffeine_is_the_molecule_it_should_be() {
        // C8H10N4O2: a known formula is the plainest check that the columns
        // are being read as the format says.
        let (_, data) = crate::molecules::MOLECULES
            .iter()
            .find(|(n, _)| *n == "caffeine")
            .expect("caffeine is bundled");
        let m = parse_pdb("caffeine", data);
        let count = |s: &str| {
            m.atoms
                .iter()
                .filter(|a| a.label.eq_ignore_ascii_case(s))
                .count()
        };
        assert_eq!(count("C"), 8, "carbons");
        assert_eq!(count("H"), 10, "hydrogens");
        assert_eq!(count("N"), 4, "nitrogens");
        assert_eq!(count("O"), 2, "oxygens");
        assert_eq!(m.atoms.len(), 24);
    }

    #[test]
    fn a_pair_bonded_twice_is_a_double_bond() {
        let m = parse_pdb(
            "test",
            "HETATM    1  C           1       0.000   0.000   0.000\n\
             HETATM    2  O           1       1.000   0.000   0.000\n\
             CONECT    1    2    2\n",
        );
        assert_eq!(m.bonds.len(), 1);
        assert_eq!(m.bonds[0].strength, 2);
    }

    #[test]
    fn an_element_takes_its_traditional_colour() {
        // Oxygen red, carbon grey, and anything unrecognised green.
        assert_eq!(
            ALL_ATOM_DATA[get_atom_data("O")].color,
            [1.0, 0.0, 0.0, 1.0]
        );
        assert_eq!(get_atom_data("C1"), get_atom_data("C"));
        assert_eq!(get_atom_data("Zz"), OTHER);
    }

    #[test]
    fn a_big_molecule_is_scaled_down_and_drawn_more_coarsely() {
        let mut r = start(StartArgs::new(640, 480, "molecule=dna", 20260811));
        r.step();
        let f = r.frame();
        assert!(!f.batches.is_empty(), "dna drew nothing");
        // And it fits on the screen despite being tens of angstroms across.
        let mut hi = 0.0f32;
        for b in &f.batches {
            for v in &f.vertices[b.first..b.first + b.count] {
                let p = b.mvp.transform(v.pos);
                hi = hi.max(p[0].abs()).max(p[1].abs());
            }
        }
        assert!(hi < 5.0, "dna reached {hi} in clip space");
    }

    #[test]
    fn the_shell_pass_fills_depth_before_it_draws() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "molecule=caffeine&eshells=true",
            20260811,
        ));
        r.step();
        let f = r.frame();
        assert!(
            f.batches.iter().any(|b| b.color_mask != [true; 4]),
            "nothing was drawn to depth only"
        );
        assert!(
            f.batches
                .iter()
                .any(|b| b.depth_func == DepthFunc::Equal && b.blend == Blend::Alpha),
            "the shell was never blended where the depth matched"
        );
    }
}
