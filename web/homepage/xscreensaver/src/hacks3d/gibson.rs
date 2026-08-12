//! Port of `hacks/glx/gibson.c`.
//!
//! ```text
//! gibson, Copyright © 2020-2025 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Hacking the Gibson, as per the 1995 classic film, HACKERS.
//!
//! In the movie, this was primarily a practical effect: the towers were
//! edge-lit etched perspex, each about four feet tall.
//! ```
//!
//! A grid of glowing towers scrolling past, every face of every one covered in
//! text, with a billboard drifting through announcing that access has been
//! granted or denied.
//!
//! The text is not laid out a letter at a time. Two long strings are each
//! rasterised into a texture once, and a face of a tower is panels sampling
//! random parts of that texture, which is why the towers all read as different
//! screenfuls of the same nonsense. Sampling past the end of the texture wraps
//! round, so a panel's vertical offset can be anything at all.
//!
//! Every so often two towers trade a panel, and a panel switches between the
//! small background text and one big block of it, so the wall of text keeps
//! changing without anything being laid out again.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::easing::{Ease, ease};
use crate::runtime::gl::{Blend, Fog, Shape};
use crate::runtime::rotator::Rotator;
use crate::runtime::texfont::{Metrics, TexFont};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, frand, random,
};

/// The size of one tile of the ground.
const GROUND_QUAD_SIZE: f32 = 30.0;

/// The menu entries the big text is assembled from, in upstream's order.
const MENU: &[&str] = &[
    "\nACCESS TO THIS COMPUTER AND\nITS DATA IS RESTRICTED TO\nAUTHORIZED PERSONNEL ONLY\n\n",
    "\n  PASSWORD ACCEPTED\n             GOD\n\n",
    "PERSONNEL   >>>\n",
    "SEA ROUTINGS   >>>\n",
    "GARBAGE   >>>\n",
    "COMP. SERVICING   >>>\n",
    "COMPANY BUDGETS   >>>\n",
    "SCIENTIFIC BUDGETS   >>>\n",
    "COMPANY POLICIES   >>>\n",
    "ANNUAL RETURNS   >>>\n",
    "MINE RESEARCH   >>>\n",
    "CENTRAL LIBRARY   >>>\n",
    "QUANTATIVE SPEC.   >>>\n",
    "PAYMENT LEVELS   >>>\n",
    "CENTRAL SERVER   >>>\n",
    "GARBAGE   >>>\n",
    "KNMTS. DVPNT.   >>>\n",
    "LICENSING   >>>\n",
    "RELATIONS   >>>\n",
    "TIME SHEET RECS.   >>>\n",
    "RD. PRT. ROUTINGS   >>>\n",
    "RECRUITMENT   >>>\n",
    "TNKR. EXPENDITURE   >>>\n",
    "MINE DEVELOPMENT   >>>\n",
    "GARBAGE   >>>\n",
    "ANNUAL BUDGETS   >>>\n",
    "OIL LOCATIONS   >>>\n",
    "TIME SHEET RECS.   >>>\n",
    "RD. PRT. ROUTINGS   >>>\n",
    "KINEMATICS   >>>\n",
    "TPS. REPORTS   >>>\n",
    "BLAST FRNC. STATUS   >>>\n",
    "ACCOUNTANTS   >>>\n",
    "SHIPPING FORCASTS   >>>\n",
    "INDST. REPORTS   >>>\n",
    "EXPLOR. DVLT.   >>>\n",
    "WRHSE. EXPEND.   >>>\n",
    "GARBAGE   >>>\n",
    "RELOCATIONS   >>>\n",
    "AIRFREIGHT STATUS   >>>\n",
    "TPGC. EXPEND.   >>>\n",
    "SEA-BOARD LAWS   >>>\n",
    "COMPOSITE PLANTS   >>>\n",
    "NUCLEAR RESEARCH   >>>\n",
    "BALLAST REPORTS   >>>\n",
    "\nFILE 1\nWAITING FOR BACK-UP\n\nFILE 2\nWAITING FOR BACK-UP\n\nFILE 3\n\
     WAITING FOR BACK-UP\n\nFILE 4\nWAITING FOR BACK-UP\n",
    "\n",
];

/// What the billboard says as it drifts past.
const BILLBOARDS: [&str; 10] = [
    "ACCESS GRANTED",
    "ACCESS GRANTED",
    "ACCESS DENIED",
    "ACCESS DENIED",
    "ACCESS DENIED",
    "ACCESS DENIED",
    "ACCESS DENIED",
    "PASSWORD ACCEPTED",
    " GIVE ME\nA COOKIE",
    "MESS WITH THE BEST\n  DIE LIKE THE REST",
];

