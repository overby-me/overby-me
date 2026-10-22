//! Port of `hacks/fireworkx.c`.
//!
//! ```text
//! Fireworkx 2.2 - Pyrotechnic explosions simulation,
//! an eyecandy, live animating colorful fireworks super-blasts..!
//! Copyright (GPL) 1999-2013 Rony B Chandran <ronybc@gmail.com>
//!
//! From Kerala, INDIA
//! Website: http://www.ronybc.com
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! 2004-OCT: ronybc: Landed on Xscreensaver..!
//! 2012-DEC: ronybc: Almost rewrite of the last version (>4 years old)
//!           with SSE2 optimization, colored light flashes,
//!           HSV color and many visual and speed improvements.
//!
//! Additional coding:
//! Support for different display color modes: put_image()
//! Jean-Pierre Demailly <Jean-Pierre.Demailly@ujf-grenoble.fr>
//!
//! Fixed array access problems by beating on it with a large hammer.
//! Nicholas Miell <nmiell@gmail.com>
//!
//! Help 'free'ing up of memory with needed 'XSync's.
//! Renuka S <renuka@ronybc.com>
//! Rugmini R Chandran <rugmini@ronybc.com>
//! ```
//!
//! Four shells at a time, five hundred sparks each, and two effects that do
//! all the work. The first is a blur that writes back over its own input: each
//! pixel becomes eight parts itself and one part each of its neighbours, and
//! because the result goes straight back into the buffer the blur runs into
//! the pixels it has not reached yet. That feedback is the smoke trail, and it
//! costs nothing to keep.
//!
//! The second is the flash. Each shell keeps a field of one over the distance
//! to its centre, computed once when it goes off, and the four fields are
//! added into the picture in the shell's colour and faded a half percent a
//! frame. That is why the whole sky lights up in the colour of the burst
//! rather than only the sparks being visible.
//!
//! The physics is twelve sub-steps a frame, so the sparks move smoothly at any
//! frame rate. A spark that reaches the ground has one chance in five of
//! bouncing, badly, and otherwise stops burning.
//!
//! Upstream keeps two copies of everything: an SSE2 version and a plain one.
//! This is the plain one.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{hsv_to_rgb, rgb};
use crate::runtime::{
    About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XEvent, frand, random_below,
};

/// A fixed number: the SSE2 path is written around it.
const SHELLCOUNT: usize = 4;
const PIXCOUNT: usize = 500;
const SHELL_LIFE_DEFAULT: i32 = 32;
const SHELL_LIFE_RATIO: u32 = 6;
const POWDER: f32 = 5.0;
const FTWEAK: u32 = 12;
const FLASH_ZOOM: f64 = 0.8;
const G_ACCELERATION: f32 = 0.001;

/// `rnd`: will return zero, so divide with care.
fn rnd(x: u32) -> u32 {
    random_below(x as i32) as u32
}

#[derive(Clone, Copy, Default)]
struct Firepix {
    burn: u32,
    x: f32,
    y: f32,
    xv: f32,
    yv: f32,
}

#[derive(Clone, Default)]
struct Fireshell {
    cx: i64,
    cy: i64,
    seq_number: usize,
    life: u32,
    bicolor: u32,
    flies: bool,
    hshift: bool,
    vshift: bool,
    mortar_fired: bool,
    explode_y: i64,
    air_drag: f32,
    vshift_phase: f64,
    flash_r: f32,
    flash_g: f32,
    flash_b: f32,
    h: i32,
    s: f64,
    v: f64,
    r: u8,
    g: u8,
    b: u8,
    fpix: Vec<Firepix>,
}

impl Fireshell {
    fn roll_rgb(&mut self) {
        let (r, g, b) = hsv_to_rgb(self.h, self.s, self.v);
        self.r = (r >> 8) as u8;
        self.g = (g >> 8) as u8;
        self.b = (b >> 8) as u8;
    }

    fn mix_colors(&mut self) {
        self.h = rnd(360) as i32;
        self.s = frand(0.4) + 0.6;
        self.v = 1.0;
        self.roll_rgb();

        let flash = (rnd(444) + 111) as f32; // Mega Joules!
        self.flash_r = self.r as f32 * flash;
        self.flash_g = self.g as f32 * flash;
        self.flash_b = self.b as f32 * flash;
    }

    fn rotate_hue(&mut self, dh: i32) {
        self.h += dh;
        self.s -= 0.001;
        self.roll_rgb();
    }

    fn wave_value(&mut self) {
        self.vshift_phase += 0.008;
        self.v = self.vshift_phase.sin().abs();
        self.roll_rgb();
    }
}

