//! Port of `hacks/glx/juggler3d.c`.
//!
//! ```text
//! Juggle3D, Copyright (c) 1996-2007 Tim Auckland <tda10.geo@yahoo.com>
//! and Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! "Juggler3D" Written by Tim Auckland
//! Rewritten in OpenGL by Jamie Zawinski
//!
//! Version 1.0 Nov 2007
//! ```
//!
//! A stick figure juggling, and most of the file is about deciding what to
//! juggle rather than about drawing it.
//!
//! Patterns are written in siteswap, where a number is how many beats a thrown
//! object stays in the air: a 3 comes down two throws later, a 5 four throws
//! later, and 0 is an empty hand. A pattern's average is how many objects it
//! takes. The library here is written in the Cambridge variant, in which a
//! number counts *throws to skip* rather than beats, which has the property
//! that any two patterns can be run one after the other and still make sense,
//! so the performance can be built by picking library patterns at random and
//! concatenating them. `adam` converts that back into heights.
//!
//! Turning a pattern into a picture is a pipeline, and every step is a pass
//! over the same list. `part` splits each throw into a throw and a catch and
//! gives each one a hand, alternating. `lob` walks forward from every throw to
//! find the catch that will receive it, which is what says which object is in
//! the air and where it will land, and threads the hands together the same
//! way. `positions` turns the symbols into times and places: how long a throw
//! takes, whether the hand is inside or outside or crossing. `projectile` puts
//! a parabola under every flight, or three of them for a bounce, or a fall,
//! a rest and a kick for a kick-up. `hands` fits a pair of cubic splines
//! through each hand's throw, catch and next throw, so the hand arrives moving
//! rather than teleporting. Only then is anything drawn.
//!
//! The arms are inverse kinematics of the simplest kind: given where the hand
//! should be, put the elbow where a two-link arm would have to bend, and if
//! the target is out of reach, point at it instead.
//!
//! Upstream's trail of past positions is not kept. Its X11 version used the
//! trail to erase what it had drawn, and the OpenGL version keeps appending to
//! it but only ever reads the newest entry, so the list grows forever and the
//! objects it belongs to are never freed. Only the newest position is kept
//! here, which draws the same picture. The `--tail` knob went with it: it sets
//! a field nothing reads.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_random_colormap};
use crate::runtime::gl::Shape;
use crate::runtime::shapes::unit_sphere;
use crate::runtime::texfont::TexFont;
use crate::runtime::tube::tube;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent,
    random_below, screenhack_event_helper,
};

/* Figure */
const ARMLENGTH: f32 = 50.0;
/// Shoulder width.
const SX: f64 = 25.0;

/// Build all the models assuming a 480px high scene.
const SCENE_HEIGHT: f64 = 480.0;

/// Inverse of the chances of using an odd object in the pattern.
const OBJMIXPROB: i32 = 20;

const BODY_COLOR_1: [f32; 4] = [0.9, 0.7, 0.5, 1.0];
const BODY_COLOR_2: [f32; 4] = [0.6, 0.4, 0.2, 1.0];

const BOUNCEOVER: i32 = 10;
const KICKMIN: i32 = 7;
const THROWMAX: i32 = 20;

/// A slot that is not filled in. Upstream uses a null pointer.
const NIL: usize = usize::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ObjType {
    Ball,
    Club,
    Knife,
    Ring,
    BBall,
}

impl ObjType {
    const ALL: [ObjType; 5] = [
        ObjType::Ball,
        ObjType::Club,
        ObjType::Knife,
        ObjType::Ring,
        ObjType::BBall,
    ];

    /// Length of the object's handle.
    fn handle(self) -> f64 {
        match self {
            ObjType::Ball | ObjType::BBall => 0.0,
            _ => 15.0,
        }
    }

    /// Coefficient of restitution. A perfect bounce is 1.
    fn cor(self) -> f64 {
        match self {
            ObjType::Ball => 0.9,
            /* Clubs don't bounce too well */
            ObjType::Club => 0.55,
            /* Knives don't bounce */
            ObjType::Knife => 0.0,
            ObjType::Ring => 0.8,
            ObjType::BBall => 0.2,
        }
    }