/// One quad of a tower's text: four corners, each a position and a place in
/// the string's texture.
#[derive(Clone, Copy)]
struct Quad {
    v: [[f32; 3]; 4],
    uv: [[f32; 2]; 4],
    /// The backing panel behind a block of big text is a flat wash rather
    /// than more text.
    backing: bool,
}

/// One face's worth of text, generated once and then only shuffled about.
type Panel = Vec<Quad>;

#[derive(Clone)]
struct Tower {
    x: f32,
    y: f32,
    /// How far it has risen out of the floor, from nought to one.
    h: f32,
    /// One bit per face: whether it is showing the big text or the small.
    face_mode: u32,
    bg: Vec<Panel>,
    fg: Vec<Panel>,
}

/// One of the two strings, rasterised into a texture of its own.
struct Text {
    texid: u32,
    metrics: Metrics,
    ascent: i32,
    descent: i32,
    em_width: i32,
}

struct Gibson {
    font: TexFont,
    text: Vec<Text>,
    towers: Vec<Tower>,
    rot: Rotator,
    rot2: Rotator,
    ground_y: f32,
    billboard_y: f32,
    billboard_text: &'static str,
    startup: bool,
    tower_color: [f32; 4],
    tower_color2: [f32; 4],
    edge_color: [f32; 4],
    bg_color: [f32; 4],
    ground_color: [f32; 4],
    ground_dark: [f32; 4],
    ground: u32,

    aspect: f32,
    speed: f32,
    grid_width: usize,
    grid_height: f32,
    grid_depth: usize,
    grid_spacing: f32,
    columns: usize,
    texture: bool,
    wire: bool,
}

fn resource_color(g: &Gl, key: &str) -> [f32; 4] {
    let pixel = crate::runtime::color::parse_color(g.res.string(key))
        .unwrap_or(crate::runtime::color::WHITE);
    let (r, gg, b) = crate::runtime::color::unrgb(pixel);
    [r as f32 / 255.0, gg as f32 / 255.0, b as f32 / 255.0, 1.0]
}

impl Gibson {
    /// `draw_tower_face_text`: the panels of text on one face, worked out
    /// once and kept, because their vertical offsets into the string are
    /// random and would otherwise flicker every frame.
    fn face_text(&self, height: f32, which: bool) -> Panel {
        let n = usize::from(which);
        let t = &self.text[n];
        // Upstream pads its string texture out to a power of two and uses
        // these to say how much of it the ink covers. Here the texture is
        // exactly the ink, so they are one.
        let twratio = 1.0;
        let thratio = 1.0;
        let aspect = (t.ascent + t.descent) as f32 / t.em_width as f32;

        let sx = if which {
            1.0
        } else {
            1.0 / self.columns as f32
        };
        let sy = if which { height * 0.8 } else { sx * 4.0 };
        let lines_in_tex =
            (t.metrics.ascent + t.metrics.descent) as f32 / (t.ascent + t.descent) as f32;
        // How many lines of the string to put in each panel.
        let tex_lines: f32 = if which { 3.0 } else { 8.0 };
        // Upstream also carries a horizontal step and a running vertical
        // one, and then overwrites both at the top of each pass, so all that
        // survives is the height of a panel in the string.
        let mut tsy = sy * thratio * tex_lines / lines_in_tex * aspect;

        let margin = 0.2;
        let mut m2 = margin / 2.0 / if which { 1.0 } else { self.columns as f32 };
        let mut m3 = m2 / if which { 1.0 } else { height };
        let h2 = height * if which { 1.0 - margin } else { 1.0 };
        // The big text gets a wash behind it; the small text does not.
        let bg_p = which && self.texture && !self.wire;

        let mut out = Vec::new();
        let mut x1 = 0.0f32;
        while x1 < 1.0 {
            let x2 = x1 + sx;
            let (tx1, tx2) = (0.0, twratio);
            let mut z = if which { 0.05 } else { 0.0 };
            let mut y2 = h2;
            while y2 > 0.0 {
                let mut y1 = y2 - sy * (1.0 - margin);
                // Clip the panel to the bottom of the tower face.
                if y1 < 0.0 {
                    tsy = y2 / (y2 - y1);
                    y1 = 0.0;
                }
                // Start anywhere in the string: the texture repeats, so an
                // offset of hundreds is as good as one of tenths.
                let ty1 = frand(((t.metrics.ascent + t.metrics.descent) as f64) * 0.8) as f32;
                let ty2 = ty1 + tsy;

                out.push(Quad {
                    v: [
                        [x1 + m2, y1 + m3, z],
                        [x2 - m2, y1 + m3, z],
                        [x2 - m2, y2 - m3, z],
                        [x1 + m2, y2 - m3, z],
                    ],
                    uv: [[tx1, ty2], [tx2, ty2], [tx2, ty1], [tx1, ty1]],
                    backing: false,
                });

                if bg_p {
                    z -= 0.1;
                    m2 -= 0.03;
                    m3 -= 0.03;
                    out.push(Quad {
                        v: [
                            [x1 + m2, y1 + m3, z],
                            [x2 - m2, y1 + m3, z],
                            [x2 - m2, y2 - m3, z],
                            [x1 + m2, y2 - m3, z],
                        ],
                        uv: [[0.0; 2]; 4],
                        backing: true,
                    });
                }

                if which {
                    break;
                }
                y2 -= sy;
            }
            x1 += sx;
        }
        out
    }

