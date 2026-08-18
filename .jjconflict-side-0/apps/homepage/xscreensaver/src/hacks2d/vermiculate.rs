//! Port of `hacks/vermiculate.c`.
//!
//! ```text
//!  @(#) vermiculate.c
//!  @(#) Copyright (C) 2001 Tyler Pierce (tyler@alumni.brown.edu)
//!  The full program, with documentation, is available at:
//!    http://freshmeat.net/projects/fdm
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
//! Worms crawl one pixel per step. Each holds a heading in whole degrees and
//! leaves a trail in its own colour, and each remembers the last few hundred
//! pixels it stood on. When that ring buffer wraps the worm rubs out the oldest
//! entry, so what slides across the screen is a body of fixed length rather
//! than a line that only grows.
//!
//! The screen is also the collision detector. Every pixel drawn is recorded in
//! a shadow buffer of colour indices, and a worm reads back the pixel it just
//! stepped onto. Anything nonzero is already occupied: another worm's body, a
//! wall of the grid, the border. What happens next depends on the flags that
//! worm is carrying. It can turn a random right angle, reverse, refuse to move,
//! or, if the obstacle is a wall, reflect off it properly by working out from
//! two more probes whether the wall it hit runs vertically or horizontally. One
//! flag lets a worm eat walls instead: it rubs out the whole segment it ran
//! into, so a screen that starts as a lattice gets chewed open.
//!
//! Seven steering rules are where the shapes come from. A worm jitters within a
//! few degrees; or snaps its heading to some slice of the circle, optionally
//! locking to the axes or the diagonals; or turns by a constant every step,
//! which is a circle; or turns by an amount that drifts and reverses, which is
//! a spiral; or holds a turn until a counter runs out and then reverses it,
//! which is a coil; or alternates turning and going straight, which is arcs
//! joined by straight runs; or plays a fixed sequence of turns on a loop, which
//! is what draws the repeating motifs. A worm can also be handed another worm
//! to chase, by heading, by the tip of its tail, or by its own tail, which is
//! how the strings of followers get formed.
//!
//! None of that is configured in code. The hack embeds a small keyboard
//! language and ships ten canned programs written in it, and picks one at
//! random at startup; the letters really were keystrokes in the author's
//! original interactive program, which is why the parser is written as a
//! keyboard reader against a string. `AEBMN222222223#CAR9CAD4CAOV`, the first
//! of the ten, reads as: autopalette on, erasing off, border off, make nine
//! worms in steering mode two, then three times over select every worm and set
//! its turn rate, its slice to a quarter circle, and its orientation to
//! vertical. `C` and its friends each open a selection, which can be a list of
//! worm numbers, all of them, all worms in a given mode, or the complement of
//! what was just selected, and `#` ends whatever is open.
//!
//! Four departures from the C, none of them visible in a default run.
//!
//! Upstream reads its `speed` knob and then throws it away: whenever no program
//! was supplied on the command line, which is the default, it overwrites the
//! knob with the speed the chosen program was written for. So the slider the
//! config XML advertises does nothing at all. Here it still defers to the
//! program's own speed, but a value the panel has actually set wins, which is
//! what the slider claims to do.
//!
//! Upstream takes a program as a command-line string. A free-text box is the
//! one control the panel here has no widget for, so the ten built-in programs
//! are offered by name instead, with random as the default.
//!
//! The tail colours are the worm colours shifted up by the maximum worm count,
//! and with the maximum number of worms on screen the topmost of them indexes
//! one past the end of upstream's palette. The palette here is one entry longer
//! so that colour exists.
//!
//! Upstream also re-rolls the program after a tick count, except that its tick
//! counter is a local that is zeroed on entry to every frame and its inner loop
//! is capped well below the threshold, so the branch cannot be reached. One
//! program therefore runs for the life of the saver, which is what upstream
//! does too.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{BLACK, Pixel, WHITE, make_random_colormap};
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, SelectItem, StartArgs, XEvent, random,
    screenhack_event_helper,
};

const DEGS: i32 = 360;
const DEGS2: i32 = DEGS / 2;
const DEGS4: i32 = DEGS / 4;
const DEGS8: i32 = DEGS / 8;
const DTOR: f64 = 0.0174532925;
/// The most worms that can ever exist.
const THRMAX: i32 = 120;
/// Colour indices: black, one per worm, then one per worm's tail.
const TAILMAX: usize = THRMAX as usize * 2 + 1;
/// The highest steering mode a program can name.
const TMODES: i32 = 7;
/// The longest body, and so the size of the position ring buffer.
const RLMAX: i32 = 200;
/// The longest turn sequence steering mode seven can be given.
const TSMAX: i32 = 50;
const SPEEDINC: i32 = 10;
const SPEEDMAX: i32 = 1000;

/// The ten programs upstream ships, each with the step rate it was written for.
const SAMPLE_PROGRAMS: &[(&str, i32)] = &[
    ("AEBMN222222223#CAR9CAD4CAOV", 150),
    ("mn333#c23#f1#]]]]]]]]]]]3bc9#r9#c78#f9#ma4#", 600),
    ("AEBMN22222#CAD4CAORc1#f2#c1#r6", 100),
    ("aebmnrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrr#", 500),
    ("mn6rrrrrrrrrrrrrrr#by1i#lcalc1#fnyav", 200),
    (
        "mn1rrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrr#by1i#lcalc1#fn",
        2000,
    ),
    ("baeMn333333333333333333333333#CerrYerCal", 800),
    (
        "baeMn1111111111111111111111111111111111111111111111111111111111#Cer9YesYevYerCal",
        1200,
    ),
    (
        "baMn111111222222333333444444555555#Ct1#lCt2#lCt3#lCt4#lCt5#lCerrYerYet",
        1400,
    ),
    (
        "baMn111111222222333333444444555555#Ct1#lCt2#lCt3#lCt4#lCt5#lCerrYerYetYt1i#lYt1i#sYt1#v",
        1400,
    ),
];

