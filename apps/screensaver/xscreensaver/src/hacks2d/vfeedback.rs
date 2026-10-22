//! Port of `hacks/vfeedback.c`.
//!
//! ```text
//! vfeedback, Copyright © 2018-2025 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Simulates video feedback: pointing a video camera at an NTSC television.
//!
//! Created: 4-Aug-2018.
//! ```
//!
//! There is no picture. Every frame this reads back the window it drew last
//! frame, which is the television's own screen, crops a slightly rotated and
//! rescaled rectangle out of it, and broadcasts that back to the same set. That
//! is all a camera pointed at its own monitor does, and the tunnel, the spirals
//! and the colours that arrive from nowhere are what the loop does on its own.
//!
//! The camera never stops moving: it drifts between short pans, zooms and
//! rotations, and every so often somebody walks past the screen and leaves a
//! reflection on the glass, which the loop then treats as picture and drags
//! round the tunnel with everything else.
//!
//! The set's knobs are spun once at startup and then left alone, so what the
//! loop settles into is decided before it starts: with the contrast low it
//! finds a tunnel, and with the contrast high it runs away and the screen goes
//! white, which is what happens when you do this with a real camera and a real
//! television.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::analogtv::{self, AnalogTv, Input, Reception};
use crate::runtime::color::BLACK;
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixmap, Runner, SaverDef, Screenhack, StartArgs, XEvent, XImage, frand,
    random, random_below, screenhack_event_helper,
};

/// Where the camera is pointing: a rectangle in the window, in units of the
/// window, plus the angle it is held at.
#[derive(Clone, Copy, Default)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    th: f64,
}

/// What the camera is doing.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// The set is still warming up, so there is nothing worth filming yet.
    Powerup,
    Idle,
    Move,
}

struct VFeedback {
    /// The size of the picture the camera hands to the transmitter. Upstream
    /// fixes this at 640x480 whatever the window is, and so does this: it is
    /// the camera's sensor, not the screen.
    w: i32,
    h: i32,
    pix: Option<Pixmap>,
    gc: Gc,
    start: f64,
    last_time: f64,
    noise: f64,
    /// How far through the current move, 0 to 1.
    value: f64,
    /// The same, for the reflection on the glass.
    svalue: f64,
    speed: f64,
    dx: f64,
    dy: f64,
    ds: f64,
    dth: f64,
    rect: Rect,
    /// Where the move started, so the eased move can be measured from it.
    orect: Rect,
    specular: (i32, i32, f64),
    state: Mode,
    tv: AnalogTv,
    rec: Reception,
    inp: Input,
    button_down_p: bool,
    mouse: (i32, i32),
    mouse_th: f64,
    dragmode: bool,
}

/// `ease (EASE_IN_OUT_SINE, x)`: still at both ends, quickest in the middle,
/// which is what stops a pan from starting with a jerk.
fn ease_in_out_sine(x: f64) -> f64 {
    -((std::f64::consts::PI * x).cos() - 1.0) / 2.0
}

/// Randomise the set's own controls, and where in the signal the frame starts,
/// which rolls the picture once.
fn twiddle_knobs(st: &mut VFeedback) {
    st.rec.ofs = (random() as usize) % analogtv::SIGNAL_LEN;
    st.rec.level = 0.8 + frand(1.0);
    st.tv.color_control = (frand(1.0) * randsign()) as f32;
    st.tv.contrast_control = (0.4 + frand(1.0)) as f32;
    st.tv.tint_control = frand(360.0) as f32;
}

/// Pick a new place for the camera to be pointing from.
fn twiddle_camera(st: &mut VFeedback) {
    st.rect.x = frand(0.1) * randsign();
    st.rect.y = frand(0.1) * randsign();
    st.rect.w = 1.0 + frand(0.4) * randsign();
    st.rect.h = st.rect.w;
    st.rect.th = 0.2 + frand(1.0) * randsign();
}