struct State {
    flash_on: bool,
    shoot: bool,
    /// Rounded down to a multiple of four and of two respectively, as
    /// upstream does, so both effects can work in whole blocks.
    width: i64,
    height: i64,
    max_shell_life: u32,
    delay: u32,
    flash_fade: f32,
    /// One over the distance to each shell's centre, at half resolution,
    /// interleaved by shell.
    light_map: Vec<f32>,
    /// The two picture buffers, in blue, green, red, alpha byte order, each
    /// with a row of gutter before and after so the blur can read past the
    /// edges.
    mem1: Vec<u8>,
    mem2: Vec<u8>,
    /// Byte offset of the picture within each buffer.
    palaka: usize,
    shells: Vec<Fireshell>,
    deferred: u32,
}

impl State {
    fn render_light_map(&mut self, n: usize) {
        let (cx, cy) = (self.shells[n].cx, self.shells[n].cy);
        let mut v = self.shells[n].seq_number;
        let mut y = 0;
        while y < self.height {
            let mut x = 0;
            while x < self.width {
                let dx = (cx - x) as f64;
                let dy = (cy - y) as f64;
                let mut f = (dx * dx + dy * dy).sqrt() + 4.0;
                f = FLASH_ZOOM / f;
                f += f.powf(0.1) * frand(0.0001); // Dither.
                self.light_map[v] = f as f32;
                v += SHELLCOUNT;
                x += 2;
            }
            y += 2;
        }
    }

    fn recycle(&mut self, n: usize, x: i64, y: i64) {
        {
            let (shoot, height, max_life) = (self.shoot, self.height, self.max_shell_life);
            let fs = &mut self.shells[n];
            fs.mortar_fired = shoot;
            fs.explode_y = y;
            fs.cx = x;
            fs.cy = if shoot { height } else { y };
            fs.life = rnd(max_life) + max_life / SHELL_LIFE_RATIO;
            fs.life += if rnd(25) == 0 { max_life * 5 } else { 0 };
            fs.air_drag = 1.0 - (rnd(200) as f32) / (10000.0 + fs.life as f32);
            fs.bicolor = if rnd(5) == 0 { 120 } else { 0 };
            fs.flies = rnd(10) == 0; // Flies' motion.
            fs.hshift = rnd(5) == 0; // Hue shifting.
            fs.vshift = rnd(10) == 0; // Value shifting.
            fs.vshift_phase = std::f64::consts::FRAC_PI_2;

            let pixlife = rnd(fs.life) + fs.life / 10 + 1;
            for i in 0..PIXCOUNT {
                let fp = &mut fs.fpix[i];
                fp.burn = rnd(pixlife) + 32;
                fp.xv = frand(2.0) as f32 * POWDER - POWDER;
                fp.yv = (POWDER * POWDER - fp.xv * fp.xv).sqrt() * (frand(2.0) as f32 - 1.0);
                fp.x = x as f32;
                fp.y = y as f32;
            }
            fs.mix_colors();
        }
        self.render_light_map(n);
    }

    fn recycle_oldest(&mut self, x: i64, y: i64) {
        let mut oldest = 0;
        for n in 0..SHELLCOUNT {
            if self.shells[n].life < self.shells[oldest].life {
                oldest = n;
            }
        }
        self.recycle(oldest, x, y);
    }

