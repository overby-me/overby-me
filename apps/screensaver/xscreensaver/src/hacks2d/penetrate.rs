//! Port of `hacks/penetrate.c`.
//!
//! ```text
//! Copyright (c) 1999 Adam Miller adum@aya.yale.edu
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! penetrate simulates the arcade classic with the cities and the stuff
//! shooting down from the sky and stuff. The computer plays against itself,
//! desperately defending the forces of good against those thingies raining
//! down. Bonus cities are awarded at ever-increasing intervals. Every five
//! levels appears a bonus round. The computer player gets progressively
//! more intelligent as the game progresses. Better aim, more economical with
//! ammo, and better target selection. Points are in the bottom right, and
//! high score is in the bottom left. Start with -smart to have the computer
//! player skip the learning process.
//! ```
//!
//! What makes it worth watching is that the player is bad at first and gets
//! better. Its aim tightens level by level, and three habits come in one
//! percentage point at a time: being *choosy* (ignore a warhead falling on a
//! city that is already rubble), *economic* (do not put a second interceptor on
//! something already covered), and *careful* (take the lowest warhead first,
//! because it is the one about to land). Watch a fresh game and it wastes
//! everything on the first two levels; leave it running and it stops missing.
//!
//! Upstream stops the world with `usleep` for the banner between levels, the
//! half second each surviving city takes to be counted, and the three seconds
//! it sits on GAME OVER. A page cannot block, so the between-levels sequence is
//! a small state machine here and each of those sleeps is a frame boundary: the
//! hack returns the delay it wanted and picks up on the other side.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{XColor, hsv_to_rgb};
use crate::runtime::font::Font;
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixel, Runner, SaverDef, Screenhack, SelectItem, StartArgs, random_below,
};

const SLEEP_TIME: u32 = 10000;
const CITY_PAUSE: u32 = 500_000;
const LEVEL_PAUSE: u32 = 1_000_000;
const SCORE_MISSILE: i64 = 100;
const FIRST_BONUS: i64 = 5000;
const MIN_RATE: i32 = 30;
const MAX_RADIUS: i32 = 100;

const MAX_MISSILES: usize = 256;
const MAX_BOOMS: usize = 512;
const MAX_LASERS: usize = 128;
const BOOM_RAD: i32 = 40;
const NUM_CITIES: usize = 5;
const LASER_LENGTH: f32 = 12.0;
const MISSILE_SPEED: f32 = 0.003;

const EXP_HELP: f32 = 0.2;
const SPEED_DIFF: f32 = 3.5;
const MAX_TO_GROUND: f32 = 0.75;

#[derive(Clone, Copy, Default)]
struct Missile {
    alive: bool,
    x: i32,
    y: i32,
    startx: i32,
    starty: i32,
    endx: i32,
    endy: i32,
    dcity: usize,
    pos: f32,
    /// How many interceptors are already on their way to this one.
    enemies: i32,
    /// Its hue, which is also what decides whether it splits.
    jenis: i32,
    splits: i32,
    color: XColor,
}

#[derive(Clone, Copy, Default)]
struct Boom {
    alive: bool,
    x: i32,
    y: i32,
    rad: i32,
    /// Set when this came from an interceptor, which cannot then be set off by
    /// its own blast.
    oflaser: bool,
    max: i32,
    outgoing: bool,
    color: XColor,
}

#[derive(Clone, Copy, Default)]
struct City {
    alive: bool,
    x: i32,
    color: XColor,
}

#[derive(Clone, Copy, Default)]
struct Laser {
    alive: bool,
    x: i32,
    y: i32,
    /// Where it is aimed, which is also where it stops.
    endy: i32,
    oldx: i32,
    oldy: i32,
    oldx2: i32,
    oldy2: i32,
    velx: f32,
    vely: f32,
    fposx: f32,
    fposy: f32,
    len_mul: f32,
    color: XColor,
    target: usize,
}

/// Where the between-levels sequence has got to. Upstream writes it as one
/// function with `usleep` in the middle; each of those sleeps is one of these.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Playing,
    /// The "Level N Cleared" banner is up.
    Banner,
    /// Counting the surviving cities back in, one every half second.
    Awarding(usize),
    /// Sitting on GAME OVER.
    Dead,
    BonusCity,
    BonusRound,
}