fn randsign() -> f64 {
    if random_below(2) == 0 { 1.0 } else { -1.0 }
}

impl VFeedback {
    /// Point the camera at the screen and read off what it sees: the window,
    /// rotated and rescaled into the 640x480 the transmitter wants.
    fn grab_rectangle(&mut self, d: &mut Dpy) -> XImage {
        let (ww, wh) = (d.width(), d.height());
        let pix = self.pix.get_or_insert_with(|| Pixmap::new(ww, wh));
        pix.copy_area(&self.gc, d.win_ref(), 0, 0, ww, wh, 0, 0);

        // The reflection on the glass, which is in front of the screen and so
        // gets filmed along with it.
        if self.specular.2 != 0.0 {
            let p = 0.2;
            let r = if self.svalue < p {
                self.svalue / p
            } else if self.svalue >= 1.0 - p {
                (1.0 - self.svalue) / p
            } else {
                1.0
            };
            let s = self.specular.2 * ease_in_out_sine(r * 2.0);
            pix.fill_arc(
                &self.gc,
                self.specular.0 - (s / 2.0) as i32,
                self.specular.1 - (s / 2.0) as i32,
                s as i32,
                s as i32,
                0,
                360 * 64,
            );
        }

        let mut out = XImage::new(self.w, self.h);
        let c = self.rect.th.cos();
        let s = self.rect.th.sin();
        for oy in 0..out.height() {
            let doy = f64::from(oy) / f64::from(out.height());
            let diy = self.rect.h * doy + self.rect.y - 0.5;

            let dix_mul = self.rect.w / f64::from(out.width());
            let dix_add = (-0.5 + self.rect.x) * self.rect.w;
            let mut ix_mul = c * f64::from(ww);
            let mut iy_mul = s * f64::from(wh);
            // Both offsets are measured with the unscaled multipliers, then the
            // multipliers take the zoom, so the inner loop is one multiply-add.
            let ix_add = (-diy * s + 0.5) * f64::from(ww) + dix_add * ix_mul;
            let iy_add = (diy * c + 0.5) * f64::from(wh) + dix_add * iy_mul;
            ix_mul *= dix_mul;
            iy_mul *= dix_mul;

            for ox in 0..out.width() {
                let ix = (f64::from(ox) * ix_mul + ix_add) as i32;
                let iy = (f64::from(ox) * iy_mul + iy_add) as i32;
                let p = if ix >= 0 && ix < ww && iy >= 0 && iy < wh {
                    pix.get_pixel(ix, iy)
                } else {
                    BLACK
                };
                out.put_pixel(ox, oy, p);
            }
        }
        out
    }
}

impl Screenhack for VFeedback {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let then = d.time;

        if self.state == Mode::Move {
            let v = ease_in_out_sine(self.value);
            self.rect.x = self.orect.x + self.dx * v;
            self.rect.y = self.orect.y + self.dy * v;
            self.rect.th = self.orect.th + self.dth * v;
            self.rect.w = self.orect.w * (1.0 + (self.ds * v));
            self.rect.h = self.orect.h * (1.0 + (self.ds * v));
        }

