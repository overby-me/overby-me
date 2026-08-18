//! Port of `hacks/pong.c`.
//!
//! ```text
//! pong, Copyright (c) 2003 Jeremy English <jenglish@myself.com>
//! A pong screen saver
//!
//! Modified by Brian Sawicki <sawicki@imsa.edu> to fix a small bug.
//! Before this fix after a certain point the paddles would be too
//! small for the program to effectively hit the ball.  The score would
//! then skyrocket as the paddles missed most every time. Added a max
//! so that once a paddle gets 10 the entire game restarts.  Special
//! thanks to Scott Zager for some help on this.
//!
//! Modified by Trevor Blackwell <tlb@tlb.org> to use analogtv.[ch] display.
//! Also added gradual acceleration of the ball, shrinking of paddles, and
//! scorekeeping.
//!
//! Modified by Gereon Steffens <gereon@steffens.org> to add -clock and -noise
//! options.  In clock mode, the score reflects the current time, and the
//! paddles simply stop moving when it's time for the other side to score.
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
//! The 1971 game, played by two machines that are trying but not very hard.
//! The paddle works out where the ball is going by extrapolating from its
//! current velocity, then moves at a fixed rate towards it, so as the ball
//! speeds up it starts arriving before the paddle does. The paddles also shrink
//! a twentieth every point, so a rally that goes on gets harder for both sides
//! until somebody reaches ten and the whole game starts again.
//!
//! In clock mode the score is the time: the right paddle is the hour and the
//! left is the minute, and whichever side needs to concede simply stops moving
//! until it has. That means the display updates a few seconds after the minute
//! actually turns, which suits it.
//!
//! Nothing here draws to the screen. The game is drawn into a composite video
//! signal and [`crate::runtime::analogtv`] receives it, which is where the
//! snow, the bloom and the soft edges come from. Note that the sync is set up
//! *without* a colour burst, so the receiver finds no colour and the picture is
//! blackandwhite, which is what a 1971 Pong on a colour set looked like.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::analogtv::{self, AnalogTv, Input, Reception, TvFont, lcp_to_ntsc};
use crate::runtime::{
    About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, random, random_below,
};

const PONG_W: i32 = analogtv::VIS_LEN as i32;
const PONG_H: i32 = analogtv::VISLINES as i32;
const PONG_TMARG: i32 = 10;

#[derive(Clone, Copy, Default)]
struct Paddle {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    /// Set while this side is waiting for the ball to come back, during which
    /// it does not move at all.
    wait: bool,
    /// Set once it has committed to a position for this rally.
    lock: bool,
    score: i32,
}

#[derive(Clone, Copy, Default)]
struct Ball {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

struct Pong {
    clock: bool,
    l_paddle: Paddle,
    r_paddle: Paddle,
    ball: Ball,
    bx: i32,
    by: i32,
    m_unit: i32,
    paddle_rate: i32,
    noise: f64,

    tv: AnalogTv,
    inp: Input,
    reception: Reception,

    paddle_ntsc: [i32; 4],
    field_ntsc: [i32; 4],
    ball_ntsc: [i32; 4],
    score_ntsc: [i32; 4],
    net_ntsc: [i32; 4],

    score_font: TvFont,
}

/// The regular pong font. If you think we have not learned anything since
/// the early 70s, look at this for a while.
const FONT_SMALL: [&str; 10] = [
    "*****  **  **  **  *****",
    "   *   *   *   *   *   *",
    "****   ******   *   ****",
    "****   *****   *   *****",
    "*  **  *****   *   *   *",
    "*****   ****   *   *****",
    "*****   *****  **  *****",
    "****   *   *   *   *   *",
    "*****  ******  **  *****",
    "*****  *****   *   *   *",
];

/// The clock font: hand-crafted at double size, which looks better.
const FONT_BIG: [&str; 10] = [
    "####### ####### ##   ## ##   ## ##   ## ##   ## ##   ## ##   ## ##   ## ####### ####### ",
    "   ##      ##      ##      ##      ##      ##      ##      ##      ##      ##      ##   ",
    "####### #######      ##      ## ####### ####### ##      ##      ##      ####### ####### ",
    "####### #######      ##      ## ####### #######      ##      ##      ## ####### ####### ",
    "##   ## ##   ## ##   ## ##   ## ####### #######      ##      ##      ##      ##      ## ",
    "####### ####### ##      ##      ####### #######      ##      ##      ## ####### ####### ",
    "####### ####### ##      ##      ####### ####### ##   ## ##   ## ##   ## ####### ####### ",
    "####### #######      ##      ##      ##      ##      ##      ##      ##      ##      ## ",
    "####### ####### ##   ## ##   ## ####### ####### ##   ## ##   ## ##   ## ####### ####### ",
    "####### ####### ##   ## ##   ## ####### #######      ##      ##      ## ####### ####### ",
];

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let (w, h) = (d.width(), d.height());
    let mut tv = AnalogTv::new(w, h);
    tv.set_defaults(
        d.res.float("TVColor") as f32,
        d.res.float("TVTint") as f32,
        d.res.float("TVBrightness") as f32,
        d.res.float("TVContrast") as f32,
    );