    /// Where each face of a tower sits: the top, then the four walls.
    fn face_matrix(&self, g: &mut Gl, face: usize) -> f32 {
        let height = self.grid_height;
        match face {
            0 => {
                g.glx.translate(0.0, 0.0, height);
                1.0
            }
            1 => {
                g.glx.rotate(90.0, 1.0, 0.0, 0.0);
                g.glx.rotate(-90.0, 0.0, 1.0, 0.0);
                g.glx.translate(-1.0, 0.0, 0.0);
                height
            }
            2 => {
                g.glx.rotate(90.0, 1.0, 0.0, 0.0);
                g.glx.rotate(180.0, 0.0, 1.0, 0.0);
                g.glx.translate(-1.0, 0.0, 1.0);
                height
            }
            3 => {
                g.glx.rotate(90.0, 1.0, 0.0, 0.0);
                g.glx.rotate(90.0, 0.0, 1.0, 0.0);
                g.glx.translate(0.0, 0.0, 1.0);
                height
            }
            _ => {
                g.glx.rotate(90.0, 1.0, 0.0, 0.0);
                height
            }
        }
    }

    /// `draw_tower_face` mode 0: the wall of a face and the strips of light
    /// along its edges, which is what makes the towers read as etched perspex.
    fn draw_face_walls(&self, g: &mut Gl, height: f32) {
        if self.wire {
            return;
        }
        let m = 0.015;
        let z = -0.0005;
        g.glx.texturing(false);
        g.glx.normal3f(0.0, 0.0, 1.0);
        let c = self.bg_color;
        g.glx.color4f(c[0], c[1], c[2], c[3]);
        g.glx.begin(Shape::Quads);
        for v in [
            [0.0, 0.0, z * 2.0],
            [1.0, 0.0, z * 2.0],
            [1.0, height, z * 2.0],
            [0.0, height, z * 2.0],
        ] {
            g.glx.vertex3f(v[0], v[1], v[2]);
        }
        g.glx.end();

        let c = self.edge_color;
        g.glx.color4f(c[0], c[1], c[2], c[3]);
        g.glx.begin(Shape::Quads);
        for v in [
            // left
            [0.0, 0.0, z],
            [m, 0.0, z],
            [m, height, z],
            [0.0, height, z],
            // right
            [1.0 - m, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, height, z],
            [1.0 - m, height, z],
            // bottom
            [m, 0.0, 0.0],
            [1.0 - m, 0.0, 0.0],
            [1.0 - m, m, 0.0],
            [m, m, 0.0],
            // top
            [m, height - m, z],
            [1.0 - m, height - m, z],
            [1.0 - m, height, z],
            [m, height, z],
        ] {
            g.glx.vertex3f(v[0], v[1], v[2]);
        }
        g.glx.end();
    }

