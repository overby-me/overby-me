//! Port of `hacks/filmleader.c`.
//!
//! ```text
//! filmleader, Copyright © 2018-2025 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Simulate an SMPTE Universal Film Leader playing on an analog television.
//! ```
//!
//! The countdown a projectionist threads up before a reel: a number, a sweep
//! hand going round once a second, two rings and a cross. Between the numbers
//! are the frames nobody was meant to look at, the ones with PICTURE START and
//! the reel and subject boxes, some of them upside down or on their side
//! because of how the leader is spliced.
//!
//! Nothing here is drawn to the screen. It is drawn to a canvas and then
//! *broadcast*: [`crate::runtime::analogtv`] modulates it into a composite
//! video signal and demodulates it again, and the softness, the colour fringes
//! and the roll are what comes back out. Each time the countdown restarts the
//! set is retuned, so the signal is sometimes strong and sometimes barely
//! there.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::analogtv::{self, AnalogTv, Input, Reception};
use crate::runtime::font::Font;
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Pixmap, Runner, SaverDef, Screenhack, StartArgs, XEvent, frand,
    random, random_below, screenhack_event_helper,
};

/// A frame of the leader that is not a number: which font, and whether it is
/// the right way up.
struct Blurb {
    /// When in the countdown it appears.
    t: f64,
    /// 3 draws light on dark; 2 turns it on its side; 1 flips it over.
    k: i32,
    f: i32,
    s: [&'static str; 4],
}

const BLURBS: [Blurb; 18] = [
    Blurb {
        t: 9.1,
        k: 3,
        f: 1,
        s: ["PICTURE", "  START", "", ""],
    },
    Blurb {
        t: 10.0,
        k: 2,
        f: 1,
        s: ["    16", "SOUND", "START", ""],
    },
    Blurb {
        t: 10.5,
        k: 2,
        f: 1,
        s: ["    32", "SOUND", "START", ""],
    },
    Blurb {
        t: 11.6,
        k: 2,
        f: 0,
        s: ["PICTURE", "COMPANY", "SERIES", ""],
    },
    Blurb {
        t: 11.7,
        k: 2,
        f: 0,
        s: ["XSCRNSAVER", "", "", ""],
    },
    Blurb {
        t: 11.9,
        k: 2,
        f: 0,
        s: ["REEL No.", "PROD No.", "PLAY DATE", ""],
    },
    Blurb {
        t: 12.2,
        k: 0,
        f: 0,
        s: ["    SMPTE     ", "UNIVERSAL", "   LEADER", ""],
    },
    Blurb {
        t: 12.3,
        k: 0,
        f: 1,
        s: ["X           ", "X", "X", "X"],
    },
    Blurb {
        t: 12.4,
        k: 0,
        f: 0,
        s: ["    SMPTE     ", "UNIVERSAL", "   LEADER", ""],
    },
    Blurb {
        t: 12.5,
        k: 3,
        f: 1,
        s: ["PICTURE", "", "", ""],
    },
    Blurb {
        t: 12.7,
        k: 3,
        f: 1,
        s: ["HEAD", "", "", ""],
    },
    Blurb {
        t: 12.8,
        k: 2,
        f: 1,
        s: ["OOOO", "", "ASPECT", "TYPE OF"],
    },
    Blurb {
        t: 12.9,
        k: 2,
        f: 0,
        s: ["SOUND", "", "RATIO", ""],
    },
    Blurb {
        t: 13.2,
        k: 1,
        f: 1,
        s: ["                  ", "PICTURE", "", ""],
    },
    Blurb {
        t: 13.3,
        k: 1,
        f: 0,
        s: ["REEL No.      ", "COLOR", "", ""],
    },
    Blurb {
        t: 13.4,
        k: 1,
        f: 0,
        s: ["LENGTH        ", "", "", "ROLL"],
    },
    Blurb {
        t: 13.5,
        k: 1,
        f: 0,
        s: ["SUBJECT", "", "", ""],
    },
    Blurb {
        t: 13.9,
        k: 1,
        f: 1,
        s: ["     ^", "SPLICE", " HERE", ""],
    },
];

struct FilmLeader {
    w: i32,
    h: i32,
    bg: Pixel,
    text_color: Pixel,
    ring_color: Pixel,
    trace_color: Pixel,
    font: Font,
    font2: Font,
    font3: Font,
    /// The canvas the leader is drawn on before it goes on the air.
    pix: Pixmap,
    gc: Gc,
    start: f64,
    last_time: f64,
    value: f64,
    stop: i32,
    noise: f64,
    tv: AnalogTv,
    inp: Input,
    rec: Reception,
    button_down_p: bool,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let (ww, wh) = (d.width(), d.height());
    let mut tv = AnalogTv::new(ww, wh);
    tv.set_defaults(
        d.res.float("TVColor") as f32,
        d.res.float("TVTint") as f32,
        d.res.float("TVBrightness") as f32,
        d.res.float("TVContrast") as f32,
    );
    tv.powerup = 0.0;
    tv.color_control += frand(0.3) as f32;