    let clock = d.res.bool("clock");
    let (fw, fh, glyphs): (i32, i32, &[&str; 10]) = if clock {
        (8, 12, &FONT_BIG)
    } else {
        (4, 6, &FONT_SMALL)
    };
    let mut score_font = TvFont::new(fw, fh);
    for (i, g) in glyphs.iter().enumerate() {
        score_font.set_char(b'0' + i as u8, g);
    }
    if !clock {
        score_font.y_mult *= 2;
        score_font.x_mult *= 2;
    }

    let mut inp = Input::new();
    // No colour burst: a 1971 Pong had nothing to say about colour, and the
    // receiver will find none.
    inp.setup_sync(false, false);

    let l_paddle = Paddle {
        x: 8,
        y: 100,
        w: 16,
        h: PONG_H / 4,
        wait: true,
        lock: false,
        score: 0,
    };
    let mut r_paddle = l_paddle;
    r_paddle.x = PONG_W - 8 - r_paddle.w;
    r_paddle.wait = false;

    let m_unit = d.res.int("speed");
    let mut st = Pong {
        clock,
        l_paddle,
        r_paddle,
        ball: Ball {
            x: PONG_W / 2,
            y: PONG_H / 2,
            w: 16,
            h: 16,
        },
        bx: m_unit,
        by: m_unit,
        m_unit,
        paddle_rate: m_unit - 1,
        noise: d.res.float("noise"),
        tv,
        inp,
        reception: Reception {
            level: 2.0,
            ofs: 0,
            multipath: 0.0,
            ..Reception::default()
        },
        paddle_ntsc: [0; 4],
        field_ntsc: [0; 4],
        ball_ntsc: [0; 4],
        score_ntsc: [0; 4],
        net_ntsc: [0; 4],
        score_font,
    };

    st.reset_score(d);
    st.start_game();

    st.field_ntsc = lcp_to_ntsc(f64::from(analogtv::BLACK_LEVEL), 0.0, 0.0);
    st.ball_ntsc = lcp_to_ntsc(100.0, 0.0, 0.0);
    st.paddle_ntsc = lcp_to_ntsc(100.0, 0.0, 0.0);
    st.score_ntsc = lcp_to_ntsc(100.0, 0.0, 0.0);
    st.net_ntsc = lcp_to_ntsc(100.0, 0.0, 0.0);

    let f = st.field_ntsc;
    st.inp.draw_solid(
        analogtv::VIS_START as i32,
        analogtv::VIS_END as i32,
        analogtv::TOP as i32,
        analogtv::BOT as i32,
        f,
    );

    Box::new(st)
}

fn hit_top_bottom(p: &mut Paddle) {
    if p.y <= PONG_TMARG {
        p.y = PONG_TMARG;
    }
    if p.y + p.h >= PONG_H {
        p.y = PONG_H - p.h;
    }
}

impl Pong {
    fn reset_score(&mut self, d: &Dpy) {
        if self.clock {
            /* init score to current time */
            let secs = d.wall_clock();
            self.r_paddle.score = (secs / 3600.0) as i32 % 24;
            self.l_paddle.score = (secs / 60.0) as i32 % 60;
        } else {
            self.r_paddle.score = 0;
            self.l_paddle.score = 0;
        }
    }

    /// Starts a Whole New Game.
    fn new_game(&mut self, d: &Dpy) {
        self.serve();
        self.reset_score(d);
        self.l_paddle.h = PONG_H / 4;
        self.r_paddle.h = PONG_H / 4;
        // Adjust the paddles again, because they were just made longer.
        hit_top_bottom(&mut self.l_paddle);
        hit_top_bottom(&mut self.r_paddle);
    }