    /// Heavier objects don't get thrown as high.
    fn weight(self) -> f64 {
        match self {
            ObjType::BBall => 5.0,
            _ => 1.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Status {
    Atch,
    Thratch,
    Action,
    LinkedAction,
    Pthratch,
    Bpredictor,
    Predictor,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Throwable {
    Empty,
    Full,
    Ball,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hand {
    Left = 0,
    Right = 1,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    Throw,
    Catch,
}

/// `x + t*(c + t*(b + t*a))`, upstream's `CUBIC`.
#[derive(Clone, Copy, Default, Debug)]
struct Spline {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
}

impl Spline {
    fn at(&self, t: f64) -> f64 {
        ((self.a * t + self.b) * t + self.c) * t + self.d
    }
}

#[derive(Clone, Copy, Default, Debug)]
struct Point {
    x: f64,
    y: f64,
}

/// An arbitrary object being juggled. Each trajectory references one, and the
/// count is how many do.
#[derive(Clone)]
struct Object {
    prev: usize,
    next: usize,
    ty: ObjType,
    color: usize,
    /// Reference count.
    count: i32,
    /// The object is in use this frame.
    active: bool,
    /// Where it was last seen. Upstream keeps a whole trail and reads only
    /// this.
    last: Trace,
}

#[derive(Clone, Copy, Default)]
struct Trace {
    x: f64,
    y: f64,
    angle: f64,
    divisions: i32,
}

/// A segment of juggling action. A list of these is the performance, and it
/// goes through the pipeline a stage at a time.
#[derive(Clone)]
struct Traj {
    prev: usize,
    next: usize,
    status: Status,

    /* Throw */
    posn: u8,
    height: i32,
    adam: i32,
    pattern: Option<String>,
    name: Option<String>,

    /* Action */
    hand: Hand,
    action: Action,

    /* LinkedAction */
    object: usize,
    divisions: i32,
    angle: f64,
    spin: f64,
    balllink: usize,
    handlink: usize,

    /* PThratch */
    /// Moving juggler.
    cx: f64,
    x: f64,
    y: f64,
    dx: f64,
    dy: f64,

    /* Predictor */
    ty: Throwable,
    start: i64,
    finish: i64,
    xp: Spline,
    yp: Spline,
}

impl Default for Traj {
    fn default() -> Traj {
        Traj {
            prev: NIL,
            next: NIL,
            status: Status::Atch,
            posn: b' ',
            height: 0,
            adam: 0,
            pattern: None,
            name: None,
            hand: Hand::Left,
            action: Action::Throw,
            object: NIL,
            divisions: 0,
            angle: 0.0,
            spin: 0.0,
            balllink: NIL,
            handlink: NIL,
            cx: 0.0,
            x: 0.0,
            y: 0.0,
            dx: 0.0,
            dy: 0.0,
            ty: Throwable::Empty,
            start: 0,
            finish: 0,
            xp: Spline::default(),
            yp: Spline::default(),
        }
    }
}

/// A doubly-linked list in a vector, which is how upstream's `calloc` and
/// pointer chasing come across. Slot zero is the head that the list runs
/// round; freed slots go on a list and are handed out again, so a run that
/// never ends does not grow without bound.
struct Ring<T> {
    v: Vec<T>,
    free: Vec<usize>,
}

/// Every animal in the list has these, so the ring can splice without knowing
/// what it holds.
trait Linked: Default + Clone {
    fn links(&mut self) -> (&mut usize, &mut usize);
    fn prev(&self) -> usize;
    fn next(&self) -> usize;
}

impl Linked for Traj {
    fn links(&mut self) -> (&mut usize, &mut usize) {
        (&mut self.prev, &mut self.next)
    }
    fn prev(&self) -> usize {
        self.prev
    }
    fn next(&self) -> usize {
        self.next
    }
}

impl Default for Object {
    fn default() -> Object {
        Object {
            prev: NIL,
            next: NIL,
            ty: ObjType::Ball,
            color: 0,
            count: 0,
            active: false,
            last: Trace::default(),
        }
    }
}

impl Linked for Object {
    fn links(&mut self) -> (&mut usize, &mut usize) {
        (&mut self.prev, &mut self.next)
    }
    fn prev(&self) -> usize {
        self.prev
    }
    fn next(&self) -> usize {
        self.next
    }
}

impl<T: Linked> Ring<T> {
    /// A list of nothing but its own head, which is upstream's
    /// `ADD_ELEMENT(t, sp->head, sp->head)`.
    fn new() -> Ring<T> {
        let mut head = T::default();
        {
            let (p, n) = head.links();
            *p = 0;
            *n = 0;
        }
        Ring {
            v: vec![head],
            free: Vec::new(),
        }
    }

    /// `ADD_ELEMENT`: a new element after `at`.
    fn add_after(&mut self, at: usize) -> usize {
        let i = match self.free.pop() {
            Some(i) => {
                self.v[i] = T::default();
                i
            }
            None => {
                self.v.push(T::default());
                self.v.len() - 1
            }
        };
        let after = self.v[at].next();
        {
            let (p, n) = self.v[i].links();
            *p = at;
            *n = after;
        }
        *self.v[at].links().1 = i;
        *self.v[after].links().0 = i;
        i
    }

    /// `REMOVE`: unlink and give the slot back.
    fn remove(&mut self, i: usize) {
        debug_assert!(i != 0, "the head is never removed");
        let (p, n) = (self.v[i].prev(), self.v[i].next());
        *self.v[p].links().1 = n;
        *self.v[n].links().0 = p;
        self.v[i] = T::default();
        self.free.push(i);
    }

    /// How many elements the list holds, not counting its head.
    #[cfg(test)]
    fn live(&self) -> usize {
        self.v.len() - self.free.len() - 1
    }
}

/* Pattern Library */

/// List of popular patterns, in any order.
///
/// Patterns should be given in Adam notation so the generator can concatenate
/// them safely. Null descriptions are ok. Height notation will be displayed
/// automatically.
const PORTFOLIO: &[(&str, &str)] = &[
    ("[+2 1]", "Typical 2 ball juggler"),
    ("[2 0]", "2 in 1 hand"),
    ("[2 0 1]", ""),
    ("[+2 0 +2 0 0]", ""),
    ("[+2 0 1 2 2]", ""),
    ("[2 0 1 1]", ""),
    ("[3]", "3 cascade"),
    ("[+3]", "reverse 3 cascade"),
    ("[=3]", "cascade 3 under arm"),
    ("[&3]", "cascade 3 catching under arm"),
    ("[_3]", "bouncing 3 cascade"),
    ("[+3 x3 =3]", "Mill's mess"),
    ("[3 2 1]", ""),
    ("[3 3 1]", ""),
    ("[3 1 2]", "See-saw"),
    ("[=3 3 1 2]", ""),
    ("[=3 2 2 3 1 2]", "=4 5 1 2 stretched"),
    ("[+3 3 1 3]", "anemic shower box"),
    ("[3 3 1]", ""),
    ("[+3 2 3]", ""),
    ("[+3 1]", "3 shower"),
    ("[_3 1]", "bouncing 3 shower"),
    ("[3 0 3 0 3]", "shake 3 out of 5"),
    ("[3 3 3 0 0]", "flash 3 out of 5"),
    ("[3 3 0]", "complete waste of a 5 ball juggler"),
    ("[3 3 3 0 0 0 0]", "3 flash"),
    ("[+3 0 +3 0 +3 0 0]", ""),
    ("[3 2 2 0 3 2 0 2 3 0 2 2 0]", ""),
    ("[3 0 2 0]", ""),
    ("[_3 2 1]", ""),
    ("[_3 0 1]", ""),
    ("[1 _3 1 _3 0 1 _3 0]", ""),
    ("[_3 2 1 _3 1 2 1]", ""),
    ("[4]", "4 cascade"),
    ("[+4 3]", "4 ball half shower"),
    ("[4 4 2]", ""),
    ("[+4 4 4 +4]", "4 columns"),
    ("[+4 3 +4]", ""),
    ("[4 3 4 4]", ""),
    ("[4 3 3 4]", ""),
    ("[4 3 2 4", ""),
    ("[+4 1]", "4 shower"),
    ("[4 4 4 4 0]", "learning 5"),
    ("[+4 x4 =4]", "Mill's mess for 4"),
    ("[+4 2 1 3]", ""),
    ("[4 4 1 4 1 4]", ""),
    ("[_4 _4 _4 1 _4 1]", ""),
    ("[_4 3 3]", ""),
    ("[_4 3 1]", ""),
    ("[_4 2 1]", ""),
    ("[_4 3 3 3 0]", ""),
    ("[_4 1 3 1]", ""),
    ("[_4 1 3 1 2]", ""),
    ("[5]", "5 cascade"),
    ("[_5 _5 _5 _5 _5 5 5 5 5 5]", ""),
    ("[+5 x5 =5]", "Mill's mess for 5"),
    ("[5 4 4]", ""),
    ("[_5 4 4]", ""),
    ("[1 2 3 4 5 5 5 5 5]", "5 ramp"),
    ("[5 4 5 3 1]", ""),
    ("[_5 4 1 +4]", ""),
    ("[_5 4 +4 +4]", ""),
    ("[_5 4 4 4 1]", ""),
    ("[_5 4 4 5 1]", ""),
    ("[_5 4 4 +4 4 0]", ""),
    ("[6]", "6 cascade"),
    ("[+6 5]", ""),
    ("[6 4]", ""),
    ("[+6 3]", ""),
    ("[6 5 4 4]", ""),
    ("[+6 5 5 5]", ""),
    ("[6 0 6]", ""),
    ("[_6 0 _6]", ""),
    ("[_7]", "bouncing 7 cascade"),
    ("[7]", "7 cascade"),
    ("[7 6 6 6 6]", "Gatto's High Throw"),
];

/// Where the patterns for each ball count start in the sorted portfolio, and
/// how many there are.
#[derive(Clone, Copy, Default)]
struct PatternIndex {
    start: usize,
    number: usize,
}

fn get_num_balls(j: &str) -> i32 {
    let mut balls = 0;
    let mut h = 0;
    for c in j.bytes() {
        if c.is_ascii_digit() {
            h = 10 * h + (c - b'0') as i32;
        } else {
            if h > balls {
                balls = h;
            }
            h = 0;
        }
    }
    balls
}

struct Juggle {
    rot: Rotator,
    trackball: Trackball,
    font: TexFont,

    scale: f64,
    cx: f64,
    gr: f64,
    trajs: Ring<Traj>,
    objects: Ring<Object>,
    /// Two frames of three joints a side: shoulder, elbow, hand.
    arm: [[[Point; 3]; 2]; 2],
    /// The pattern being juggled, as it is written on screen.
    pattern: String,
    count: f64,
    num_balls: i32,
    /// Millisecond timer.
    time: i64,
    objtypes: ObjType,
    colors: Vec<XColor>,
    sorted: Vec<(&'static str, &'static str)>,
    index: Vec<PatternIndex>,
    minballs: i32,
    maxballs: i32,
    cycles: i32,
    wire: bool,
    /// Which objects the user allows.
    allowed: Vec<ObjType>,
    scene_width: f64,
}

/// `THROW_CATCH_INTERVAL`, and the two intervals derived from it.
impl Juggle {
    fn throw_catch(&self) -> f64 {
        self.count
    }
    fn throw_null(&self) -> f64 {
        self.count * 0.5
    }
    fn catch_throw(&self) -> f64 {
        self.count * 0.2
    }

    fn choose_object(&self) -> ObjType {
        self.allowed[random_below(self.allowed.len() as i32) as usize]
    }

    /* Programming */

    fn add_throw(&mut self, ty: u8, h: i32, adam_notation: bool, name: Option<&str>) {
        let tail = self.trajs.v[0].prev;
        let i = self.trajs.add_after(tail);
        let t = &mut self.trajs.v[i];
        t.object = NIL;
        t.name = name.map(str::to_string);
        t.posn = ty;
        if adam_notation {
            t.adam = h;
            t.height = 0;
            t.status = Status::Atch;
        } else {
            t.height = h;
            t.status = Status::Thratch;
        }
    }

    /// Add a Thratch to the performance.
    fn program(&mut self, patn: &str, name: Option<&str>, cycles: i32) {
        let mut i = 0;
        while i < cycles {
            // `title` is the pattern name to be supplied to the first throw of
            // a sequence. If no name is given, use an empty title so that the
            // sequences are still delimited.
            let mut title: Option<String> = Some(name.unwrap_or("").to_string());
            let mut ty = b' ';
            let mut h = 0;
            let mut seen = false;
            let mut notation_adam = false;
            for &p in patn.as_bytes() {
                if p.is_ascii_digit() {
                    seen = true;
                    h = 10 * h + (p - b'0') as i32;
                    continue;
                }
                let mut nn = notation_adam;
                match p {
                    /* begin Adam notation */
                    b'[' => notation_adam = true,
                    /* Inside throw */
                    b'-' => ty = b' ',
                    /* Outside, cross, cross catch, both, bounce, kickup */
                    b'+' | b'=' | b'&' | b'x' | b'_' | b'k' => ty = p,
                    /* Lose ball, then fall through to end Adam notation */
                    b'*' | b']' | b' ' => {
                        if p == b'*' {
                            seen = true;
                            h = -1;
                        }
                        if p != b' ' {
                            nn = false;
                        }
                        if seen {
                            i += 1;
                            self.add_throw(ty, h, notation_adam, title.as_deref());
                            title = None;
                            ty = b' ';
                            h = 0;
                            seen = false;
                        }
                        notation_adam = nn;
                    }
                    /* Anything else is a typo in the pattern, which upstream
                    warns about and ignores. */
                    _ => {}
                }
            }
            if seen {
                /* end of sequence */
                self.add_throw(ty, h, notation_adam, title.as_deref());
            }
            i += 1;
        }
    }

    /// Convert Adam notation into heights.
    fn adam(&mut self) {
        let mut t = self.trajs.v[0].next;
        while t != 0 {
            if self.trajs.v[t].status == Status::Atch {
                let mut a = self.trajs.v[t].adam;
                self.trajs.v[t].height = 0;
                let mut p = self.trajs.v[t].next;
                while a > 0 {
                    if p == 0 {
                        /* Indicate end of processing for name() */
                        self.trajs.v[t].height = -9;
                        return;
                    }
                    let pv = &self.trajs.v[p];
                    if pv.status != Status::Atch || pv.adam < 0 || pv.adam >= a {
                        a -= 1;
                    }
                    self.trajs.v[t].height += 1;
                    p = self.trajs.v[p].next;
                }
                let tv = &mut self.trajs.v[t];
                if tv.height > BOUNCEOVER && tv.posn == b' ' {
                    /* high defaults can be bounced */
                    tv.posn = b'_';
                } else if tv.height < 3 && tv.posn == b'_' {
                    /* Can't bounce short throws. */
                    tv.posn = b' ';
                }
                if tv.height < KICKMIN && tv.posn == b'k' {
                    /* Can't kick short throws */
                    tv.posn = b' ';
                }
                if tv.height > THROWMAX {
                    /* Use kicks for ridiculously high throws */
                    tv.posn = b'k';
                }
                tv.status = Status::Thratch;
            }
            t = self.trajs.v[t].next;
        }
    }

    /// Discover converted heights and update the sequence title.
    fn name(&mut self) {
        let mut t = self.trajs.v[0].next;
        while t != 0 {
            if self.trajs.v[t].status == Status::Thratch && self.trajs.v[t].name.is_some() {
                let mut buffer = String::new();
                let mut p = t;
                loop {
                    if p == 0 || self.trajs.v[p].height < 0 {
                        /* end of reliable data */
                        return;
                    }
                    let pv = &self.trajs.v[p];
                    if pv.posn == b' ' {
                        buffer.push_str(&format!(" {}", pv.height));
                    } else {
                        buffer.push_str(&format!(" {}{}", pv.posn as char, pv.height));
                    }
                    if buffer.len() > 500 {
                        // otherwise this could eventually overflow. It'll be
                        // too big to display anyway.
                        break;
                    }
                    p = self.trajs.v[p].next;
                    if !(p != t && self.trajs.v[p].name.is_none()) {
                        break;
                    }
                }
                let name = self.trajs.v[t].name.take().unwrap();
                if !name.is_empty() {
                    buffer.push_str(&format!(", {name}"));
                }
                self.trajs.v[t].pattern = Some(buffer);
            }
            t = self.trajs.v[t].next;
        }
    }

    /// Split Thratch notation into explicit throws and catches. Usually a
    /// catch follows a throw in the same hand, but take care of special cases.
    ///
    /// `..n1..` becomes `.. LTn RT1 LC RC ..`, and `..nm..` becomes
    /// `.. LTn LC RTm RC ..`.
    fn part(&mut self) {
        let mut hand = if random_below(2) == 1 {
            Hand::Right
        } else {
            Hand::Left
        };

        let mut t = self.trajs.v[0].next;
        while t != 0 {
            if self.trajs.v[t].status > Status::Thratch {
                hand = self.trajs.v[t].hand;
            } else if self.trajs.v[t].status == Status::Thratch {
                /* plausibility check */
                {
                    let tv = &mut self.trajs.v[t];
                    if tv.height <= 2 && tv.posn == b'_' {
                        /* no short bounces */
                        tv.posn = b' ';
                    }
                    if tv.height <= 1 && (tv.posn == b'=' || tv.posn == b'&') {
                        /* 1's need close catches */
                        tv.posn = b' ';
                    }
                }

                /*         throw          catch    */
                let (posn, caught) = match self.trajs.v[t].posn {
                    b' ' => (b'-', b'+'),
                    b'+' => (b'+', b'-'),
                    b'=' => (b'=', b'+'),
                    b'&' => (b'+', b'='),
                    b'x' => (b'=', b'='),
                    b'_' => (b'_', b'-'),
                    b'k' => (b'k', b'k'),
                    _ => (b'=', b'+'),
                };
                self.trajs.v[t].posn = caught;

                hand = if hand == Hand::Left {
                    Hand::Right
                } else {
                    Hand::Left
                };
                let height = self.trajs.v[t].height;
                {
                    let tv = &mut self.trajs.v[t];
                    tv.status = Status::Action;
                    tv.hand = hand;
                    tv.action = Action::Catch;
                }

                let mut p = self.trajs.v[t].prev;
                if height == 1 && p != 0 {
                    /* '1's are thrown earlier than usual */
                    p = self.trajs.v[p].prev;
                }

                let nt = self.trajs.add_after(p);
                let n = &mut self.trajs.v[nt];
                n.object = NIL;
                n.status = Status::Action;
                n.action = Action::Throw;
                n.height = height;
                n.hand = hand;
                n.posn = posn;
            }
            t = self.trajs.v[t].next;
        }
    }

    /// Connect up throws and catches to figure out which ball goes where. Do
    /// the same with the juggler's hands.
    fn lob(&mut self, npixels: usize) {
        let mut t = self.trajs.v[0].next;
        while t != 0 {
            if self.trajs.v[t].status != Status::Action {
                t = self.trajs.v[t].next;
                continue;
            }
            if self.trajs.v[t].action == Action::Throw {
                if self.trajs.v[t].ty == Throwable::Empty {
                    /* Create new Object */
                    let tail = self.objects.v[0].prev;
                    let o = self.objects.add_after(tail);
                    let color = 1 + random_below((npixels as i32 - 2).max(1)) as usize;
                    /* Small chance of picking a random object instead of the
                    current theme. */
                    let ty = if random_below(OBJMIXPROB) == 0 {
                        self.choose_object()
                    } else {
                        self.objtypes
                    };
                    let ov = &mut self.objects.v[o];
                    ov.count = 1;
                    ov.active = false;
                    ov.color = color;
                    ov.ty = ty;
                    self.trajs.v[t].object = o;
                }

                /* Balls can change divisions at each throw */
                /* no, that looks stupid. -jwz */
                if self.trajs.v[t].divisions < 1 {
                    self.trajs.v[t].divisions = 2 * (random_below(2) + 1);
                }

                /* search forward for next catch in this hand */
                let mut p = self.trajs.v[t].next;
                while self.trajs.v[t].handlink == NIL {
                    if self.trajs.v[p].status < Status::Action || p == 0 {
                        return;
                    }
                    if self.trajs.v[p].action == Action::Catch
                        && self.trajs.v[p].hand == self.trajs.v[t].hand
                    {
                        self.trajs.v[t].handlink = p;
                    }
                    p = self.trajs.v[p].next;
                }

                if self.trajs.v[t].height > 0 {
                    let mut h = self.trajs.v[t].height - 1;

                    /* search forward for next ball catch */
                    let mut p = self.trajs.v[t].next;
                    while self.trajs.v[t].balllink == NIL {
                        if self.trajs.v[p].status < Status::Action || p == 0 {
                            self.trajs.v[t].handlink = NIL;
                            return;
                        }
                        if self.trajs.v[p].action == Action::Catch {
                            h -= 1;
                            if h < 1 {
                                /* caught: complete trajectory */
                                self.trajs.v[t].balllink = p;
                                self.trajs.v[p].ty = Throwable::Full;
                                self.dup_object(p, t);
                                self.trajs.v[p].angle = self.trajs.v[t].angle;
                                self.trajs.v[p].divisions = self.trajs.v[t].divisions;
                            }
                        }
                        p = self.trajs.v[p].next;
                    }
                }
                /* thrown */
                self.trajs.v[t].ty = Throwable::Empty;
            } else {
                /* search forward for next throw from this hand */
                let mut p = self.trajs.v[t].next;
                while self.trajs.v[t].handlink == NIL {
                    if self.trajs.v[p].status < Status::Action || p == 0 {
                        return;
                    }
                    if self.trajs.v[p].action == Action::Throw
                        && self.trajs.v[p].hand == self.trajs.v[t].hand
                    {
                        self.trajs.v[p].ty = self.trajs.v[t].ty; /* pass ball */
                        self.dup_object(p, t); /* pass object */
                        self.trajs.v[p].divisions = self.trajs.v[t].divisions;
                        self.trajs.v[t].handlink = p;
                    }
                    p = self.trajs.v[p].next;
                }
            }
            self.trajs.v[t].status = Status::LinkedAction;
            t = self.trajs.v[t].next;
        }
    }

    /// `DUP_OBJECT`: `n` takes a reference to whatever `t` is holding.
    fn dup_object(&mut self, n: usize, t: usize) {
        let o = self.trajs.v[t].object;
        self.trajs.v[n].object = o;
        if o != NIL {
            self.objects.v[o].count += 1;
        }
    }

    /// Clap when both hands are empty.
    fn clap(&mut self) {
        let mut t = self.trajs.v[0].next;
        while t != 0 {
            let tv = &self.trajs.v[t];
            let idle = tv.status == Status::LinkedAction
                && tv.action == Action::Catch
                && tv.ty == Throwable::Empty
                && tv.handlink != NIL
                && self.trajs.v[tv.handlink].height == 0;
            if idle {
                /* Completely idle hand */
                let mut p = self.trajs.v[t].next;
                while p != 0 {
                    let pv = &self.trajs.v[p];
                    if pv.status == Status::LinkedAction
                        && pv.action == Action::Catch
                        && pv.hand != self.trajs.v[t].hand
                    {
                        /* Next catch other hand */
                        if pv.ty == Throwable::Empty
                            && pv.handlink != NIL
                            && self.trajs.v[pv.handlink].height == 0
                        {
                            /* Also completely idle: move the first hand's
                            empty throw to meet the second hand's empty
                            catch. */
                            let link = self.trajs.v[t].handlink;
                            self.trajs.v[link].posn = b'^';
                            self.trajs.v[p].posn = b'^';
                        }
                        /* Only need first catch */
                        break;
                    }
                    p = self.trajs.v[p].next;
                }
            }
            t = self.trajs.v[t].next;
        }
    }

    /// Convert hand position symbols into actual time/space coordinates.
    fn positions(&mut self) {
        /* Make sure we're not lost in the past */
        let mut now = self.time as f64;
        let mut t = self.trajs.v[0].next;
        while t != 0 {
            let status = self.trajs.v[t].status;
            if status >= Status::Pthratch {
                now = self.trajs.v[t].start as f64;
            } else if status == Status::Action || status == Status::LinkedAction {
                // Allow ACTIONs to be annotated, but we won't mark them ready
                // for the next stage.
                let pose = SX / 2.0;

                /* time */
                if self.trajs.v[t].action == Action::Catch {
                    /* Throw-to-catch */
                    if self.trajs.v[t].ty == Throwable::Empty {
                        /* failed catch is short */
                        now += self.throw_null().trunc();
                    } else {
                        now += self.throw_catch().trunc();
                    }
                } else {
                    /* Catch-to-throw */
                    let o = self.trajs.v[t].object;
                    now += if o != NIL {
                        (self.catch_throw() * self.objects.v[o].ty.weight()).trunc()
                    } else {
                        self.catch_throw().trunc()
                    };
                }

                if self.trajs.v[t].start == 0 {
                    self.trajs.v[t].start = now as i64;
                } else {
                    /* Concatenated performances may need clock resync */
                    now = self.trajs.v[t].start as f64;
                }

                self.trajs.v[t].cx = 0.0;

                /* space */
                let mut yo = 90.0;

                /* Add room for the handle */
                let o = self.trajs.v[t].object;
                if self.trajs.v[t].action == Action::Catch && o != NIL {
                    yo -= self.objects.v[o].ty.handle();
                }

                let xo = match self.trajs.v[t].posn {
                    b'-' => SX - pose,
                    b'_' | b'k' | b'+' => SX + pose,
                    b'~' | b'=' => {
                        yo += pose;
                        -SX - pose
                    }
                    /* clap */
                    b'^' => {
                        yo += pose * 2.0;
                        0.0
                    }
                    _ => 0.0,
                };

                let tv = &mut self.trajs.v[t];
                tv.angle = -std::f64::consts::FRAC_PI_2;
                tv.x = tv.cx + if tv.hand == Hand::Left { xo } else { -xo };
                tv.y = yo;

                /* Only mark complete if it was already linked */
                if tv.status == Status::LinkedAction {
                    tv.status = Status::Pthratch;
                }
            }
            t = self.trajs.v[t].next;
        }
    }

    /// The spin rate for a trajectory. Different types of throw need different
    /// spins: a club has to arrive handle first, a ball can spin any way it
    /// likes.
    fn spinrate(&self, ty: ObjType, old: f64, dt: f64, height: i32, turns: i32, togo: f64) -> f64 {
        if ty.handle() != 0.0 {
            /* Clubs */
            (turns as f64 * 2.0 * std::f64::consts::PI + togo) / dt
        } else if height == 0 {
            /* Balls already spinning */
            old / 2.0
        } else {
            /* Balls */
            random_below(height * 10) as f64 / 20.0 / ty.weight() * 2.0 * std::f64::consts::PI / dt
        }
    }

    fn end_spin(&self, t: usize) -> f64 {
        let tv = &self.trajs.v[t];
        tv.angle + tv.spin * (tv.finish - tv.start) as f64
    }

    /// Set the initial angle of the catch following hand movement `t` to the
    /// final angle of the throw `n`, and the subsequent throw to the same.
    fn match_spins_on_catch(&mut self, t: usize, n: usize) {
        let link = self.trajs.v[t].balllink;
        let o = self.trajs.v[link].object;
        if o != NIL && self.objects.v[o].ty.handle() == 0.0 {
            let a = self.end_spin(n);
            self.trajs.v[link].angle = a;
            let hl = self.trajs.v[link].handlink;
            if hl != NIL {
                self.trajs.v[hl].angle = a;
            }
        }
    }

    fn find_bounce(&self, yo: f64, yf: f64, yc: f64, tc: f64, cor: f64) -> f64 {
        // tb = time to bounce, yt = height at catch time after one bounce.
        // One or three roots according to timing; find one by interval
        // bisection.
        let mut tb = tc;
        let mut dy = 0.0;
        let e = 1.0; /* permissible error in yc */
        let mut i = tc / 2.0;
        while i > 0.0001 {
            if tb == 0.0 {
                break;
            }
            dy = (yf - yo) / tb + self.gr / 2.0 * tb;
            let dt = tc - tb;
            let yt = -cor * dy * dt + self.gr / 2.0 * dt * dt + yf;
            if yt < yc + e {
                tb -= i;
            } else if yt > yc - e {
                tb += i;
            } else {
                break;
            }
            i /= 2.0;
        }
        if dy * self.throw_catch() < -200.0 {
            /* bounce too hard */
            tb = -1.0;
        }
        tb
    }

    fn new_predictor(&mut self, t: usize, start: i64, finish: i64, angle: f64) -> usize {
        let at = self.trajs.v[t].prev;
        let n = self.trajs.add_after(at);
        let o = self.trajs.v[t].object;
        self.trajs.v[n].object = o;
        if o != NIL {
            self.objects.v[o].count += 1;
        }
        let div = self.trajs.v[t].divisions;
        let nv = &mut self.trajs.v[n];
        nv.divisions = div;
        nv.ty = Throwable::Ball;
        nv.status = Status::Predictor;
        nv.start = start;
        nv.finish = finish;
        nv.angle = angle;
        n
    }

    /// Turn abstract timings into physically appropriate object trajectories.
    fn projectile(&mut self) {
        /* Floor height */
        let yf = 0.0;

        let mut t = self.trajs.v[0].next;
        while t != 0 {
            let next = self.trajs.v[t].next;
            if self.trajs.v[t].status != Status::Pthratch || self.trajs.v[t].action != Action::Throw
            {
                t = next;
                continue;
            }
            let link = self.trajs.v[t].balllink;
            if link == NIL {
                /* Zero Throw */
                self.trajs.v[t].status = Status::Bpredictor;
                t = next;
                continue;
            }
            if self.trajs.v[link].handlink == NIL {
                /* Incomplete */
                return;
            }
            if link == self.trajs.v[t].handlink {
                // '2' height: hold on to the ball. No need to consider
                // flourishes, `hands` does that anyway.
                self.trajs.v[t].ty = Throwable::Full;
                /* Zero spin to avoid wrist injuries */
                self.trajs.v[t].spin = 0.0;
                self.match_spins_on_catch(t, t);
                self.trajs.v[t].dx = 0.0;
                self.trajs.v[t].dy = 0.0;
                self.trajs.v[t].status = Status::Bpredictor;
                t = next;
                continue;
            }

            let o = self.trajs.v[t].object;
            let oty = if o != NIL {
                self.objects.v[o].ty
            } else {
                ObjType::Ball
            };

            let mut done = false;
            if self.trajs.v[t].posn == b'_' {
                /* Bounce once */
                let tb = self.trajs.v[t].start
                    + self.find_bounce(
                        self.trajs.v[t].y,
                        yf,
                        self.trajs.v[link].y,
                        (self.trajs.v[link].start - self.trajs.v[t].start) as f64,
                        oty.cor(),
                    ) as i64;

                if tb < self.trajs.v[t].start {
                    /* bounce too hard: use a regular throw */
                    self.trajs.v[t].posn = b'+';
                } else {
                    /* dx is constant across both trajectories */
                    self.trajs.v[t].dx = (self.trajs.v[link].x - self.trajs.v[t].x)
                        / (self.trajs.v[link].start - self.trajs.v[t].start) as f64;

                    /* ball follows parabola down */
                    let start = self.trajs.v[t].start;
                    let angle = self.trajs.v[t].angle;
                    let n = self.new_predictor(t, start, tb, angle);
                    let dt = (self.trajs.v[n].finish - self.trajs.v[n].start) as f64;
                    /* Ball rate 4, no flight or matching club turns */
                    self.trajs.v[n].spin = self.spinrate(oty, 0.0, dt, 4, 0, 0.0);
                    self.trajs.v[t].dy = (yf - self.trajs.v[t].y) / dt - self.gr / 2.0 * dt;
                    let (x, dx, y, dy) = (
                        self.trajs.v[t].x,
                        self.trajs.v[t].dx,
                        self.trajs.v[t].y,
                        self.trajs.v[t].dy,
                    );
                    make_parabola(&mut self.trajs.v[n], x, dx, y, dy, self.gr);

                    /* ball follows parabola up */
                    let (nf, es) = (self.trajs.v[n].finish, self.end_spin(n));
                    let m = self.new_predictor(t, nf, self.trajs.v[link].start, es);
                    let dt = (self.trajs.v[m].finish - self.trajs.v[m].start) as f64;
                    /* Use previous ball rate, no flight club turns */
                    let togo = self.trajs.v[link].angle - self.trajs.v[m].angle;
                    let prev = self.trajs.v[n].spin;
                    self.trajs.v[m].spin = self.spinrate(oty, prev, dt, 0, 0, togo);
                    self.match_spins_on_catch(t, m);
                    let dy2 = (self.trajs.v[link].y - yf) / dt - self.gr / 2.0 * dt;
                    let (lx, dx) = (self.trajs.v[link].x, self.trajs.v[t].dx);
                    make_parabola(&mut self.trajs.v[m], lx - dx * dt, dx, yf, dy2, self.gr);

                    self.trajs.v[t].status = Status::Bpredictor;
                    done = true;
                }
            } else if self.trajs.v[t].posn == b'k' {
                /* Drop & Kick */
                let td = self.trajs.v[t].start + 2 * self.throw_catch() as i64;
                let tk = self.trajs.v[link].start - 5 * self.throw_catch() as i64;

                /* Fall to ground */
                let (start, angle) = (self.trajs.v[t].start, self.trajs.v[t].angle);
                let n = self.new_predictor(t, start, td, angle);
                let dt = (self.trajs.v[n].finish - self.trajs.v[n].start) as f64;
                let togo = self.trajs.v[link].angle - self.trajs.v[n].angle;
                /* Ball spin rate 4, no flight club turns */
                self.trajs.v[n].spin = self.spinrate(oty, 0.0, dt, 4, 0, togo);
                self.trajs.v[t].dx = (self.trajs.v[link].x - self.trajs.v[t].x) / dt;
                self.trajs.v[t].dy = (yf - self.trajs.v[t].y) / dt - self.gr / 2.0 * dt;
                let (x, dx, y, dy) = (
                    self.trajs.v[t].x,
                    self.trajs.v[t].dx,
                    self.trajs.v[t].y,
                    self.trajs.v[t].dy,
                );
                make_parabola(&mut self.trajs.v[n], x, dx, y, dy, self.gr);

                /* Rest on ground */
                let (nf, es) = (self.trajs.v[n].finish, self.end_spin(n));
                let ov = self.new_predictor(t, nf, tk, es);
                self.trajs.v[ov].spin = 0.0;
                let lx = self.trajs.v[link].x;
                make_parabola(&mut self.trajs.v[ov], lx, 0.0, yf, 0.0, 0.0);

                /* Kick up */
                let (of, es) = (self.trajs.v[ov].finish, self.end_spin(ov));
                let m = self.new_predictor(t, of, self.trajs.v[link].start, es);
                let dt = (self.trajs.v[m].finish - self.trajs.v[m].start) as f64;
                /* Match receiving hand, ball rate 4, one flight club turn */
                let togo = self.trajs.v[link].angle - self.trajs.v[m].angle;
                self.trajs.v[m].spin = self.spinrate(oty, 0.0, dt, 4, 1, togo);
                self.match_spins_on_catch(t, m);
                let dy = (self.trajs.v[link].y - yf) / dt - self.gr / 2.0 * dt;
                let lx = self.trajs.v[link].x;
                make_parabola(&mut self.trajs.v[m], lx, 0.0, yf, dy, self.gr);

                self.trajs.v[t].status = Status::Bpredictor;
                done = true;
            }

            if !done {
                /* Regular flight, no bounce: ball follows parabola */
                let (start, angle) = (self.trajs.v[t].start, self.trajs.v[t].angle);
                let n = self.new_predictor(t, start, self.trajs.v[link].start, angle);
                let dt = (self.trajs.v[link].start - self.trajs.v[t].start) as f64;
                /* Regular spin */
                let h = self.trajs.v[t].height;
                let togo = self.trajs.v[link].angle - self.trajs.v[n].angle;
                self.trajs.v[n].spin = self.spinrate(oty, 0.0, dt, h, h / 2, togo);
                self.match_spins_on_catch(t, n);
                self.trajs.v[t].dx = (self.trajs.v[link].x - self.trajs.v[t].x) / dt;
                self.trajs.v[t].dy =
                    (self.trajs.v[link].y - self.trajs.v[t].y) / dt - self.gr / 2.0 * dt;
                let (x, dx, y, dy) = (
                    self.trajs.v[t].x,
                    self.trajs.v[t].dx,
                    self.trajs.v[t].y,
                    self.trajs.v[t].dy,
                );
                make_parabola(&mut self.trajs.v[n], x, dx, y, dy, self.gr);

                self.trajs.v[t].status = Status::Bpredictor;
            }
            t = next;
        }
    }

    /// Turn abstract hand motions into cubic splines.
    fn hands(&mut self) {
        let mut t = self.trajs.v[0].next;
        while t != 0 {
            /* no throw => no velocity */
            if self.trajs.v[t].status != Status::Bpredictor {
                t = self.trajs.v[t].next;
                continue;
            }
            let u = self.trajs.v[t].handlink;
            if u == NIL {
                /* no next catch */
                t = self.trajs.v[t].next;
                continue;
            }
            let v = self.trajs.v[u].handlink;
            if v == NIL {
                /* no next throw */
                t = self.trajs.v[t].next;
                continue;
            }

            // A double spline takes the hand from the throw, through the
            // catch, to the next throw.
            self.trajs.v[t].finish = self.trajs.v[u].start;
            self.trajs.v[t].status = Status::Predictor;
            self.trajs.v[u].finish = self.trajs.v[v].start;
            self.trajs.v[u].status = Status::Predictor;

            // Make sure an empty hand's spin matches the thrown object in
            // case it had a handle.
            let (ta, ua, va) = (
                self.trajs.v[t].angle,
                self.trajs.v[u].angle,
                self.trajs.v[v].angle,
            );
            let (ts, us, vs) = (
                self.trajs.v[t].start as f64,
                self.trajs.v[u].start as f64,
                self.trajs.v[v].start as f64,
            );
            self.trajs.v[t].spin = if self.trajs.v[t].hand == Hand::Left {
                -1.0
            } else {
                1.0
            } * ((ua - ta) / (us - ts)).abs();
            self.trajs.v[u].spin =
                if (self.trajs.v[v].hand == Hand::Left) != (self.trajs.v[v].posn == b'+') {
                    -1.0
                } else {
                    1.0
                } * ((va - ua) / (vs - us)).abs();

            let (tx, tdx, ux, vx, vdx) = (
                self.trajs.v[t].x,
                self.trajs.v[t].dx,
                self.trajs.v[u].x,
                self.trajs.v[v].x,
                self.trajs.v[v].dx,
            );
            let (sx1, sx2) = make_spline_pair(tx, tdx, ts, ux, us, vx, vdx, vs);
            self.trajs.v[t].xp = sx1;
            self.trajs.v[u].xp = sx2;

            let (ty, tdy, uy, vy, vdy) = (
                self.trajs.v[t].y,
                self.trajs.v[t].dy,
                self.trajs.v[u].y,
                self.trajs.v[v].y,
                self.trajs.v[v].dy,
            );
            let (sy1, sy2) = make_spline_pair(ty, tdy, ts, uy, us, vy, vdy, vs);
            self.trajs.v[t].yp = sy1;
            self.trajs.v[u].yp = sy2;

            t = self.trajs.v[t].next;
        }
    }

    /// NOTE: the returned x, y are adjusted for arm reach.
    fn reach_arm(&mut self, side: Hand, p: &mut Point) {
        let s = self.arm[1][side as usize][SHOULDER];
        let (h, e) = find_elbow(40.0, *p, s, 25.0);
        *p = h;
        self.arm[1][side as usize][HAND] = h;
        self.arm[1][side as usize][ELBOW] = e;
    }

    fn trajectory_destroy(&mut self, t: usize) {
        let o = self.trajs.v[t].object;
        if o != NIL {
            self.objects.v[o].count -= 1;
            if self.objects.v[o].count < 1 {
                self.objects.remove(o);
            }
        }
        self.trajs.remove(t);
    }
}

const SHOULDER: usize = 2;
const ELBOW: usize = 1;
const HAND: usize = 0;

/// Compute a single spline from `x0` with velocity `dx0` at time `t0` to `x1`
/// with velocity `dx1` at time `t1`.
fn make_spline(x0: f64, dx0: f64, t0: f64, x1: f64, dx1: f64, t1: f64) -> Spline {
    let x10 = x1 - x0;
    let t10 = t1 - t0;
    let a = ((dx0 + dx1) * t10 - 2.0 * x10) / (t10 * t10 * t10);
    let b = (3.0 * x10 - (2.0 * dx0 + dx1) * t10) / (t10 * t10);
    let c = dx0;
    let d = x0;
    Spline {
        a,
        b: -3.0 * a * t0 + b,
        c: (3.0 * a * t0 - 2.0 * b) * t0 + c,
        d: ((-a * t0 + b) * t0 - c) * t0 + d,
    }
}

/// A pair of splines. The first goes from `x0` with velocity `dx0` at `t0` to
/// `x1` at `t1`; the second from `x1` at `t1` to `x2` with velocity `dx2` at
/// `t2`. The arrival and departure velocities at `x1` must be the same.
#[allow(clippy::too_many_arguments)]
fn make_spline_pair(
    x0: f64,
    dx0: f64,
    t0: f64,
    x1: f64,
    t1: f64,
    x2: f64,
    dx2: f64,
    t2: f64,
) -> (Spline, Spline) {
    let x10 = x1 - x0;
    let x21 = x2 - x1;
    let t21 = t2 - t1;
    let t10 = t1 - t0;
    let t20 = t2 - t0;
    let dx1 = (3.0 * x10 * t21 * t21 + 3.0 * x21 * t10 * t10 + 3.0 * dx0 * t10 * t21 * t21
        - dx2 * t10 * t10 * t21
        - 4.0 * dx0 * t10 * t21 * t21)
        / (2.0 * t10 * t21 * t20);
    (
        make_spline(x0, dx0, t0, x1, dx1, t1),
        make_spline(x1, dx1, t1, x2, dx2, t2),
    )
}

/// A ballistic path as a pair of degenerate splines: x goes at a constant
/// velocity, y with a constant acceleration.
fn make_parabola(n: &mut Traj, x: f64, dx: f64, y: f64, dy: f64, g: f64) {
    let t = n.start as f64;
    n.xp = Spline {
        a: 0.0,
        b: 0.0,
        c: dx,
        d: -dx * t + x,
    };
    n.yp = Spline {
        a: 0.0,
        b: g / 2.0,
        c: -g * t + dy,
        d: g / 2.0 * t * t - dy * t + y,
    };
}

/// Given a target, put the hand there if it can reach, and otherwise point at
/// it. Returns the hand and the elbow.
fn find_elbow(armlength: f64, p: Point, s: Point, z: f64) -> (Point, Point) {
    let x = p.x - s.x;
    let y = p.y - s.y;
    let h2 = x * x + y * y + z * z;
    if h2 > 4.0 * armlength * armlength {
        let t = armlength / h2.sqrt();
        (
            Point {
                x: 2.0 * t * x + s.x,
                y: 2.0 * t * y + s.y,
            },
            Point {
                x: t * x + s.x,
                y: t * y + s.y,
            },
        )
    } else {
        let r = (x * x + z * z).sqrt();
        let t = (4.0 * armlength * armlength / h2 - 1.0).sqrt();
        (
            Point {
                x: x + s.x,
                y: y + s.y,
            },
            Point {
                x: x * (1.0 + y * t / r) / 2.0 + s.x,
                y: (y - r * t) / 2.0 + s.y,
            },
        )
    }
}

/* Rendering */

/// Lifted from `sphere.c`: a sphere with `stripes` bands in a second colour,
/// which is what makes a spinning ball read as spinning.
fn striped_unit_sphere(
    g: &mut Gl,
    stacks: i32,
    slices: i32,
    stripes: i32,
    color1: [f32; 4],
    color2: [f32; 4],
    wire: bool,
) {
    let stacks2 = stacks * 2;
    g.glx.front_face_cw(true);

    for j in 0..stacks {
        let theta1 =
            j as f64 * (2.0 * std::f64::consts::PI) / stacks2 as f64 - std::f64::consts::FRAC_PI_2;
        let theta2 = (j + 1) as f64 * (2.0 * std::f64::consts::PI) / stacks2 as f64
            - std::f64::consts::FRAC_PI_2;

        // Upstream's condition: the poles and anything not on a stripe
        // boundary take the first colour.
        let k = stacks / (stripes + 1).max(1);
        let banded = j == 0 || j == stacks - 1 || (k != 0 && j % k != 0);
        g.glx
            .material_ambient_diffuse(if banded { color1 } else { color2 });

        g.glx.begin(if wire {
            Shape::Lines
        } else {
            Shape::TriangleStrip
        });
        for i in 0..=slices {
            let theta3 = i as f64 * (2.0 * std::f64::consts::PI) / slices as f64;
            for theta in [theta2, theta1] {
                let e = [
                    (theta.cos() * theta3.cos()) as f32,
                    theta.sin() as f32,
                    (theta.cos() * theta3.sin()) as f32,
                ];
                g.glx.normal3f(e[0], e[1], e[2]);
                g.glx.vertex3f(e[0], e[1], e[2]);
            }
        }
        g.glx.end();
    }
}

impl Juggle {
    fn show_arms(&mut self, g: &mut Gl) {
        let slices = 12;
        let thickness = 7.0;
        let soffx = 10.0;
        let soffy = 11.0;

        g.glx.front_face_cw(false);

        for side in [Hand::Left, Hand::Right] {
            let s = side as usize;
            let mut a = [[0.0f32; 2]; 3];
            for (i, joint) in a.iter_mut().enumerate() {
                joint[0] = (self.scene_width / 2.0 + self.arm[1][s][i].x * self.scale) as f32;
                joint[1] = (SCENE_HEIGHT - self.arm[1][s][i].y * self.scale) as f32;
                self.arm[0][s][i] = self.arm[1][s][i];
            }
            let sx = a[2][0] - if side == Hand::Left { soffx } else { -soffx };
            let sy = a[2][1] + soffy;

            g.glx.material_ambient_diffuse(BODY_COLOR_1);

            /* Upper arm */
            tube(
                &mut g.glx,
                [sx, sy, 0.0],
                [a[1][0], a[1][1], ARMLENGTH / 2.0],
                thickness,
                0.0,
                slices,
                true,
                true,
                self.wire,
            );

            /* Lower arm */
            tube(
                &mut g.glx,
                [a[1][0], a[1][1], ARMLENGTH / 2.0],
                [a[0][0], a[0][1], ARMLENGTH],
                thickness * 0.8,
                0.0,
                slices,
                true,
                true,
                self.wire,
            );

            g.glx.material_ambient_diffuse(BODY_COLOR_2);

            for (at, scale) in [
                ([sx, sy, 0.0], 9.0),
                ([a[1][0], a[1][1], ARMLENGTH / 2.0], 4.0),
                ([a[0][0], a[0][1], ARMLENGTH], 8.0),
            ] {
                g.glx.push_matrix();
                g.glx.translate(at[0], at[1], at[2]);
                g.glx.scale(scale, scale, scale);
                unit_sphere(&mut g.glx, slices, slices, self.wire);
                g.glx.pop_matrix();
            }
        }
    }

    fn show_figure(&mut self, g: &mut Gl, init: bool) {
        /*      +-----+ 9
                |  6  |
             10 +--+--+
             2 +---+---+ 3
                \  5  /
                 \   /
                  \ /
                 1 +
                  / \
                 /   \
              0 +-----+ 4
        */
        let figure: [[f64; 2]; 11] = [
            [15.0, 70.0],   /* 0  Left Hip */
            [0.0, 90.0],    /* 1  Waist */
            [SX, 130.0],    /* 2  Left Shoulder */
            [-SX, 130.0],   /* 3  Right Shoulder */
            [-15.0, 70.0],  /* 4  Right Hip */
            [0.0, 130.0],   /* 5  Neck */
            [0.0, 140.0],   /* 6  Chin */
            [SX, 0.0],      /* 7  Left Foot */
            [-SX, 0.0],     /* 8  Right Foot */
            [-17.0, 174.0], /* 9  Head1 */
            [17.0, 140.0],  /* 10 Head2 */
        ];
        let mut a = [[0.0f32; 2]; 11];
        for i in 0..figure.len() {
            a[i][0] = (self.scene_width / 2.0 + (self.cx + figure[i][0]) * self.scale) as f32;
            a[i][1] = (SCENE_HEIGHT - figure[i][1] * self.scale) as f32;
        }

        g.glx.material_ambient_diffuse([1.0, 1.0, 1.0, 1.0]);
        g.glx.front_face_cw(false);

        let scale0 = (a[10][0] - a[9][0]) / 2.0;
        let slices = 12;
        let wire = self.wire;

        g.glx.push_matrix();
        g.glx.translate(a[6][0], a[6][1] - scale0, 0.0);
        g.glx.scale(scale0, scale0, scale0);

        /* Head */
        g.glx.material_ambient_diffuse(BODY_COLOR_1);
        g.glx.push_matrix();
        g.glx.scale(0.75, 0.75, 0.75);
        g.glx.translate(0.0, 0.3, 0.0);
        g.glx.push_matrix();
        g.glx.translate(0.0, 0.0, 0.35);
        tube(
            &mut g.glx,
            [0.0, 0.0, 0.0],
            [0.0, 1.1, 0.0],
            0.64,
            0.0,
            slices,
            true,
            true,
            wire,
        );
        g.glx.pop_matrix();
        g.glx.scale(0.9, 0.9, 1.0);
        unit_sphere(&mut g.glx, 2 * slices, 2 * slices, wire);
        g.glx.pop_matrix();

        /* Neck, torso, belly, hips: a stack of balls up the middle. */
        for (color, up, scale) in [
            (BODY_COLOR_2, 1.1, 0.35),
            (BODY_COLOR_1, 1.1, 0.0),
            (BODY_COLOR_2, 1.0, 0.6),
            (BODY_COLOR_1, 0.8, 0.85),
        ] {
            g.glx.material_ambient_diffuse(color);
            g.glx.translate(0.0, up, 0.0);
            g.glx.push_matrix();
            if scale == 0.0 {
                /* The torso is the one that is not a plain sphere. */
                g.glx.scale(0.9, 1.0, 0.9);
            } else {
                g.glx.scale(scale, scale, scale);
            }
            unit_sphere(&mut g.glx, slices, slices, wire);
            g.glx.pop_matrix();
        }

        /* Legs */
        g.glx.translate(0.0, 0.7, 0.0);

        for i in [-1.0f32, 1.0] {
            g.glx.push_matrix();

            g.glx.rotate(i * 10.0, 0.0, 0.0, 1.0);
            g.glx.translate(-i * 0.65, 0.0, 0.0);

            /* Hip socket */
            g.glx.material_ambient_diffuse(BODY_COLOR_2);
            g.glx.scale(0.45, 0.45, 0.45);
            unit_sphere(&mut g.glx, slices, slices, wire);

            /* Thigh, knee, calf, ankle, all up the leg. */
            for (color, up, shape) in [
                (BODY_COLOR_1, 0.6, Some((3.5, 1.0))),
                (BODY_COLOR_2, 4.4, None),
                (BODY_COLOR_1, 4.7, Some((4.7, 0.8))),
                (BODY_COLOR_2, 9.7, None),
            ] {
                g.glx.material_ambient_diffuse(color);
                g.glx.push_matrix();
                g.glx.translate(0.0, up, 0.0);
                match shape {
                    Some((len, dia)) => {
                        tube(
                            &mut g.glx,
                            [0.0, 0.0, 0.0],
                            [0.0, len, 0.0],
                            dia,
                            0.0,
                            slices,
                            true,
                            true,
                            wire,
                        );
                    }
                    None => {
                        let s = if up == 4.4 { 0.7 } else { 0.5 };
                        g.glx.scale(s, s, s);
                        unit_sphere(&mut g.glx, slices, slices, wire);
                    }
                }
                g.glx.pop_matrix();
            }

            /* Foot */
            g.glx.material_ambient_diffuse(BODY_COLOR_1);
            g.glx.push_matrix();
            g.glx.rotate(-i * 10.0, 0.0, 0.0, 1.0);
            g.glx.translate(-i * 1.75, 9.7, 0.9);
            g.glx.scale(0.4, 1.0, 1.0);
            tube(
                &mut g.glx,
                [0.0, 0.0, 0.0],
                [0.0, 0.6, 0.0],
                1.9,
                0.0,
                slices * 4,
                true,
                true,
                wire,
            );
            g.glx.pop_matrix();

            g.glx.pop_matrix();
        }
        g.glx.pop_matrix();

        self.arm[1][Hand::Left as usize][SHOULDER].x = self.cx + figure[2][0];
        self.arm[1][Hand::Right as usize][SHOULDER].x = self.cx + figure[3][0];
        if init {
            /* Initialise arms */
            for i in 0..2 {
                let l = Hand::Left as usize;
                let r = Hand::Right as usize;
                self.arm[i][l][SHOULDER].y = figure[2][1];
                self.arm[i][l][ELBOW] = Point {
                    x: figure[2][0],
                    y: figure[1][1],
                };
                self.arm[i][l][HAND] = Point {
                    x: figure[0][0],
                    y: figure[1][1],
                };
                self.arm[i][r][SHOULDER].y = figure[3][1];
                self.arm[i][r][ELBOW] = Point {
                    x: figure[3][0],
                    y: figure[1][1],
                };
                self.arm[i][r][HAND] = Point {
                    x: figure[4][0],
                    y: figure[1][1],
                };
            }
        }
    }

    /// Where a prop sits on screen, or `None` if it has flown off the top and
    /// would wrap.
    fn prop_at(&self, s: &Trace) -> Option<(f32, f32)> {
        if s.y * self.scale > SCENE_HEIGHT * 2.0 {
            return None;
        }
        Some((
            (self.scene_width / 2.0 + s.x * self.scale) as f32,
            (SCENE_HEIGHT - s.y * self.scale) as f32,
        ))
    }

    fn rgb(&self, color: usize) -> [f32; 4] {
        let c = &self.colors[color % self.colors.len()];
        [
            c.red as f32 / 65536.0,
            c.green as f32 / 65536.0,
            c.blue as f32 / 65536.0,
            1.0,
        ]
    }

    fn show_object(&self, g: &mut Gl, ty: ObjType, color: usize, s: &Trace) {
        let Some((x, y)) = self.prop_at(s) else {
            return;
        };
        let c1 = self.rgb(color);
        let dark = [c1[0] / 3.0, c1[1] / 3.0, c1[2] / 3.0, 1.0];
        let white = [1.0, 1.0, 1.0, 1.0];
        let radius = (12.0 * self.scale) as f32;
        let angle = (s.angle / std::f64::consts::PI * 180.0) as f32;
        let wire = self.wire;

        g.glx.front_face_cw(false);

        match ty {
            ObjType::Ball => {
                /* BALLRADIUS is the arm width, which is scaled the same way */
                let scale = 8.0 * (self.scale as f32).sqrt();
                g.glx.push_matrix();
                g.glx.translate(x, y, 0.0);
                g.glx.scale(scale, scale, scale);
                g.glx.rotate(angle, 1.0, 1.0, 0.0);
                striped_unit_sphere(g, 24, 24, s.divisions, c1, dark, wire);
                g.glx.pop_matrix();
            }
            ObjType::Club => {
                let slices = 16;
                g.glx.push_matrix();
                g.glx.translate(x, y, 0.0);
                g.glx.scale(radius, radius, radius);
                /* put end of handle in hand */
                g.glx.translate(0.0, 0.0, 2.0);
                g.glx.rotate(angle, 1.0, 0.0, 0.0);

                g.glx.push_matrix();
                g.glx.scale(0.5, 1.0, 0.5);
                striped_unit_sphere(g, slices, slices, 4, white, c1, wire);
                g.glx.pop_matrix();
                g.glx.material_ambient_diffuse(white);
                tube(
                    &mut g.glx,
                    [0.0, 0.0, 0.0],
                    [0.0, 2.0, 0.0],
                    0.2,
                    0.0,
                    slices,
                    true,
                    true,
                    wire,
                );

                g.glx.translate(0.0, 2.0, 0.0);
                g.glx.scale(0.25, 0.25, 0.25);
                unit_sphere(&mut g.glx, slices, slices, wire);
                g.glx.pop_matrix();
            }
            ObjType::Knife => {
                let slices = 8;
                g.glx.push_matrix();
                g.glx.translate(x, y, 0.0);
                g.glx.scale(2.0, 2.0, 2.0);
                /* put end of handle in hand */
                g.glx.translate(0.0, 0.0, 2.0);
                g.glx.rotate(angle, 1.0, 0.0, 0.0);
                /* flatten blade */
                g.glx.scale(0.3, 1.0, 1.0);
                g.glx.translate(0.0, 6.0, 0.0);
                g.glx.rotate(180.0, 1.0, 0.0, 0.0);

                g.glx.material_ambient_diffuse(c1);
                tube(
                    &mut g.glx,
                    [0.0, 0.0, 0.0],
                    [0.0, 10.0, 0.0],
                    1.0,
                    0.0,
                    slices,
                    true,
                    true,
                    wire,
                );

                g.glx.translate(0.0, 12.0, 0.0);
                g.glx.scale(0.7, 10.0, 0.7);
                g.glx.material_ambient_diffuse(white);
                unit_sphere(&mut g.glx, slices, slices, wire);
                g.glx.pop_matrix();
            }
            ObjType::Ring => {
                let slices = 24;
                let width = std::f32::consts::PI * 2.0 / slices as f32;
                let (ra, rb, thickness) = (1.0f32, 0.7f32, 0.15f32);

                g.glx.push_matrix();
                /* back of ring in hand */
                g.glx.translate(0.0, 0.0, 12.0);
                g.glx.translate(x, y, 0.0);
                g.glx.scale(radius, radius, radius);
                g.glx.rotate(90.0, 0.0, 1.0, 0.0);
                g.glx.rotate(angle, 0.0, 0.0, 1.0);

                g.glx.material_ambient_diffuse(c1);

                /* discs */
                for j in [-1.0f32, 1.0] {
                    let z = j * thickness / 2.0;
                    g.glx.front_face_cw(j >= 0.0);
                    g.glx.normal3f(0.0, 0.0, j);
                    for i in 0..slices + 1 {
                        g.glx.material_ambient_diffuse(if i % (slices / 3) != 0 {
                            c1
                        } else {
                            dark
                        });
                        g.glx.begin(Shape::QuadStrip);
                        let th = i as f32 * width;
                        let (cth, sth) = (th.cos(), th.sin());
                        g.glx.vertex3f(cth * ra, sth * ra, z);
                        g.glx.vertex3f(cth * rb, sth * rb, z);
                        let th = (i + 1).min(slices) as f32 * width;
                        let (cth, sth) = (th.cos(), th.sin());
                        g.glx.vertex3f(cth * ra, sth * ra, z);
                        g.glx.vertex3f(cth * rb, sth * rb, z);
                        g.glx.end();
                    }
                }

                /* outer and inner rings */
                for (front_cw, r) in [(false, ra), (true, rb)] {
                    g.glx.front_face_cw(front_cw);
                    g.glx.begin(Shape::QuadStrip);
                    for i in 0..slices + 1 {
                        let th = i as f32 * width;
                        let (cth, sth) = (th.cos(), th.sin());
                        let n = if front_cw { -1.0 } else { 1.0 };
                        g.glx.normal3f(n * cth, n * sth, 0.0);
                        // The inner ring is upstream's, warts and all: it uses
                        // the outer radius for y, so the hole is an oval.
                        let ry = if front_cw { ra } else { r };
                        g.glx.vertex3f(cth * r, sth * ry, thickness / 2.0);
                        g.glx.vertex3f(cth * r, sth * ry, -thickness / 2.0);
                    }
                    g.glx.end();
                }

                g.glx.front_face_cw(false);
                g.glx.pop_matrix();
            }
            ObjType::BBall => {
                let slices = 16;
                g.glx.push_matrix();
                /* position on top of hand */
                g.glx.translate(0.0, -6.0, 5.0);
                g.glx.translate(x, y, 0.0);
                g.glx.scale(radius, radius, radius);
                g.glx.rotate(angle, 1.0, 0.0, 1.0);

                g.glx.material_ambient_diffuse(c1);
                unit_sphere(&mut g.glx, slices, slices, wire);

                g.glx.rotate(90.0, 0.0, 0.0, 1.0);
                g.glx.translate(0.0, 0.0, 0.81);
                g.glx.scale(0.15, 0.15, 0.15);
                g.glx.material_ambient_diffuse(dark);
                for i in 0..3 {
                    g.glx.push_matrix();
                    g.glx.translate(0.0, 0.0, 1.0);
                    g.glx.rotate(360.0 * i as f32 / 3.0, 0.0, 0.0, 1.0);
                    g.glx.translate(2.0, 0.0, 0.0);
                    g.glx.rotate(18.0, 0.0, 1.0, 0.0);
                    g.glx.begin(if wire {
                        Shape::LineLoop
                    } else {
                        Shape::TriangleFan
                    });
                    g.glx.vertex3f(0.0, 0.0, 0.0);
                    for j in (0..=slices).rev() {
                        let th = j as f32 * std::f32::consts::PI * 2.0 / slices as f32;
                        g.glx.vertex3f(th.cos(), th.sin(), 0.0);
                    }
                    g.glx.end();
                    g.glx.pop_matrix();
                }
                g.glx.pop_matrix();
            }
        }
    }
}

const MAXPAT: i32 = 10;
const MAXREPEAT: i32 = 300;
/// Larger makes num_ball changes less likely.
const CHANGE_BIAS: i32 = 8;
/// Larger makes hand movements less likely.
const POSITION_BIAS: i32 = 20;

impl Juggle {
    /// The arm positions `show_figure` works out on the way past, without the
    /// drawing. Upstream calls the whole thing for this side effect at times
    /// when there is no frame to draw into.
    fn init_arms(&mut self) {
        let figure2 = [SX, 130.0];
        let figure3 = [-SX, 130.0];
        let figure1y = 90.0;
        let figure0x = 15.0;
        let figure4x = -15.0;
        self.arm[1][Hand::Left as usize][SHOULDER].x = self.cx + figure2[0];
        self.arm[1][Hand::Right as usize][SHOULDER].x = self.cx + figure3[0];
        for i in 0..2 {
            let l = Hand::Left as usize;
            let r = Hand::Right as usize;
            self.arm[i][l][SHOULDER].y = figure2[1];
            self.arm[i][l][ELBOW] = Point {
                x: figure2[0],
                y: figure1y,
            };
            self.arm[i][l][HAND] = Point {
                x: figure0x,
                y: figure1y,
            };
            self.arm[i][r][SHOULDER].y = figure3[1];
            self.arm[i][r][ELBOW] = Point {
                x: figure3[0],
                y: figure1y,
            };
            self.arm[i][r][HAND] = Point {
                x: figure4x,
                y: figure1y,
            };
        }
    }

    /// Append new throws to the programme and run the whole pipeline over it.
    fn refill(&mut self, npixels: usize) {
        let mut count = 0;
        while count < self.cycles {
            let l = random_below(MAXPAT) + 1;
            let t = random_below(MAXREPEAT.min(self.cycles - count)) + 1;

            /* vary number of balls */
            {
                let mut new_balls = self.num_balls;
                let change = if new_balls == 2 {
                    /* Do not juggle 2 that often */
                    random_below(2 + CHANGE_BIAS / 4)
                } else {
                    random_below(2 + CHANGE_BIAS)
                };
                match change {
                    0 => new_balls += 1,
                    1 => new_balls -= 1,
                    _ => {}
                }
                if new_balls < self.minballs {
                    new_balls += 2;
                }
                if new_balls > self.maxballs {
                    new_balls -= 2;
                }
                if new_balls < self.num_balls {
                    /* lose ball */
                    self.program("[*]", None, 1);
                }
                self.num_balls = new_balls;
            }

            count += t;
            let idx = self.index[self.num_balls.clamp(0, self.maxballs) as usize];
            if random_below(2) != 0 && idx.number > 0 {
                /* Pick from the portfolio */
                let p = idx.start + random_below(idx.number as i32) as usize;
                let (pat, name) = self.sorted[p];
                self.program(pat, if name.is_empty() { None } else { Some(name) }, t);
            } else {
                /* Invent a new pattern */
                let mut b = String::from("[");
                let mut maxseen = false;
                for _ in 0..l {
                    let (mut m, mut n);
                    loop {
                        /* Triangular distribution: high values more likely */
                        m = random_below(self.num_balls + 1);
                        n = random_below(self.num_balls + 1);
                        if m < n {
                            break;
                        }
                    }
                    if n == self.num_balls {
                        maxseen = true;
                    }
                    match random_below(5 + POSITION_BIAS) {
                        0 => b.push('+'), /* Outside throw */
                        1 => b.push('='), /* Cross throw */
                        2 => b.push('&'), /* Cross catch */
                        3 => b.push('x'), /* Cross throw and catch */
                        4 => b.push('_'), /* Bounce */
                        _ => {}           /* Inside throw (default) */
                    }
                    b.push((b'0' + n as u8) as char);
                    b.push(' ');
                }
                b.push(']');
                if maxseen {
                    self.program(&b, None, t);
                }
            }
        }

        self.adam();
        self.name();
        self.part();
        self.lob(npixels);
        self.clap();
        self.positions();
        self.projectile();
        self.hands();
    }

    /// Throw away whatever is not being juggled right now, pick a new prop,
    /// and program a new performance.
    fn change(&mut self, npixels: usize) {
        /* Strip pending trajectories */
        let mut t = self.trajs.v[0].next;
        while t != 0 {
            let next = self.trajs.v[t].next;
            if self.trajs.v[t].start > self.time || self.trajs.v[t].finish < self.time {
                self.trajectory_destroy(t);
            }
            t = next;
        }

        /* Pick the current object theme */
        self.objtypes = self.choose_object();

        self.refill(npixels);
        self.init_arms();
    }
}

impl Hack3d for Juggle {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        let delay = g.res.int("delay").max(0) as u32;

        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.clear();

        g.glx.push_matrix();
        g.glx.translate(0.0, -3.0, 0.0);

        {
            let down = self.trackball.button_down();
            let (x, y, z) = self.rot.position(!down);
            g.glx.translate(
                (x as f32 - 0.5) * 8.0,
                (y as f32 - 0.5) * 3.0,
                (z as f32 - 0.5) * 15.0,
            );

            let m = self.trackball.matrix();
            g.glx.mult_matrix(m);

            let (x, mut y, z) = self.rot.rotation(!down);
            /* always face forward */
            if y < 0.8 {
                y = 0.8 - (y - 0.8);
            }
            if y > 1.2 {
                y = 1.2 - (y - 1.2);
            }

            g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
            g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
            g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);
        }

        let scale = 20.0 / SCENE_HEIGHT as f32;
        g.glx.scale(scale, scale, scale);

        g.glx.rotate(180.0, 0.0, 0.0, 1.0);
        g.glx.translate(
            -(self.scene_width as f32) / 2.0,
            -(SCENE_HEIGHT as f32) / 2.0,
            0.0,
        );
        g.glx.translate(0.0, -150.0, 0.0);

        /* Update timer */
        self.time += (delay / 1000) as i64;

        let mut future = 0;
        let mut pattern: Option<String> = None;

        /* First pass: move arms and strip out expired elements */
        let mut traj = self.trajs.v[0].next;
        while traj != 0 {
            let next = self.trajs.v[traj].next;
            if self.trajs.v[traj].status != Status::Predictor {
                // Skip any elements that need further processing. We could
                // remove them, but there shouldn't be many and they would be
                // needed if we ever got the pattern refiller working.
                traj = next;
                continue;
            }
            if self.trajs.v[traj].start > future {
                /* Lookahead to the end of the show */
                future = self.trajs.v[traj].start;
            }
            if self.time < self.trajs.v[traj].start {
                /* early */
                traj = next;
                continue;
            }
            if self.time >= self.trajs.v[traj].finish {
                /* expired */
                self.trajectory_destroy(traj);
                traj = next;
                continue;
            }

            /* working */
            if let Some(p) = &self.trajs.v[traj].pattern {
                pattern = Some(p.clone());
            }

            let ty = self.trajs.v[traj].ty;
            if ty == Throwable::Empty || ty == Throwable::Full {
                /* Only interested in hands on this pass */
                let (mut xd, mut yd) = (0.0, 0.0);
                if self.trajs.v[traj].object != NIL {
                    /* Balls are always caught at the bottom */
                    xd = 0.0;
                    yd = -4.0;
                }
                let t = self.time as f64;
                let mut p = Point {
                    x: self.trajs.v[traj].xp.at(t) - xd,
                    y: self.trajs.v[traj].yp.at(t) + yd,
                };
                let hand = self.trajs.v[traj].hand;
                self.reach_arm(hand, &mut p);

                /* Store updated hand position */
                self.trajs.v[traj].x = p.x + xd;
                self.trajs.v[traj].y = p.y - yd;
            }
            if ty == Throwable::Ball || ty == Throwable::Full {
                /* Only interested in objects on this pass */
                let t = self.time as f64;
                let (x, y) = if ty == Throwable::Full {
                    /* Adjusted these in the first pass */
                    (self.trajs.v[traj].x, self.trajs.v[traj].y)
                } else {
                    (self.trajs.v[traj].xp.at(t), self.trajs.v[traj].yp.at(t))
                };

                let tv = &self.trajs.v[traj];
                let trace = Trace {
                    x,
                    y,
                    angle: tv.angle + tv.spin * (self.time - tv.start) as f64,
                    divisions: tv.divisions,
                };
                let o = tv.object;
                if o != NIL {
                    self.objects.v[o].last = trace;
                    self.objects.v[o].active = true;
                }
            }
            traj = next;
        }

        self.show_figure(g, false);
        self.show_arms(g);

        /* Draw Objects */
        g.glx.translate(0.0, 0.0, ARMLENGTH);
        let mut o = self.objects.v[0].next;
        while o != 0 {
            let next = self.objects.v[o].next;
            if self.objects.v[o].active {
                let (ty, color, last) = (
                    self.objects.v[o].ty,
                    self.objects.v[o].color,
                    self.objects.v[o].last,
                );
                self.show_object(g, ty, color, &last);
                self.objects.v[o].active = false;
            }
            o = next;
        }

        /* Save the pattern name so we can erase it when it changes */
        if let Some(p) = pattern
            && p != self.pattern
        {
            self.pattern = p;
        }

        g.glx.pop_matrix();

        let (w, h) = (g.width(), g.height());
        self.font
            .print_label(&mut g.glx, &self.pattern, w, h, 1, [1.0, 1.0, 0.0, 1.0]);

        if future < self.time + 100 * self.throw_catch() as i64 {
            self.refill(self.colors.len());
        }

        delay
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let h = height.max(1) as f32 / width as f32;

        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, 1.0 / h, 1.0, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        self.scene_width = SCENE_HEIGHT * (width as f64 / height.max(1) as f64);
        // Use MIN so that users can resize in interesting ways, eg narrow
        // windows for tall patterns.
        self.scale = (SCENE_HEIGHT / 480.0).min(self.scene_width / 160.0);
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            true
        } else if screenhack_event_helper(event) {
            let n = self.colors.len();
            self.change(n);
            true
        } else {
            false
        }
    }
}

fn make(g: &mut Gl) -> Juggle {
    let wire = g.res.bool("wireframe");

    if !wire {
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        g.glx.depth_test(true);
        g.glx.cull_face(true);

        g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
        g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_specular(0, [0.0, 1.0, 1.0, 1.0]);
    }

    let mut allowed: Vec<ObjType> = ObjType::ALL
        .into_iter()
        .filter(|o| {
            g.res.bool(match o {
                ObjType::Ball => "balls",
                ObjType::Club => "clubs",
                ObjType::Knife => "knives",
                ObjType::Ring => "rings",
                ObjType::BBall => "bballs",
            })
        })
        .collect();
    if allowed.is_empty() {
        /* Have to juggle something! */
        allowed.push(ObjType::Ball);
    }

    // Sort the library by how many objects each pattern needs, and note where
    // each group starts, so that a pattern for a given number can be picked.
    let mut sorted: Vec<(&'static str, &'static str)> = PORTFOLIO.to_vec();
    sorted.sort_by_key(|p| get_num_balls(p.0));
    let maxof = get_num_balls(sorted[sorted.len() - 1].0);
    let mut index = vec![PatternIndex::default(); (maxof + 2) as usize];
    let mut minballs = 0;
    let mut maxballs = 1;
    let mut numpat = 0;
    for (i, p) in sorted.iter().enumerate() {
        let b = get_num_balls(p.0);
        if b > maxballs {
            index[maxballs as usize].number = numpat;
            if numpat == 0 {
                minballs = b;
            }
            maxballs = b;
            numpat = 1;
            index[maxballs as usize].start = i;
        } else {
            numpat += 1;
        }
    }
    index[maxballs as usize].number = numpat;

    let count = g.res.int("count").unsigned_abs() as f64;
    let mut st = Juggle {
        rot: Rotator::new(0.0, 0.05, 0.0, 0.05, 0.001, false),
        trackball: Trackball::new(),
        font: TexFont::load(&mut g.glx, "sans-serif 18"),
        scale: 1.0,
        cx: 0.0,
        gr: 0.0,
        trajs: Ring::new(),
        objects: Ring::new(),
        arm: [[[Point::default(); 3]; 2]; 2],
        pattern: String::new(),
        count: if count == 0.0 { 200.0 } else { count },
        num_balls: 0,
        time: 0,
        objtypes: ObjType::Ball,
        colors: make_random_colormap(256, true),
        sorted,
        index,
        minballs,
        maxballs,
        cycles: g.res.int("cycles").clamp(1, 5000),
        wire,
        allowed,
        scene_width: SCENE_HEIGHT,
    };

    if st.maxballs > 0 {
        st.num_balls = st.minballs + random_below(st.maxballs - st.minballs);
    }

    /* Discovers information about the juggler's proportions */
    st.init_arms();

    // "7" should be about three times the height of the juggler's shoulders
    let h = 3.0 * st.arm[0][Hand::Right as usize][SHOULDER].y;
    let t = 7.0 * st.throw_catch();
    st.gr = -(4.0 * h / (t * t));

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    let n = st.colors.len();
    st.change(n);
    st
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    Box::new(make(g))
}

const DEFAULTS: &[&str] = &[
    "*delay:      10000",
    "*count:        200",
    "*cycles:      1000",
    "*showFPS:    False",
    "*wireframe:  False",
    "*titleFont:  sans-serif 18",
    "*balls:       True",
    "*clubs:       True",
    "*knives:      True",
    "*rings:       True",
    "*bballs:      True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("count", "Speed", 50.0, 1000.0, 10.0, 0, "200").inverted(),
    Opt::slider(
        "cycles",
        "Performance length",
        50.0,
        1000.0,
        10.0,
        0,
        "1000",
    ),
    Opt::boolean("balls", "Balls", "true"),
    Opt::boolean("clubs", "Clubs", "true"),
    Opt::boolean("rings", "Rings", "true"),
    Opt::boolean("knives", "Knives", "true"),
    Opt::boolean("bballs", "Bowling balls", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "juggler3d",
    label: "Juggler 3D",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Tim Auckland and Jamie Zawinski",
        year: "2002",
        video: Some("https://www.youtube.com/watch?v=TJkKaXBOvCA"),
        blurb: "A 3D juggling stick-person, with Cambridge juggling pattern \
                notation used to describe the patterns juggled.",
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

    /// A display with the saver's own defaults in it, which `Gl::for_test`
    /// does not carry.
    fn display() -> Gl {
        crate::runtime::ya_rand_init(20260812);
        let mut g = Gl::for_test(640, 480);
        for d in DEFAULTS {
            if let Some((k, v)) = d.split_once(':') {
                g.res.set(k.trim().trim_start_matches(['.', '*']), v.trim());
            }
        }
        g
    }

    fn juggler(g: &mut Gl) -> Juggle {
        let mut st = make(g);
        st.reshape(g, 640, 480);
        st
    }

    /// A pattern with nothing but the pipeline run over it, so the stages can
    /// be checked without the drawing.
    fn programmed(pat: &str, cycles: i32) -> Juggle {
        crate::runtime::ya_rand_init(20260812);
        let mut st = Juggle {
            rot: Rotator::new(0.0, 0.05, 0.0, 0.05, 0.001, false),
            trackball: Trackball::new(),
            font: TexFont::load(&mut Gl::for_test(64, 64).glx, "sans-serif 18"),
            scale: 1.0,
            cx: 0.0,
            gr: -0.001,
            trajs: Ring::new(),
            objects: Ring::new(),
            arm: [[[Point::default(); 3]; 2]; 2],
            pattern: String::new(),
            count: 200.0,
            num_balls: 3,
            time: 0,
            objtypes: ObjType::Ball,
            colors: make_random_colormap(256, true),
            sorted: PORTFOLIO.to_vec(),
            index: vec![PatternIndex::default(); 12],
            minballs: 2,
            maxballs: 7,
            cycles: 100,
            wire: false,
            allowed: vec![ObjType::Ball],
            scene_width: 640.0,
        };
        st.init_arms();
        st.program(pat, None, cycles);
        st
    }

    #[test]
    fn a_siteswap_averages_to_the_number_of_objects() {
        // The theorem the whole notation rests on: a pattern is juggleable
        // exactly when its throws average to a whole number, and that number
        // is how many objects it takes. Every pattern in the library has to
        // pass, or the juggler would be dropping.
        for &(pat, name) in PORTFOLIO {
            let mut st = programmed(pat, 300);
            st.adam();
            let mut heights = Vec::new();
            let mut t = st.trajs.v[0].next;
            while t != 0 {
                if st.trajs.v[t].height >= 0 {
                    heights.push(st.trajs.v[t].height);
                }
                t = st.trajs.v[t].next;
            }
            assert!(heights.len() > 60, "{pat}: only {} throws", heights.len());
            // Drop the tail, where `adam` runs out of lookahead, and then
            // look for a prefix that is a whole number of repeats: the
            // average only comes out exact over one of those, and how long
            // the pattern is is not written down anywhere.
            heights.truncate(heights.len() - 12);
            let mut sum: i32 = heights.iter().sum();
            let balls = (heights.len() - 20..heights.len()).rev().find_map(|n| {
                let s = sum;
                sum -= heights[n];
                let n = n as i32 + 1;
                (s % n == 0 && (1..=8).contains(&(s / n))).then_some(s / n)
            });
            assert!(
                balls.is_some(),
                "{pat} ({name}): no whole number of repeats averages to a \
                 whole number of objects: {heights:?}"
            );
        }
    }

    #[test]
    fn every_throw_is_caught_by_a_hand_that_then_throws() {
        let mut st = programmed("[3]", 200);
        st.adam();
        st.name();
        st.part();
        st.lob(256);

        // `part` splits each throw into a throw and a catch, so there are
        // twice as many, and they alternate hands.
        let mut throws = 0;
        let mut catches = 0;
        let mut linked = 0;
        let mut t = st.trajs.v[0].next;
        while t != 0 {
            let tv = &st.trajs.v[t];
            // Only what `part` reached: whatever `adam` ran out of lookahead
            // for is still sitting at its earlier stage.
            if tv.status < Status::Action {
                t = tv.next;
                continue;
            }
            match tv.action {
                Action::Throw => throws += 1,
                Action::Catch => catches += 1,
            }
            if tv.status == Status::LinkedAction && tv.handlink != NIL {
                linked += 1;
                // A hand's next event is always in the same hand.
                assert_eq!(
                    st.trajs.v[tv.handlink].hand, tv.hand,
                    "a hand handed off to the other one"
                );
            }
            t = st.trajs.v[t].next;
        }
        assert_eq!(throws, catches, "{throws} throws but {catches} catches");
        assert!(linked > 20, "only {linked} linked");

        // A three-cascade needs three objects. The whole performance is
        // programmed at once, so every throw that will ever be made already
        // has one; what matters is that they are shared between throws
        // rather than made fresh for each.
        assert!(st.objects.live() >= 3, "{}", st.objects.live());
        assert!(
            (st.objects.live() as i32) < throws,
            "{} objects for {throws} throws",
            st.objects.live()
        );
    }

    #[test]
    fn a_thrown_object_comes_back_down() {
        let mut st = programmed("[3]", 200);
        st.adam();
        st.name();
        st.part();
        st.lob(256);
        st.clap();
        st.positions();
        st.projectile();
        st.hands();

        let mut flights = 0;
        let mut t = st.trajs.v[0].next;
        while t != 0 {
            let tv = &st.trajs.v[t];
            if tv.status == Status::Predictor && tv.ty == Throwable::Ball {
                flights += 1;
                let (a, b) = (tv.start as f64, tv.finish as f64);
                assert!(b > a, "a flight that ends before it starts");
                // The path is a parabola opening downwards, so the object is
                // higher in the middle than at either end.
                let mid = tv.yp.at((a + b) / 2.0);
                assert!(
                    mid > tv.yp.at(a) - 1.0 && mid > tv.yp.at(b) - 1.0,
                    "an object that did not go up: {} {} {}",
                    tv.yp.at(a),
                    mid,
                    tv.yp.at(b)
                );
                // And it never goes below the floor.
                for k in 0..=10 {
                    let t = a + (b - a) * k as f64 / 10.0;
                    assert!(tv.yp.at(t) > -1.0, "through the floor at {t}");
                }
            }
            t = st.trajs.v[t].next;
        }
        assert!(flights > 20, "only {flights} flights");
    }

    #[test]
    fn an_arm_reaches_or_points() {
        let s = Point { x: 25.0, y: 130.0 };
        // Within reach: the hand lands on the target and the elbow is bent.
        let p = Point { x: 30.0, y: 100.0 };
        let (h, e) = find_elbow(40.0, p, s, 25.0);
        assert!((h.x - p.x).abs() < 1e-9 && (h.y - p.y).abs() < 1e-9);
        let bend = ((e.x - s.x).powi(2) + (e.y - s.y).powi(2)).sqrt();
        assert!(bend > 1.0 && bend < 60.0, "{bend}");

        // Out of reach: the hand is on the line to the target, at the full
        // stretch of both bones.
        let far = Point { x: 500.0, y: 130.0 };
        let (h, e) = find_elbow(40.0, far, s, 25.0);
        assert!(h.x < far.x, "the arm stretched all the way there");
        assert!((h.y - s.y).abs() < 1e-9, "{h:?}");
        // The elbow is halfway along, so the two bones are the same length.
        // They come to a little under forty because the arm reaches out in z
        // as well and only x and y are answered.
        let upper = ((e.x - s.x).powi(2) + (e.y - s.y).powi(2)).sqrt();
        let lower = ((h.x - e.x).powi(2) + (h.y - e.y).powi(2)).sqrt();
        assert!((upper - lower).abs() < 1e-6, "{upper} vs {lower}");
        assert!((39.0..40.0).contains(&upper), "{upper}");
    }

    #[test]
    fn the_juggler_juggles() {
        let mut g = display();
        let mut st = juggler(&mut g);
        let mut most = 0;
        let mut seen_props = 0;
        for _ in 0..600 {
            g.glx.start_frame(640, 480);
            st.draw(&mut g);
            most = most.max(g.glx.frame().batches.len());
            seen_props = seen_props.max(st.objects.live());
            assert!(!g.glx.frame().batches.is_empty());
        }
        // Something is in the air, and the list has not run away.
        assert!(seen_props >= 2, "{seen_props}");
        assert!(st.trajs.v.len() < 4000, "{}", st.trajs.v.len());
        assert!(most < 2000, "{most}");
        // And the hands have moved off where they started.
        let h = st.arm[1][Hand::Left as usize][HAND];
        assert!(h.x != 15.0 || h.y != 90.0, "{h:?}");
    }
}