    let mut inp = Input::new();
    inp.setup_sync(true, false);

    let rec = Reception {
        level: frand(1.0).powf(3.0) * 2.0 + 0.05,
        ofs: (random() as usize) % analogtv::SIGNAL_LEN,
        multipath: 0.0,
        ..Reception::default()
    };

    // Rendered into a 16:9 canvas, since that is what most screens are these
    // days. That means the circle is squashed on a 4:3 screen.
    let r = 16.0 / 9.0;
    let (mut w, mut h) = (712, (712.0 / r) as i32);
    if ww < wh {
        std::mem::swap(&mut w, &mut h);
    }
    let side = w.max(h);

    Box::new(FilmLeader {
        w,
        h,
        bg: d.res.pixel("textBackground"),
        text_color: d.res.pixel("textColor"),
        ring_color: d.res.pixel("ringColor"),
        trace_color: d.res.pixel("traceColor"),
        font: Font::load(d.res.string("numberFont")),
        font2: Font::load(d.res.string("numberFont2")),
        font3: Font::load(d.res.string("numberFont3")),
        pix: Pixmap::new(side, side),
        gc: Gc::new(d.res.pixel("textColor"), d.res.pixel("textBackground")),
        start: 0.0,
        last_time: 0.0,
        value: 18.0, /* Leave time for powerup */
        stop: 2 + random_below(5),
        noise: d.res.float("noise"),
        tv,
        inp,
        rec,
        button_down_p: false,
    })
}

impl FilmLeader {
    fn fill(&mut self, c: Pixel) {
        self.gc.set_foreground(c);
        let (w, h) = (self.w, self.h);
        self.pix.fill_rectangle(&self.gc, 0, 0, w, h);
    }

    /// One of the frames between the numbers.
    fn draw_blurb(&mut self, i: usize) {
        let b = &BLURBS[i];
        let font = match b.f {
            1 => self.font2,
            2 => self.font,
            _ => self.font3,
        };
        let (ink, paper) = if b.k == 3 {
            (self.text_color, self.bg)
        } else {
            (self.bg, self.text_color)
        };
        self.fill(paper);
        self.gc.set_foreground(ink);

        let line_height = font.ascent() + font.descent();
        let rbearing = font.text_width(b.s[0]);
        let x = (self.w - rbearing) / 2;
        let mut y = (f64::from(self.h) * 0.1) as i32 + font.ascent();

        for s in b.s {
            if !s.is_empty() {
                self.pix.draw_string(&self.gc, &font, x, y, s);
            }
            y += (f64::from(line_height) * 1.5) as i32;
        }

        if b.k == 2 {
            /* Rotate clockwise and flip */
            let wh = self.w.min(self.h);
            let (ox, oy) = ((self.w - wh) / 2, (self.h - wh) / 2);
            let src = self.pix.sub_image(ox, oy, wh, wh);
            self.fill(paper);
            for y in 0..wh {
                for x in 0..wh {
                    self.pix.put_pixel(ox + y, oy + x, src.get_pixel(x, y));
                }
            }
        } else if b.k == 1 {
            /* Flip vertically */
            let (w, h) = (self.w, self.h);
            let src = self.pix.sub_image(0, 0, w, h);
            for y in 0..h {
                for x in 0..w {
                    self.pix.put_pixel(x, h - y - 1, src.get_pixel(x, y));
                }
            }
        }
    }
}

impl Screenhack for FilmLeader {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let then = d.time;
        let r = 1.0 - (self.value - self.value.floor());
        let ivalue = self.value as i32;