struct Penetrate {
    draw_gc: Gc,
    erase_gc: Gc,
    font: Font,
    score_font: Font,
    /// The colour the banners are drawn in, which upstream took from the
    /// cities before hardcoding it.
    level_fg: XColor,
    /// The scores are drawn in this, and the drawing GC has by then been left
    /// on whatever colour the last warhead was.
    fg: Pixel,

    bgrowth: i32,
    lrate: i32,
    startlrate: i32,
    loop_: i64,
    score: i64,
    highscore: i64,
    next_bonus: i64,
    num_bonus: i64,
    bround: bool,
    last_laser: i64,
    gamez: i32,
    /// How wide the player's aim scatters. Smaller is better.
    aim: i32,
    econpersen: i32,
    choosypersen: i32,
    carefulpersen: i32,
    smart: bool,

    missile: Vec<Missile>,
    boom: Vec<Boom>,
    city: [City; NUM_CITIES],
    laser: Vec<Laser>,
    blive: [bool; NUM_CITIES],

    level: i32,
    lev_missiles: i32,
    lev_freq: i32,

    xlim: i32,
    ylim: i32,
    draw_reset: bool,
    pscale: i32,

    phase: Phase,
    /// Carried across the level-change steps, which upstream keeps on its
    /// stack because it never returns in the middle of them.
    liv: [bool; NUM_CITIES],
    sumlive: i32,
    freecity: bool,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let (xlim, ylim) = (d.width(), d.height());
    let mut pscale = 1;
    if xlim > 2560 || ylim > 2560 {
        pscale *= 3; /* Retina displays */
    }

    let fg = d.res.pixel("foreground");
    let bg = d.res.pixel("background");

    let mut city = [City::default(); NUM_CITIES];
    for m in city.iter_mut() {
        m.alive = true;
        m.color = XColor::from_rgb16(0xFFFF, 0x8888, 0x1111);
    }

    let mut bgrowth = d.res.int("bgrowth");
    let mut lrate = d.res.int("lrate");
    if bgrowth < 0 {
        bgrowth = 2;
    }
    if lrate < 0 {
        lrate = 2;
    }

    let mut st = Penetrate {
        draw_gc: Gc::new(fg, bg),
        erase_gc: Gc::new(bg, bg),
        font: Font::load("monospace 38"),
        score_font: Font::load("sans-serif 18"),
        // Level colour was city[0].color, which upstream then hardcoded.
        level_fg: XColor::from_rgb16(0xFFFF, 0x8888, 0x1111),
        fg,
        bgrowth,
        lrate,
        startlrate: lrate,
        loop_: 0,
        score: 0,
        highscore: 0,
        next_bonus: FIRST_BONUS,
        num_bonus: 0,
        bround: false,
        last_laser: 0,
        gamez: 0,
        aim: 180,
        econpersen: 0,
        choosypersen: 0,
        carefulpersen: 0,
        smart: d.res.bool("smart"),
        missile: vec![Missile::default(); MAX_MISSILES],
        boom: vec![Boom::default(); MAX_BOOMS],
        city,
        laser: vec![Laser::default(); MAX_LASERS],
        blive: [false; NUM_CITIES],
        level: 0,
        lev_missiles: 0,
        lev_freq: 1,
        xlim,
        ylim,
        draw_reset: false,
        pscale,
        phase: Phase::Playing,
        liv: [false; NUM_CITIES],
        sumlive: 0,
        freecity: false,
    };
    st.level_fg = XColor::from_rgb16(0xFFFF, 0x8888, 0x1111);
    d.clear_window();
    Box::new(st)
}

impl Penetrate {
    fn explode(&mut self, x: i32, y: i32, max: i32, color: XColor, oflaser: bool) {
        let Some(m) = self.boom.iter_mut().find(|b| !b.alive) else {
            return;
        };
        m.alive = true;
        m.x = x;
        m.y = y;
        m.rad = 0;
        m.max = max.min(MAX_RADIUS);
        m.outgoing = true;
        m.color = color;
        m.oflaser = oflaser;
    }