        if !self.button_down_p {
            self.value += 0.03 * self.speed;
            if self.value > 1.0 || self.state == Mode::Powerup {
                self.orect = self.rect;
                self.value = 0.0;
                self.dx = 0.0;
                self.dy = 0.0;
                self.ds = 0.0;
                self.dth = 0.0;

                match self.state {
                    Mode::Powerup => self.state = Mode::Idle,
                    Mode::Idle => {
                        self.state = Mode::Move;
                        if random_below(5) == 0 {
                            self.ds = frand(0.2) * randsign(); /* zoom */
                        }
                        if random_below(3) == 0 {
                            self.dth = frand(0.2) * randsign(); /* rotate */
                        }
                        if random_below(8) == 0 {
                            self.dx = frand(0.05) * randsign(); /* pan */
                            self.dy = frand(0.05) * randsign();
                        }
                        if random_below(2000) == 0 {
                            twiddle_knobs(self);
                            if random_below(10) == 0 {
                                twiddle_camera(self);
                            }
                        }
                    }
                    Mode::Move => {
                        self.state = Mode::Idle;
                        self.value = 0.3;
                    }
                }
            }

            // A reflection somewhere on the glass, to mix the loop up with a
            // little light from the room it is standing in.
            if self.specular.2 != 0.0 {
                self.svalue += 0.01 * self.speed;
                if self.svalue > 1.0 {
                    self.svalue = 0.0;
                    self.specular.2 = 0.0;
                }
            } else if random_below(300) == 0 {
                let cx = d.width() / 2;
                let cy = d.height() / 2;
                let ww = 4 + (self.rect.h * f64::from(d.height())) as i32 / 12;
                self.specular.0 = cx + (random_below(ww) as f64 * randsign()) as i32;
                self.specular.1 = cy + (random_below(ww) as f64 * randsign()) as i32;
                self.specular.2 = f64::from(ww) * (0.8 + frand(0.4));
                self.svalue = 0.0;
            }
        }

        if self.last_time == 0.0 {
            self.start = then;
        }

        if self.state != Mode::Powerup {
            let img = self.grab_rectangle(d);
            self.tv.load_ximage(&mut self.inp, &img, None, 0, 0, 0, 0);
        }

        self.rec.update();
        {
            let (tv, rec, inp) = (&mut self.tv, &self.rec, &self.inp);
            tv.draw(d.win(), self.noise, &[(rec, inp)]);
        }

        self.tv.powerup = (then - self.start) as f32;
        self.last_time = then;

        // Upstream measures the frame and asks for the rest of a 29.97 Hz frame
        // time; here the runtime decides when the next frame happens.
        (1_000_000.0 / 29.97) as u32
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.tv.configure(width, height);
        self.pix = None;
        let _ = d;
    }

    fn event(&mut self, d: &mut Dpy, event: &XEvent) -> bool {
        // Pan with the left button, rotate with the others.
        match *event {
            XEvent::ButtonPress { x, y, button } if (1..=3).contains(&button) => {
                self.button_down_p = true;
                self.mouse = (x, y);
                self.mouse_th = self.rect.th;
                self.dragmode = button == 1;
                return true;
            }
            XEvent::ButtonRelease { button, .. } if (1..=3).contains(&button) => {
                self.button_down_p = false;
                return true;
            }
            XEvent::MotionNotify { x, y } if self.button_down_p => {
                if self.dragmode {
                    let dx = f64::from(self.mouse.0 - x);
                    let dy = f64::from(self.mouse.1 - y);
                    self.rect.x += dx / f64::from(d.width()) * self.rect.w;
                    self.rect.y += dy / f64::from(d.height()) * self.rect.h;
                    self.mouse = (x, y);
                } else {
                    // Rotate by the angle the drag has swept round the centre.
                    let a1 = -f64::from(self.mouse.1 - d.height() / 2)
                        .atan2(f64::from(self.mouse.0 - d.width() / 2));
                    let a2 = -f64::from(y - d.height() / 2).atan2(f64::from(x - d.width() / 2));
                    self.rect.th = a2 - a1 + self.mouse_th;
                }
                self.settle();
                return true;
            }
            // Zoom with the wheel.
            XEvent::ButtonPress { button, .. } if button == 4 || button == 6 => {
                self.zoom(1.0 - 0.02);
                return true;
            }
            XEvent::ButtonPress { button, .. } if button == 5 || button == 7 => {
                self.zoom(1.0 + 0.02);
                return true;
            }
            XEvent::KeyPress { key } => {
                let i = 0.02;
                match key {
                    /* rotate with <> */
                    '<' | ',' => self.rect.th += i,
                    '>' | '.' => self.rect.th -= i,
                    /* zoom with += */
                    '-' | '_' => return self.zoom(1.0 + i),
                    '=' | '+' => return self.zoom(1.0 - i),
                    /* tv controls with T, C, B, O */
                    't' => self.tv.tint_control += 5.0,
                    'T' => self.tv.tint_control -= 5.0,
                    'c' => self.tv.color_control += 0.1,
                    'C' => self.tv.color_control -= 0.1,
                    'b' => self.tv.brightness_control += 0.01,
                    'B' => self.tv.brightness_control -= 0.01,
                    'o' => self.tv.contrast_control += 0.1,
                    'O' => self.tv.contrast_control -= 0.1,
                    'r' => self.rec.level += 0.01,
                    'R' => self.rec.level -= 0.01,
                    _ => {
                        if screenhack_event_helper(event) {
                            // Space or return respins the knobs.
                            twiddle_knobs(self);
                            self.settle();
                            return true;
                        }
                        return false;
                    }
                }
                self.settle();
                return true;
            }
            _ => {}
        }

        if screenhack_event_helper(event) {
            twiddle_knobs(self);
            self.settle();
            return true;
        }
        false
    }
}