    /// The next point, with both paddles a little shorter than last time.
    fn start_game(&mut self) {
        self.serve();
        if self.l_paddle.h > 10 {
            self.l_paddle.h = self.l_paddle.h * 19 / 20;
        }
        if self.r_paddle.h > 10 {
            self.r_paddle.h = self.r_paddle.h * 19 / 20;
        }
    }

    fn serve(&mut self) {
        self.ball.x = PONG_W / 2;
        self.ball.y = PONG_H / 2;
        self.bx = self.m_unit;
        self.by = self.m_unit;

        // Randomised a little so games on two screens are not identical.
        if random() & 1 != 0 {
            self.by = -self.by;
        }
        self.ball.y += random_below(PONG_H / 6) - PONG_H / 3;

        self.l_paddle.wait = true;
        self.l_paddle.lock = false;
        self.r_paddle.wait = false;
        self.r_paddle.lock = false;
        self.paddle_rate = self.m_unit - 1;
    }

    fn ball_hit_top_bottom(&mut self) {
        if self.ball.y <= PONG_TMARG || self.ball.y + self.ball.h >= PONG_H {
            self.by = -self.by;
        }
    }

    fn hit_paddle(&mut self, d: &Dpy) {
        if self.ball.x + self.ball.w >= self.r_paddle.x && self.bx > 0 {
            /* we are traveling to the right */
            if self.ball.y + self.ball.h > self.r_paddle.y
                && self.ball.y < self.r_paddle.y + self.r_paddle.h
            {
                self.bx = -self.bx;
                self.l_paddle.wait = false;
                self.r_paddle.wait = true;
                self.r_paddle.lock = false;
                self.l_paddle.lock = false;
            } else if self.clock {
                self.reset_score(d);
            } else {
                self.r_paddle.score += 1;
                if self.r_paddle.score >= 10 {
                    self.new_game(d);
                } else {
                    self.start_game();
                }
            }
        }

        if self.ball.x <= self.l_paddle.x + self.l_paddle.w && self.bx < 0 {
            /* we are traveling to the left */
            if self.ball.y + self.ball.h > self.l_paddle.y
                && self.ball.y < self.l_paddle.y + self.l_paddle.h
            {
                self.bx = -self.bx;
                self.l_paddle.wait = true;
                self.r_paddle.wait = false;
                self.r_paddle.lock = false;
                self.l_paddle.lock = false;
            } else if self.clock {
                self.reset_score(d);
            } else {
                self.l_paddle.score += 1;
                if self.l_paddle.score >= 10 {
                    self.new_game(d);
                } else {
                    self.start_game();
                }
            }
        }
    }

    /// Where a paddle decides to be. It extrapolates the ball's path from its
    /// current velocity and moves at a fixed rate, which is what makes it start
    /// missing as the ball speeds up.
    fn p_logic(&mut self, left: bool) {
        let p = if left { &self.l_paddle } else { &self.r_paddle };
        if p.wait {
            return;
        }
        let mut targ = if self.bx > 0 {
            self.ball.y + self.by * (self.r_paddle.x - self.ball.x) / self.bx
        } else if self.bx < 0 {
            self.ball.y - self.by * (self.ball.x - self.l_paddle.x - self.l_paddle.w) / self.bx
        } else {
            self.ball.y
        };
        targ = targ.clamp(0, PONG_H);

        let rate = self.paddle_rate;
        let p = if left {
            &mut self.l_paddle
        } else {
            &mut self.r_paddle
        };
        if targ < p.y && !p.lock {
            p.y -= rate;
        } else if targ > p.y + p.h && !p.lock {
            p.y += rate;
        } else {
            let move_by = (targ - (p.y + p.h / 2)).clamp(-rate, rate);
            p.y += move_by;
            p.lock = true;
        }
    }

    fn paint_paddle(&mut self, left: bool) {
        let p = if left { self.l_paddle } else { self.r_paddle };
        let vs = analogtv::VIS_START as i32;
        self.inp.draw_solid(
            vs + p.x,
            vs + p.x + p.w,
            analogtv::TOP as i32,
            analogtv::BOT as i32,
            self.field_ntsc,
        );
        self.inp.draw_solid(
            vs + p.x,
            vs + p.x + p.w,
            analogtv::TOP as i32 + p.y,
            analogtv::TOP as i32 + p.y + p.h,
            self.paddle_ntsc,
        );
    }