    /// One panel of text, as recorded.
    fn draw_panel(&self, g: &mut Gl, panel: &Panel, which: bool) {
        let c = if which {
            self.tower_color2
        } else {
            self.tower_color
        };
        for q in panel {
            if q.backing {
                g.glx.texturing(false);
                g.glx.color4f(1.0, 1.0, 1.0, 0.2);
            } else {
                g.glx.texturing(self.texture && !self.wire);
                if self.texture && !self.wire {
                    g.glx.bind_texture(self.text[usize::from(which)].texid);
                }
                g.glx.color4f(c[0], c[1], c[2], c[3]);
            }
            g.glx.begin(if self.wire {
                Shape::LineLoop
            } else {
                Shape::Quads
            });
            for i in 0..4 {
                g.glx.tex_coord2f(q.uv[i][0], q.uv[i][1]);
                g.glx.vertex3f(q.v[i][0], q.v[i][1], q.v[i][2]);
            }
            g.glx.end();
        }
    }

    /// `animate_towers`: shuffle panels between towers, flip some of them
    /// between the two texts, and scroll everything forward.
    fn animate(&mut self) {
        let n = self.towers.len();
        let faces = 5;
        for _ in 0..20 {
            // Trade two towers' background panels, now and then.
            if random().is_multiple_of(20) {
                let (i, j) = (random() as usize % n, random() as usize % n);
                let k = random() as usize % faces;
                let d1 = self.towers[i].bg[k].clone();
                let d2 = self.towers[j].bg[k].clone();
                self.towers[i].bg[k] = d2;
                self.towers[j].bg[k] = d1;
            }
            // And their foreground panels every time round.
            {
                let (i, j) = (random() as usize % n, random() as usize % n);
                let k = random() as usize % faces;
                let d1 = self.towers[i].fg[k].clone();
                let d2 = self.towers[j].fg[k].clone();
                self.towers[i].fg[k] = d2;
                self.towers[j].fg[k] = d1;
            }
            // Re-choose which text each face shows every so often, and show
            // the big one rarely.
            for t in &mut self.towers {
                for k in 0..faces {
                    let frames = 500;
                    let fg_chance = if k == 0 { 100000 } else { 10 };
                    let o = t.face_mode & (1 << k) != 0;
                    let new = if !random().is_multiple_of(frames) {
                        o
                    } else {
                        random().is_multiple_of(fg_chance)
                    };
                    t.face_mode = (t.face_mode & !(1 << k)) | ((new as u32) << k);
                }
            }
        }

        let min = -3.0;
        let max = self.grid_depth as f32 * (1.0 + self.grid_spacing) - self.grid_spacing - 1.0;
        let yspeed = self.speed * 0.05;
        for t in &mut self.towers {
            t.h = (t.h + self.speed * 0.01).min(1.0);
            t.y -= yspeed;
            if t.y < min {
                t.h = 0.0;
                t.y = max;
            }
        }
        // Sorting by depth improves the frame rate slightly.
        self.towers
            .sort_by(|a, b| b.y.partial_cmp(&a.y).unwrap_or(std::cmp::Ordering::Equal));

        self.ground_y -= yspeed / GROUND_QUAD_SIZE;
        if self.ground_y < 1.0 {
            self.ground_y += 1.0;
        }
        self.billboard_y -= yspeed;
        if self.billboard_y < min {
            self.billboard_y = max * (1.0 + frand(8.0) as f32);
            self.billboard_text = BILLBOARDS[random() as usize % BILLBOARDS.len()];
        }
    }