impl VFeedback {
    /// Take the camera off whatever move it was making and leave it where the
    /// user just put it.
    fn settle(&mut self) {
        self.value = 0.0;
        self.state = Mode::Idle;
        self.orect = self.rect;
    }

    fn zoom(&mut self, i: f64) -> bool {
        self.orect = self.rect;
        self.rect.w *= i;
        self.rect.h *= i;
        self.rect.x += (self.orect.w - self.rect.w) / 2.0;
        self.rect.y += (self.orect.h - self.rect.h) / 2.0;
        self.settle();
        true
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut tv = AnalogTv::new(d.width(), d.height());
    tv.set_defaults(
        d.res.float("TVColor") as f32,
        d.res.float("TVTint") as f32,
        d.res.float("TVBrightness") as f32,
        d.res.float("TVContrast") as f32,
    );
    tv.powerup = 0.0;

    let mut inp = Input::new();
    inp.setup_sync(true, false);

    let mut st = VFeedback {
        w: 640,
        h: 480,
        pix: None,
        gc: Gc::new(d.res.pixel("foreground"), BLACK),
        start: 0.0,
        last_time: 0.0,
        noise: d.res.float("noise"),
        value: 0.0,
        svalue: 0.0,
        speed: d.res.float("speed"),
        dx: 0.0,
        dy: 0.0,
        ds: 0.0,
        dth: 0.0,
        rect: Rect::default(),
        orect: Rect::default(),
        specular: (0, 0, 0.0),
        state: Mode::Powerup,
        tv,
        rec: Reception {
            multipath: 0.0,
            ..Reception::default()
        },
        inp,
        button_down_p: false,
        mouse: (0, 0),
        mouse_th: 0.0,
        dragmode: false,
    };
    twiddle_camera(&mut st);
    twiddle_knobs(&mut st);
    st.orect = st.rect;
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    ".foreground:  #CCCC44",
    ".background:  #000000",
    "*noise:       0.02",
    "*speed:       1.0",
    "*TVColor:         70",
    "*TVTint:          5",
    "*TVBrightness:    1.5",
    "*TVContrast:    150",
];

const OPTS: &[Opt] = &[
    Opt::slider("TVColor", "Color Knob", 0.0, 400.0, 5.0, 0, "70"),
    Opt::slider("TVTint", "Tint Knob", 0.0, 360.0, 5.0, 0, "5"),
    Opt::slider("noise", "Noise", 0.0, 0.2, 0.005, 3, "0.02"),
    Opt::slider(
        "TVBrightness",
        "Brightness Knob",
        -75.0,
        100.0,
        1.0,
        1,
        "1.5",
    ),
    Opt::slider("TVContrast", "Contrast Knob", 0.0, 500.0, 5.0, 0, "150"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "vfeedback",
    label: "VFeedback",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2018",
        video: Some("https://www.youtube.com/watch?v=I_MkW0CW4QM"),
        blurb: "A video camera pointed at the television it is plugged into.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