    /// One sub-step of one shell. Returns the remaining life, so zero means
    /// the shell is spent.
    fn explode(&mut self, n: usize) -> u32 {
        let (w, h) = (self.width, self.height);
        let palaka = self.palaka;

        if self.shells[n].mortar_fired {
            self.shells[n].cy -= 1;
            if self.shells[n].cy == self.shells[n].explode_y {
                self.shells[n].mortar_fired = false;
                self.shells[n].mix_colors();
                self.render_light_map(n);
            } else {
                let fs = &mut self.shells[n];
                let f = 50.0 + (fs.cy - fs.explode_y) as f32 * 10.0;
                fs.flash_r = f;
                fs.flash_g = f;
                fs.flash_b = f;
                // The rising mortar leaves a bright speck behind it.
                let o = palaka as i64 + (fs.cy * w + fs.cx + rnd(5) as i64 - 2) * 4;
                if o >= 0 && (o as usize) + 2 < self.mem1.len() {
                    let o = o as usize;
                    self.mem1[o] = (rnd(32) + 128) as u8;
                    self.mem1[o + 1] = (rnd(32) + 128) as u8;
                    self.mem1[o + 2] = (rnd(32) + 128) as u8;
                }
                return 1;
            }
        }

        {
            let fade = self.flash_fade;
            let fs = &mut self.shells[n];
            if (fs.bicolor + 1).is_multiple_of(50) {
                fs.rotate_hue(180);
            }
            if fs.bicolor != 0 {
                fs.bicolor -= 1;
            }
            if fs.hshift {
                let dh = rnd(8) as i32;
                fs.rotate_hue(dh);
            }
            if fs.vshift {
                fs.wave_value();
            }
            if fs.flash_r > 1.0 {
                fs.flash_r *= fade;
            }
            if fs.flash_g > 1.0 {
                fs.flash_g *= fade;
            }
            if fs.flash_b > 1.0 {
                fs.flash_b *= fade;
            }
        }

        let (air_drag, flies) = (self.shells[n].air_drag, self.shells[n].flies);
        let (r, g, b) = (self.shells[n].r, self.shells[n].g, self.shells[n].b);

        for i in 0..PIXCOUNT {
            let fp = &mut self.shells[n].fpix[i];
            if fp.burn == 0 {
                continue;
            }
            fp.burn -= 1;
            if flies {
                fp.xv = fp.xv * air_drag + frand(0.1) as f32 - 0.05;
                fp.x += fp.xv;
                fp.yv = fp.yv * air_drag + frand(0.1) as f32 - 0.05 + G_ACCELERATION;
                fp.y += fp.yv;
            } else {
                fp.xv = fp.xv * air_drag + frand(0.01) as f32 - 0.005;
                fp.x += fp.xv;
                fp.yv = fp.yv * air_drag + frand(0.005) as f32 - 0.0025 + G_ACCELERATION;
                fp.y += fp.yv;
            }
            if fp.y > h as f32 {
                if rnd(5) == 3 {
                    fp.yv *= -0.24;
                    fp.y = h as f32;
                } else {
                    // Touch muddy ground.
                    fp.burn = 0;
                }
            }
            let (x, y) = (fp.x, fp.y);
            if x < w as f32 && x > 0.0 && y < h as f32 && y > 0.0 {
                let o = palaka + ((y as i64 * w + x as i64) * 4) as usize;
                self.mem1[o] = b;
                self.mem1[o + 1] = g;
                self.mem1[o + 2] = r;
            }
        }

        self.shells[n].life -= 1;
        self.shells[n].life
    }

    /// Eight parts the pixel and one part each neighbour, written back over
    /// the input so the blur runs into itself. The brighter copy goes to the
    /// second buffer, which is what gets shown.
    fn glow_blur(&mut self) {
        let stride = (self.width * 4) as usize;
        let pm = self.palaka;
        let pa = self.palaka - stride;
        let pb = self.palaka + stride;
        let nn = (self.width * self.height * 4) as usize;

        let mut n = 0;
        while n < nn {
            for c in 0..3 {
                let q = self.mem1[pm + n + c] as u32
                    + self.mem1[pm + n + c + 4] as u32 * 8
                    + self.mem1[pm + n + c + 8] as u32
                    + self.mem1[pa + n + c] as u32
                    + self.mem1[pa + n + c + 4] as u32
                    + self.mem1[pa + n + c + 8] as u32
                    + self.mem1[pb + n + c] as u32
                    + self.mem1[pb + n + c + 4] as u32
                    + self.mem1[pb + n + c + 8] as u32;
                self.mem1[pm + n + c + 4] = (q >> 4) as u8;
                self.mem2[pm + n + c + 4] = if q > 2047 { 255 } else { (q >> 3) as u8 };
            }
            n += 4;
        }
    }

    /// Add each shell's flash field into the picture, in its own colour.
    fn chromo_2x2_light(&mut self) {
        let nl = (self.width * 4) as usize;
        let mut mem = self.palaka;
        let mut v = 0usize;

        let mut rgbf = [0.0f32; SHELLCOUNT * 4];
        for (n, fs) in self.shells.iter().enumerate() {
            rgbf[n * 4] = fs.flash_r;
            rgbf[n * 4 + 1] = fs.flash_g;
            rgbf[n * 4 + 2] = fs.flash_b;
        }

        let addbs = |c: u8, i: f32| -> u8 {
            let i = i + c as f32;
            if i > 255.0 { 255 } else { i as u8 }
        };

        for _ in 0..self.height / 2 {
            for _ in 0..self.width / 2 {
                let (l0, l1, l2, l3) = (
                    self.light_map[v],
                    self.light_map[v + 1],
                    self.light_map[v + 2],
                    self.light_map[v + 3],
                );
                let r = rgbf[0] * l0 + rgbf[4] * l1 + rgbf[8] * l2 + rgbf[12] * l3;
                let g = rgbf[1] * l0 + rgbf[5] * l1 + rgbf[9] * l2 + rgbf[13] * l3;
                let b = rgbf[2] * l0 + rgbf[6] * l1 + rgbf[10] * l2 + rgbf[14] * l3;

                for row in [mem, mem + nl] {
                    for px in [row, row + 4] {
                        self.mem2[px] = addbs(self.mem2[px], b);
                        self.mem2[px + 1] = addbs(self.mem2[px + 1], g);
                        self.mem2[px + 2] = addbs(self.mem2[px + 2], r);
                    }
                }

                mem += 8;
                v += 4;
            }
            mem += nl;
        }
    }