    /// Send a warhead down at a city. `src` is set when this one budded off
    /// another halfway down.
    fn launch(&mut self, src: Option<usize>) {
        let (xlim, ylim) = (self.xlim, self.ylim);
        let Some(i) = self.missile.iter().position(|m| !m.alive) else {
            return;
        };

        let mut m = Missile {
            alive: true,
            startx: random_below(xlim),
            starty: 0,
            endy: ylim,
            pos: 0.0,
            jenis: random_below(360),
            splits: 0,
            ..Missile::default()
        };
        if m.jenis < 50 {
            let j = (f32::from(ylim as i16) * 0.4) as i32;
            if j != 0 {
                m.splits = random_below(j);
                if (m.splits as f32) < ylim as f32 * 0.08 {
                    m.splits = 0;
                }
            }
        }

        /* special if we're from another missile */
        if let Some(src) = src {
            let msrc = self.missile[src];
            let mut dc = random_below(NUM_CITIES as i32 - 1) as usize;
            if dc == msrc.dcity {
                dc += 1;
            }
            m.dcity = dc;
            m.startx = msrc.x;
            m.starty = msrc.y;
            if m.starty as f32 > ylim as f32 * 0.4 || m.splits <= m.starty {
                m.splits = 0; /* too far down already */
            }
            m.jenis = msrc.jenis;
        } else {
            m.dcity = random_below(NUM_CITIES as i32) as usize;
        }
        m.endx = self.city[m.dcity].x + random_below(20) - 10;
        m.x = m.startx;
        m.y = m.starty;
        m.enemies = 0;

        let (r, g, b) = hsv_to_rgb(m.jenis, 1.0, 1.0);
        m.color = XColor::from_rgb16(r, g, b);
        self.missile[i] = m;
    }

    /// Pick a warhead and put an interceptor on it. The three habits below are
    /// what the player learns; each is rolled against its own percentage every
    /// time it shoots.
    fn fire(&mut self) -> bool {
        let (xlim, ylim) = (self.xlim, self.ylim);
        let ytargetmin = (ylim as f32 * 0.75) as i32;

        let mut choosy = random_below(100) < self.choosypersen;
        let economic = random_below(100) < self.econpersen;
        let careful = random_below(100) < self.carefulpersen;

        let livecity = self.city.iter().filter(|c| c.alive).count() as i32;
        if livecity == 0 {
            return true; /* no guns */
        }
        let Some(li) = self.laser.iter().position(|l| !l.alive) else {
            return true;
        };

        /* if no missiles on target, no need to be choosy */
        if choosy {
            let choo = self
                .missile
                .iter()
                .filter(|m| m.alive && m.y <= ytargetmin && self.city[m.dcity].alive)
                .count();
            if choo == 0 {
                choosy = false;
            }
        }

        let mut suitor = [false; MAX_MISSILES];
        let mut cnt = 0;
        for (j, mis) in self.missile.iter().enumerate() {
            if !mis.alive || mis.y > ytargetmin {
                continue;
            }
            if choosy && !self.city[mis.dcity].alive {
                continue;
            }
            let ey = mis.starty as f32
                + (mis.endy - mis.starty) as f32
                    * (mis.pos + EXP_HELP + (1.0 - mis.pos) / SPEED_DIFF);
            if ey > ylim as f32 * MAX_TO_GROUND {
                continue; /* too far down */
            }
            cnt += 1;
            suitor[j] = true;
        }

        /* count missiles that are on target and not being targeted */
        let mut untargeted = 0;
        if choosy && economic {
            for (j, s) in suitor.iter().enumerate() {
                if *s && self.missile[j].enemies == 0 {
                    untargeted += 1;
                }
            }
        }

        let mut deepest = 0;
        if economic {
            for (j, s) in suitor.iter_mut().enumerate() {
                if *s
                    && cnt > 1
                    && self.missile[j].enemies > 0
                    && (self.missile[j].enemies > 1 || untargeted == 0)
                {
                    *s = false;
                    cnt -= 1;
                }
                /* who's closest? biggest threat */
                if *s && self.missile[j].y > deepest {
                    deepest = self.missile[j].y;
                }
            }
        }

        if deepest > 0 && careful {
            /* only target deepest missile */
            cnt = 1;
            for (j, s) in suitor.iter_mut().enumerate() {
                if *s && self.missile[j].y != deepest {
                    *s = false;
                }
            }
        }

        if cnt == 0 {
            return true; /* no targets available */
        }
        let mut pick = random_below(cnt);
        let mut misnum = None;
        for (j, s) in suitor.iter().enumerate() {
            if *s {
                if pick == 0 {
                    misnum = Some(j);
                    break;
                }
                pick -= 1;
            }
        }
        let Some(misnum) = misnum else {
            return true; /* shouldn't happen */
        };
        let mis = self.missile[misnum];

        let mut dcity = random_below(livecity);
        let mut from = 0;
        for (j, c) in self.city.iter().enumerate() {
            if c.alive {
                if dcity == 0 {
                    from = j;
                    break;
                }
                dcity -= 1;
            }
        }

        let lead = mis.pos + EXP_HELP + (1.0 - mis.pos) / SPEED_DIFF;
        let ex = mis.startx as f32 + (mis.endx - mis.startx) as f32 * lead;
        let ey = mis.starty as f32 + (mis.endy - mis.starty) as f32 * lead;
        let endx = ex as i32 + random_below(16) - 8 + random_below(self.aim) - self.aim / 2;
        let endy = ey as i32 + random_below(16) - 8 + random_below(self.aim) - self.aim / 2;
        if ey > ylim as f32 * MAX_TO_GROUND {
            return false; /* too far down */
        }
        self.missile[misnum].enemies += 1;

        let startx = self.city[from].x;
        let starty = ylim;
        let velx = (endx - startx) as f32 / 100.0;
        let vely = (endy - starty) as f32 / 100.0;
        self.laser[li] = Laser {
            alive: true,
            target: misnum,
            endy,
            x: startx,
            y: starty,
            oldx: -1,
            oldy: -1,
            oldx2: -1,
            oldy2: -1,
            fposx: startx as f32,
            fposy: starty as f32,
            velx,
            vely,
            len_mul: -(LASER_LENGTH / vely),
            color: XColor::from_rgb16(0xFFFF, 0xFFFF, 0x0000),
        };
        let _ = xlim;
        true
    }

