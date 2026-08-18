//! Port of `hacks/fiberlamp.c`.
//!
//! ```text
//! Copyright (c) 2005 by Tim Auckland <tda10.geo@yahoo.com>
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
//! "fiberlamp" shows Fiber Optic Lamp.  Since there is no closed-form
//! solution to the large-amplitude cantilever equation, the flexible
//! fiber is modeled as a set of descrete nodes.
//!
//! Revision History:
//! 13-Jan-2005: Initial development.
//! ```
//!
//! A fibre-optic lamp. Each fibre is a chain of twenty nodes with a real
//! second-order cantilever simulation running along it: every node carries the
//! load of the ones beyond it, resists bending, and is damped. The tips are lit
//! from a colour wheel that turns slowly, and the fibres are sorted back to
//! front each frame so the near ones draw over the far ones. Every so often the
//! lamp is knocked and the whole bundle sways.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::parse_color;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo};
use crate::runtime::{
    About, Dpy, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, XPoint, frand,
};

/// Angular spread at the base, in degrees.
const SPREAD: f64 = 30.0;
/// Nodes in a fibre. High values have stability problems unless `DT` is small.
const NODES: usize = 20;

// Physics parameters. Tuned carefully to keep realism and avoid instability.
/// Time increment: low is slow, high is less stable.
const DT: f64 = 0.5;
/// Rigidity: low droops, high is stiff.
const PY: f64 = 0.12;
/// Damping: low allows oscillations, high is boring.
const DAMPING: f64 = 0.055;

/// Length of a node. Uniform except for shorter ones at the tips, which is
/// what gives the colour highlights their size. The sum over a whole fibre is
/// exactly one.
fn len_of(a: usize) -> f64 {
    if a < NODES - 3 {
        1.0 / (NODES as f64 - 2.5)
    } else {
        0.25 / (NODES as f64 - 2.5)
    }
}

#[derive(Clone, Copy, Default)]
struct Node {
    phi: f64,
    phidash: f64,
    eta: f64,
    etadash: f64,
    x: f64,
    y: f64,
    z: f64,
}

#[derive(Clone)]
struct Fiber {
    node: [Node; NODES],
    draw: [XPoint; NODES],
}

struct FiberLamp {
    mi: ModeInfo,
    psi: f64,
    dpsi: f64,
    count: i64,
    /// Where the base of the bundle currently sits, after a knock.
    cx: f64,
    /// Where the window sat last frame, so the bundle can be deflected by
    /// however far it has been dragged since.
    rx: i32,
    ry: i32,
    fibers: Vec<Fiber>,
    bright: Pixel,
    medium: Pixel,
    dim: Pixel,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // UNIFORM_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Uniform);
    let nfibers = mi.count.max(1) as usize;

    let named = |spec: &str, fallback: Pixel| parse_color(spec).unwrap_or(fallback);
    let (bright, medium, dim) = if mi.npixels() > 2 {
        // Colours for the fibre bodies. The tips are handled separately.
        (
            named("#E0E0C0", mi.white),
            named("#808070", mi.white),
            named("#404020", mi.black),
        )
    } else {
        (mi.white, mi.white, mi.black)
    };

    let mut linewidth = 1;
    if mi.width > 2560 || mi.height > 2560 {
        linewidth = 3; // Retina displays
    }

    let fibers = (0..nfibers)
        .map(|_| {
            let phi = std::f64::consts::PI / 180.0 * frand(SPREAD);
            let eta = frand(2.0 * std::f64::consts::PI) - std::f64::consts::PI;
            let mut node = [Node::default(); NODES];
            for n in node.iter_mut() {
                n.phi = phi;
                n.eta = eta;
            }
            node[0].etadash = 0.002 / DT;
            Fiber {
                node,
                draw: [XPoint::default(); NODES],
            }
        })
        .collect();

    let mut st = FiberLamp {
        psi: frand(2.0 * std::f64::consts::PI),
        dpsi: 0.01,
        count: 0,
        cx: 0.0,
        rx: 0,
        ry: 0,
        fibers,
        bright,
        medium,
        dim,
        mi,
    };
    st.mi.gc.set_line_width(linewidth);
    d.clear_window();
    st.knock();
    Box::new(st)
}

impl FiberLamp {
    /// Knock the lamp, so the whole bundle sways.
    fn knock(&mut self) {
        let scale = (self.mi.width / 2).max(1) as f64;
        self.cx = (frand(scale / 4.0) - scale / 8.0) / scale;
        self.count = 0;
    }

    /// One bubble pass is enough to keep the fibres back to front: the order
    /// only changes slowly.
    fn sort_fibers(&mut self) {
        for i in 1..self.fibers.len() {
            if self.fibers[i - 1].node[NODES - 1].z > self.fibers[i].node[NODES - 1].z {
                self.fibers.swap(i - 1, i);
            }
        }
    }
}

impl Screenhack for FiberLamp {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let mut ww = self.mi.width;
        let mut hh = self.mi.height;
        let mut cx = ww / 2;
        let mut cy = hh;