    fn put_image(&self, d: &mut Dpy) {
        let (w, h) = (self.width, self.height);
        for y in 0..h {
            let row = self.palaka + (y * w * 4) as usize;
            for x in 0..w {
                let o = row + (x * 4) as usize;
                let p = rgb(self.mem2[o + 2], self.mem2[o + 1], self.mem2[o]);
                d.win().put_pixel(x as i32, y as i32, p);
            }
        }
    }

    fn resize(&mut self, width: i32, height: i32) {
        self.width = (width - width % 4) as i64;
        self.height = (height - height % 2) as i64;
        let (w, h) = (self.width.max(4), self.height.max(2));
        self.width = w;
        self.height = h;

        let cells = ((h + 2) * w + 8) as usize;
        self.mem1 = vec![0u8; cells * 4];
        self.mem2 = vec![0u8; cells * 4];
        self.palaka = (w * 4) as usize + 16;
        self.light_map = vec![0.0; (w * h) as usize];
        for n in 0..SHELLCOUNT {
            self.render_light_map(n);
        }
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut max_shell_life = d.res.int("maxlife");
    // Transition from xscreensaver <= 5.20.
    if max_shell_life > 100 {
        max_shell_life = 100;
    }
    if max_shell_life < 0 {
        max_shell_life = SHELL_LIFE_DEFAULT;
    }
    let max_shell_life = 10.0f64.powf(max_shell_life as f64 / 50.0 + 2.7) as u32;
    let flash_fade = if max_shell_life < 1000 { 0.998 } else { 0.995 };

    let mut st = State {
        flash_on: d.res.bool("flash"),
        shoot: d.res.bool("shoot"),
        width: 4,
        height: 2,
        max_shell_life,
        delay: d.res.int("delay").max(0) as u32,
        flash_fade,
        light_map: Vec::new(),
        mem1: Vec::new(),
        mem2: Vec::new(),
        palaka: 0,
        shells: (0..SHELLCOUNT)
            .map(|n| Fireshell {
                seq_number: n,
                fpix: vec![Firepix::default(); PIXCOUNT],
                ..Fireshell::default()
            })
            .collect(),
        deferred: 0,
    };
    st.resize(d.width(), d.height());

    for n in 0..SHELLCOUNT {
        let (x, y) = (rnd(st.width as u32) as i64, rnd(st.height as u32) as i64);
        st.recycle(n, x, y);
    }
    d.clear_window();
    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        for _ in 0..FTWEAK {
            for n in 0..SHELLCOUNT {
                if self.explode(n) == 0 {
                    let (x, y) = (
                        rnd(self.width as u32) as i64,
                        rnd(self.height as u32) as i64,
                    );
                    self.recycle(n, x, y);
                }
            }
        }

        while self.deferred > 0 {
            self.deferred -= 1;
            let (x, y) = (
                rnd(self.width as u32) as i64,
                rnd(self.height as u32) as i64,
            );
            self.recycle_oldest(x, y);
        }

        self.glow_blur();
        if self.flash_on {
            self.chromo_2x2_light();
        }
        self.put_image(d);
        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.resize(width, height);
        d.clear_window();
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if let XEvent::ButtonPress { x, y, .. } = *event {
            self.recycle_oldest(x as i64, y as i64);
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    "*delay: 10000",
    "*maxlife: 32",
    "*flash: True",
    "*shoot: False",
    "*verbose: False",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("maxlife", "Activity", 0.0, 100.0, 1.0, 0, "32"),
    Opt::boolean("flash", "Light flash", "true"),
    Opt::boolean("shoot", "Shells upward", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "fireworkx",
    label: "Fireworkx",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Rony B Chandran",
        year: "2004",
        video: Some("https://www.youtube.com/watch?v=-l9BfvnFIPM"),
        blurb: "Exploding fireworks.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