        let blurb = BLURBS
            .iter()
            .position(|b| self.value >= b.t && self.value <= b.t + 1.0 / 15.0);

        if let Some(i) = blurb {
            self.draw_blurb(i);
        } else if self.value < 2.0 || self.value >= 9.0 {
            /* Black screen */
            self.fill(self.text_color);
        } else {
            self.fill(self.bg);

            if r > 1.0 / 30.0 {
                /* Sweep line and background */
                let x = self.w / 2
                    + (f64::from(self.w)
                        * (std::f64::consts::PI * 2.0 * r - std::f64::consts::FRAC_PI_2).cos())
                        as i32;
                let y = self.h / 2
                    + (f64::from(self.h)
                        * (std::f64::consts::PI * 2.0 * r - std::f64::consts::FRAC_PI_2).sin())
                        as i32;

                self.gc.set_foreground(self.trace_color);
                let (w, h) = (self.w, self.h);
                self.pix.fill_arc(
                    &self.gc,
                    -w,
                    -h,
                    w * 3,
                    h * 3,
                    90 * 64,
                    90 * 64 - ((r + 0.25) * 360.0 * 64.0) as i32,
                );

                self.gc.set_foreground(self.text_color);
                self.gc.set_line_width(1);
                self.pix.draw_line(&self.gc, w / 2, h / 2, x, y);

                self.gc.set_line_width(2);
                self.pix.draw_line(&self.gc, w / 2, 0, w / 2, h);
                self.pix.draw_line(&self.gc, 0, h / 2, w, h / 2);
            }

            /* Big number */
            let s = ((b'0' + ivalue.clamp(0, 9) as u8) as char).to_string();
            let font = self.font;
            let rbearing = font.text_width(&s);
            let x = (self.w - rbearing) / 2;
            let y = (self.h + (font.ascent() - font.descent())) / 2;
            self.gc.set_foreground(self.text_color);
            self.pix.draw_string(&self.gc, &font, x, y, &s);

            /* Annotations on 7 and 4 */
            if (self.value >= 7.75 && self.value <= 7.85)
                || (self.value >= 4.00 && self.value <= 4.25)
            {
                let font = self.font2;
                self.gc.set_foreground(self.bg);

                let s = if ivalue == 4 { "C" } else { "M" };
                let rbearing = font.text_width(s);
                let y = (f64::from(self.h) * 0.1) as i32 + font.ascent();
                let x = (f64::from(self.w) * 0.1) as i32;
                self.pix.draw_string(&self.gc, &font, x, y, s);
                let x = (f64::from(self.w) * 0.9) as i32 - rbearing;
                self.pix.draw_string(&self.gc, &font, x, y, s);

                let s = if ivalue == 4 { "F" } else { "35" };
                let rbearing = font.text_width(s);
                let y = (f64::from(self.h) * 0.95) as i32;
                let x = (f64::from(self.w) * 0.1) as i32;
                self.pix.draw_string(&self.gc, &font, x, y, s);
                let x = (f64::from(self.w) * 0.9) as i32 - rbearing;
                self.pix.draw_string(&self.gc, &font, x, y, s);
            }

            if r > 1.0 / 30.0 {
                /* Two rings around number */
                let r2 = f64::from(self.w) / f64::from(self.h);
                let ss = if d.width() < d.height() { 0.5 } else { 1.0 };

                self.gc.set_foreground(self.ring_color);
                self.gc.set_line_width((f64::from(self.w) * 0.025) as i32);

                let mut w2 = (f64::from(self.w) * 0.8 * ss / r2) as i32;
                let mut h2 = (f64::from(self.h) * 0.8 * ss) as i32;
                let x = (self.w - w2) / 2;
                let y = (self.h - h2) / 2;
                self.pix.draw_arc(&self.gc, x, y, w2, h2, 0, 360 * 64);

                w2 = (f64::from(w2) * 0.8) as i32;
                h2 = (f64::from(h2) * 0.8) as i32;
                let x = (self.w - w2) / 2;
                let y = (self.h - h2) / 2;
                self.pix.draw_arc(&self.gc, x, y, w2, h2, 0, 360 * 64);
            }
        }