    fn draw_score(&mut self, d: &mut Dpy) {
        let (xlim, ylim) = (self.xlim, self.ylim);
        let height = self.score_font.ascent() + self.score_font.descent();

        let buf = format!("{}", self.score);
        let width = self.score_font.text_width(&buf);
        d.win().fill_rectangle(
            &self.erase_gc,
            xlim - width - 6,
            ylim - height - 2,
            width + 6,
            height + 2,
        );
        let font = self.score_font;
        let mut gc = self.draw_gc.clone();
        gc.set_foreground(self.fg);
        d.win()
            .draw_string(&gc, &font, xlim - width - 2, ylim - 2, &buf);

        let buf = format!("{}", self.highscore);
        let width = self.score_font.text_width(&buf);
        d.win()
            .fill_rectangle(&self.erase_gc, 4, ylim - height - 2, width + 4, height + 2);
        d.win().draw_string(&gc, &font, 4, ylim - 2, &buf);
    }

    fn add_score(&mut self, d: &mut Dpy, dif: i64) {
        if !self.city.iter().any(|c| c.alive) {
            return; /* no cities, not possible to score */
        }
        self.score += dif;
        if self.score > self.highscore {
            self.highscore = self.score;
        }
        self.draw_score(d);
    }

    fn draw_city(&mut self, d: &mut Dpy, x: i32, y: i32, col: XColor) {
        let p = self.pscale;
        self.draw_gc.set_foreground(col.pixel);
        d.win()
            .fill_rectangle(&self.draw_gc, x - 30 * p, y - 40 * p, 60 * p, 40 * p);
        d.win()
            .fill_rectangle(&self.draw_gc, x - 20 * p, y - 50 * p, 10 * p, 10 * p);
        d.win()
            .fill_rectangle(&self.draw_gc, x + 10 * p, y - 50 * p, 10 * p, 10 * p);
    }

    fn draw_cities(&mut self, d: &mut Dpy) {
        let (xlim, ylim) = (self.xlim, self.ylim);
        for i in 0..NUM_CITIES {
            if !self.city[i].alive {
                continue;
            }
            let x = (i as i32 + 1) * (xlim / (NUM_CITIES as i32 + 1));
            self.city[i].x = x;
            let col = self.city[i].color;
            self.draw_city(d, x, ylim, col);
        }
    }

    /// Centre one of the banners.
    fn banner(&mut self, d: &mut Dpy, text: &str, y: i32) {
        let width = self.font.text_width(text);
        let (font, mut gc) = (self.font, self.draw_gc.clone());
        gc.set_foreground(self.level_fg.pixel);
        d.win()
            .draw_string(&gc, &font, self.xlim / 2 - width / 2, y, text);
    }