    fn paint_ball(&mut self, ntsc: [i32; 4]) {
        let vs = analogtv::VIS_START as i32;
        let b = self.ball;
        self.inp.draw_solid(
            vs + b.x,
            vs + b.x + b.w,
            analogtv::TOP as i32 + b.y,
            analogtv::TOP as i32 + b.y + b.h,
            ntsc,
        );
    }

    fn paint_score(&mut self) {
        let top = analogtv::TOP as i32;
        let strip = 10 + self.score_font.char_h * self.score_font.y_mult;
        self.inp.draw_solid(
            analogtv::VIS_START as i32,
            analogtv::VIS_END as i32,
            top,
            top + strip,
            self.field_ntsc,
        );

        let fmt = |n: i32| {
            if self.clock {
                format!("{:02}", n % 256)
            } else {
                format!("{}", n % 256)
            }
        };
        let (font, ntsc) = (&self.score_font, self.score_ntsc);
        let buf = fmt(self.r_paddle.score);
        self.inp
            .draw_string(font, &buf, analogtv::VIS_START as i32 + 130, top + 8, ntsc);
        let buf = fmt(self.l_paddle.score);
        self.inp
            .draw_string(font, &buf, analogtv::VIS_END as i32 - 200, top + 8, ntsc);
    }

    fn paint_net(&mut self) {
        let x = (analogtv::VIS_START as i32 + analogtv::VIS_END as i32) / 2;
        let mut y = analogtv::TOP as i32;
        while y < analogtv::BOT as i32 {
            self.inp.draw_solid(x - 2, x + 2, y, y + 3, self.net_ntsc);
            self.inp
                .draw_solid(x - 2, x + 2, y + 3, y + 6, self.field_ntsc);
            y += 6;
        }
    }
}

impl Screenhack for Pong {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.clock {
            let secs = d.wall_clock();
            let hour = (secs / 3600.0) as i32 % 24;
            let min = (secs / 60.0) as i32 % 60;
            if self.r_paddle.score != hour {
                /* l paddle must score */
                self.r_paddle.wait = true;
            } else if self.l_paddle.score != min {
                /* r paddle must score */
                self.l_paddle.wait = true;
            }
        }

        let field = self.field_ntsc;
        self.paint_ball(field); /* erase */

        self.ball.x += self.bx;
        self.ball.y += self.by;

        if !self.clock {
            /* in non-clock mode, occasionally increase ball speed */
            if random_below(40) == 0 {
                if self.bx > 0 {
                    self.bx += 1;
                } else {
                    self.bx -= 1;
                }
            }
        }

        self.p_logic(false);
        self.p_logic(true);

        hit_top_bottom(&mut self.r_paddle);
        hit_top_bottom(&mut self.l_paddle);

        self.ball_hit_top_bottom();
        self.hit_paddle(d);

        self.paint_score();
        self.paint_net();
        self.paint_paddle(false);
        self.paint_paddle(true);
        let ball = self.ball_ntsc;
        self.paint_ball(ball);

        self.reception.update();
        {
            let (tv, rec, inp) = (&mut self.tv, &self.reception, &self.inp);
            tv.draw(d.win(), self.noise, &[(rec, inp)]);
        }

        (1_000_000.0 / 29.97) as u32
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.tv.configure(width, height);
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*speed:      6",
    "*noise:      0.04",
    "*clock:      false",
    "*TVColor:         70",
    "*TVTint:          5",
    "*TVBrightness:    3",
    "*TVContrast:    150",
    "*fpsSolid:	     True",
];

const OPTS: &[Opt] = &[
    Opt::slider("speed", "Game speed", 1.0, 20.0, 1.0, 0, "6"),
    Opt::slider("noise", "Noise", 0.0, 5.0, 0.05, 2, "0.04"),
    Opt::boolean("clock", "Clock mode", "false"),
    Opt::slider("TVBrightness", "Brightness Knob", -75.0, 100.0, 1.0, 0, "3"),
    Opt::slider("TVContrast", "Contrast Knob", 0.0, 500.0, 5.0, 0, "150"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "pong",
    label: "Pong",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jeremy English, Trevor Blackwell and Jamie Zawinski",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=2jPmisDwwi0"),
        blurb: "The 1971 Pong home video game, on an old colour TV set.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