/// `random1`: upstream's `ya_random () % i`. Nothing asks for a zero modulus in
/// a reachable path, and C would divide by zero if anything did.
fn random1(i: i32) -> i32 {
    if i <= 0 {
        return 0;
    }
    (random() % i as u32) as i32
}

/// `wraparound`: one step of wrapping, not a modulus. Every caller has already
/// kept the value within one range of the interval.
fn wrap_i(v: i32, lower: i32, upper: i32) -> i32 {
    if v >= upper {
        v - (upper - lower)
    } else if v < lower {
        v + (upper - lower)
    } else {
        v
    }
}

fn wrap_f(v: f64, lower: f64, upper: f64) -> f64 {
    if v >= upper {
        v - (upper - lower)
    } else if v < lower {
        v + (upper - lower)
    } else {
        v
    }
}

/// `bankmod`: how a program's `T`, `Y` and `N` prefixes act on a flag.
fn bankmod(boolop: u8, flag: &mut bool) {
    match boolop {
        b'T' => *flag = !*flag,
        b'Y' => *flag = true,
        b'N' => *flag = false,
        _ => {}
    }
}

/// `linedata`: one worm.
#[derive(Clone)]
struct Line {
    deg: i32,
    spiturn: i32,
    turnco: i32,
    turnsize: i32,
    col: u8,
    dead: bool,

    orichar: u8,
    x: f64,
    y: f64,
    tmode: i32,
    tsc: i32,
    tslen: i32,
    tclim: i32,
    otslen: i32,
    ctinc: i32,
    reclen: i32,
    recpos: i32,
    circturn: i32,
    prey: i32,
    slice: i32,
    xrec: [i32; RLMAX as usize + 1],
    yrec: [i32; RLMAX as usize + 1],
    turnseq: [i32; TSMAX as usize],
    filled: bool,
    killwalls: bool,
    vhfollow: bool,
    selfbounce: bool,
    tailfollow: bool,
    realbounce: bool,
    little: bool,
}

impl Default for Line {
    fn default() -> Self {
        Self {
            deg: 0,
            spiturn: 0,
            turnco: 0,
            turnsize: 0,
            col: 0,
            dead: false,
            orichar: 0,
            x: 0.0,
            y: 0.0,
            tmode: 0,
            tsc: 0,
            tslen: 0,
            tclim: 0,
            otslen: 0,
            ctinc: 0,
            reclen: 0,
            recpos: 0,
            circturn: 0,
            prey: 0,
            slice: 0,
            xrec: [0; RLMAX as usize + 1],
            yrec: [0; RLMAX as usize + 1],
            turnseq: [0; TSMAX as usize],
            filled: false,
            killwalls: false,
            vhfollow: false,
            selfbounce: false,
            tailfollow: false,
            realbounce: false,
            little: false,
        }
    }
}

/// The window and the shadow buffer that mirrors it, which is what the worms
/// read to find out what they have run into.
struct Screen {
    wid: i32,
    hei: i32,
    /// How big a "pixel" is. Upstream fattens it on a Retina display.
    pscale: i32,
    point: Vec<u8>,
    colors: [Pixel; TAILMAX + 1],
    gc: Gc,
}

impl Screen {
    /// `sp`: set a point, in the window and in the shadow buffer.
    fn sp(&mut self, d: &mut Dpy, x: i32, y: i32, c: u8) {
        self.gc.set_foreground(self.colors[c as usize]);
        d.win()
            .fill_rectangle(&self.gc, x, y, self.pscale, self.pscale);
        self.point[(self.wid * y + x) as usize] = c;
    }

    /// `gp`: get a point.
    fn gp(&self, x: i32, y: i32) -> u8 {
        self.point[(self.wid * y + x) as usize]
    }

    /// `redraw`: put the shadow buffer back on screen, which is how a new
    /// palette reaches the pixels that are already drawn.
    fn redraw(&mut self, d: &mut Dpy, x: i32, y: i32, width: i32, height: i32) {
        for xc in x..x + width {
            for yc in y..y + height {
                let c = self.point[(self.wid * yc + xc) as usize];
                if c != 0 {
                    self.sp(d, xc, yc, c);
                }
            }
        }
    }

    fn clearscreen(&mut self, d: &mut Dpy) {
        d.clear_window();
        self.point.fill(0);
    }
}

struct Vermiculate {
    s: Screen,
    thread: Vec<Line>,
    /// The worms the program has currently selected.
    bank: Vec<u8>,
    boxw: i32,
    boxh: i32,
    curviness: i32,
    gridden: i32,
    ogd: i32,
    bordcorn: i32,
    bordcol: u8,
    threads: i32,
    /// The last character read from the program.
    ch: u8,
    speed: i32,
    erasing: bool,
    autopal: bool,
    reset_p: bool,
    cyc: i32,
    sinof: [f64; DEGS as usize],
    cosof: [f64; DEGS as usize],
    tanof: [f64; DEGS as usize],
    /// The program, and how far through it we are.
    prog: Vec<u8>,
    pos: usize,
}