    fn loop_missiles(&mut self, d: &mut Dpy) {
        let (xlim, ylim) = (self.xlim, self.ylim);
        let p = self.pscale;
        for i in 0..MAX_MISSILES {
            if !self.missile[i].alive {
                continue;
            }
            let (old_x, old_y) = (self.missile[i].x, self.missile[i].y);
            {
                let m = &mut self.missile[i];
                m.pos += MISSILE_SPEED;
                m.x = m.startx + ((m.endx - m.startx) as f32 * m.pos) as i32;
                m.y = m.starty + ((m.endy - m.starty) as f32 * m.pos) as i32;
            }
            let m = self.missile[i];

            self.draw_gc.set_line_width(4 * p);
            self.draw_gc.set_foreground(m.color.pixel);
            d.win().draw_line(&self.draw_gc, old_x, old_y, m.x, m.y);

            /* maybe split off a new missile? */
            if self.missile[i].splits != 0 && m.y > self.missile[i].splits {
                self.missile[i].splits = 0;
                self.launch(Some(i));
            }

            let mut max = 0;
            if m.y >= ylim {
                self.missile[i].alive = false;
                if self.city[m.dcity].alive {
                    self.city[m.dcity].alive = false;
                    self.explode(m.x, m.y, BOOM_RAD * 2, m.color, false);
                }
            }

            /* check hitting explosions */
            for j in 0..MAX_BOOMS {
                let b = self.boom[j];
                if !b.alive {
                    continue;
                }
                let dx = (self.missile[i].x - b.x).abs();
                let dy = (self.missile[i].y - b.y).abs();
                let r = b.rad + 2 * p;
                if dx < r && dy < r && dx * dx + dy * dy < r * r {
                    self.missile[i].alive = false;
                    max = b.max + self.bgrowth - BOOM_RAD;
                    self.add_score(d, SCORE_MISSILE);
                }
            }

            if !self.missile[i].alive {
                /* we just died */
                let m = self.missile[i];
                self.explode(m.x, m.y, BOOM_RAD + max, m.color, false);
                self.erase_gc.set_line_width(4 * p);
                // In a perfect world we could erase one line from the start to
                // here. This is not a perfect world: the track was drawn a
                // segment at a time and has to come up the same way.
                let (mut old_x, mut old_y) = (m.startx, m.starty);
                let mut my_pos = MISSILE_SPEED;
                while my_pos <= m.pos {
                    let x = m.startx + ((m.endx - m.startx) as f32 * my_pos) as i32;
                    let y = m.starty + ((m.endy - m.starty) as f32 * my_pos) as i32;
                    d.win().draw_line(&self.erase_gc, old_x, old_y, x, y);
                    old_x = x;
                    old_y = y;
                    my_pos += MISSILE_SPEED;
                }
                self.missile[i].x = old_x;
                self.missile[i].y = old_y;
            }
        }
        let _ = xlim;
    }

    fn loop_lasers(&mut self, d: &mut Dpy) {
        let ylim = self.ylim;
        let p = self.pscale;
        let miny = (ylim as f32 * 0.8) as i32;
        for i in 0..MAX_LASERS {
            if !self.laser[i].alive {
                continue;
            }
            let m = self.laser[i];
            if m.oldx != -1 {
                self.erase_gc.set_line_width(2 * p);
                d.win()
                    .draw_line(&self.erase_gc, m.oldx2, m.oldy2, m.oldx, m.oldy);
            }

            let (x, y);
            {
                let m = &mut self.laser[i];
                m.fposx += m.velx;
                m.fposy += m.vely;
                m.x = m.fposx as i32;
                m.y = m.fposy as i32;
                x = (m.fposx + (-m.velx * m.len_mul)) as i32;
                y = (m.fposy + (-m.vely * m.len_mul)) as i32;
                m.oldx = x;
                m.oldy = y;
            }
            let m = self.laser[i];

            self.draw_gc.set_line_width(2 * p);
            self.draw_gc.set_foreground(m.color.pixel);
            d.win().draw_line(&self.draw_gc, m.x, m.y, x, y);

            {
                let m = &mut self.laser[i];
                m.oldx2 = m.x;
                m.oldy2 = m.y;
                m.oldx = x;
                m.oldy = y;
                if m.y < m.endy {
                    m.alive = false;
                }
            }

            /* check hitting explosions */
            if self.laser[i].y < miny {
                for j in 0..MAX_BOOMS {
                    let b = self.boom[j];
                    if !b.alive || b.oflaser {
                        continue;
                    }
                    let dx = (self.laser[i].x - b.x).abs();
                    let dy = (self.laser[i].y - b.y).abs();
                    let r = b.rad + 2 * p;
                    if dx < r && dy < r && dx * dx + dy * dy < r * r {
                        self.laser[i].alive = false;
                        // One less enemy on that warhead: this one probably
                        // did not make it.
                        let t = self.laser[i].target;
                        if self.missile[t].alive {
                            self.missile[t].enemies -= 1;
                        }
                    }
                }
            }

            if !self.laser[i].alive {
                /* we just died */
                let m = self.laser[i];
                d.win().draw_line(&self.erase_gc, m.x, m.y, x, y);
                self.explode(m.x, m.y, BOOM_RAD, m.color, true);
            }
        }
    }