    /// `draw_billboard`: a slab of colour with one of the film's lines on it,
    /// drifting through the towers.
    fn draw_billboard(&self, g: &mut Gl) {
        let m = self.font.metrics(self.billboard_text);
        let w = m.width as f32;
        let h = (m.ascent + m.descent) as f32;
        let s = 0.95 / w;
        let margin = w * 0.1;
        let margin2 = margin * 1.7;
        let y = self.grid_height * 0.3;

        g.glx.push_matrix();
        g.glx.translate(-0.5, self.billboard_y, y);
        g.glx.rotate(90.0, 1.0, 0.0, 0.0);
        g.glx.scale(s, s * 1.5, s);

        let mut c = self.tower_color2;
        c[3] = 0.6;
        g.glx.texturing(false);
        g.glx.color4f(c[0], c[1], c[2], c[3]);
        g.glx.begin(if self.wire {
            Shape::LineLoop
        } else {
            Shape::Quads
        });
        g.glx.normal3f(0.0, 0.0, 1.0);
        for v in [
            [-margin, -margin2],
            [-margin, h + margin2],
            [w + margin, h + margin2],
            [w + margin, -margin2],
        ] {
            g.glx.vertex3f(v[0], v[1], 0.0);
        }
        g.glx.end();

        if self.texture && !self.wire {
            c[3] = 1.0;
            g.glx.color4f(c[0], c[1], c[2], c[3]);
            g.glx.translate(0.0, m.descent as f32, 0.0);
            self.font.print_string(&mut g.glx, self.billboard_text);
        }
        g.glx.pop_matrix();
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let font = TexFont::load(&mut g.glx, g.res.string("towerFont"));

    // The big text is a run of menu entries picked at random, and the small
    // text is twenty lines of whatever a computer in 1995 printed when it had
    // nothing to say.
    let mut big = String::new();
    for _ in 0..MENU.len() {
        big.push_str(MENU[random() as usize % MENU.len()]);
    }
    let mut small = String::new();
    for _ in 0..20 {
        match random() % 11 {
            0 => small.push_str(&format!("{:X}\n", random())),
            1 => small.push_str(&format!("{:X}\n", random() % 0xFFFFFF)),
            2 => small.push_str(&format!("{:X}\n", random() % 0xFFFF)),
            3 => small.push_str(&format!("{}\n", random() % 0xFFFFFF)),
            4 => small.push_str(&format!("{}\n", random() % 0xFFFF)),
            5 => small.push_str(&format!("{}\n", random() % 0xFFF)),
            6 => small.push_str("00000000\n"),
            7 => small.push_str("{{{{{{{{\n"),
            8 => small.push_str("[][][][][][]\n"),
            9 => small.push_str("DEFAULT\n"),
            _ => small.push('\n'),
        }
    }

    let em_width = font.metrics(" ").width;
    let mut text = Vec::new();
    for s in [&small, &big] {
        let (texid, _, _, metrics) = font.string_to_texture(&mut g.glx, s);
        text.push(Text {
            texid,
            metrics,
            ascent: font.ascent(),
            descent: font.descent(),
            em_width,
        });
    }

    let mut tower_color = resource_color(g, "towerText");
    let tower_color2 = resource_color(g, "towerText2");
    let mut bg_color = resource_color(g, "towerColor");
    let mut edge_color = bg_color;
    edge_color[3] = 0.7;
    bg_color[3] = 1.0;
    tower_color[3] = 1.0;

    let ground_color = resource_color(g, "groundColor");
    let mut ground_dark = resource_color(g, "towerColor");
    ground_dark[0] *= 0.05;
    ground_dark[1] *= 0.05;
    ground_dark[2] *= 0.3;
    ground_dark[3] = 1.0;

    let speed = g.res.float("speed") as f32;
    let mut this = Gibson {
        font,
        text,
        towers: Vec::new(),
        rot: Rotator::new(0.0, 0.0, 0.0, 0.0, 0.007 * speed as f64, true),
        rot2: Rotator::new(0.0, 0.0, 0.0, 0.0, 0.01 * speed as f64, true),
        ground_y: 0.0,
        billboard_y: 0.0,
        billboard_text: BILLBOARDS[0],
        startup: true,
        tower_color,
        tower_color2,
        edge_color,
        bg_color,
        ground_color,
        ground_dark,
        ground: 0,
        aspect: 1.0,
        speed,
        grid_width: g.res.int("gridWidth").max(1) as usize,
        grid_height: g.res.int("gridHeight").max(1) as f32,
        grid_depth: g.res.int("gridDepth").max(1) as usize,
        grid_spacing: (g.res.float("gridSpacing") as f32).max(1.0),
        columns: g.res.int("columns").max(1) as usize,
        texture: g.res.bool("texture"),
        wire,
    };

    // Every tower's panels are worked out once here; after that they are only
    // traded about.
    let ww = this.grid_width as f32 * (1.0 + this.grid_spacing) - this.grid_spacing;
    let hh = this.grid_depth as f32 * (1.0 + this.grid_spacing) - this.grid_spacing;
    for y in 0..this.grid_depth {
        for x in 0..this.grid_width {
            let mut t = Tower {
                x: (x as f32 * ww / (this.grid_width.max(2) - 1) as f32) - ww / 2.0,
                y: (y as f32 * hh / this.grid_depth as f32) + 6.0,
                h: -(y as f32) / this.grid_depth as f32 / 2.0,
                face_mode: 0,
                bg: Vec::new(),
                fg: Vec::new(),
            };
            for face in 0..5 {
                let height = if face == 0 { 1.0 } else { this.grid_height };
                t.bg.push(this.face_text(height, false));
                t.fg.push(this.face_text(height * 0.7, true));
            }
            this.towers.push(t);
        }
    }

    // The ground is a grid of glowing tiles, built once.
    this.ground = build_ground(g, &this);
    this.animate();

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

/// `draw_ground`: twenty by twenty cells, each carrying one of a few circuit
/// traces at one of eight orientations.
fn build_ground(g: &mut Gl, this: &Gibson) -> u32 {
    let cells = 20;
    let cell_size = 1.0f32;
    let z = -0.005;

    let list = g.glx.gen_lists(1);
    g.glx.new_list(list);
    g.glx.push_matrix();
    g.glx.scale(1.0 / cells as f32, 1.0 / cells as f32, 1.0);
    g.glx
        .translate(-cells as f32 / 2.0, -cells as f32 / 2.0, 0.0);
    g.glx.translate(0.5, 0.0, 0.0);

    // A dark quad under the whole tile, so the traces read against it.
    let c = this.ground_dark;
    g.glx.color4f(c[0], c[1], c[2], c[3]);
    g.glx.begin(Shape::Quads);
    for v in [
        [0.0, 0.0],
        [cells as f32 * cell_size, 0.0],
        [cells as f32 * cell_size, cells as f32 * cell_size],
        [0.0, cells as f32 * cell_size],
    ] {
        g.glx.vertex3f(v[0], v[1], z);
    }
    g.glx.end();

    let c = this.ground_color;
    g.glx.color4f(c[0], c[1], c[2], c[3]);
    for y in 0..cells {
        for x in 0..cells {
            let (a, b, cc, d, w) = (0.0f32, 1.0 / 3.0, 2.0 / 3.0, 1.0f32, 0.02f32);
            g.glx.push_matrix();
            g.glx.translate(x as f32, y as f32, 0.0);
            g.glx.normal3f(0.0, 0.0, 1.0);
            match random() % 4 {
                0 => {
                    g.glx.rotate(90.0, 0.0, 0.0, 1.0);
                    g.glx.translate(0.0, -1.0, 0.0);
                }
                1 => {
                    g.glx.rotate(-90.0, 0.0, 0.0, 1.0);
                    g.glx.translate(-1.0, 0.0, 0.0);
                }
                2 => {
                    g.glx.rotate(180.0, 0.0, 0.0, 1.0);
                    g.glx.translate(-1.0, -1.0, 0.0);
                }
                _ => {}
            }
            if random().is_multiple_of(2) {
                g.glx.scale(-1.0, -1.0, 1.0);
                g.glx.translate(-1.0, -1.0, 0.0);
            }
            let strips: &[&[[f32; 2]]] = if random().is_multiple_of(2) {
                &[
                    &[[a, b + w], [a, b - w], [b + w, a], [b - w, a]],
                    &[
                        [a, cc + w],
                        [a, cc - w],
                        [b + w, cc + w],
                        [b, cc - w],
                        [cc + w, b + w],
                        [cc - w, b],
                        [cc + w, a],
                        [cc - w, a],
                    ],
                ]
            } else {
                &[
                    &[
                        [a + w, d],
                        [a, d],
                        [a + w, d],
                        [a, d - w],
                        [b + w, cc - w],
                        [b - w, cc - w],
                        [b + w, a],
                        [b - w, a],
                    ],
                    &[
                        [b + w, d],
                        [b - w, d],
                        [cc + w, cc - w],
                        [cc - w, cc - w],
                        [cc + w, a],
                        [cc - w, a],
                    ],
                ]
            };
            for s in strips {
                g.glx.begin(if this.wire {
                    Shape::LineLoop
                } else {
                    Shape::QuadStrip
                });
                for v in *s {
                    g.glx.vertex3f(v[0], v[1], 0.0);
                }
                g.glx.end();
            }
            g.glx.pop_matrix();
        }
    }
    g.glx.pop_matrix();
    g.glx.end_list();
    list
}

impl Hack3d for Gibson {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        g.glx.viewport(0, 0, width, height);
        self.aspect = width as f32 / height as f32;
    }

    fn event(&mut self, _g: &mut Gl, _event: &XEvent) -> bool {
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        // A very wide field of view: the towers have to run away steeply.
        g.glx.perspective(
            100.0,
            self.aspect * 4.0,
            1.0,
            20.0 * self.grid_depth as f32 * 1.5 * (1.0 + self.grid_spacing),
        );
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 1.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        g.glx.clear();
        g.glx.cull_face(true);
        g.glx.depth_test(true);
        g.glx.texturing(false);
        g.glx.color_material(true);
        g.glx.lighting(!self.wire);
        if !self.wire {
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 0.4, 0.2, 0.4, 0.0);
            g.glx.light_ambient(0, [0.2, 0.2, 0.2, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(0, [1.0, 1.0, 1.0, 1.0]);
        }

        g.glx.push_matrix();
        g.glx.scale(10.0, 10.0, 10.0);
        g.glx.translate(0.0, -1.0, 0.0);
        g.glx.rotate(-82.0, 1.0, 0.0, 0.0);

        // The camera drifts up and down and tilts a little.
        let (maxx, maxy, maxz) = (40.0f32, 1.5f32, 100.0f32);
        let minh = -(self.grid_height / 2.0);
        let maxh = -(self.grid_height / 20.0);
        let (x, _, z) = self.rot.position(true);
        let z = minh + (z as f32 * (maxh - minh));
        g.glx
            .translate((x as f32 - 0.5) * self.grid_spacing * 0.005, 0.0, z);
        let (x, y, z) = self.rot2.position(true);
        g.glx.rotate(maxx / 2.0 - x as f32 * maxx, 1.0, 0.0, 0.0);
        g.glx.rotate(maxy / 2.0 - y as f32 * maxy, 0.0, 1.0, 0.0);
        g.glx.rotate(maxz / 2.0 - z as f32 * maxz, 0.0, 0.0, 1.0);

        // Two tiles of ground, one behind the other, scrolling towards you.
        g.glx.push_matrix();
        g.glx.scale(GROUND_QUAD_SIZE, GROUND_QUAD_SIZE, 1.0);
        g.glx.translate(0.0, self.ground_y - 1.5, 0.0);
        g.glx.call_list(self.ground);
        g.glx.translate(0.0, 1.0, 0.0);
        g.glx.call_list(self.ground);
        g.glx.pop_matrix();

        g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_shininess(128.0);

        g.glx.push_matrix();
        if self.grid_width & 1 == 1 {
            // Stay between towers.
            g.glx.translate((self.grid_spacing + 1.0) / 2.0, 0.0, 0.0);
        }
        g.glx.fog(if self.wire {
            None
        } else {
            Some(Fog::Linear {
                start: 0.0,
                end: 100.0,
                color: [0.0, 0.0, 0.0, 1.0],
            })
        });

        // Black out the floor under each tower's base.
        g.glx.texturing(false);
        g.glx.blend(Blend::Off);
        g.glx.depth_test(false);
        g.glx.color4f(0.0, 0.0, 0.0, 1.0);
        for t in &self.towers {
            g.glx.push_matrix();
            g.glx.translate(t.x, t.y, 0.0);
            g.glx.normal3f(0.0, 0.0, 1.0);
            g.glx.begin(if self.wire {
                Shape::LineLoop
            } else {
                Shape::Quads
            });
            for v in [[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]] {
                g.glx.vertex3f(v[0], v[1], 0.01);
            }
            g.glx.end();
            g.glx.pop_matrix();
        }

        if !self.wire {
            g.glx.blend(Blend::AlphaAdd);
            g.glx.cull_face(false);
            // Everything glows through everything else once the towers are up.
            g.glx.depth_test(self.startup);
        }

        let towers = self.towers.clone();
        for t in &towers {
            g.glx.push_matrix();
            g.glx.translate(
                t.x,
                t.y - 1.0,
                -self.grid_height * ease(Ease::InOutSine, (1.0 - t.h) as f64) as f32,
            );

            // The walls of all five faces.
            g.glx.push_matrix();
            g.glx.translate(-0.5, 0.5, 0.0);
            for face in 0..5 {
                g.glx.push_matrix();
                let height = self.face_matrix(g, face);
                self.draw_face_walls(g, height);
                g.glx.pop_matrix();
            }
            g.glx.pop_matrix();

            if self.wire || self.texture {
                g.glx.push_matrix();
                g.glx.translate(-0.5, 0.5, 0.0);
                for face in 0..5 {
                    g.glx.push_matrix();
                    self.face_matrix(g, face);
                    let big = t.face_mode & (1 << face) != 0;
                    let panel = if big { &t.fg[face] } else { &t.bg[face] };
                    self.draw_panel(g, panel, big);
                    g.glx.pop_matrix();
                }
                g.glx.pop_matrix();
            }
            g.glx.pop_matrix();
        }
        g.glx.pop_matrix();

        self.draw_billboard(g);
        g.glx.pop_matrix();
        g.glx.fog(None);

        self.animate();
        if self.startup && self.towers.last().is_some_and(|t| t.h >= 1.0) {
            self.startup = false;
        }

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:        20000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*groundColor:  #8A2BE2",
    "*towerColor:   #4444FF",
    "*towerText:    #DDDDFF",
    "*towerText2:   #FF0000",
    "*towerFont:    sans-serif bold 48",
    "*speed:        1.0",
    "*texture:      True",
    "*gridWidth:    6",
    "*gridHeight:   7",
    "*gridDepth:    6",
    "*gridSpacing:  2.0",
    "*columns:      5",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("speed", "Speed", 0.1, 10.0, 0.1, 1, "1.0"),
    Opt::slider("gridWidth", "Grid width", 1.0, 20.0, 1.0, 0, "6"),
    Opt::slider("gridHeight", "Tower height", 1.0, 20.0, 1.0, 0, "7"),
    Opt::slider("gridDepth", "Grid depth", 1.0, 20.0, 1.0, 0, "6"),
    Opt::slider("gridSpacing", "Spacing", 1.0, 10.0, 0.5, 1, "2.0"),
    Opt::slider("columns", "Text columns", 1.0, 10.0, 1.0, 0, "5"),
    Opt::boolean("texture", "Show text", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "gibson",
    label: "Gibson",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2020",
        video: Some("https://www.youtube.com/watch?v=_gOhMR3TrHA"),
        blurb: "Hacking the Gibson, as per the 1995 classic film, Hackers.",
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

    /// Thirty-six towers of five faces, each face with a panel of each text.
    #[test]
    fn the_grid_is_towers_of_five_faces() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let f = r.frame();
        assert!(!f.batches.is_empty(), "nothing was drawn");
        assert!(
            f.vertices
                .iter()
                .all(|v| v.pos.iter().all(|c| c.is_finite())),
            "a vertex went to NaN"
        );
    }

    /// The panels sample random parts of the string texture, and because it
    /// repeats, an offset well past the end is meaningful.
    #[test]
    fn the_text_is_sampled_from_all_over_the_string() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "gridWidth=2&gridDepth=2",
            20260811,
        ));
        r.step();
        let f = r.frame();
        let uvs: Vec<f32> = f
            .batches
            .iter()
            .filter(|b| b.texture.is_some())
            .flat_map(|b| f.vertices[b.first..b.first + b.count].iter())
            .map(|v| v.uv[1])
            .collect();
        assert!(!uvs.is_empty(), "no text was drawn");
        let hi = uvs.iter().copied().fold(f32::MIN, f32::max);
        assert!(
            hi > 1.0,
            "the highest texture coordinate is {hi}, so nothing wrapped"
        );
    }

