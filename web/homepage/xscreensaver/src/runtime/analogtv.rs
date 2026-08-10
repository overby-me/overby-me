//! Port of `hacks/analogtv.c`: an NTSC television, simulated from the signal up.
//!
//! ```text
//! analogtv, Copyright (c) 2003-2018 Trevor Blackwell <tlb@tlb.org>
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
//! Six of the hacks draw into this rather than onto the screen. What they get
//! back is not a filter over their picture: the picture is *modulated into a
//! composite video signal* and then demodulated by a simulated receiver, and
//! everything that makes it look like television falls out of doing that
//! honestly.
//!
//! A line of signal is 912 samples at four times the colour subcarrier, and
//! carries sync, back porch, colour burst, picture and front porch at their
//! real positions in the 63.5 microseconds a line takes. Luma and chroma share
//! that one wire: chroma rides on a subcarrier, and the receiver gets it back
//! by multiplying by a reference it recovers from the colour burst at the start
//! of each line. That is why the colours smear sideways and the luma does not,
//! why fine vertical detail turns into false colour, and why a signal with no
//! burst comes out in black and white: all of it is the demodulator doing what
//! a demodulator does.
//!
//! The rest is the set itself, and Trevor Blackwell's notes on why each piece
//! is there are kept on the functions below: the horizontal oscillator that
//! also drove the high voltage, so a bright line is a wider line; the beam
//! slowing at the right, so the right edge is squashed and brighter; and the
//! sync separator hunting for the pulse, so a bad signal bends the top of the
//! picture.
//!
//! Three things here that upstream has and this does not, none of them visible.
//! It splits the work across a thread pool; this is one loop. It has a whole
//! path for eight-bit colormapped displays, which a canvas is not. And it packs
//! pixels according to the visual's masks, where here they are always RGBA.

use super::color::{Pixel, rgb};
use super::fb::{Fb, XImage};
use super::rand::{frand, random};

/* We don't handle interlace here */
pub const V: usize = 262;
pub const TOP: usize = 30;
pub const VISLINES: usize = 200;
pub const BOT: usize = TOP + VISLINES;

/// This really defines our sampling rate, 4x the colorburst frequency. Handily
/// equal to the Apple II's dot clock.
pub const H: usize = 912;

/* Each line is 63500 nS long. The sync pulse is 4700 nS long, etc.
Define sync, back porch, colorburst, picture, and front porch positions */
pub const SYNC_START: usize = 0;
pub const BP_START: usize = 4700 * H / 63500;
pub const CB_START: usize = 5800 * H / 63500;
/// `signal[row][PIC_START]` is the first displayed pixel.
pub const PIC_START: usize = 9400 * H / 63500;
pub const PIC_LEN: usize = 52600 * H / 63500;
pub const FP_START: usize = 62000 * H / 63500;
pub const PIC_END: usize = FP_START;

/// TVs scan past the edges of the picture tube, so normally you only want to
/// use about the middle 3/4 of the nominal scan line.
pub const VIS_START: usize = PIC_START + PIC_LEN / 8;
pub const VIS_END: usize = PIC_START + PIC_LEN * 7 / 8;
pub const VIS_LEN: usize = VIS_END - VIS_START;

pub const GHOSTFIR_LEN: usize = 4;

/* analogtv.signal is in IRE units, as defined below: */
pub const WHITE_LEVEL: i32 = 100;
pub const GRAY50_LEVEL: i32 = 55;
pub const GRAY30_LEVEL: i32 = 35;
pub const BLACK_LEVEL: i32 = 10;
pub const BLANK_LEVEL: i32 = 0;
pub const SYNC_LEVEL: i32 = -40;
pub const CB_LEVEL: i32 = 20;

pub const SIGNAL_LEN: usize = V * H;

/// The number of intensity levels we deal with for gamma correction &c.
const CV_MAX: usize = 1024;

/// Corresponds to 2400 vertical pixels, beyond which it interpolates extra
/// black lines.
const MAX_LINEHEIGHT: usize = 12;

const FASTRND_A: u32 = 1103515245;
const FASTRND_C: u32 = 12345;

/// One channel's worth of composite video, as a camera would have produced it.
///
/// The extra line at the end is a copy of the first, so the receiver can index
/// past the bottom without worrying about the wrap.
pub struct Input {
    pub signal: Vec<i8>,
}

impl Default for Input {
    fn default() -> Self {
        Self::new()
    }
}

impl Input {
    pub fn new() -> Input {
        Input {
            signal: vec![0; (V + 1) * H],
        }
    }

    #[inline]
    pub fn at(&self, line: usize, x: usize) -> i8 {
        self.signal[line * H + x]
    }

    #[inline]
    pub fn set(&mut self, line: usize, x: usize, v: i8) {
        self.signal[line * H + x] = v;
    }

    /// `analogtv_setup_sync`: put the sync pulses, blanking and (optionally)
    /// the colour burst into every line. Without the burst the receiver finds
    /// no colour and the picture comes out grey, which is exactly what happened
    /// to a computer in text mode.
    pub fn setup_sync(&mut self, do_cb: bool, do_ssavi: bool) {
        let synclevel = if do_ssavi { WHITE_LEVEL } else { SYNC_LEVEL } as i8;

        for lineno in 0..V {
            let vsync = (3..7).contains(&lineno);
            let mut i = SYNC_START;
            if vsync {
                while i < BP_START {
                    self.set(lineno, i, BLANK_LEVEL as i8);
                    i += 1;
                }
                while i < H {
                    self.set(lineno, i, synclevel);
                    i += 1;
                }
            } else {
                while i < BP_START {
                    self.set(lineno, i, synclevel);
                    i += 1;
                }
                while i < PIC_START {
                    self.set(lineno, i, BLANK_LEVEL as i8);
                    i += 1;
                }
                while i < FP_START {
                    self.set(lineno, i, BLACK_LEVEL as i8);
                    i += 1;
                }
            }
            while i < H {
                self.set(lineno, i, BLANK_LEVEL as i8);
                i += 1;
            }

            if do_cb {
                /* 9 cycles of colorburst */
                let mut i = CB_START;
                while i < CB_START + 36 {
                    let a = self.at(lineno, i + 1) as i32 + CB_LEVEL;
                    self.set(lineno, i + 1, a as i8);
                    let b = self.at(lineno, i + 3) as i32 - CB_LEVEL;
                    self.set(lineno, i + 3, b as i8);
                    i += 4;
                }
            }
        }
    }

    /// `analogtv_draw_solid`: a rectangle of signal, in the four-sample pattern
    /// that carries one colour.
    pub fn draw_solid(&mut self, left: i32, right: i32, top: i32, bot: i32, ntsc: [i32; 4]) {
        let right = right.max(left + 4);
        let bot = bot.max(top + 1);
        for y in top.max(0)..bot.min(V as i32) {
            for x in left.max(0)..right.min(H as i32) {
                self.set(y as usize, x as usize, ntsc[(x & 3) as usize] as i8);
            }
        }
    }