    fn loop_booms(&mut self, d: &mut Dpy) {
        let p = self.pscale;
        if self.loop_ & 1 == 0 {
            return;
        }
        for i in 0..MAX_BOOMS {
            let m = self.boom[i];
            if !m.alive {
                continue;
            }
            if m.outgoing {
                let m = &mut self.boom[i];
                m.rad += 1;
                if m.rad >= m.max {
                    m.outgoing = false;
                }
                let (x, y, rad, col) = (m.x, m.y, m.rad, m.color);
                self.draw_gc.set_line_width(p);
                self.draw_gc.set_foreground(col.pixel);
                d.win().draw_arc(
                    &self.draw_gc,
                    x - rad * p,
                    y - rad * p,
                    rad * 2 * p,
                    rad * 2 * p,
                    0,
                    360 * 64,
                );
            } else {
                let (x, y, rad) = (m.x, m.y, m.rad);
                self.erase_gc.set_line_width(p);
                d.win().draw_arc(
                    &self.erase_gc,
                    x - rad * p,
                    y - rad * p,
                    rad * 2 * p,
                    rad * 2 * p,
                    0,
                    360 * 64,
                );
                let m = &mut self.boom[i];
                m.rad -= 1;
                if m.rad <= 0 {
                    m.alive = false;
                }
            }
        }
    }

    /// After they die, let's change a few things.
    fn improve(&mut self) {
        if self.smart || self.level > 20 {
            return; /* no need, really */
        }
        self.aim -= 4;
        if self.level <= 2 {
            self.aim -= 8;
        }
        if self.level <= 5 {
            self.aim -= 6;
        }
        if self.gamez < 3 {
            self.aim -= 10;
        }
        self.carefulpersen += 6;
        self.choosypersen += 4;
        if self.level <= 5 {
            self.choosypersen += 3;
        }
        self.econpersen += 4;
        self.lrate -= 2;
        if self.startlrate < MIN_RATE {
            if self.lrate < self.startlrate {
                self.lrate = self.startlrate;
            }
        } else if self.lrate < MIN_RATE {
            self.lrate = MIN_RATE;
        }
        if self.level <= 5 {
            self.econpersen += 3;
        }
        self.aim = self.aim.max(1);
        self.choosypersen = self.choosypersen.min(100);
        self.carefulpersen = self.carefulpersen.min(100);
        self.econpersen = self.econpersen.min(100);
    }

    /// The head of `NewLevel`, down to the first sleep. Returns how long to
    /// hold the banner for, or zero when there is no banner to hold.
    fn new_level(&mut self, d: &mut Dpy) -> u32 {
        let (xlim, ylim) = (self.xlim, self.ylim);
        if self.level == 0 {
            self.level += 1;
            self.end_level();
            return 0;
        }

        /* check for a free city */
        self.freecity = false;
        if self.score >= self.next_bonus {
            self.num_bonus += 1;
            self.next_bonus += FIRST_BONUS * self.num_bonus;
            self.freecity = true;
        }

        self.sumlive = 0;
        for i in 0..NUM_CITIES {
            if self.bround {
                self.city[i].alive = self.blive[i];
            }
            self.liv[i] = self.city[i].alive;
            self.sumlive += i32::from(self.liv[i]);
            if !self.bround {
                self.city[i].alive = false;
            }
        }

        /* print out screen */
        d.win().fill_rectangle(&self.erase_gc, 0, 0, xlim, ylim);
        let buf = if self.bround {
            "Bonus Round Over".to_string()
        } else if self.sumlive != 0 || self.freecity {
            format!("Level {} Cleared", self.level)
        } else {
            "GAME OVER".to_string()
        };
        let h = self.font.ascent() + self.font.descent();
        self.banner(d, &buf, ylim / 2 - h / 2);
        self.phase = Phase::Banner;
        1_000_000
    }