    /// Towers rise out of the floor when they appear and scroll towards the
    /// viewer, wrapping round to the back when they pass.
    #[test]
    fn the_towers_scroll_towards_you() {
        let mut r = start(StartArgs::new(640, 480, "speed=10", 20260811));
        r.step();
        let first = r.frame().batches[0].modelview.0[13];
        for _ in 0..60 {
            r.step();
        }
        let later = r.frame().batches[0].modelview.0[13];
        assert!(
            (first - later).abs() > 0.001,
            "nothing moved: {first} then {later}"
        );
    }

    /// The billboard says one of the film's lines, and changes when it has
    /// gone past.
    #[test]
    fn the_billboard_reads_from_the_film() {
        assert!(BILLBOARDS.contains(&"ACCESS DENIED"));
        assert!(BILLBOARDS.iter().any(|s| s.contains("MESS WITH THE BEST")));
        // Access is denied five times as often as it is granted.
        let denied = BILLBOARDS.iter().filter(|s| **s == "ACCESS DENIED").count();
        let granted = BILLBOARDS
            .iter()
            .filter(|s| **s == "ACCESS GRANTED")
            .count();
        assert!(
            denied > granted,
            "{denied} denied is not more than {granted}"
        );
    }

    /// The menu the tower text is built from is upstream's, in its order.
    #[test]
    fn the_tower_text_is_the_computers_menu() {
        assert_eq!(MENU.len(), 47, "a menu entry went missing");
        assert!(MENU[0].contains("RESTRICTED TO"));
        assert!(MENU[1].contains("GOD"));
        assert_eq!(
            MENU.iter().filter(|s| s.starts_with("GARBAGE")).count(),
            4,
            "the garbage is not where it was"
        );
    }
}