impl Vermiculate {
    fn new(d: &mut Dpy) -> Self {
        let wid = d.width();
        let hei = d.height();
        let pscale = if wid > 2560 || hei > 2560 { 3 } else { 1 };

        // Pick the program before anything else reads the resources, so the
        // speed it was written for is in hand.
        let choice = d.res.string("program").to_string();
        let n = match choice.parse::<usize>() {
            Ok(n) if (1..=SAMPLE_PROGRAMS.len()).contains(&n) => n - 1,
            _ => random1(SAMPLE_PROGRAMS.len() as i32) as usize,
        };
        let (prog, prog_speed) = SAMPLE_PROGRAMS[n];
        let speed = if d.res.is_overridden("speed") {
            d.res.int("speed").max(1)
        } else {
            prog_speed
        };

        let mut sinof = [0.0; DEGS as usize];
        let mut cosof = [0.0; DEGS as usize];
        let mut tanof = [0.0; DEGS as usize];
        for deg in (0..DEGS as usize).rev() {
            sinof[deg] = (deg as f64 * DTOR).sin();
            cosof[deg] = (deg as f64 * DTOR).cos();
            // The tangent is unbounded at the quarters, so borrow the neighbour
            // that was computed one step earlier.
            tanof[deg] = if deg as i32 % DEGS4 == 0 {
                tanof[deg + 1]
            } else {
                (deg as f64 * DTOR).tan()
            };
        }

        let mut st = Self {
            s: Screen {
                wid,
                hei,
                pscale,
                point: vec![0; (wid * hei) as usize],
                colors: [BLACK; TAILMAX + 1],
                gc: Gc::new(WHITE, d.res.pixel("background")),
            },
            thread: vec![Line::default(); THRMAX as usize],
            bank: Vec::new(),
            boxw: 10,
            boxh: 10,
            curviness: 30,
            gridden: 0,
            ogd: 8,
            bordcorn: 0,
            bordcol: 1,
            threads: 4,
            ch: 0,
            speed,
            erasing: true,
            autopal: false,
            reset_p: true,
            cyc: 0,
            sinof,
            cosof,
            tanof,
            prog: prog.as_bytes().to_vec(),
            pos: 0,
        };

        for thr in 1..=THRMAX {
            st.firstinit(thr);
            st.newonscreen(thr);
        }
        st.randpal();
        // Upstream redraws here, but nothing has been drawn yet.
        st.consume_instring(d);
        st
    }

    /// `firstinit`: the settings a worm keeps for its whole life, unless a
    /// program changes them.
    fn firstinit(&mut self, thr: i32) {
        let lp = &mut self.thread[(thr - 1) as usize];
        lp.col = (thr + 1) as u8;
        lp.prey = 0;
        lp.tmode = 1;
        lp.slice = DEGS / 3;
        lp.orichar = b'R';
        lp.spiturn = 5;
        lp.selfbounce = false;
        lp.realbounce = false;
        lp.vhfollow = false;
        lp.tailfollow = false;
        lp.killwalls = false;
        lp.little = false;
        lp.ctinc = random1(2) * 2 - 1;
        lp.circturn = ((thr % 2) * 2 - 1) * ((thr - 1) % 7 + 1);
        lp.tsc = 1;
        lp.tslen = 6;
        lp.turnseq[0] = 6;
        lp.turnseq[1] = -6;
        lp.turnseq[2] = 6;
        lp.turnseq[3] = 6;
        lp.turnseq[4] = -6;
        lp.turnseq[5] = 6;
        lp.tclim = DEGS / 2 / 12;
    }

    /// `newonscreen`: drop a worm somewhere new, pointing somewhere new.
    fn newonscreen(&mut self, thr: i32) {
        let (wid, hei) = (self.s.wid, self.s.hei);
        let lp = &mut self.thread[(thr - 1) as usize];
        lp.filled = false;
        lp.dead = false;
        lp.reclen = if lp.little {
            random1(10) + 5
        } else {
            random1(RLMAX - 30) + 30
        };
        lp.deg = random1(DEGS);
        lp.y = random1(hei) as f64;
        lp.x = random1(wid) as f64;
        lp.recpos = 0;
        lp.turnco = 2;
        lp.turnsize = random1(4) + 2;
    }

    /// `randpal`. Upstream asks for one colour fewer than this and leaves the
    /// last tail colour reading off the end of its array.
    fn randpal(&mut self) {
        let cols = make_random_colormap(TAILMAX, true);
        for (i, c) in cols.iter().enumerate() {
            self.s.colors[i + 1] = c.pixel;
        }
    }

    /// `palupdate`: recolour what is already drawn, but only once the program
    /// has finished being read, so a program that changes several things at
    /// once redraws once at the end rather than after every letter.
    fn palupdate(&mut self, d: &mut Dpy, force: bool) {
        if force || !self.wasakeypressed() {
            let (w, h) = (self.s.wid, self.s.hei);
            self.s.redraw(d, 0, 0, w, h);
        }
    }