    /// The step after the banner: either start counting cities back in, or sit
    /// on GAME OVER, or go straight on.
    fn after_banner(&mut self, d: &mut Dpy) -> u32 {
        let (xlim, ylim) = (self.xlim, self.ylim);
        if self.bround {
            return self.after_cities(d);
        }
        if self.sumlive != 0 || self.freecity {
            /* draw live cities */
            let p = self.pscale;
            d.win()
                .fill_rectangle(&self.erase_gc, 0, ylim - 100 * p, xlim, 100 * p);

            let buf = format!("X {}", i64::from(self.level) * 100);
            /* how much they get, plus the width of a city and a spacer */
            let sumwidth = self.font.text_width(&buf) + 60 + 40;
            let col = self.city[0].color;
            let cy = (ylim as f32 * 0.70) as i32;
            self.draw_city(d, xlim / 2 - sumwidth / 2 + 30, cy, col);
            let (font, mut gc) = (self.font, self.draw_gc.clone());
            gc.set_foreground(self.level_fg.pixel);
            d.win().draw_string(
                &gc,
                &font,
                xlim / 2 - sumwidth / 2 + 40 + 60,
                (ylim as f32 * 0.7) as i32,
                &buf,
            );
            self.phase = Phase::Awarding(0);
            return self.award_city(d);
        }
        /* we're dead */
        self.phase = Phase::Dead;
        3_000_000
    }

    /// One surviving city counted back in, then a pause before the next.
    fn award_city(&mut self, d: &mut Dpy) -> u32 {
        let Phase::Awarding(mut i) = self.phase else {
            return 0;
        };
        while i < NUM_CITIES && !self.liv[i] {
            i += 1;
        }
        if i >= NUM_CITIES {
            return self.after_cities(d);
        }
        self.city[i].alive = true;
        self.add_score(d, 100 * i64::from(self.level));
        self.draw_cities(d);
        self.phase = Phase::Awarding(i + 1);
        CITY_PAUSE
    }

    /// After GAME OVER: a new game, with the player a little better than last
    /// time unless it was already playing perfectly.
    fn after_dead(&mut self, d: &mut Dpy) -> u32 {
        self.gamez += 1;
        self.improve();
        for c in self.city.iter_mut() {
            c.alive = true;
        }
        self.level = 0;
        self.loop_ = 1;
        self.score = 0;
        self.next_bonus = FIRST_BONUS;
        self.num_bonus = 0;
        self.draw_cities(d);
        self.after_cities(d)
    }

    /// The tail of `NewLevel`: the bonus city, then the bonus round.
    fn after_cities(&mut self, d: &mut Dpy) -> u32 {
        let (xlim, ylim) = (self.xlim, self.ylim);
        if self.freecity && self.sumlive < 5 {
            let mut ncnt = random_below(5 - self.sumlive) + 1;
            for i in 0..NUM_CITIES {
                if !self.city[i].alive {
                    ncnt -= 1;
                    if ncnt == 0 {
                        self.city[i].alive = true;
                    }
                }
            }
            self.banner(d, "Bonus City", ylim / 4);
            self.draw_cities(d);
            self.phase = Phase::BonusCity;
            return 1_000_000;
        }
        let _ = xlim;
        self.after_bonus_city(d)
    }