        // On the air.
        let (w, h) = (self.w, self.h);
        let img = self.pix.sub_image(0, 0, w, h);
        self.tv.load_ximage(&mut self.inp, &img, None, 0, 0, 0, 0);
        self.rec.update();
        {
            let (tv, rec, inp) = (&mut self.tv, &self.rec, &self.inp);
            tv.draw(d.win(), self.noise, &[(rec, inp)]);
        }

        if !self.button_down_p {
            if self.last_time == 0.0 {
                self.start = then;
            } else {
                self.value -= then - self.last_time;
            }

            if self.value <= 0.0 || (r > 0.9 && self.value <= f64::from(self.stop)) {
                self.value = if random_below(20) != 0 { 8.9 } else { 15.0 };
                self.stop = if random_below(50) != 0 { 2 } else { 1 } + random_below(5);

                if self.value > 9.0 {
                    /* Spin the knobs again */
                    self.rec.level = frand(1.0).powf(3.0) * 2.0 + 0.05;
                    self.rec.ofs = (random() as usize) % analogtv::SIGNAL_LEN;
                    self.tv.color_control += frand(0.3) as f32 - 0.15;
                }
            }
        }

        self.tv.powerup = (then - self.start) as f32;
        self.last_time = then;

        // Upstream measures how long the frame took and asks for the rest of a
        // 29.97 Hz frame time. Here the runtime is what decides when the next
        // frame happens, so this just asks for the frame time.
        (1_000_000.0 / 29.97) as u32
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.tv.configure(width, height);
        if (self.w > self.h) != (width > height) {
            std::mem::swap(&mut self.w, &mut self.h);
        }
        let _ = d;
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        match event {
            XEvent::ButtonPress { .. } => {
                self.button_down_p = true;
                true
            }
            XEvent::ButtonRelease { .. } => {
                self.button_down_p = false;
                true
            }
            _ if screenhack_event_helper(event) => {
                self.value = 15.0;
                self.rec.level = frand(1.0).powf(3.0) * 2.0 + 0.05;
                self.rec.ofs = (random() as usize) % analogtv::SIGNAL_LEN;
                self.tv.color_control += frand(0.3) as f32 - 0.15;
                true
            }
            _ => false,
        }
    }
}

const DEFAULTS: &[&str] = &[
    ".background:  #000000",
    "*textBackground: #9999DD",
    "*textColor:      #000015",
    "*ringColor:      #DDDDFF",
    "*traceColor:     #555577",
    // Note: these font sizes aren't relative to screen pixels, but to the
    // 712 x Y canvas we draw in, which is then scaled by analogtv.
    "*numberFont:  Helvetica Bold 170",
    "*numberFont2: Helvetica 50",
    "*numberFont3: Helvetica 36",
    "*noise:       0.04",
    "*TVColor:         70",
    "*TVTint:          5",
    "*TVBrightness:    3",
    "*TVContrast:    150",
    "*Background:      Black",
    "*fpsSolid:	     True",
];

const OPTS: &[Opt] = &[
    Opt::slider("TVColor", "Color Knob", 0.0, 400.0, 5.0, 0, "70"),
    Opt::slider("TVTint", "Tint Knob", 0.0, 360.0, 5.0, 0, "5"),
    Opt::slider("noise", "Noise", 0.0, 0.2, 0.005, 3, "0.04"),
    Opt::slider("TVBrightness", "Brightness Knob", -75.0, 100.0, 1.0, 0, "3"),
    Opt::slider("TVContrast", "Contrast Knob", 0.0, 500.0, 5.0, 0, "150"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "filmleader",
    label: "Film Leader",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2018",
        video: Some("https://www.youtube.com/watch?v=Cng7hmsuLo0"),
        blurb: "An SMPTE Universal Film Leader, on an analog television.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