    /// `bordupdate`: draw the two edges of the screen that are walls, which
    /// corner they meet in being one of the things a program can rotate.
    fn bordupdate(&mut self, d: &mut Dpy) {
        let (xmax, ymax) = (self.s.wid - 1, self.s.hei - 1);
        let ybord = if self.bordcorn == 0 || self.bordcorn == 1 {
            0
        } else {
            ymax
        };
        let xbord = if self.bordcorn == 0 || self.bordcorn == 3 {
            0
        } else {
            xmax
        };
        let c = self.bordcol;
        for x in 0..=xmax {
            self.s.sp(d, x, ybord, c);
        }
        for y in 0..=ymax {
            self.s.sp(d, xbord, y, c);
        }
    }

    /// `gridupdate`: scatter the walls of a lattice, each cell edge present or
    /// missing by a roll against the density.
    fn gridupdate(&mut self, d: &mut Dpy, interruptible: bool) {
        if self.gridden <= 0 {
            return;
        }
        let (xmax, ymax) = (self.s.wid - 1, self.s.hei - 1);
        let mut x = 0;
        while x <= xmax && !(self.wasakeypressed() && interruptible) {
            let mut y = 0;
            while y <= ymax {
                if random1(15) < self.gridden {
                    let max = (x + self.boxw).min(xmax);
                    for xc in x..=max {
                        self.s.sp(d, xc, y, 1);
                    }
                }
                if random1(15) < self.gridden {
                    let max = (y + self.boxh).min(ymax);
                    for yc in y..=max {
                        self.s.sp(d, x, yc, 1);
                    }
                }
                y += self.boxh;
            }
            x += self.boxw;
        }
    }

    // ---- the program ------------------------------------------------------

    fn wasakeypressed(&self) -> bool {
        self.pos < self.prog.len()
    }

    /// `readkey`. Running off the end reads as `#`, which closes whatever the
    /// program left open.
    fn readkey(&mut self) -> u8 {
        if self.pos >= self.prog.len() {
            b'#'
        } else {
            let c = self.prog[self.pos];
            self.pos += 1;
            c.to_ascii_uppercase()
        }
    }

    fn inbank(&self, thr: i32) -> bool {
        self.bank.contains(&(thr as u8))
    }

    /// `pickbank`: read a selection of worms. Numbers pick worms by hand, `A`
    /// takes every worm that exists, `E` every worm that could exist, `T` every
    /// worm in one steering mode, `I` inverts what is selected so far, and `+`
    /// and `-` walk the cursor along.
    fn pickbank(&mut self, d: &mut Dpy) {
        let mut thr: i32 = 1;
        self.bank.clear();
        self.ch = 0;
        loop {
            // Upstream spins here if every worm is already selected; its own
            // exit test below means that cannot happen.
            for _ in 0..self.threads {
                if !self.inbank(thr) {
                    break;
                }
                thr = thr % self.threads + 1;
            }

            self.palupdate(d, false);
            self.ch = self.readkey();
            self.palupdate(d, false);
            match self.ch {
                b'+' | b'-' => {
                    for _ in 0..=self.threads {
                        thr += if self.ch == b'+' { 1 } else { -1 };
                        thr = wrap_i(thr, 1, self.threads + 1);
                        if !self.inbank(thr) {
                            break;
                        }
                    }
                }
                b' ' => self.bank.push(thr as u8),
                b'1'..=b'9' => {
                    let t = (self.ch - b'0') as i32;
                    if t <= self.threads {
                        self.bank.push(t as u8);
                    }
                }
                b'I' => {
                    let mut tbank = Vec::new();
                    for c in 1..=self.threads {
                        if !self.inbank(c) {
                            tbank.push(c as u8);
                        }
                    }
                    self.bank = tbank;
                }
                b'T' => {
                    self.ch = self.readkey();
                    if (b'1'..=b'9').contains(&self.ch) {
                        let m = (self.ch - b'0') as i32;
                        for c in 1..=self.threads {
                            if self.thread[(c - 1) as usize].tmode == m {
                                self.bank.push(c as u8);
                            }
                        }
                    }
                }
                b'A' => self.bank = (1..=self.threads).map(|c| c as u8).collect(),
                b'E' => self.bank = (1..=THRMAX).map(|c| c as u8).collect(),
                _ => {}
            }
            if self.bank.len() as i32 >= self.threads
                || self.ch == b'N'
                || self.ch == b'\r'
                || self.ch == b'#'
            {
                break;
            }
        }
        if self.bank.is_empty() && self.ch != b'N' {
            self.bank.push(thr as u8);
        }
        self.palupdate(d, false);
    }