        if ww > hh * 5 || hh > ww * 5 {
            // Window has a weird aspect ratio.
            if ww > hh {
                hh = ww;
                cy = hh / 4;
            } else {
                ww = hh;
                cx = 0;
                cy = hh * 3 / 4;
            }
        }

        self.psi += self.dpsi; // Turn the colour wheel.
        self.sort_fibers();

        // A canvas never moves, so this settles to zero after the first frame.
        let (x, y) = (cx, cy);
        let (dx, dy) = ((self.rx - x) as f64, (self.ry - y) as f64);
        let base_cx = self.cx;

        for f in self.fibers.iter_mut() {
            f.node[0].eta += DT * f.node[0].etadash;
            f.node[0].x = base_cx;
            f.node[NODES - 2].x *= 0.1 * dy;
            f.node[NODES - 2].x += 0.05 * dx;

            // Second-order differential equation, node by node.
            for i in 1..NODES {
                let p = f.node[i - 1];
                let n = f.node[i];
                let mut pload = 0.0;
                let mut eload = 0.0;
                let pstress = (n.phi - p.phi) * PY;
                let estress = (n.eta - p.eta) * PY;
                let dxi = n.x - p.x;
                let dzi = n.z - p.z;
                let li = (dxi * dxi + dzi * dzi).sqrt() / len_of(i);
                let drag = DAMPING * len_of(i) * len_of(i) * (NODES * NODES) as f64;

                if li > 0.0 {
                    for j in i + 1..NODES {
                        let nn = f.node[j];
                        let dxj = nn.x - n.x;
                        let dzj = nn.z - n.z;
                        // Radial and transverse load from everything further
                        // out along the fibre.
                        pload += len_of(j) * (dxi * dxj + dzi * dzj) / li;
                        eload += len_of(j) * (dxi * dzj - dzi * dxj) / li;
                    }
                }

                {
                    let n = &mut f.node[i];
                    n.phidash += DT * (pload - pstress - drag * n.phidash) / len_of(i);
                    n.phi += DT * n.phidash;
                    n.etadash += DT * (eload - estress - drag * n.etadash) / len_of(i);
                    n.eta += DT * n.etadash;
                }

                let p = f.node[i - 1];
                let (sp, cp) = (p.phi.sin(), p.phi.cos());
                let (se, ce) = (p.eta.sin(), p.eta.cos());
                let l = len_of(i - 1);
                f.node[i].x = p.x + l * ce * sp;
                f.node[i].y = p.y - l * cp;
                f.node[i].z = p.z + l * se * sp;

                f.draw[i - 1] = XPoint {
                    x: cx + (ww as f64 / 2.0 * f.node[i].x) as i32,
                    y: cy + (ww as f64 / 2.0 * f.node[i].y) as i32,
                };
            }
        }

        let black = self.mi.black;
        self.mi.gc.set_foreground(black);
        let (w, h) = (self.mi.width, self.mi.height);
        d.win().fill_rectangle(&self.mi.gc, 0, 0, w, h);

        let npixels = self.mi.npixels();
        for at in 0..self.fibers.len() {
            let node1 = self.fibers[at].node[1];
            let last_z = self.fibers[at].node[NODES - 1].z;

            let ax = node1.x - base_cx + 0.025;
            let ay = node1.z + 0.02;
            let angle = ay.atan2(ax) + self.psi;

            let tipcolor = if npixels > 0 {
                let mut c =
                    (npixels as f64 * angle / (2.0 * std::f64::consts::PI)) as i32 % npixels;
                if c < 0 {
                    c += npixels;
                }
                self.mi.pixel(c as usize)
            } else {
                self.mi.white
            };

            let (tiplen, fibercolor) = if node1.z < 0.0 {
                (2, self.dim) // Back
            } else if last_z < 0.7 {
                (3, self.medium) // Middle
            } else {
                (3, self.bright) // Front
            };

            self.mi.gc.set_foreground(fibercolor);
            let body = self.fibers[at].draw;
            d.win().draw_lines(&self.mi.gc, &body[..NODES - tiplen]);

            self.mi.gc.set_foreground(tipcolor);
            d.win()
                .draw_lines(&self.mi.gc, &body[NODES - 1 - tiplen..NODES - 1]);
        }

        self.rx = x;
        self.ry = y;

        self.count += 1;
        if self.count > self.mi.cycles as i64 {
            self.knock();
        }

        self.mi.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.mi.reshape(width, height);
        d.clear_window();
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*delay: 10000",
    "*count: 500",
    "*cycles: 10000",
    "*ncolors: 64",
    "*fpsTop: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("count", "Fibers", 10.0, 500.0, 10.0, 0, "500"),
    Opt::slider(
        "cycles",
        "Time between knocks",
        100.0,
        10000.0,
        100.0,
        0,
        "10000",
    ),
];

pub static DEF: SaverDef = SaverDef {
    slug: "fiberlamp",
    label: "Fiber Lamp",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Tim Auckland",
        year: "2005",
        video: Some("https://www.youtube.com/watch?v=PvYKJ-vkxE0"),
        blurb: "A fiber-optic lamp. Groovy.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