    /// The same in fractions of the visible picture, given a colour as luma,
    /// chroma and phase rather than as samples.
    pub fn draw_solid_rel_lcp(
        &mut self,
        left: f64,
        right: f64,
        top: f64,
        bot: f64,
        luma: f64,
        chroma: f64,
        phase: f64,
    ) {
        let topi = (TOP as f64 + VISLINES as f64 * top) as i32;
        let boti = (TOP as f64 + VISLINES as f64 * bot) as i32;
        let lefti = (VIS_START as f64 + VIS_LEN as f64 * left) as i32;
        let righti = (VIS_START as f64 + VIS_LEN as f64 * right) as i32;
        self.draw_solid(lefti, righti, topi, boti, lcp_to_ntsc(luma, chroma, phase));
    }
}

/// A colour, as the four samples of one subcarrier cycle that encode it.
///
/// Luma is the average of the four and chroma is the amplitude of the sine they
/// trace out; the phase of that sine is the hue. That is the whole of NTSC
/// colour in one line of arithmetic.
pub fn lcp_to_ntsc(luma: f64, chroma: f64, phase: f64) -> [i32; 4] {
    let mut ntsc = [0; 4];
    for (i, n) in ntsc.iter_mut().enumerate() {
        let w = 90.0 * i as f64 + phase;
        let val = luma + chroma * (std::f64::consts::PI / 180.0 * w).cos();
        *n = val.clamp(0.0, 127.0) as i32;
    }
    ntsc
}

/// How one channel arrives at the aerial: how strong, how delayed, and what it
/// bounced off on the way.
pub struct Reception {
    pub ofs: usize,
    pub level: f64,
    pub multipath: f64,
    pub freqerr: f64,
    pub ghostfir: [f64; GHOSTFIR_LEN],
    pub ghostfir2: [f64; GHOSTFIR_LEN],
    pub hfloss: f64,
    pub hfloss2: f64,
}

impl Default for Reception {
    fn default() -> Self {
        Reception {
            ofs: 0,
            level: 0.0,
            multipath: 0.0,
            freqerr: 0.0,
            ghostfir: [0.0; GHOSTFIR_LEN],
            ghostfir2: [0.0; GHOSTFIR_LEN],
            hfloss: 0.0,
            hfloss2: 0.0,
        }
    }
}