    fn after_bonus_city(&mut self, d: &mut Dpy) -> u32 {
        let (xlim, ylim) = (self.xlim, self.ylim);
        let p = self.pscale;
        d.win()
            .fill_rectangle(&self.erase_gc, 0, 0, xlim, ylim - 100 * p);

        if !self.bround {
            self.level += 1;
        }
        if self.level == 1 {
            self.next_bonus = FIRST_BONUS;
        }

        if self.level > 3 && self.level % 5 == 1 {
            if self.bround {
                self.bround = false;
                self.draw_cities(d);
            } else {
                /* bonus round */
                self.bround = true;
                self.lev_missiles = 20 + self.level * 10;
                self.lev_freq = 10;
                for i in 0..NUM_CITIES {
                    self.blive[i] = self.city[i].alive;
                }
                let h = self.font.ascent() + self.font.descent();
                self.banner(d, "Bonus Round", ylim / 2 - h / 2);
                self.phase = Phase::BonusRound;
                return 1_000_000;
            }
        }
        self.end_level();
        self.phase = Phase::Playing;
        0
    }

    fn after_bonus_round(&mut self, d: &mut Dpy) -> u32 {
        let (xlim, ylim) = (self.xlim, self.ylim);
        let p = self.pscale;
        d.win()
            .fill_rectangle(&self.erase_gc, 0, 0, xlim, ylim - 100 * p);
        self.end_level();
        self.phase = Phase::Playing;
        0
    }

    /// `END_LEVEL`: how much is coming and how fast.
    fn end_level(&mut self) {
        if !self.bround {
            self.lev_missiles = 5 + self.level * 3;
            if self.level > 5 {
                self.lev_missiles += self.level * 5;
            }
            self.lev_freq = (120 - self.level * 5).max(MIN_RATE);
        }
        /* ready to fire */
        self.last_laser = 0;
    }
}

impl Screenhack for Penetrate {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        // Pick up wherever the between-levels sequence left off.
        match self.phase {
            Phase::Playing => {}
            Phase::Banner => return self.after_banner(d),
            Phase::Awarding(_) => return self.award_city(d),
            Phase::Dead => return self.after_dead(d),
            Phase::BonusCity => return self.after_bonus_city(d),
            Phase::BonusRound => return self.after_bonus_round(d),
        }

        if self.draw_reset {
            self.draw_reset = false;
            self.draw_cities(d);
        }

        self.xlim = d.width();
        self.ylim = d.height();

        /* see if just started */
        if self.loop_ == 0 {
            if self.smart {
                self.choosypersen = 100;
                self.econpersen = 100;
                self.carefulpersen = 100;
                self.lrate = MIN_RATE;
                self.aim = 1;
            }
            let wait = self.new_level(d);
            self.draw_score(d);
            self.loop_ += 1;
            if wait != 0 {
                return wait;
            }
            return SLEEP_TIME;
        }

        self.loop_ += 1;

        if self.lev_missiles == 0 {
            let busy = self.missile.iter().any(|m| m.alive)
                || self.boom.iter().any(|b| b.alive)
                || self.laser.iter().any(|l| l.alive);
            if !busy {
                // Okay, nothing's alive: start the end of level countdown.
                self.phase = Phase::Playing;
                let wait = self.new_level(d);
                return if wait == 0 { LEVEL_PAUSE } else { wait };
            }
        } else if random_below(self.lev_freq.max(1)) == 0 {
            self.launch(None);
            self.lev_missiles -= 1;
        }

        if self.loop_ - self.last_laser >= i64::from(self.lrate) && self.fire() {
            self.last_laser = self.loop_;
        }

        if self.loop_ & 7 == 0 {
            self.draw_reset = true;
        }

        self.loop_missiles(d);
        self.loop_lasers(d);
        self.loop_booms(d);

        SLEEP_TIME
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.xlim = width;
        self.ylim = height;
        d.clear_window();
    }
}

const DEFAULTS: &[&str] = &[
    ".background:	black",
    ".foreground:	white",
    "*fpsTop:	true",
    "*fpsSolid:	true",
    "*bgrowth:	5",
    "*lrate:	80",
    "*smart:	False",
];

const MODES: &[SelectItem] = &[
    SelectItem {
        value: "False",
        label: "Start badly, but learn",
    },
    SelectItem {
        value: "True",
        label: "Always play well",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("bgrowth", "Explosions", 1.0, 20.0, 1.0, 0, "5"),
    Opt::slider("lrate", "Lasers", 10.0, 200.0, 5.0, 0, "80").inverted(),
    Opt::select("smart", "Skill", MODES, "False"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "penetrate",
    label: "Penetrate",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Adam Miller",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=iuutzMOVYgI"),
        blurb: "Something like the classic arcade game Missile Command.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