    /// `consume_instring`: run the program to its end.
    fn consume_instring(&mut self, d: &mut Dpy) {
        while self.wasakeypressed() {
            self.ch = self.readkey();
            match self.ch {
                // Make worms, either appending to the ones that exist or
                // replacing them, each digit naming a steering mode.
                b'M' => {
                    self.ch = self.readkey();
                    if self.ch == b'A' || self.ch == b'N' {
                        let othreads = self.threads;
                        if self.ch == b'N' {
                            self.threads = 0;
                        }
                        loop {
                            self.ch = self.readkey();
                            match self.ch {
                                b'1'..=b'9' => {
                                    self.threads += 1;
                                    self.thread[(self.threads - 1) as usize].tmode =
                                        (self.ch - b'0') as i32;
                                }
                                b'R' => {
                                    self.threads += 1;
                                    self.thread[(self.threads - 1) as usize].tmode =
                                        random1(TMODES) + 1;
                                }
                                _ => {}
                            }
                            if self.ch == b'\r' || self.ch == b'#' || self.threads == THRMAX {
                                break;
                            }
                        }
                        if self.threads == 0 {
                            self.threads = othreads;
                        }
                        self.reset_p = true;
                    }
                }

                // Change a selection of worms.
                b'C' => {
                    self.pickbank(d);
                    if !self.bank.is_empty() {
                        self.ch = self.readkey();
                        let bank = self.bank.clone();
                        match self.ch {
                            // The slice of the circle headings snap to.
                            b'D' => {
                                self.ch = self.readkey();
                                match self.ch {
                                    b'1'..=b'9' => {
                                        let n = (self.ch - b'0') as i32;
                                        for &b in &bank {
                                            self.thread[(b - 1) as usize].slice = DEGS / n;
                                        }
                                    }
                                    b'M' => {
                                        for &b in &bank {
                                            self.thread[(b - 1) as usize].slice = 0;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            // The turn sequence for steering mode seven, read
                            // digit by digit, with every other worm in the
                            // selection given the mirror image of it.
                            b'S' => {
                                for &b in &bank {
                                    let l = &mut self.thread[(b - 1) as usize];
                                    l.otslen = l.tslen;
                                    l.tslen = 0;
                                }
                                loop {
                                    let oldch = self.ch;
                                    self.ch = self.readkey();
                                    if self.ch.is_ascii_digit() {
                                        let v = (self.ch - b'0') as i32;
                                        for (n, b) in bank.iter().enumerate() {
                                            let l = &mut self.thread[(b - 1) as usize];
                                            if l.tslen >= TSMAX {
                                                continue;
                                            }
                                            l.tslen += 1;
                                            let mut t = v;
                                            if oldch == b'-' {
                                                t = -t;
                                            }
                                            if (n + 1) % 2 == 0 {
                                                t = -t;
                                            }
                                            l.turnseq[(l.tslen - 1) as usize] = t;
                                        }
                                    }
                                    let first = self.bank[0];
                                    if self.ch == b'\r'
                                        || self.ch == b'#'
                                        || self.thread[(first - 1) as usize].tslen == TSMAX
                                    {
                                        break;
                                    }
                                }
                                for &b in &bank {
                                    let l = &mut self.thread[(b - 1) as usize];
                                    if l.tslen == 0 {
                                        l.tslen = l.otslen;
                                    }
                                    let sum: i32 = l.turnseq[..l.tslen as usize].iter().sum();
                                    // How long to hold each turn of the
                                    // sequence so the worm comes back around.
                                    l.tclim = if sum == 0 { 1 } else { DEGS2 / sum.abs() };
                                    l.tsc = random1(l.tslen) + 1;
                                }
                            }
                            // The steering mode.
                            b'T' => {
                                self.ch = self.readkey();
                                for &b in &bank {
                                    let l = &mut self.thread[(b - 1) as usize];
                                    match self.ch {
                                        b'1'..=b'9' => l.tmode = (self.ch - b'0') as i32,
                                        b'R' => l.tmode = random1(TMODES) + 1,
                                        _ => {}
                                    }
                                }
                            }
                            // Lock headings to the axes or the diagonals.
                            b'O' => {
                                self.ch = self.readkey();
                                let o = self.ch;
                                for &b in &bank {
                                    self.thread[(b - 1) as usize].orichar = o;
                                }
                            }
                            // Give this selection a second selection to chase.
                            b'F' => {
                                let fbank = self.bank.clone();
                                self.pickbank(d);
                                for (n, fb) in fbank.iter().enumerate() {
                                    let prey = if self.ch == b'N' {
                                        0
                                    } else {
                                        self.bank[n % self.bank.len()] as i32
                                    };
                                    self.thread[(fb - 1) as usize].prey = prey;
                                }
                            }
                            // Chase in a ring: each worm follows the next.
                            b'L' => {
                                for (n, b) in bank.iter().enumerate() {
                                    self.thread[(b - 1) as usize].prey =
                                        bank[(n + 1) % bank.len()] as i32;
                                }
                            }
                            // How hard a chasing worm may turn.
                            b'R' => {
                                self.ch = self.readkey();
                                for &b in &bank {
                                    let l = &mut self.thread[(b - 1) as usize];
                                    match self.ch {
                                        b'1'..=b'9' => l.circturn = 10 - (self.ch - b'0') as i32,
                                        b'R' => l.circturn = random1(7) + 1,
                                        _ => {}
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }

                // Toggle, set or clear one flag on a selection of worms.
                b'T' | b'Y' | b'N' => {
                    let boolop = self.ch;
                    self.pickbank(d);
                    if !self.bank.is_empty() {
                        self.ch = self.readkey();
                        for b in self.bank.clone() {
                            let l = &mut self.thread[(b - 1) as usize];
                            match self.ch {
                                b'S' => bankmod(boolop, &mut l.selfbounce),
                                b'V' => bankmod(boolop, &mut l.vhfollow),
                                b'R' => bankmod(boolop, &mut l.realbounce),
                                b'L' => bankmod(boolop, &mut l.little),
                                b'T' => bankmod(boolop, &mut l.tailfollow),
                                b'K' => bankmod(boolop, &mut l.killwalls),
                                _ => {}
                            }
                        }
                    }
                }

                // Move the border to the next corner, by erasing it and drawing
                // it again somewhere else.
                b'R' if self.bordcol == 1 => {
                    self.bordcol = 0;
                    self.bordupdate(d);
                    self.bordcorn = (self.bordcorn + 1) % 4;
                    self.bordcol = 1;
                    self.bordupdate(d);
                }

                // A bare digit sets the steering mode of every worm there could
                // ever be, not just the ones that exist.
                b'1'..=b'9' => {
                    let m = (self.ch - b'0') as i32;
                    for c in 0..THRMAX as usize {
                        self.thread[c].tmode = m;
                    }
                }

                b'E' => self.erasing = !self.erasing,
                b'P' => {
                    self.randpal();
                    self.palupdate(d, true);
                }

                // The grid, sized and thinned out here.
                b'G' => {
                    let mut dimch = b'B';
                    let mut gridchanged = true;
                    if self.gridden == 0 {
                        self.gridden = self.ogd;
                    }
                    loop {
                        let mut msize = 0;
                        if gridchanged {
                            self.s.clearscreen(d);
                            self.gridupdate(d, true);
                        }
                        self.ch = self.readkey();
                        gridchanged = true;
                        match self.ch {
                            b'+' => msize = 1,
                            b'-' => msize = -1,
                            b']' => {
                                if self.gridden < 15 {
                                    self.gridden += 1;
                                }
                            }
                            b'[' => {
                                if self.gridden > 0 {
                                    self.gridden -= 1;
                                }
                            }
                            b'O' => {
                                self.ogd = self.gridden;
                                self.gridden = 0;
                            }
                            // Upstream falls through from the square case into
                            // the ones that name a dimension, so asking for a
                            // square leaves neither dimension selected and the
                            // size keys do nothing from then on.
                            b'S' => {
                                self.boxw = self.boxh;
                                dimch = b'S';
                            }
                            b'W' | b'H' | b'B' => dimch = self.ch,
                            _ => gridchanged = false,
                        }
                        if dimch == b'W' || dimch == b'B' {
                            self.boxw += msize;
                        }
                        if dimch == b'H' || dimch == b'B' {
                            self.boxh += msize;
                        }
                        if self.boxw == 0 {
                            self.boxw = 1;
                        }
                        if self.boxh == 0 {
                            self.boxh = 1;
                        }
                        if self.ch == b'\r' || self.ch == b'#' || self.ch == b'O' {
                            break;
                        }
                    }
                }

                b'A' => self.autopal = !self.autopal,
                b'B' => {
                    self.bordcol = 1 - self.bordcol;
                    self.bordupdate(d);
                }
                b'-' => self.speed = (self.speed - SPEEDINC).max(1),
                b'+' => self.speed = (self.speed + SPEEDINC).min(SPEEDMAX),
                b'/' if self.curviness > 5 => self.curviness -= 5,
                b'*' if self.curviness < 50 => self.curviness += 5,
                // One more worm, or one fewer, wiped off the screen as it goes.
                b']' if self.threads < THRMAX => {
                    self.threads += 1;
                    let thr = self.threads;
                    self.newonscreen(thr);
                }
                b'[' if self.threads > 1 => {
                    let l = &self.thread[(self.threads - 1) as usize];
                    let lastpos = if l.filled { l.reclen - 1 } else { l.recpos };
                    for c in 0..=lastpos as usize {
                        let (x, y) = {
                            let l = &self.thread[(self.threads - 1) as usize];
                            (l.xrec[c], l.yrec[c])
                        };
                        self.s.sp(d, x, y, 0);
                    }
                    self.threads -= 1;
                }
                _ => {}
            }
        }
    }

    // ---- the worms --------------------------------------------------------

    /// `move`: one step of one worm. False once it can no longer move.
    fn move_thread(&mut self, d: &mut Dpy, thr: i32) -> bool {
        let i = (thr - 1) as usize;
        if self.thread[i].dead {
            return false;
        }

        let prey = self.thread[i].prey;
        if prey == 0 {
            let curviness = self.curviness;
            let lp = &mut self.thread[i];
            match lp.tmode {
                // Stagger.
                1 => lp.deg += random1(2 * lp.turnsize + 1) - lp.turnsize,
                // Snap to a slice of the circle, and now and then jump a whole
                // slice sideways.
                2 => {
                    if lp.slice == DEGS || lp.slice == DEGS2 || lp.slice == DEGS4 {
                        if lp.orichar == b'D' {
                            if lp.deg % DEGS4 != DEGS8 {
                                lp.deg = DEGS4 * random1(4) + DEGS8;
                            }
                        } else if lp.orichar == b'V' && lp.deg % DEGS4 != 0 {
                            lp.deg = DEGS4 * random1(4);
                        }
                    }
                    if random1(100) == 0 {
                        if lp.slice == 0 {
                            lp.deg = lp.deg - DEGS4 + random1(DEGS2);
                        } else {
                            lp.deg += (random1(2) * 2 - 1) * lp.slice;
                        }
                    }
                }
                // A circle.
                3 => lp.deg += lp.circturn,
                // A spiral: the turn drifts outwards, then reverses.
                4 => {
                    if lp.spiturn.abs() > 11 {
                        lp.spiturn = 5;
                    } else {
                        lp.deg += lp.spiturn;
                    }
                    if random1(15 - lp.spiturn.abs()) == 0 {
                        lp.spiturn += lp.ctinc;
                        if lp.spiturn.abs() > 10 {
                            lp.ctinc *= -1;
                        }
                    }
                }
                // A coil: hold a turn, then reverse it.
                5 => {
                    lp.turnco = lp.turnco.abs() - 1;
                    if lp.turnco == 0 {
                        lp.turnco = curviness + random1(10);
                        lp.circturn *= -1;
                    }
                    lp.deg += lp.circturn;
                }
                // Arcs joined by straight runs: the counter is positive while
                // turning and negative while going straight.
                6 => {
                    if lp.turnco.abs() == 1 {
                        lp.turnco *= -(random1(DEGS2 / lp.circturn.abs()) + 5);
                    } else if lp.turnco == 0 {
                        lp.turnco = 2;
                    } else if lp.turnco > 0 {
                        lp.turnco -= 1;
                        lp.deg += lp.circturn;
                    } else {
                        lp.turnco += 1;
                    }
                }
                // A fixed sequence of turns, each held for the same count.
                7 => {
                    lp.turnco += 1;
                    if lp.turnco > lp.tclim {
                        lp.turnco = 1;
                        // A selection that named the same worm twice could have
                        // left the length at zero, which C would divide by.
                        lp.tsc = (lp.tsc % lp.tslen.max(1)) + 1;
                    }
                    lp.deg += lp.turnseq[(lp.tsc - 1) as usize];
                }
                _ => {}
            }
        } else {
            // Steer towards whatever this worm is chasing.
            let p = &self.thread[(prey - 1) as usize];
            let lp = &self.thread[i];
            let (dx, dy) = if lp.tailfollow || prey == thr {
                (
                    p.xrec[p.recpos as usize] as f64 - lp.x,
                    p.yrec[p.recpos as usize] as f64 - lp.y,
                )
            } else {
                (p.x - lp.x, p.y - lp.y)
            };
            let (vhfollow, deg, circturn) = (lp.vhfollow, lp.deg, lp.circturn);

            // The quadrant or, if it may only travel along the axes, the axis.
            let mut desdeg = if vhfollow {
                if dx.abs() > dy.abs() {
                    if dx > 0.0 { 0 } else { 2 * DEGS4 }
                } else if dy > 0.0 {
                    DEGS4
                } else {
                    3 * DEGS4
                }
            } else if dx > 0.0 {
                if dy > 0.0 { DEGS8 } else { 7 * DEGS8 }
            } else if dy > 0.0 {
                3 * DEGS8
            } else {
                5 * DEGS8
            };

            let newdeg = if desdeg - desdeg % DEGS4 != deg - deg % DEGS4 || vhfollow {
                // Far enough off course to be worth an exact heading.
                if !vhfollow {
                    desdeg = wrap_i((dy.atan2(dx) / DTOR) as i32, 0, DEGS);
                }
                if (desdeg - deg).abs() <= circturn.abs() {
                    desdeg
                } else if desdeg > deg {
                    deg + if desdeg - deg > DEGS2 {
                        -circturn.abs()
                    } else {
                        circturn.abs()
                    }
                } else {
                    deg + if deg - desdeg > DEGS2 {
                        circturn.abs()
                    } else {
                        -circturn.abs()
                    }
                }
            } else {
                // Already in the right quadrant: nudge across the line to it.
                deg + if self.tanof[deg as usize] > dy / dx {
                    -circturn.abs()
                } else {
                    circturn.abs()
                }
            };
            self.thread[i].deg = newdeg;
        }

        self.thread[i].deg = wrap_i(self.thread[i].deg, 0, DEGS);

        let (wid, hei) = (self.s.wid, self.s.hei);
        let (oldx, oldy) = (self.thread[i].x, self.thread[i].y);
        let deg = self.thread[i].deg as usize;
        let (stepx, stepy) = (self.cosof[deg], self.sinof[deg]);
        {
            let lp = &mut self.thread[i];
            lp.x = wrap_f(lp.x + stepx, 0.0, wid as f64);
            lp.y = wrap_f(lp.y + stepy, 0.0, hei as f64);
        }

        // Upstream's `xi` and `yi` are macros, so every use re-reads the
        // position: this block sees where the worm moved to, and the trail
        // record below sees where a bounce may have put it back.
        let xi = self.thread[i].x as i32;
        let yi = self.thread[i].y as i32;

        let oldcol = self.s.gp(xi, yi);
        if oldcol != 0 {
            let mut vertwall = false;
            let mut horiwall = false;
            let t = &self.thread[i];
            if oldcol == 1 && ((t.killwalls && self.gridden > 0) || t.realbounce) {
                // Two more probes say which way the wall runs.
                vertwall = self.s.gp(xi, oldy as i32) == 1;
                horiwall = self.s.gp(oldx as i32, yi) == 1;
            }
            if oldcol == 1 && self.thread[i].realbounce && (vertwall || horiwall) {
                let t = &mut self.thread[i];
                t.deg = if vertwall { -t.deg + DEGS2 } else { -t.deg };
            } else {
                let t = &mut self.thread[i];
                if (oldcol != t.col && t.realbounce) || (oldcol == t.col && t.selfbounce) {
                    t.deg += DEGS4 * (random1(2) * 2 - 1);
                } else if oldcol != t.col {
                    t.deg += DEGS2;
                }
            }
            if self.thread[i].killwalls && self.gridden > 0 && oldcol == 1 {
                // Eat the whole wall segment, up to where it meets the next.
                let (boxw, boxh) = (self.boxw, self.boxh);
                if vertwall && xi < wid - 1 {
                    let mut yy = yi - yi % boxh;
                    while yy <= yi - yi % boxh + boxh && yy < hei {
                        if self.s.gp(xi + 1, yy) != 1 || yy == hei - 1 {
                            self.s.sp(d, xi, yy, 0);
                        }
                        yy += 1;
                    }
                }
                if horiwall && yi < hei - 1 {
                    let mut xx = xi - xi % boxw;
                    while xx <= xi - xi % boxw + boxw && xx < wid {
                        if self.s.gp(xx, yi + 1) != 1 || xx == wid - 1 {
                            self.s.sp(d, xx, yi, 0);
                        }
                        xx += 1;
                    }
                }
            }
            if oldcol != self.thread[i].col || self.thread[i].selfbounce {
                let t = &mut self.thread[i];
                t.x = oldx;
                t.y = oldy;
            }
            self.thread[i].deg = wrap_i(self.thread[i].deg, 0, DEGS);
        }

        let xi = self.thread[i].x as i32;
        let yi = self.thread[i].y as i32;
        let col = self.thread[i].col;
        self.s.sp(d, xi, yi, col);

        let erasing = self.erasing;
        if self.thread[i].filled {
            let t = &self.thread[i];
            let (rx, ry) = (t.xrec[t.recpos as usize], t.yrec[t.recpos as usize]);
            // Erasing rubs the tail out; otherwise it is left behind in the
            // worm's second colour and the drawing accumulates.
            let c = if erasing { 0 } else { col + THRMAX as u8 };
            self.s.sp(d, rx, ry, c);
        }

        let t = &mut self.thread[i];
        t.yrec[t.recpos as usize] = yi;
        t.xrec[t.recpos as usize] = xi;
        if t.recpos == t.reclen - 1 {
            t.filled = true;
        }
        if t.filled && !erasing {
            // With nothing being erased a worm can get walled in by its own
            // trail and stop. A body whose every recorded position is the same
            // pixel is one that is no longer going anywhere.
            let mut co = t.recpos;
            t.dead = true;
            loop {
                let nextco = wrap_i(co + 1, 0, t.reclen);
                if t.yrec[co as usize] != t.yrec[nextco as usize]
                    || t.xrec[co as usize] != t.xrec[nextco as usize]
                {
                    t.dead = false;
                }
                co = nextco;
                if !t.dead || co == t.recpos {
                    break;
                }
            }
        }
        t.recpos = wrap_i(t.recpos + 1, 0, t.reclen);
        !t.dead
    }

    /// `waitabit`: spend the frame budget. More worms means more steps per
    /// frame at the same speed, so the delay grows to pay for them.
    fn waitabit(&mut self) -> u32 {
        let mut result = 0;
        self.cyc += self.threads;
        while self.cyc > self.speed {
            result += 10_000;
            self.cyc -= self.speed;
        }
        result
    }
}

impl Screenhack for Vermiculate {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let mut this_delay = 0;
        let mut steps = 0;
        loop {
            if self.reset_p {
                self.reset_p = false;
                self.s.clearscreen(d);
                for thr in 1..=self.threads {
                    self.newonscreen(thr);
                }
                if self.autopal {
                    self.randpal();
                    self.palupdate(d, false);
                }
                self.bordupdate(d);
                self.gridupdate(d, false);
            }

            let mut alltrap = true;
            for thr in 1..=self.threads {
                if self.move_thread(d, thr) {
                    alltrap = false;
                }
            }
            if alltrap {
                self.reset_p = true;
            }
            if self.speed != SPEEDMAX {
                this_delay = self.waitabit();
            }

            if this_delay != 0 || steps >= 1000 {
                break;
            }
            steps += 1;
        }
        this_delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.s.wid = width;
        self.s.hei = height;
        self.s.point = vec![0; (width * height) as usize];
        // Upstream leaves the worms where they were, which on a window that
        // shrank is off the end of the buffer it just allocated.
        for thr in 1..=THRMAX {
            self.newonscreen(thr);
        }
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.reset_p = true;
            return true;
        }
        false
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    Box::new(Vermiculate::new(d))
}

const DEFAULTS: &[&str] = &[
    ".background: Black",
    "*ticks: 20000",
    "*fpsSolid: true",
    "*speed: 1",
    "*instring: ",
    "*ignoreRotation: True",
];

const PROGRAMS: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random program",
    },
    SelectItem {
        value: "1",
        label: "Right angles (9)",
    },
    SelectItem {
        value: "2",
        label: "Chasing circles (15)",
    },
    SelectItem {
        value: "3",
        label: "Loose right angles (5)",
    },
    SelectItem {
        value: "4",
        label: "Mixed modes (35)",
    },
    SelectItem {
        value: "5",
        label: "Short ring (16)",
    },
    SelectItem {
        value: "6",
        label: "Long ring (62)",
    },
    SelectItem {
        value: "7",
        label: "Bouncing circles (24)",
    },
    SelectItem {
        value: "8",
        label: "Bouncing ring (58)",
    },
    SelectItem {
        value: "9",
        label: "Five modes (30)",
    },
    SelectItem {
        value: "10",
        label: "Five modes, mixed (30)",
    },
];

const OPTS: &[Opt] = &[
    Opt::select("program", "Program", PROGRAMS, "random"),
    Opt::slider("speed", "Duration", 1.0, 1000.0, 1.0, 0, "1"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "vermiculate",
    label: "Vermiculate",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Tyler Pierce",
        year: "2001",
        video: Some("https://www.youtube.com/watch?v=YSg9KY-qw5o"),
        blurb: "Squiggly worm-like paths.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