impl Reception {
    /// Wander the ghosting filter about. With no multipath this settles to a
    /// fixed slight ring; with it, the reflections come and go the way they do
    /// when something moves near the aerial.
    pub fn update(&mut self) {
        if self.multipath > 0.0 {
            for g in self.ghostfir2.iter_mut() {
                *g += -(*g / 16.0) + self.multipath * (frand(0.02) - 0.01);
            }
            if random().is_multiple_of(20) {
                let i = (random() as usize) % GHOSTFIR_LEN;
                self.ghostfir2[i] = self.multipath * (frand(0.08) - 0.04);
            }
            for i in 0..GHOSTFIR_LEN {
                self.ghostfir[i] = 0.8 * self.ghostfir[i] + 0.2 * self.ghostfir2[i];
            }
        } else {
            for (i, g) in self.ghostfir.iter_mut().enumerate() {
                *g = if i >= GHOSTFIR_LEN / 2 {
                    (if i & 1 != 0 { 0.04 } else { -0.08 }) / GHOSTFIR_LEN as f64
                } else {
                    0.0
                };
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Yiq {
    y: f32,
    i: f32,
    q: f32,
}

#[derive(Clone, Copy, Default)]
struct Level {
    index: usize,
    value: f64,
}

/// The television.
pub struct AnalogTv {
    /// Where the picture is drawn before it goes to the window, and where in
    /// the window it lands.
    image: Fb,
    usewidth: i32,
    useheight: i32,
    xrepl: i32,
    subwidth: i32,
    screen_xo: i32,
    screen_yo: i32,
    window_width: i32,
    window_height: i32,

    agclevel: f32,

    /* If you change these, call set_demod */
    pub tint_control: f32,
    pub color_control: f32,
    pub brightness_control: f32,
    pub contrast_control: f32,
    pub height_control: f32,
    pub width_control: f32,
    pub squish_control: f32,
    pub horiz_desync: f32,
    pub squeezebottom: f32,
    pub powerup: f32,

    pub flutter_horiz_desync: bool,

    /// Hash: the small white streaks that appear all over the screen when
    /// someone is running the vacuum cleaner. `shrinkpulse` squashes the
    /// picture horizontally for one frame, which is the line voltage dropping
    /// when a big motor starts.
    hashnoise_on: bool,
    hashnoise_enable: bool,
    shrinkpulse: i32,

    crtload: [f32; V],
    /// Gamma-corrected intensity, one entry per level of the 1024 the
    /// demodulator works in.
    gamma: [u8; CV_MAX],

    tint_i: f32,
    tint_q: f32,

    cur_hsync: usize,
    line_hsync: [usize; V],
    cur_vsync: usize,
    cb_phase: [f32; 4],
    line_cb_phase: Vec<[f32; 4]>,

    channel_change_cycles: usize,
    rx_signal_level: f64,
    /// The aerial: noise plus every channel that is being received, summed.
    /// Two lines longer than a frame so a line can be read past the end.
    rx_signal: Vec<f32>,

    leveltable: [[Level; MAX_LINEHEIGHT + 1]; MAX_LINEHEIGHT + 1],

    random0: u32,
    random1: u32,

    puheight: f32,
    need_clear: bool,
}

/// `puramp`: how far the set has warmed up, on whatever curve the caller asks
/// for. This is why the picture swells and brightens for the first second or
/// two rather than simply appearing.
fn puramp(it: &AnalogTv, tc: f32, start: f32, over: f32) -> f32 {
    let pt = it.powerup - start;
    if pt < 0.0 {
        return 0.0;
    }
    if pt > 900.0 || pt / tc > 8.0 {
        return 1.0;
    }
    let ret = (1.0 - (-pt / tc).exp()) * over;
    if ret > 1.0 {
        return 1.0;
    }
    ret * ret
}

impl AnalogTv {
    pub fn new(width: i32, height: i32) -> AnalogTv {
        let mut gamma = [0u8; CV_MAX];
        for (i, g) in gamma.iter_mut().enumerate() {
            let intensity = (f64::from(i as u32) / 256.0).powf(0.8) * 65535.0;
            *g = (intensity.min(65535.0) as u32 >> 8) as u8;
        }

        let mut it = AnalogTv {
            image: Fb::new(1, 1),
            usewidth: 0,
            useheight: 0,
            xrepl: 1,
            subwidth: 1,
            screen_xo: 0,
            screen_yo: 0,
            window_width: width,
            window_height: height,
            agclevel: 1.0,
            tint_control: 0.0,
            color_control: 0.0,
            brightness_control: 0.0,
            contrast_control: 0.0,
            height_control: 1.0,
            width_control: 1.0,
            squish_control: 0.0,
            horiz_desync: 0.0,
            squeezebottom: 0.0,
            powerup: 1000.0,
            flutter_horiz_desync: false,
            hashnoise_on: false,
            hashnoise_enable: true,
            shrinkpulse: -1,
            crtload: [0.0; V],
            gamma,
            tint_i: 0.0,
            tint_q: 0.0,
            cur_hsync: 0,
            line_hsync: [0; V],
            cur_vsync: 0,
            cb_phase: [0.0; 4],
            line_cb_phase: vec![[0.0; 4]; V],
            channel_change_cycles: 0,
            rx_signal_level: 0.0,
            rx_signal: vec![0.0; SIGNAL_LEN + 2 * H],
            leveltable: [[Level::default(); MAX_LINEHEIGHT + 1]; MAX_LINEHEIGHT + 1],
            random0: 0,
            random1: 0,
            puheight: 1.0,
            need_clear: true,
        };
        it.configure(width, height);
        it
    }

    /// `analogtv_set_defaults`, less the resource lookups: the caller reads
    /// those, since the prefix differs per hack.
    pub fn set_defaults(&mut self, color: f32, tint: f32, brightness: f32, contrast: f32) {
        self.tint_control = tint;
        self.color_control = color / 100.0;
        self.brightness_control = brightness / 100.0;
        self.contrast_control = contrast / 100.0;
        self.height_control = 1.0;
        self.width_control = 1.0;
        self.squish_control = 0.0;
        self.powerup = 1000.0;
        self.hashnoise_on = false;
        self.hashnoise_enable = true;
        self.horiz_desync = frand(10.0) as f32 - 5.0;
        self.squeezebottom = frand(5.0) as f32 - 1.0;
    }

    pub fn usewidth(&self) -> i32 {
        self.usewidth
    }

    pub fn useheight(&self) -> i32 {
        self.useheight
    }

    /// Pick the size of the picture inside the window.
    ///
    /// If the window is very small, don't let the image get lower than the
    /// actual TV resolution. If its shape is close to 4:3 or 16:9, or is
    /// completely weird, fill it. Otherwise letterbox or pillarbox, but not
    /// both. And if the height is within 2.5% of a multiple of the scan line
    /// count, make it exact, which maps 1024 to 1000.
    pub fn configure(&mut self, width: i32, height: i32) {
        self.window_width = width;
        self.window_height = height;
        let oldwidth = self.usewidth;
        let oldheight = self.useheight;

        let percent = 0.15;
        let min_ratio = 4.0 / 3.0 * (1.0 - percent);
        let max_ratio = 16.0 / 9.0 * (1.0 + percent);
        let crazy_min_ratio = 10.0;
        let crazy_max_ratio = 1.0 / crazy_min_ratio;
        let height_snap = 0.025;

        let mut hlim = height;
        let mut wlim = width;
        let ratio = f64::from(wlim) / f64::from(hlim.max(1));

        if wlim < 266 || hlim < 200 {
            wlim = 266;
            hlim = 200;
        } else if ratio > min_ratio && ratio < max_ratio {
            /* close enough */
        } else if ratio >= max_ratio {
            wlim = (f64::from(hlim) * max_ratio) as i32;
        } else {
            hlim = (f64::from(wlim) / min_ratio) as i32;
        }

        if ratio < crazy_min_ratio || ratio > crazy_max_ratio {
            if ratio < crazy_min_ratio {
                hlim = height;
            } else {
                wlim = width;
            }
        }

        let height_diff = ((hlim + VISLINES as i32 / 2) % VISLINES as i32) - VISLINES as i32 / 2;
        if height_diff != 0 && f64::from(height_diff.abs()) < f64::from(hlim) * height_snap {
            hlim -= height_diff;
        }

        /* Most times this doesn't change */
        if wlim != oldwidth || hlim != oldheight {
            self.usewidth = wlim;
            self.useheight = hlim;
            self.xrepl = (1 + self.usewidth / 640).min(2);
            self.subwidth = self.usewidth / self.xrepl;
            self.image = Fb::new(self.usewidth.max(1), self.useheight.max(1));
        }

        self.screen_xo = (width - self.usewidth) / 2;
        self.screen_yo = (height - self.useheight) / 2;
        self.need_clear = true;
    }

    /// `analogtv_setup_frame`: the once-a-frame wobbles.
    fn setup_frame(&mut self) {
        if self.flutter_horiz_desync {
            /* Horizontal sync during vertical sync instability. */
            self.horiz_desync += -0.10 * (self.horiz_desync - 3.0)
                + ((random() & 0xff) as i32 - 0x80) as f32
                    * ((random() & 0xff) as i32 - 0x80) as f32
                    * ((random() & 0xff) as i32 - 0x80) as f32
                    * 0.000001;
        }

        if self.hashnoise_enable && !self.hashnoise_on && random().is_multiple_of(10000) {
            self.hashnoise_on = true;
            self.shrinkpulse = (random() % V as u32) as i32;
        }
        if random().is_multiple_of(1000) {
            self.hashnoise_on = false;
        }

        if self.rx_signal_level != 0.0 {
            self.agclevel = (1.0 / self.rx_signal_level) as f32;
        }
    }

    /// Fill a stretch of the aerial with noise, which is what is there when
    /// nothing is transmitting.
    fn init_signal(&mut self, noiselevel: f64, start: usize, end: usize) {
        let mut fastrnd = rnd_seek(FASTRND_A, FASTRND_C, self.random0, start as u32);
        let noisemul = (noiselevel * 150.0).sqrt() as f32 / 2147483647.0;
        let mut nm1 = signed(fastrnd) as f32 * noisemul;
        for p in self.rx_signal[start..end].iter_mut() {
            let nm2 = nm1;
            fastrnd = fastrnd.wrapping_mul(FASTRND_A).wrapping_add(FASTRND_C);
            nm1 = signed(fastrnd) as f32 * noisemul;
            *p = nm1 * nm2;
        }
    }

    /// Add one station to what the aerial is hearing, with its ghosts.
    fn add_signal(&mut self, rec: &Reception, inp: &Input, start: usize, end: usize, ec: usize) {
        let level = rec.level as f32;
        let hfloss = rec.hfloss as f32;
        let mut fastrnd = rnd_seek(FASTRND_A, FASTRND_C, self.random1, start as u32);

        let noise_decay = 0.99995f32;
        let mut noise_ampl = 1.3 * noise_decay.powi(start as i32);

        let ec = ec.min(end);
        let mut s = (start + rec.ofs) % SIGNAL_LEN;
        let mut p = start;

        // A big noisy transition, which is what changing channel sounds like.
        // The noise is the same strength whatever the signal is.
        while p < ec {
            let sig0 = f32::from(inp.signal[s]);
            let noise = signed(fastrnd) as f32 * (50.0 / 2147483647.0);
            fastrnd = fastrnd.wrapping_mul(FASTRND_A).wrapping_add(FASTRND_C);
            self.rx_signal[p] += sig0 * level * (1.0 - noise_ampl) + noise * noise_ampl;
            noise_ampl *= noise_decay;
            p += 1;
            s += 1;
            if s >= SIGNAL_LEN {
                s = 0;
            }
        }

        let mut dp = [0.0f32; 5];
        let mut s2 = s;
        for d in dp.iter_mut().skip(1) {
            s2 = (s2 + SIGNAL_LEN - 4) % SIGNAL_LEN;
            *d = (i32::from(inp.signal[s2])
                + i32::from(inp.signal[s2 + 1])
                + i32::from(inp.signal[s2 + 2])
                + i32::from(inp.signal[s2 + 3])) as f32;
        }

        while p < end {
            let sig0 = f32::from(inp.signal[s]);
            let sig1 = f32::from(inp.signal[s + 1]);
            let sig2 = f32::from(inp.signal[s + 2]);
            let sig3 = f32::from(inp.signal[s + 3]);
            dp[0] = sig0 + sig1 + sig2 + sig3;

            // Ghosting, typical of RF monitor cables. This corresponds to a
            // pretty long cable, but looks right.
            let sigr = dp[1] * rec.ghostfir[0] as f32
                + dp[2] * rec.ghostfir[1] as f32
                + dp[3] * rec.ghostfir[2] as f32
                + dp[4] * rec.ghostfir[3] as f32;
            dp[4] = dp[3];
            dp[3] = dp[2];
            dp[2] = dp[1];
            dp[1] = dp[0];

            self.rx_signal[p] += (sig0 + sigr + sig2 * hfloss) * level;
            self.rx_signal[p + 1] += (sig1 + sigr + sig3 * hfloss) * level;
            self.rx_signal[p + 2] += (sig2 + sigr + sig0 * hfloss) * level;
            self.rx_signal[p + 3] += (sig3 + sigr + sig1 * hfloss) * level;

            p += 4;
            s += 4;
            if s >= SIGNAL_LEN {
                s -= SIGNAL_LEN;
            }
        }
    }

    /// Hunt for the sync pulses, vertical first and then one per line, and note
    /// the phase of each line's colour burst.
    ///
    /// This is the sync separator, and doing it by looking at the signal rather
    /// than by knowing where the lines are is what makes a weak or distorted
    /// signal bend the picture instead of merely dirtying it.
    fn sync(&mut self) {
        let mut cur_hsync = self.cur_hsync;
        let mut cur_vsync = self.cur_vsync;
        let cbfc = 1.0f32 / 128.0;

        // C leaves the index one past the end when the loop finds nothing,
        // and that value is used, so the sentinel has to match.
        let mut found = 32i32;
        for i in -32..32i32 {
            let lineno = ((cur_vsync as i32 + i + V as i32) % V as i32) as usize;
            let base = lineno * H;
            let mut filt = 0.0f32;
            let mut j = 0;
            while j < H {
                filt += self.rx_signal[base + j];
                j += H / 16;
            }
            filt *= self.agclevel;
            let osc = (V as i32 + i) as f32 / V as f32;
            if osc >= 1.05 + 0.0002 * filt {
                found = i;
                break;
            }
        }
        cur_vsync = ((cur_vsync as i32 + found + V as i32) % V as i32) as usize;

        for lineno in 0..V {
            if lineno > 5 && lineno < V - 3 {
                /* ignore vsync interval */
                let mut lineno2 = (lineno + cur_vsync + V) % V;
                if lineno2 == 0 {
                    lineno2 = V;
                }
                let base = lineno2 * H + cur_hsync;
                let mut found = 8i32;
                for i in -8..8i32 {
                    let osc = (H as i32 + i) as f32 / H as f32;
                    let at = |k: i32| {
                        let idx = base as i32 + k;
                        if idx < 0 || idx as usize >= self.rx_signal.len() {
                            0.0
                        } else {
                            self.rx_signal[idx as usize]
                        }
                    };
                    let filt = (at(i - 3) + at(i - 2) + at(i - 1) + at(i)) * self.agclevel;
                    if osc >= 1.005 + 0.0001 * filt {
                        found = i;
                        break;
                    }
                }
                cur_hsync = ((cur_hsync as i32 + found + H as i32) % H as i32) as usize;
            }

            self.line_hsync[lineno] = (cur_hsync + PIC_START + H) % H;

            // The colourburst is a few cycles after the sync pulse and is nine
            // cycles long; look at the middle five and remember its phase,
            // because that is the only reference for what the chroma means.
            if lineno > 15 {
                let base = lineno * H + (cur_hsync & !3);
                for i in CB_START + 8..CB_START + 36 - 8 {
                    let s = self.rx_signal[base + i];
                    self.cb_phase[i & 3] =
                        self.cb_phase[i & 3] * (1.0 - cbfc) + s * self.agclevel * cbfc;
                }
            }

            let mut tot = 0.1f32;
            for p in self.cb_phase {
                tot += p * p;
            }
            let cbgain = 32.0 / tot.sqrt();
            for i in 0..4 {
                self.line_cb_phase[lineno][i] = self.cb_phase[i] * cbgain;
            }
        }

        self.cur_hsync = cur_hsync;
        self.cur_vsync = cur_vsync;
    }

    /// Keep the total brightness of a scan line the same however many screen
    /// rows it lands on, which is what stops the picture flickering as the
    /// window is resized.
    fn setup_levels(&mut self, avgheight: f64) {
        const LEVELFAC: [f64; 3] = [-7.5, 5.5, 24.5];
        let pu = f64::from(puramp(self, 3.0, 6.0, 1.0));

        let mut height = 0;
        while (height as f64) < avgheight + 2.0 && height <= MAX_LINEHEIGHT {
            for i in 0..height {
                self.leveltable[height][i].index = 2;
            }
            if avgheight >= 3.0 {
                self.leveltable[height][0].index = 0;
            }
            if avgheight >= 5.0 && height >= 1 {
                self.leveltable[height][height - 1].index = 0;
            }
            if avgheight >= 7.0 {
                self.leveltable[height][1].index = 1;
                if height >= 2 {
                    self.leveltable[height][height - 2].index = 1;
                }
            }
            for i in 0..height {
                let idx = self.leveltable[height][i].index;
                self.leveltable[height][i].value = (40.0 + LEVELFAC[idx] * pu) / 256.0;
            }
            height += 1;
        }
    }

    /// Where on the screen one scan line lands, and where in the signal it
    /// came from.
    fn get_line(&self, lineno: usize) -> Option<(i32, i32, i32, usize)> {
        let slineno = lineno as i32 - TOP as i32;
        let uh = self.useheight;
        let mut ytop = ((f64::from(slineno * uh / VISLINES as i32 - uh / 2))
            * f64::from(self.puheight)) as i32
            + uh / 2;
        let mut ybot = ((f64::from((slineno + 1) * uh / VISLINES as i32 - uh / 2))
            * f64::from(self.puheight)) as i32
            + uh / 2;
        let signal_offset = ((lineno + self.cur_vsync + V) % V) * H + self.line_hsync[lineno];

        if ytop == ybot || ybot < 0 || ytop > uh {
            return None;
        }
        ytop = ytop.max(0);
        ybot = ybot.min(uh);
        if ybot > ytop + MAX_LINEHEIGHT as i32 {
            ybot = ytop + MAX_LINEHEIGHT as i32;
        }
        Some((slineno, ytop, ybot, signal_offset))
    }

    /// Here we model the analog circuitry of an NTSC television.
    ///
    /// It splits the signal into Y, I and Q. Y is luminance, and you get it by
    /// low-pass filtering below 3.57 MHz. I and Q are the in-phase and
    /// quadrature components of the subcarrier, recovered by multiplying by
    /// cosine and sine of it and low-pass filtering. Because the eye has less
    /// resolution in some colours than others, I is filtered at 1.5 MHz and Q
    /// at 0.5 MHz, which is why chroma smears sideways and luma does not.
    ///
    /// The filters are infinite impulse response ones from the mkfilter script
    /// at York, written out longhand.
    fn ntsc_to_yiq(
        &self,
        lineno: usize,
        sig_off: usize,
        start: usize,
        end: usize,
        out: &mut [Yiq],
    ) {
        const MAXDELAY: usize = 32;
        let phasecorr = sig_off & 3;
        let agclevel = self.agclevel;
        let brightadd = self.brightness_control * 100.0 - BLACK_LEVEL as f32;

        let cb = &self.line_cb_phase[lineno];
        let cb_i = f64::from(cb[(2 + phasecorr) & 3] - cb[phasecorr & 3]) / 16.0;
        let cb_q = f64::from(cb[(3 + phasecorr) & 3] - cb[(1 + phasecorr) & 3]) / 16.0;

        // No burst worth speaking of means no colour: this is exactly how a set
        // showed a text-mode computer in black and white.
        let colormode = cb_i * cb_i + cb_q * cb_q > 2.8;
        let mut multiq2 = [0.0f32; 4];
        if colormode {
            multiq2[0] = ((cb_i * f64::from(self.tint_i) - cb_q * f64::from(self.tint_q))
                * f64::from(self.color_control)) as f32;
            multiq2[1] = ((cb_q * f64::from(self.tint_i) + cb_i * f64::from(self.tint_q))
                * f64::from(self.color_control)) as f32;
            multiq2[2] = -multiq2[0];
            multiq2[3] = -multiq2[1];
        }

        // The delay line the filters run over, indexed backwards as they go.
        let mut delay = vec![0.0f32; MAXDELAY + PIC_LEN + 32];
        let base = PIC_LEN;

        /* Filter Y with a 4-pole low-pass Butterworth filter at 3.5 MHz. */
        for (n, i) in (start..end).enumerate() {
            let d = base - n;
            let sp = self.rx_signal[sig_off + i];
            delay[d] = sp * 0.046_990_424 * agclevel;
            delay[d + 8] = 1.0 * (delay[d + 6] + delay[d])
                + 4.0 * (delay[d + 5] + delay[d + 1])
                + 7.0 * (delay[d + 4] + delay[d + 2])
                + 8.0 * delay[d + 3]
                - 0.017_664_8 * delay[d + 12]
                - 0.486_028_8 * delay[d + 10];
            out[i].y = delay[d + 8] + brightadd;
        }

        if !colormode {
            for o in out[start..end].iter_mut() {
                o.i = 0.0;
                o.q = 0.0;
            }
            return;
        }

        /* Filter I and Q with a 3-pole low-pass Butterworth filter at 1.5 MHz. */
        let mut delay = vec![0.0f32; MAXDELAY + PIC_LEN + 32];
        for (n, i) in (start..end).enumerate() {
            let d = base - n;
            let sig = self.rx_signal[sig_off + i];

            delay[d] = sig * multiq2[i & 3] * 0.083_333_336;
            delay[d + 8] = delay[d + 5]
                + delay[d]
                + 3.0 * (delay[d + 4] + delay[d + 1])
                + 4.0 * (delay[d + 3] + delay[d + 2])
                - 0.333_333_34 * delay[d + 10];
            out[i].i = delay[d + 8];

            delay[d + 16] = sig * multiq2[(i + 3) & 3] * 0.083_333_336;
            delay[d + 24] = delay[d + 16 + 5]
                + delay[d + 16]
                + 3.0 * (delay[d + 16 + 4] + delay[d + 16 + 1])
                + 4.0 * (delay[d + 16 + 3] + delay[d + 16 + 2])
                - 0.333_333_34 * delay[d + 24 + 2];
            out[i].q = delay[d + 24];
        }
    }

    /// Write one scan line's worth of RGB into however many rows of the picture
    /// it covers, at the brightness the level table says each row gets.
    fn blast_imagerow(&mut self, rgbf: &[f32], ytop: i32, ybot: i32) {
        let lineheight = ((ybot - ytop) as usize).min(MAX_LINEHEIGHT);
        let xrepl = self.xrepl;
        for y in ytop..ybot {
            let line = (y - ytop) as usize;
            let levelmult = self.leveltable[lineheight][line].value as f32;
            let mut x = 0;
            for rpf in rgbf.chunks_exact(3) {
                let r = (rpf[0] * levelmult) as usize;
                let g = (rpf[1] * levelmult) as usize;
                let b = (rpf[2] * levelmult) as usize;
                let pix = rgb(
                    self.gamma[r.min(CV_MAX - 1)],
                    self.gamma[g.min(CV_MAX - 1)],
                    self.gamma[b.min(CV_MAX - 1)],
                );
                for _ in 0..xrepl {
                    self.image.put_pixel(x, y, pix);
                    x += 1;
                }
            }
        }
    }

    /// Draw one frame: sum the stations onto the aerial, find the sync, and
    /// demodulate every visible line onto the screen.
    pub fn draw(&mut self, window: &mut Fb, noiselevel: f64, recs: &[(&Reception, &Input)]) {
        if self.usewidth <= 0 || self.useheight <= 0 {
            return;
        }

        self.rx_signal_level = noiselevel;
        for (rec, _) in recs {
            let level = rec.level;
            self.rx_signal_level = (self.rx_signal_level * self.rx_signal_level
                + level
                    * level
                    * (1.0
                        + 4.0
                            * (rec.ghostfir[0]
                                + rec.ghostfir[1]
                                + rec.ghostfir[2]
                                + rec.ghostfir[3])))
                .sqrt();
        }

        self.setup_frame();

        self.random0 = random();
        self.random1 = random();

        // Noise, then every station on top of it.
        self.init_signal(noiselevel, 0, SIGNAL_LEN);
        for (i, (rec, inp)) in recs.iter().enumerate() {
            let ec = if i == 0 {
                self.channel_change_cycles
            } else {
                0
            };
            self.add_signal(rec, inp, 0, SIGNAL_LEN, ec);
        }
        self.channel_change_cycles = 0;

        // Two lines of the start copied to the end, so a line can be read past
        // the bottom without worrying about the wrap.
        for i in 0..2 * H {
            self.rx_signal[SIGNAL_LEN + i] = self.rx_signal[i];
        }

        self.sync();

        let baseload = 0.5f32;
        self.crtload[TOP - 1] = baseload;
        self.puheight = puramp(self, 2.0, 1.0, 1.3)
            * self.height_control
            * (1.125 - 0.125 * puramp(self, 2.0, 2.0, 1.1));

        let avg = f64::from(self.puheight) * f64::from(self.useheight) / VISLINES as f64;
        self.setup_levels(avg);

        // The tint control is a phase shift of the chroma reference, which is
        // all a tint knob ever was.
        self.tint_i =
            -((103.0 + f64::from(self.tint_control)) * std::f64::consts::PI / 180.0).cos() as f32;
        self.tint_q =
            ((103.0 + f64::from(self.tint_control)) * std::f64::consts::PI / 180.0).sin() as f32;

        // How hard the beam is working, line by line. A bright line loads the
        // horizontal oscillator, and the oscillator also made the high voltage,
        // so brightness and width were never independent.
        let mut baseload = baseload;
        for lineno in TOP..BOT {
            let Some((slineno, _, _, signal_offset)) = self.get_line(lineno) else {
                continue;
            };
            if lineno as i32 == self.shrinkpulse {
                baseload += 0.4;
                self.shrinkpulse = -1;
            }
            let mut totsignal = 0.0f32;
            for i in 0..PIC_LEN {
                totsignal += self.rx_signal[signal_offset + i];
            }
            totsignal *= self.agclevel;
            let ncl = 0.95 * self.crtload[lineno - 1]
                + 0.05
                    * (baseload
                        + (totsignal - 30000.0) / 100000.0
                        + if slineno > 184 {
                            (slineno - 184) as f32
                                * (lineno as i32 - 184) as f32
                                * 0.001
                                * self.squeezebottom
                        } else {
                            0.0
                        });
            self.crtload[lineno] = ncl;
        }

        self.draw_lines();

        if self.need_clear {
            window.clear(rgb(0, 0, 0));
            self.need_clear = false;
        }

        let mut overall_top =
            (f32::from(self.useheight as i16) * (1.0 - self.puheight) / 2.0) as i32;
        let mut overall_bot =
            (f32::from(self.useheight as i16) * (1.0 + self.puheight) / 2.0) as i32;
        overall_top = overall_top.max(0);
        overall_bot = overall_bot.min(self.useheight);

        if overall_top > 0 {
            window.clear_area(
                rgb(0, 0, 0),
                self.screen_xo,
                self.screen_yo,
                self.usewidth,
                overall_top,
            );
        }
        if self.useheight > overall_bot {
            window.clear_area(
                rgb(0, 0, 0),
                self.screen_xo,
                self.screen_yo + overall_bot,
                self.usewidth,
                self.useheight - overall_bot,
            );
        }
        if overall_bot > overall_top {
            for y in overall_top..overall_bot {
                for x in 0..self.usewidth {
                    window.put_pixel(
                        self.screen_xo + x,
                        self.screen_yo + y,
                        self.image.get_pixel(x, y),
                    );
                }
            }
        }
    }

    /// Every visible scan line, demodulated and drawn.
    fn draw_lines(&mut self) {
        let mut yiq = vec![Yiq::default(); PIC_LEN + 10];
        let mut raw_rgb = vec![0.0f32; (self.subwidth * 3) as usize];

        for lineno in TOP..BOT {
            let Some((slineno, ytop, ybot, signal_offset)) = self.get_line(lineno) else {
                continue;
            };

            // Bloom: more brightness means more load on the oscillator, which
            // means less horizontal deflection, so a bright line is wider.
            let mut bloomthisrow = -10.0 * self.crtload[lineno];
            bloomthisrow = bloomthisrow.clamp(-10.0, 2.0);
            // The set went out of sync during the vertical retrace, so the top
            // few lines are bent sideways.
            let shiftthisrow = if slineno < 16 {
                self.horiz_desync
                    * ((-0.17 * slineno as f32).exp() * (0.7 + (slineno as f32 * 0.6).cos()))
            } else {
                0.0
            };

            let viswidth = PIC_LEN as f32 * 0.79 - 5.0 * bloomthisrow;
            let middle = PIC_LEN as f32 / 2.0 - shiftthisrow;
            let scanwidth = self.width_control * puramp(self, 0.5, 0.3, 1.0);

            let mut scw = (self.subwidth as f32 * scanwidth) as i32;
            if scw > self.subwidth {
                scw = self.usewidth;
            }
            let scl = self.subwidth / 2 - scw / 2;
            let scr = self.subwidth / 2 + scw / 2;

            let pixrate = ((viswidth * 65536.0) / self.subwidth as f32 / scanwidth) as i32;
            let scanstart_i = ((middle - viswidth * 0.5) * 65536.0) as i32;
            let scanend_i = (PIC_LEN as i32 - 1) * 65536;
            // The beam slows at the right of the screen, which squashes and
            // brightens what is there.
            let squishright_i = ((middle
                + viswidth * (0.25 + 0.25 * puramp(self, 2.0, 0.0, 1.1) - self.squish_control))
                * 65536.0) as i32;
            let squishdiv = (self.subwidth / 15).max(1);

            let start = ((scanstart_i >> 16) - 10).max(0) as usize;
            let end = (((scanend_i >> 16) + 10) as usize).min(PIC_LEN + 9);
            self.ntsc_to_yiq(lineno, signal_offset, start, end, &mut yiq);

            let mut pixbright = self.contrast_control * puramp(self, 1.0, 0.0, 1.0)
                / (0.5 + 0.5 * self.puheight)
                * 1024.0
                / 100.0;
            let mut pixmultinc = pixrate;
            let mut i = scanstart_i;

            for v in raw_rgb.iter_mut() {
                *v = 0.0;
            }
            let mut rrp = (scl * 3) as usize;
            let rgb_end = (scr * 3) as usize;

            while i < 0 && rrp != rgb_end {
                raw_rgb[rrp] = 0.0;
                raw_rgb[rrp + 1] = 0.0;
                raw_rgb[rrp + 2] = 0.0;
                i += pixmultinc;
                rrp += 3;
            }
            while i < scanend_i && rrp != rgb_end {
                let pixfrac = (i & 0xffff) as f32 / 65536.0;
                let invpixfrac = 1.0 - pixfrac;
                let pati = (i >> 16) as usize;

                let interpy = yiq[pati].y * invpixfrac + yiq[pati + 1].y * pixfrac;
                let interpi = yiq[pati].i * invpixfrac + yiq[pati + 1].i * pixfrac;
                let interpq = yiq[pati].q * invpixfrac + yiq[pati + 1].q * pixfrac;

                // The inverse of the NTSC encoding matrix, which is what a set
                // implements with a handful of resistors.
                let r = (interpy + 0.948 * interpi + 0.624 * interpq) * pixbright;
                let g = (interpy - 0.276 * interpi - 0.639 * interpq) * pixbright;
                let b = (interpy - 1.105 * interpi + 1.729 * interpq) * pixbright;
                raw_rgb[rrp] = r.max(0.0);
                raw_rgb[rrp + 1] = g.max(0.0);
                raw_rgb[rrp + 2] = b.max(0.0);

                if i >= squishright_i {
                    pixmultinc += pixmultinc / squishdiv;
                    pixbright += pixbright / squishdiv as f32 / 2.0;
                }
                i += pixmultinc;
                rrp += 3;
            }
            while rrp != rgb_end {
                raw_rgb[rrp] = 0.0;
                raw_rgb[rrp + 1] = 0.0;
                raw_rgb[rrp + 2] = 0.0;
                rrp += 3;
            }

            let row = std::mem::take(&mut raw_rgb);
            self.blast_imagerow(&row, ytop, ybot);
            raw_rgb = row;
        }
    }

    /// `analogtv_load_ximage`: take a picture and encode it as a camera would,
    /// including all the bandlimiting and the YIQ modulation.
    ///
    /// This is the other half of the trick. Everything a hack draws goes on to
    /// the wire first, so the receiver above has something real to make a mess
    /// of.
    pub fn load_ximage(
        &self,
        input: &mut Input,
        pic: &XImage,
        mask: Option<&XImage>,
        xoff: i32,
        yoff: i32,
        target_w: i32,
        target_h: i32,
    ) {
        let mut x_length = PIC_LEN as i32;
        let y_overscan = 5; /* overscan this much top and bottom */
        let mut y_scanlength = VISLINES as i32 + 2 * y_overscan;

        if target_w > 0 {
            x_length = x_length * target_w / self.window_width.max(1);
        }
        if target_h > 0 {
            y_scanlength = y_scanlength * target_h / self.window_height.max(1);
        }
        let x_length = x_length.max(1);
        let y_scanlength = y_scanlength.max(1);

        let img_w = pic.width();
        let img_h = pic.height();
        let xoff = PIC_LEN as i32 * xoff / self.window_width.max(1);
        let yoff = VISLINES as i32 * yoff / self.window_height.max(1);

        let mut multiq = vec![0i32; (x_length + 4) as usize];
        for (i, m) in multiq.iter_mut().enumerate() {
            let phase = 90.0 - 90.0 * i as f64;
            *m = (-(std::f64::consts::PI / 180.0 * (phase - 303.0)).cos() * 4096.0) as i32;
        }

        for y in 0..y_scanlength {
            let picy1 = y * img_h / y_scanlength;
            let picy2 = (y * img_h + y_scanlength / 2) / y_scanlength;

            let mut fyx = [0i32; 7];
            let mut fyy = [0i32; 7];
            let mut fix = [0i32; 4];
            let mut fiy = [0i32; 4];
            let mut fqx = [0i32; 4];
            let mut fqy = [0i32; 4];

            let sy = y - y_overscan + TOP as i32 + yoff;
            if sy < 0 || sy >= V as i32 {
                continue;
            }

            for x in 0..x_length {
                let picx = x * img_w / x_length;
                if let Some(m) = mask
                    && m.get_pixel(picx, picy1) == rgb(0, 0, 0)
                {
                    continue;
                }
                let (r1, g1, b1) = chan16(pic.get_pixel(picx, picy1));
                let (r2, g2, b2) = chan16(pic.get_pixel(picx, picy2));

                // The NTSC encoding matrix, with the coefficients in .4 format.
                let rawy = (5 * r1 + 11 * g1 + 2 * b1 + 5 * r2 + 11 * g2 + 2 * b2) >> 7;
                let rawi = (10 * r1 - 4 * g1 - 5 * b1 + 10 * r2 - 4 * g2 - 5 * b2) >> 7;
                let rawq = (3 * r1 - 8 * g1 + 5 * b1 + 3 * r2 - 8 * g2 + 5 * b2) >> 7;

                /* Filter y with a 4-pole low-pass Butterworth filter at 3.5 MHz */
                fyx.rotate_left(1);
                fyx[6] = (rawy * 1897) >> 16;
                fyy.rotate_left(1);
                fyy[6] = (fyx[0] + fyx[6])
                    + 4 * (fyx[1] + fyx[5])
                    + 7 * (fyx[2] + fyx[4])
                    + 8 * fyx[3]
                    + ((-151 * fyy[2] + 8115 * fyy[3] - 38312 * fyy[4] + 36586 * fyy[5]) >> 16);
                let filt_y = fyy[6];

                /* Filter I at 1.5 MHz. 3 pole Butterworth. */
                fix.rotate_left(1);
                fix[3] = (rawi * 1413) >> 16;
                fiy.rotate_left(1);
                fiy[3] = (fix[0] + fix[3])
                    + 3 * (fix[1] + fix[2])
                    + ((16559 * fiy[0] - 72008 * fiy[1] + 109682 * fiy[2]) >> 16);
                let filt_i = fiy[3];

                /* Filter Q at 0.5 MHz. 3 pole Butterworth. */
                fqx.rotate_left(1);
                fqx[3] = (rawq * 75) >> 16;
                fqy.rotate_left(1);
                fqy[3] = (fqx[0] + fqx[3])
                    + 3 * (fqx[1] + fqx[2])
                    + ((2612 * fqy[0] - 9007 * fqy[1] + 10453 * fqy[2]) >> 12);
                let filt_q = fqy[3];

                // Luma plus chroma on the subcarrier: one wire, both signals.
                let mut composite = filt_y
                    + ((multiq[x as usize] * filt_i + multiq[x as usize + 3] * filt_q) >> 12);
                composite = ((composite * 100) >> 14) + BLACK_LEVEL;
                composite = composite.clamp(0, 125);

                let sx = x + PIC_START as i32 + xoff;
                if sx >= 0 && sx < H as i32 {
                    input.set(sy as usize, sx as usize, composite as i8);
                }
            }
        }
    }
}

/// A pixel's channels as the sixteen-bit values `XQueryColor` would have given.
fn chan16(p: Pixel) -> (i32, i32, i32) {
    let (r, g, b) = super::color::unrgb(p);
    (i32::from(r) * 257, i32::from(g) * 257, i32::from(b) * 257)
}

/// The generator's value reinterpreted as signed, which is what upstream's
/// arithmetic on it comes to.
fn signed(v: u32) -> i32 {
    v.wrapping_sub(0x7fffffff) as i32
}

/// Jump the linear congruential generator forward without running it, so a
/// stretch of noise can be filled starting anywhere.
fn rnd_seek(a: u32, c: u32, rnd: u32, dist: u32) -> u32 {
    let (mut a, mut c) = (a, c);
    let (mut a1, mut c1) = (a, c);
    a = 1;
    c = 0;
    let mut dist = dist;
    while dist != 0 {
        if dist & 1 != 0 {
            let na = a.wrapping_mul(a1);
            let nc = c1.wrapping_add(a1.wrapping_mul(c));
            a = na;
            c = nc;
        }
        dist >>= 1;
        let na = a1.wrapping_mul(a1);
        let nc = c1.wrapping_add(a1.wrapping_mul(c1));
        a1 = na;
        c1 = nc;
    }
    a.wrapping_mul(rnd).wrapping_add(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one arithmetic identity that the whole colour system rests on: four
    /// samples of a cycle whose average is the luma and whose amplitude is the
    /// chroma. If this were wrong every colour would be.
    #[test]
    fn a_colour_is_four_samples_of_one_cycle() {
        // No chroma is a flat line at the luma level, which is grey.
        assert_eq!(lcp_to_ntsc(50.0, 0.0, 0.0), [50, 50, 50, 50]);
        // Chroma at phase zero peaks on the first sample and troughs on the
        // third, and the pair either side sit at the luma.
        let n = lcp_to_ntsc(50.0, 20.0, 0.0);
        assert_eq!(n[0], 70);
        assert_eq!(n[2], 30);
        // The two quarter-turn samples want to sit at the luma, and one of
        // them comes out a whole level below it: the cosine of 270 degrees is
        // a hair negative rather than zero, and the cast truncates towards
        // zero rather than rounding. Upstream's cast does the same, so this is
        // recorded rather than corrected.
        assert_eq!(n[1], 50);
        assert_eq!(n[3], 49);
        // Turning the phase a quarter turn moves the peak along one sample,
        // which is what makes phase mean hue.
        let n = lcp_to_ntsc(50.0, 20.0, 90.0);
        assert_eq!(n[3], 70);
        assert_eq!(n[1], 30);
    }

    /// Sync has to leave the picture area at black and put the pulses where the
    /// standard says, or the receiver will not find the lines.
    #[test]
    fn sync_lands_where_the_standard_says() {
        let mut inp = Input::new();
        inp.setup_sync(true, false);

        // An ordinary line: sync low, then blanking, then black across the
        // picture.
        let line = 100;
        assert_eq!(i32::from(inp.at(line, SYNC_START)), SYNC_LEVEL);
        assert_eq!(i32::from(inp.at(line, BP_START - 1)), SYNC_LEVEL);
        assert_eq!(i32::from(inp.at(line, BP_START)), BLANK_LEVEL);
        assert_eq!(i32::from(inp.at(line, PIC_START)), BLACK_LEVEL);
        assert_eq!(i32::from(inp.at(line, PIC_END - 1)), BLACK_LEVEL);
        assert_eq!(i32::from(inp.at(line, FP_START)), BLANK_LEVEL);

        // The colour burst rides on the back porch, one cycle in four.
        assert_eq!(
            i32::from(inp.at(line, CB_START + 1)),
            BLANK_LEVEL + CB_LEVEL
        );
        assert_eq!(
            i32::from(inp.at(line, CB_START + 3)),
            BLANK_LEVEL - CB_LEVEL
        );

        // And in the vertical interval the polarity is the other way up.
        assert_eq!(i32::from(inp.at(4, SYNC_START)), BLANK_LEVEL);
        assert_eq!(i32::from(inp.at(4, H - 1)), SYNC_LEVEL);
    }

    /// The whole chain, end to end: paint a picture, put it on the air, and
    /// see what a set makes of it. A saturated colour has to survive being
    /// modulated onto a subcarrier and demodulated off it again, because that
    /// round trip is the entire point of this module and nothing else here
    /// tests the two halves against each other.
    #[test]
    fn a_colour_survives_being_broadcast() {
        let (w, h) = (320, 240);
        let mut tv = AnalogTv::new(w, h);
        tv.set_defaults(70.0, 5.0, 3.0, 150.0);
        // Skip the warm-up, which would otherwise dim the first frames.
        tv.powerup = 900.0;

        let mut inp = Input::new();
        inp.setup_sync(true, false);

        let rec = Reception {
            level: 1.0,
            ..Reception::default()
        };

        let mut window = Fb::new(w, h);
        let sample = |window: &Fb| {
            // Average a patch in the middle, where the picture certainly is.
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            let mut n = 0u32;
            for y in h / 2 - 20..h / 2 + 20 {
                for x in w / 2 - 40..w / 2 + 40 {
                    let (pr, pg, pb) = super::super::color::unrgb(window.get_pixel(x, y));
                    r += u32::from(pr);
                    g += u32::from(pg);
                    b += u32::from(pb);
                    n += 1;
                }
            }
            (r / n, g / n, b / n)
        };

        // A field of saturated red, with no noise so nothing is in doubt.
        let mut pic = Fb::new(64, 64);
        pic.clear(rgb(255, 0, 0));
        tv.load_ximage(&mut inp, &pic, None, 0, 0, 0, 0);
        // Twice: the receiver's burst tracking is a running average, so the
        // first frame is still finding the colour.
        tv.draw(&mut window, 0.0, &[(&rec, &inp)]);
        tv.draw(&mut window, 0.0, &[(&rec, &inp)]);
        let (r, g, b) = sample(&window);
        assert!(
            r > g + 20 && r > b + 20,
            "red came back as ({r},{g},{b}), which is not red"
        );

        // And blue, to prove the phase means what it should rather than that
        // one channel is simply louder.
        let mut pic = Fb::new(64, 64);
        pic.clear(rgb(0, 0, 255));
        tv.load_ximage(&mut inp, &pic, None, 0, 0, 0, 0);
        tv.draw(&mut window, 0.0, &[(&rec, &inp)]);
        tv.draw(&mut window, 0.0, &[(&rec, &inp)]);
        let (r, g, b) = sample(&window);
        assert!(
            b > r + 20 && b > g + 20,
            "blue came back as ({r},{g},{b}), which is not blue"
        );
    }

    /// Without a burst the demodulator must find no colour at all, which is how
    /// a computer in text mode came out grey.
    #[test]
    fn no_burst_means_no_colour() {
        let with = {
            let mut i = Input::new();
            i.setup_sync(true, false);
            i
        };
        let without = {
            let mut i = Input::new();
            i.setup_sync(false, false);
            i
        };
        let burst = |inp: &Input| {
            (CB_START..CB_START + 36)
                .map(|x| i32::from(inp.at(100, x)) - BLANK_LEVEL)
                .map(i32::abs)
                .sum::<i32>()
        };
        assert!(burst(&with) > 0);
        assert_eq!(burst(&without), 0);
    }
}
