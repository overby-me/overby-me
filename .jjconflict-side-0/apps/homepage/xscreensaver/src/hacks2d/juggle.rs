//! Port of `hacks/juggle.c`.
//!
//! ```text
//! juggle
//!
//! Copyright (c) 1996 by Tim Auckland <tda10.geo@yahoo.com>
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
//! ```
//!
//! A stick figure juggling, and the interesting part is not the drawing but
//! what it is given to juggle. Patterns are written in Adam Chalcraft's
//! notation, in which any sequence of numbers at all is a valid pattern, so
//! new ones can be invented at random and spliced together without checking
//! anything. Site-swap notation, the one jugglers use, does not have that
//! property: there the sequence has to make `n + s(n)` a bijection or two balls
//! arrive at once. The hack generates in Adam notation and converts, which is
//! what `adam` below does, and the pattern named on screen is the converted
//! form.
//!
//! Turning a pattern into a picture is a pipeline, and each trajectory in the
//! list carries how far along it has come. Throws and catches are split apart,
//! then linked so each throw knows which catch receives its object and which
//! catch frees its hand, then given positions and times, then turned into
//! parabolas (or, for a bounce, two parabolas meeting at the floor, with the
//! bounce time found by bisection), and finally the hands are given cubic
//! splines through catch and on to the next throw. Six kinds of object can be
//! in the air, each with its own weight, bounciness and length of trail.
//!
//! Upstream draws the name of the running pattern across the top. There is no
//! font here, and upstream skips the text itself when the font will not load,
//! so this keeps the rest of that code (the top strip is still cleared when the
//! pattern changes) and draws nothing.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, lrand, nrand};
use crate::runtime::{About, Dpy, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, XPoint};

/* Figure */
/// Objects spinning out of the plane fake perspective by squashing their
/// horizontal coordinates by this much.
const PERSPEC: f64 = 0.4;
/// Shoulder width. Upstream also defines an arm length and a pose offset and
/// then uses neither, reaching with a hard-coded 40 and posing at half a
/// shoulder width, so those are not here.
const SX: f64 = 25.0;

/// Inverse of the chance of using an odd object in the pattern.
const OBJMIXPROB: i32 = 20;

const BOUNCEOVER: i32 = 10;
const KICKMIN: i32 = 7;
const THROWMAX: i32 = 20;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum ObjType {
    #[default]
    Ball,
    Club,
    Torch,
    Knife,
    Ring,
    BBall,
}

const NUM_OBJECT_TYPES: i32 = 6;

impl ObjType {
    fn from_index(i: i32) -> Self {
        match i {
            1 => ObjType::Club,
            2 => ObjType::Torch,
            3 => ObjType::Knife,
            4 => ObjType::Ring,
            5 => ObjType::BBall,
            _ => ObjType::Ball,
        }
    }

    /// Length of the object's handle.
    fn handle(self) -> f64 {
        match self {
            ObjType::Ball | ObjType::BBall => 0.0,
            _ => 15.0,
        }
    }

    /// Minimum trail length. Torches need flames.
    fn mintrail(self) -> i32 {
        match self {
            ObjType::Torch => 20,
            _ => 1,
        }
    }

    /// Coefficient of restitution: a perfect bounce is 1. Torches and knives
    /// do not bounce, for reasons upstream gives as fire risk.
    fn cor(self) -> f64 {
        match self {
            ObjType::Ball => 0.9,
            ObjType::Club => 0.55,
            ObjType::Torch | ObjType::Knife => 0.0,
            ObjType::Ring => 0.8,
            ObjType::BBall => 0.2,
        }
    }

    /// Heavier objects do not get thrown as high.
    fn weight(self) -> f64 {
        match self {
            ObjType::BBall => 5.0,
            _ => 1.0,
        }
    }
}

/// How far along the pipeline a trajectory has come. The order is the order of
/// the stages, and the code compares statuses to ask "has this got as far as".
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Debug)]
enum Status {
    #[default]
    Atch,
    Thratch,
    Action,
    LinkedAction,
    PThratch,
    BPredictor,
    Predictor,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum Throwable {
    #[default]
    Empty,
    Full,
    Ball,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum Hand {
    #[default]
    Left,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum ActionKind {
    #[default]
    Throw,
    Catch,
}

#[derive(Clone, Copy, Default)]
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

#[derive(Clone, Copy, Default, PartialEq)]
struct DPoint {
    x: f64,
    y: f64,
}

/// A circular doubly-linked list held in a `Vec`, with a self-linked sentinel
/// per list and a free list for reuse.
///
/// Upstream uses four intrusive linked lists and edits them while walking them,
/// which is why this is an arena of indices: the shape of the algorithm is the
/// list, and turning it into anything else would mean rewriting it.
struct Arena<T> {
    nodes: Vec<Node<T>>,
    free: Vec<usize>,
}

struct Node<T> {
    next: usize,
    prev: usize,
    value: T,
}

impl<T: Default> Arena<T> {
    fn new() -> Self {
        Arena {
            nodes: Vec::new(),
            free: Vec::new(),
        }
    }

    /// A new empty list: one sentinel linked to itself.
    fn new_list(&mut self) -> usize {
        let i = self.alloc(T::default());
        self.nodes[i].next = i;
        self.nodes[i].prev = i;
        i
    }

    fn alloc(&mut self, value: T) -> usize {
        match self.free.pop() {
            Some(i) => {
                self.nodes[i] = Node {
                    next: i,
                    prev: i,
                    value,
                };
                i
            }
            None => {
                self.nodes.push(Node {
                    next: 0,
                    prev: 0,
                    value,
                });
                self.nodes.len() - 1
            }
        }
    }

    /// `ADD_ELEMENT`: a new node just after `at`.
    fn add_after(&mut self, at: usize, value: T) -> usize {
        let n = self.alloc(value);
        let after = self.nodes[at].next;
        self.nodes[n].next = after;
        self.nodes[n].prev = at;
        self.nodes[at].next = n;
        self.nodes[after].prev = n;
        n
    }

    /// `REMOVE`: unlink and return the node to the free list.
    fn remove(&mut self, i: usize) {
        let (p, n) = (self.nodes[i].prev, self.nodes[i].next);
        self.nodes[p].next = n;
        self.nodes[n].prev = p;
        self.nodes[i].value = T::default();
        self.free.push(i);
    }

    fn next(&self, i: usize) -> usize {
        self.nodes[i].next
    }

    fn prev(&self, i: usize) -> usize {
        self.nodes[i].prev
    }

    fn get(&self, i: usize) -> &T {
        &self.nodes[i].value
    }

    fn get_mut(&mut self, i: usize) -> &mut T {
        &mut self.nodes[i].value
    }
}

/// A segment of juggling action. A list of these is the whole performance, and
/// it is rewritten in place stage by stage.
#[derive(Clone, Default)]
struct Trajectory {
    status: Status,

    /* Throw */
    posn: char,
    height: i32,
    adam: i32,
    pattern: Option<String>,
    name: Option<String>,

    /* Action */
    hand: Hand,
    action: ActionKind,

    /* LinkedAction */
    object: Option<usize>,
    divisions: i32,
    angle: f64,
    spin: f64,
    balllink: Option<usize>,
    handlink: Option<usize>,

    /* PThratch */
    /// The juggler wanders, so every position is relative to where he is.
    cx: f64,
    x: f64,
    y: f64,
    dx: f64,
    dy: f64,

    /* Predictor */
    kind: Throwable,
    start: i64,
    finish: i64,
    xp: Spline,
    yp: Spline,
}

/// An object being juggled. Trajectories reference it, `count` tracks how many,
/// and it goes when nothing references it and its trail has been erased.
#[derive(Clone, Default)]
struct Object {
    kind: ObjType,
    color: i32,
    count: i32,
    active: bool,
    /// Sentinel of this object's own trail list.
    trace: usize,
    tracelen: i32,
    tail: i32,
}

/// One drawn position of an object, kept so it can be erased later.
#[derive(Clone, Copy, Default)]
struct Trace {
    x: f64,
    y: f64,
    angle: f64,
    divisions: i32,
    /// Where the head of a torch was last frame, saved because the trace it
    /// was computed from will have gone by the time this one is erased.
    dlast: DPoint,
}

/// A spline and the time it runs to. A list of them is the juggler's path
/// across the screen.
#[derive(Clone, Copy, Default)]
struct Wander {
    x: f64,
    finish: i64,
    s: Spline,
}

struct PatternEntry {
    pattern: &'static str,
    name: &'static str,
}

#[derive(Clone, Copy, Default)]
struct PatternIndex {
    start: usize,
    number: i32,
}

/// Which joint of an arm: the three points a drawn arm passes through.
const HAND: usize = 0;
const ELBOW: usize = 1;
const SHOULDER: usize = 2;

struct Juggle {
    mi: ModeInfo,
    scale: f64,
    cx: f64,
    gr: f64,

    traj: Arena<Trajectory>,
    head: usize,
    objects: Arena<Object>,
    objects_head: usize,
    traces: Arena<Trace>,
    wanders: Arena<Wander>,
    wander_head: usize,

    /// `arm[0]` is where the arms were drawn last, so they can be erased;
    /// `arm[1]` is where they are now.
    arm: [[[DPoint; 3]; 2]; 2],
    pattern: String,
    count: i32,
    num_balls: i32,
    time: i64,
    objtypes: ObjType,

    portfolio: Vec<PatternEntry>,
    index: Vec<PatternIndex>,
    minballs: i32,
    maxballs: i32,

    /* resources */
    tail: i32,
    real: bool,
    balls: bool,
    clubs: bool,
    torches: bool,
    knives: bool,
    rings: bool,
    bballs: bool,
    /// Set once the figure has been drawn, which is also what measures him.
    started: bool,
}

/* Timing based on count.  Units are milliseconds.  Juggles per second
is: 2000 / THROW_CATCH_INTERVAL + CATCH_THROW_INTERVAL */
impl Juggle {
    fn throw_catch_interval(&self) -> f64 {
        f64::from(self.count)
    }

    fn throw_null_interval(&self) -> f64 {
        f64::from(self.count) * 0.5
    }

    fn catch_throw_interval(&self) -> f64 {
        f64::from(self.count) * 0.2
    }

    fn armwidth(&self) -> i32 {
        (8.0 * self.scale.sqrt()) as i32
    }

    fn ballradius(&self) -> i32 {
        self.armwidth()
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mi = ModeInfo::new(d, ColorScheme::Random);
    let mut traj = Arena::new();
    let head = traj.new_list();
    let mut objects = Arena::new();
    let objects_head = objects.new_list();
    let mut wanders = Arena::new();
    let wander_head = wanders.new_list();

    let count = mi.count.abs();
    let mut st = Juggle {
        mi,
        scale: 1.0,
        cx: 0.0,
        gr: 0.0,
        traj,
        head,
        objects,
        objects_head,
        traces: Arena::new(),
        wanders,
        wander_head,
        arm: [[[DPoint::default(); 3]; 2]; 2],
        pattern: String::new(),
        count: if count == 0 { 200 } else { count },
        num_balls: 0,
        time: 0,
        objtypes: ObjType::Ball,
        portfolio: Vec::new(),
        index: Vec::new(),
        minballs: 0,
        maxballs: 0,
        tail: d.res.int("tail"),
        real: d.res.bool("real"),
        balls: d.res.bool("balls"),
        clubs: d.res.bool("clubs"),
        torches: d.res.bool("torches"),
        knives: d.res.bool("knives"),
        rings: d.res.bool("rings"),
        bballs: d.res.bool("bballs"),
        started: false,
    };
    if !(st.balls || st.clubs || st.torches || st.knives || st.rings || st.bballs) {
        st.balls = true; /* Have to juggle something! */
    }
    st.reset(d);
    Box::new(st)
}

impl Juggle {
    /*******************
     * list management *
     *******************/

    /// `DUP_OBJECT`: take a second reference to another trajectory's object.
    fn dup_object(&mut self, n: usize, t: usize) {
        let o = self.traj.get(t).object;
        self.traj.get_mut(n).object = o;
        if let Some(o) = o {
            self.objects.get_mut(o).count += 1;
        }
    }

    fn object_destroy(&mut self, o: usize) {
        let trace = self.objects.get(o).trace;
        while self.traces.next(trace) != trace {
            let s = self.traces.next(trace);
            self.traces.remove(s);
        }
        self.traces.remove(trace);
        self.objects.remove(o);
    }

    fn trajectory_destroy(&mut self, t: usize) {
        if let Some(o) = self.traj.get(t).object {
            let ob = self.objects.get_mut(o);
            ob.count -= 1;
            if ob.count < 1 && ob.tracelen == 0 {
                self.object_destroy(o);
            }
        }
        self.traj.remove(t);
    }

    /// `free_juggle`: throw the whole performance away.
    fn free_performance(&mut self) {
        while self.traj.next(self.head) != self.head {
            let t = self.traj.next(self.head);
            self.trajectory_destroy(t);
        }
        while self.objects.next(self.objects_head) != self.objects_head {
            let o = self.objects.next(self.objects_head);
            self.object_destroy(o);
        }
        while self.wanders.next(self.wander_head) != self.wander_head {
            let w = self.wanders.next(self.wander_head);
            self.wanders.remove(w);
        }
        self.pattern.clear();
    }

    /*******************
     * the pipeline    *
     *******************/

    fn add_throw(&mut self, kind: char, h: i32, adam_notation: bool, name: Option<&str>) {
        let at = self.traj.prev(self.head);
        let t = self.traj.add_after(at, Trajectory::default());
        let tr = self.traj.get_mut(t);
        tr.object = None;
        tr.name = name.map(str::to_string);
        tr.posn = kind;
        if adam_notation {
            tr.adam = h;
            tr.height = 0;
            tr.status = Status::Atch;
        } else {
            tr.height = h;
            tr.status = Status::Thratch;
        }
    }

    /// Add a stretch of pattern to the performance, repeating it until at
    /// least `cycles` throws have been programmed.
    fn program(&mut self, patn: &str, name: Option<&str>, cycles: i32) {
        let mut i = 0;
        while i < cycles {
            // The title goes on the first throw of a sequence. With no name,
            // an empty title still delimits one sequence from the next.
            let mut title: Option<String> = Some(name.unwrap_or("").to_string());
            let mut kind = ' ';
            let mut h = 0;
            let mut seen = false;
            let mut adam_notation = false;
            for p in patn.chars() {
                if p.is_ascii_digit() {
                    seen = true;
                    h = 10 * h + (p as i32 - '0' as i32);
                    continue;
                }
                let mut nn = adam_notation;
                match p {
                    '[' => adam_notation = true,
                    '-' => kind = ' ',
                    '+' | '=' | '&' | 'x' | '_' | 'k' => kind = p,
                    '*' | ']' | ' ' => {
                        if p == '*' {
                            seen = true;
                            h = -1;
                        }
                        if p == '*' || p == ']' {
                            nn = false;
                        }
                        if seen {
                            i += 1;
                            self.add_throw(kind, h, adam_notation, title.as_deref());
                            title = None;
                            kind = ' ';
                            h = 0;
                            seen = false;
                        }
                        adam_notation = nn;
                    }
                    // Upstream warns about an unexpected instruction on the
                    // first pass only. There is nowhere to warn.
                    _ => {}
                }
            }
            if seen {
                self.add_throw(kind, h, adam_notation, title.as_deref());
            }
            i += 1;
        }
    }

    /// Convert Adam notation into heights: how long the ball has to stay up is
    /// how many throws it takes for this one to come back round.
    fn adam(&mut self) {
        let mut t = self.traj.next(self.head);
        while t != self.head {
            if self.traj.get(t).status == Status::Atch {
                let mut a = self.traj.get(t).adam;
                self.traj.get_mut(t).height = 0;
                let mut p = self.traj.next(t);
                while a > 0 {
                    if p == self.head {
                        // Indicate end of processing for name().
                        self.traj.get_mut(t).height = -9;
                        return;
                    }
                    let pr = self.traj.get(p);
                    if pr.status != Status::Atch || pr.adam < 0 || pr.adam >= a {
                        a -= 1;
                    }
                    self.traj.get_mut(t).height += 1;
                    p = self.traj.next(p);
                }
                let tr = self.traj.get_mut(t);
                if tr.height > BOUNCEOVER && tr.posn == ' ' {
                    tr.posn = '_'; /* high defaults can be bounced */
                } else if tr.height < 3 && tr.posn == '_' {
                    tr.posn = ' '; /* Can't bounce short throws. */
                }
                if tr.height < KICKMIN && tr.posn == 'k' {
                    tr.posn = ' '; /* Can't kick short throws */
                }
                if tr.height > THROWMAX {
                    tr.posn = 'k'; /* Use kicks for ridiculously high throws */
                }
                tr.status = Status::Thratch;
            }
            t = self.traj.next(t);
        }
    }

    /// Once the heights are known, write the sequence out in the notation
    /// jugglers read, and hang it on the throw that starts the sequence.
    fn name(&mut self) {
        let mut t = self.traj.next(self.head);
        while t != self.head {
            if self.traj.get(t).status == Status::Thratch && self.traj.get(t).name.is_some() {
                let mut buffer = String::new();
                let mut p = t;
                loop {
                    if p != t && self.traj.get(p).name.is_some() {
                        break;
                    }
                    if p == self.head || self.traj.get(p).height < 0 {
                        return; /* end of reliable data */
                    }
                    let pr = self.traj.get(p);
                    if pr.posn == ' ' {
                        buffer.push_str(&format!(" {}", pr.height));
                    } else {
                        buffer.push_str(&format!(" {}{}", pr.posn, pr.height));
                    }
                    if buffer.len() > 500 {
                        break; /* it'll be too big to display anyway */
                    }
                    p = self.traj.next(p);
                }
                let tr = self.traj.get_mut(t);
                if let Some(n) = tr.name.take()
                    && !n.is_empty()
                {
                    buffer.push_str(&format!(", {n}"));
                }
                tr.pattern = Some(buffer);
            }
            t = self.traj.next(t);
        }
    }

    /// Split each throw-and-catch into an explicit throw and an explicit catch.
    /// Usually the catch follows the throw in the same hand, but the throw
    /// styles decide which hand and where.
    fn part(&mut self) {
        let mut hand = if lrand() & 1 != 0 {
            Hand::Right
        } else {
            Hand::Left
        };

        let mut t = self.traj.next(self.head);
        while t != self.head {
            let status = self.traj.get(t).status;
            if status > Status::Thratch {
                hand = self.traj.get(t).hand;
            } else if status == Status::Thratch {
                let mut posn = '=';

                {
                    let tr = self.traj.get_mut(t);
                    /* plausibility check */
                    if tr.height <= 2 && tr.posn == '_' {
                        tr.posn = ' '; /* no short bounces */
                    }
                    if tr.height <= 1 && (tr.posn == '=' || tr.posn == '&') {
                        tr.posn = ' '; /* 1's need close catches */
                    }
                    /*         throw          catch    */
                    match tr.posn {
                        ' ' => {
                            posn = '-';
                            tr.posn = '+';
                        }
                        '+' => {
                            posn = '+';
                            tr.posn = '-';
                        }
                        '=' => {
                            posn = '=';
                            tr.posn = '+';
                        }
                        '&' => {
                            posn = '+';
                            tr.posn = '=';
                        }
                        'x' => {
                            posn = '=';
                            tr.posn = '=';
                        }
                        '_' => {
                            posn = '_';
                            tr.posn = '-';
                        }
                        'k' => {
                            posn = 'k';
                            tr.posn = 'k';
                        }
                        _ => {}
                    }
                }

                hand = if hand == Hand::Left {
                    Hand::Right
                } else {
                    Hand::Left
                };
                let height = {
                    let tr = self.traj.get_mut(t);
                    tr.status = Status::Action;
                    tr.hand = hand;
                    tr.action = ActionKind::Catch;
                    tr.height
                };

                let mut p = self.traj.prev(t);
                if height == 1 && p != self.head {
                    p = self.traj.prev(p); /* '1's are thrown earlier than usual */
                }

                let nt = self.traj.add_after(p, Trajectory::default());
                let ntr = self.traj.get_mut(nt);
                ntr.object = None;
                ntr.status = Status::Action;
                ntr.action = ActionKind::Throw;
                ntr.height = height;
                ntr.hand = hand;
                ntr.posn = posn;
            }
            t = self.traj.next(t);
        }
    }

    fn choose_object(&self) -> ObjType {
        loop {
            let o = ObjType::from_index(nrand(NUM_OBJECT_TYPES));
            let ok = match o {
                ObjType::Ball => self.balls,
                ObjType::Club => self.clubs,
                ObjType::Torch => self.torches,
                ObjType::Knife => self.knives,
                ObjType::Ring => self.rings,
                ObjType::BBall => self.bballs,
            };
            if ok {
                return o;
            }
        }
    }

    /// Connect throws to catches to work out which ball goes where, and do the
    /// same with the juggler's hands.
    fn lob(&mut self) {
        let mut t = self.traj.next(self.head);
        while t != self.head {
            if self.traj.get(t).status == Status::Action {
                if self.traj.get(t).action == ActionKind::Throw {
                    if self.traj.get(t).kind == Throwable::Empty {
                        /* Create new Object */
                        let o = self.objects.add_after(self.objects_head, Object::default());
                        let trace = self.traces.new_list();
                        let color = if self.mi.npixels() > 2 {
                            1 + nrand(self.mi.npixels() - 2)
                        } else {
                            1
                        };
                        // Small chance of picking a random object instead of
                        // the current theme.
                        let kind = if nrand(OBJMIXPROB) == 0 {
                            self.choose_object()
                        } else {
                            self.objtypes
                        };
                        let tail = self.tail.max(kind.mintrail());
                        let ob = self.objects.get_mut(o);
                        ob.count = 1;
                        ob.tracelen = 0;
                        ob.active = false;
                        ob.trace = trace;
                        ob.color = color;
                        ob.kind = kind;
                        ob.tail = tail;
                        self.traj.get_mut(t).object = Some(o);
                    }

                    /* Balls can change divisions at each throw */
                    self.traj.get_mut(t).divisions = 2 * (nrand(2) + 1);

                    /* search forward for next catch in this hand */
                    let mut p = self.traj.next(t);
                    while self.traj.get(t).handlink.is_none() {
                        if self.traj.get(p).status < Status::Action || p == self.head {
                            return;
                        }
                        if self.traj.get(p).action == ActionKind::Catch
                            && self.traj.get(p).hand == self.traj.get(t).hand
                        {
                            self.traj.get_mut(t).handlink = Some(p);
                        }
                        p = self.traj.next(p);
                    }

                    if self.traj.get(t).height > 0 {
                        let mut h = self.traj.get(t).height - 1;
                        /* search forward for next ball catch */
                        let mut p = self.traj.next(t);
                        while self.traj.get(t).balllink.is_none() {
                            if self.traj.get(p).status < Status::Action || p == self.head {
                                self.traj.get_mut(t).handlink = None;
                                return;
                            }
                            if self.traj.get(p).action == ActionKind::Catch {
                                h -= 1;
                                if h < 1 {
                                    /* caught: complete the trajectory */
                                    self.traj.get_mut(t).balllink = Some(p);
                                    self.traj.get_mut(p).kind = Throwable::Full;
                                    self.dup_object(p, t); /* accept catch */
                                    let (angle, divisions) = {
                                        let tr = self.traj.get(t);
                                        (tr.angle, tr.divisions)
                                    };
                                    let pr = self.traj.get_mut(p);
                                    pr.angle = angle;
                                    pr.divisions = divisions;
                                }
                            }
                            p = self.traj.next(p);
                        }
                    }
                    self.traj.get_mut(t).kind = Throwable::Empty; /* thrown */
                } else {
                    /* search forward for next throw from this hand */
                    let mut p = self.traj.next(t);
                    while self.traj.get(t).handlink.is_none() {
                        if self.traj.get(p).status < Status::Action || p == self.head {
                            return;
                        }
                        if self.traj.get(p).action == ActionKind::Throw
                            && self.traj.get(p).hand == self.traj.get(t).hand
                        {
                            let (kind, divisions) = {
                                let tr = self.traj.get(t);
                                (tr.kind, tr.divisions)
                            };
                            self.traj.get_mut(p).kind = kind; /* pass ball */
                            self.dup_object(p, t); /* pass object */
                            self.traj.get_mut(p).divisions = divisions;
                            self.traj.get_mut(t).handlink = Some(p);
                        }
                        p = self.traj.next(p);
                    }
                }
                self.traj.get_mut(t).status = Status::LinkedAction;
            }
            t = self.traj.next(t);
        }
    }

    /// Clap when both hands are empty, by moving the idle throw and the idle
    /// catch to meet in the middle.
    fn clap(&mut self) {
        let mut t = self.traj.next(self.head);
        while t != self.head {
            let tr = self.traj.get(t);
            let idle = tr.status == Status::LinkedAction
                && tr.action == ActionKind::Catch
                && tr.kind == Throwable::Empty
                && tr.handlink.is_some_and(|h| self.traj.get(h).height == 0);
            if idle {
                let hand = tr.hand;
                let mut p = self.traj.next(t);
                while p != self.head {
                    let pr = self.traj.get(p);
                    if pr.status == Status::LinkedAction
                        && pr.action == ActionKind::Catch
                        && pr.hand != hand
                    {
                        if pr.kind == Throwable::Empty
                            && pr.handlink.is_some_and(|h| self.traj.get(h).height == 0)
                        {
                            let link = self.traj.get(t).handlink;
                            if let Some(l) = link {
                                self.traj.get_mut(l).posn = '^';
                            }
                            self.traj.get_mut(p).posn = '^';
                        }
                        break; /* Only need first catch */
                    }
                    p = self.traj.next(p);
                }
            }
            t = self.traj.next(t);
        }
    }

    /*******************
     * physics         *
     *******************/

    /// Make the juggler wander around the screen.
    fn wander(&mut self, time: i64) -> f64 {
        let mut w = self.wanders.next(self.wander_head);
        while w != self.wander_head {
            if self.wanders.get(w).finish < self.time {
                /* expired */
                let ww = w;
                w = self.wanders.prev(w);
                self.wanders.remove(ww);
            } else if self.wanders.get(w).finish > time {
                break;
            }
            w = self.wanders.next(w);
        }
        if w == self.wander_head {
            /* Need a new one */
            let at = self.wanders.prev(self.wander_head);
            let prev = *self.wanders.get(at);
            let finish = time
                + 3 * self.throw_catch_interval() as i64
                + i64::from(nrand(10 * self.throw_catch_interval() as i32));
            let x = if time == 0 {
                0.0
            } else {
                prev.x * 0.9 + f64::from(nrand(40)) - 20.0
            };
            let s = make_spline(prev.x, 0.0, prev.finish, x, 0.0, finish);
            w = self.wanders.add_after(at, Wander { x, finish, s });
        }
        let s = self.wanders.get(w).s;
        s.at(time as f64)
    }

    /// Convert hand position symbols into actual time and space coordinates.
    fn positions(&mut self) {
        let mut now = self.time; /* Make sure we're not lost in the past */
        let mut t = self.traj.next(self.head);
        while t != self.head {
            let status = self.traj.get(t).status;
            if status >= Status::PThratch {
                now = self.traj.get(t).start;
            } else if status == Status::Action || status == Status::LinkedAction {
                // Allow ACTIONs to be annotated, but don't mark them ready for
                // the next stage.
                let sx = SX;
                let pose = SX / 2.0;

                /* time */
                let tr = self.traj.get(t);
                if tr.action == ActionKind::Catch {
                    /* Throw-to-catch */
                    if tr.kind == Throwable::Empty {
                        now += self.throw_null_interval() as i64; /* failed catch is short */
                    } else {
                        now += self.throw_catch_interval() as i64;
                    }
                } else {
                    /* Catch-to-throw */
                    let weight = match tr.object {
                        Some(o) => self.objects.get(o).kind.weight(),
                        None => 1.0,
                    };
                    now += (self.catch_throw_interval() * weight) as i64;
                }

                if self.traj.get(t).start == 0 {
                    self.traj.get_mut(t).start = now;
                } else {
                    /* Concatenated performances may need clock resync */
                    now = self.traj.get(t).start;
                }

                let start = self.traj.get(t).start;
                let cx = self.wander(start);
                self.traj.get_mut(t).cx = cx;

                /* space */
                let tr = self.traj.get(t);
                let mut yo = 90.0;
                /* Add room for the handle */
                if tr.action == ActionKind::Catch
                    && let Some(o) = tr.object
                {
                    yo -= self.objects.get(o).kind.handle();
                }

                let tr = self.traj.get(t);
                let mut xo = 0.0;
                match tr.posn {
                    '-' => xo = sx - pose,
                    '_' | 'k' | '+' => xo = sx + pose,
                    '~' | '=' => {
                        xo = -sx - pose;
                        yo += pose;
                    }
                    '^' => {
                        xo = 0.0;
                        yo += pose * 2.0; /* clap */
                    }
                    _ => {}
                }

                let angle = if (tr.hand == Hand::Left)
                    ^ (tr.posn == '+' || tr.posn == '_' || tr.posn == 'k')
                {
                    -std::f64::consts::FRAC_PI_2
                } else {
                    std::f64::consts::FRAC_PI_2
                };
                let hand = tr.hand;
                let tr = self.traj.get_mut(t);
                tr.angle = angle;
                tr.x = cx + if hand == Hand::Left { xo } else { -xo };
                tr.y = yo;

                /* Only mark complete if it was already linked */
                if tr.status == Status::LinkedAction {
                    tr.status = Status::PThratch;
                }
            }
            t = self.traj.next(t);
        }
    }

    /// The spin rate for a trajectory. Different kinds of throw want different
    /// spins, and a club has to arrive handle first.
    fn spinrate(
        &self,
        kind: ObjType,
        h: usize,
        old: f64,
        dt: f64,
        height: i32,
        turns: i32,
        togo: f64,
    ) -> f64 {
        let hr = self.traj.get(h);
        let dir = if (hr.hand == Hand::Left) ^ (hr.posn == '+') {
            -1.0
        } else {
            1.0
        };

        if kind.handle() != 0.0 {
            /* Clubs */
            (dir * f64::from(turns) * 2.0 * std::f64::consts::PI + togo) / dt
        } else if height == 0 {
            /* Balls already spinning */
            old / 2.0
        } else {
            /* Balls */
            dir * f64::from(nrand(height * 10)) / 20.0 / kind.weight() * 2.0 * std::f64::consts::PI
                / dt
        }
    }

    /// The angle at the end of a spinning trajectory.
    fn end_spin(&self, t: usize) -> f64 {
        let tr = self.traj.get(t);
        tr.angle + tr.spin * (tr.finish - tr.start) as f64
    }

    /// Set the initial angle of the catch following hand movement `t` to the
    /// final angle of the throw `n`, and the subsequent throw to that plus half
    /// a turn.
    fn match_spins_on_catch(&mut self, t: usize, n: usize) {
        let Some(bl) = self.traj.get(t).balllink else {
            return;
        };
        let Some(o) = self.traj.get(bl).object else {
            return;
        };
        if self.objects.get(o).kind.handle() != 0.0 {
            return;
        }
        let angle = self.end_spin(n);
        self.traj.get_mut(bl).angle = angle;
        if let Some(hl) = self.traj.get(bl).handlink {
            self.traj.get_mut(hl).angle = angle + std::f64::consts::PI;
        }
    }

    /// Find when a bounced ball has to hit the floor for it to arrive at the
    /// catch on time. There can be three answers; take one by bisection.
    fn find_bounce(&self, yo: f64, yf: f64, yc: f64, tc: f64, cor: f64) -> f64 {
        let e = 1.0; /* permissible error in yc */
        let mut tb = tc;
        let mut dy = 0.0;
        let mut i = tc / 2.0;
        while i > 0.0001 {
            if tb == 0.0 {
                break; /* upstream warns about dividing by zero here */
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
        if dy * self.throw_catch_interval() < -200.0 {
            tb = -1.0; /* bounce too hard */
        }
        tb
    }

    fn new_predictor(&mut self, t: usize, start: i64, finish: i64, angle: f64) -> usize {
        let at = self.traj.prev(t);
        let n = self.traj.add_after(at, Trajectory::default());
        self.dup_object(n, t);
        let divisions = self.traj.get(t).divisions;
        let nr = self.traj.get_mut(n);
        nr.divisions = divisions;
        nr.kind = Throwable::Ball;
        nr.status = Status::Predictor;
        nr.start = start;
        nr.finish = finish;
        nr.angle = angle;
        n
    }

    /// Turn abstract timings into physically appropriate object trajectories.
    fn projectile(&mut self) {
        let yf = 0.0; /* Floor height */

        let mut t = self.traj.next(self.head);
        while t != self.head {
            let tr = self.traj.get(t);
            if tr.status != Status::PThratch || tr.action != ActionKind::Throw {
                t = self.traj.next(t);
                continue;
            }
            let Some(bl) = tr.balllink else {
                /* Zero Throw */
                self.traj.get_mut(t).status = Status::BPredictor;
                t = self.traj.next(t);
                continue;
            };
            if self.traj.get(bl).handlink.is_none() {
                return; /* Incomplete */
            }
            if Some(bl) == tr.handlink {
                /* '2' height: hold on to the ball. */
                let tr = self.traj.get_mut(t);
                tr.kind = Throwable::Full;
                tr.spin = 0.0; /* Zero spin to avoid wrist injuries */
                tr.dx = 0.0;
                tr.dy = 0.0;
                self.match_spins_on_catch(t, t);
                self.traj.get_mut(t).status = Status::BPredictor;
                t = self.traj.next(t);
                continue;
            }

            let kind = match self.traj.get(t).object {
                Some(o) => self.objects.get(o).kind,
                None => ObjType::Ball,
            };

            if self.traj.get(t).posn == '_' {
                /* Bounce once */
                let (ty, tstart) = {
                    let tr = self.traj.get(t);
                    (tr.y, tr.start)
                };
                let (bly, blstart, blx, blangle) = {
                    let b = self.traj.get(bl);
                    (b.y, b.start, b.x, b.angle)
                };
                let tb = tstart
                    + self.find_bounce(ty, yf, bly, (blstart - tstart) as f64, kind.cor()) as i64;

                if tb < tstart {
                    /* bounce too hard: use a regular throw */
                    self.traj.get_mut(t).posn = '+';
                } else {
                    /* dx is constant across both trajectories */
                    let dx = (blx - self.traj.get(t).x) / (blstart - tstart) as f64;
                    self.traj.get_mut(t).dx = dx;

                    /* ball follows parabola down */
                    let angle = self.traj.get(t).angle;
                    let n = self.new_predictor(t, tstart, tb, angle);
                    let dt = (self.traj.get(n).finish - self.traj.get(n).start) as f64;
                    /* Ball rate 4, no flight or matching club turns */
                    let spin = self.spinrate(kind, t, 0.0, dt, 4, 0, 0.0);
                    self.traj.get_mut(n).spin = spin;
                    let dy = (yf - ty) / dt - self.gr / 2.0 * dt;
                    self.traj.get_mut(t).dy = dy;
                    let (x, gr) = (self.traj.get(t).x, self.gr);
                    make_parabola(self.traj.get_mut(n), x, dx, ty, dy, gr);

                    /* ball follows parabola up */
                    let nfinish = self.traj.get(n).finish;
                    let nspin = self.traj.get(n).spin;
                    let end = self.end_spin(n);
                    let m = self.new_predictor(t, nfinish, blstart, end);
                    let dt = (self.traj.get(m).finish - self.traj.get(m).start) as f64;
                    /* Use previous ball rate, no flight club turns */
                    let togo = blangle - self.traj.get(m).angle;
                    let spin = self.spinrate(kind, t, nspin, dt, 0, 0, togo);
                    self.traj.get_mut(m).spin = spin;
                    self.match_spins_on_catch(t, m);
                    let dy = (bly - yf) / dt - self.gr / 2.0 * dt;
                    let gr = self.gr;
                    make_parabola(self.traj.get_mut(m), blx - dx * dt, dx, yf, dy, gr);

                    self.traj.get_mut(t).status = Status::BPredictor;
                    t = self.traj.next(t);
                    continue;
                }
            } else if self.traj.get(t).posn == 'k' {
                /* Drop & Kick */
                let (tx, ty, tstart) = {
                    let tr = self.traj.get(t);
                    (tr.x, tr.y, tr.start)
                };
                let (blx, bly, blstart, blangle, blhandlink) = {
                    let b = self.traj.get(bl);
                    (b.x, b.y, b.start, b.angle, b.handlink)
                };
                let td = tstart + 2 * self.throw_catch_interval() as i64; /* Drop time */
                let tk = blstart - 5 * self.throw_catch_interval() as i64; /* Kick */

                /* Fall to ground */
                let angle = self.traj.get(t).angle;
                let n = self.new_predictor(t, tstart, td, angle);
                let dt = (self.traj.get(n).finish - self.traj.get(n).start) as f64;
                /* Ball spin rate 4, no flight club turns */
                let togo = blangle - self.traj.get(n).angle;
                let spin = self.spinrate(kind, t, 0.0, dt, 4, 0, togo);
                self.traj.get_mut(n).spin = spin;
                let dx = (blx - tx) / dt;
                let dy = (yf - ty) / dt - self.gr / 2.0 * dt;
                {
                    let tr = self.traj.get_mut(t);
                    tr.dx = dx;
                    tr.dy = dy;
                }
                let gr = self.gr;
                make_parabola(self.traj.get_mut(n), tx, dx, ty, dy, gr);

                /* Rest on ground */
                let nfinish = self.traj.get(n).finish;
                let end = self.end_spin(n);
                let o = self.new_predictor(t, nfinish, tk, end);
                self.traj.get_mut(o).spin = 0.0;
                make_parabola(self.traj.get_mut(o), blx, 0.0, yf, 0.0, 0.0);

                /* Kick up */
                let ofinish = self.traj.get(o).finish;
                let end = self.end_spin(o);
                let m = self.new_predictor(t, ofinish, blstart, end);
                let dt = (self.traj.get(m).finish - self.traj.get(m).start) as f64;
                /* Match receiving hand, ball rate 4, one flight club turn */
                let togo = blangle - self.traj.get(m).angle;
                let spin = match blhandlink {
                    Some(h) => self.spinrate(kind, h, 0.0, dt, 4, 1, togo),
                    None => 0.0,
                };
                self.traj.get_mut(m).spin = spin;
                self.match_spins_on_catch(t, m);
                let dy = (bly - yf) / dt - self.gr / 2.0 * dt;
                let gr = self.gr;
                make_parabola(self.traj.get_mut(m), blx, 0.0, yf, dy, gr);

                self.traj.get_mut(t).status = Status::BPredictor;
                t = self.traj.next(t);
                continue;
            }

            /* Regular flight, no bounce */
            {
                let (tx, ty, tstart, tangle, theight) = {
                    let tr = self.traj.get(t);
                    (tr.x, tr.y, tr.start, tr.angle, tr.height)
                };
                let (blx, bly, blstart, blangle) = {
                    let b = self.traj.get(bl);
                    (b.x, b.y, b.start, b.angle)
                };
                let n = self.new_predictor(t, tstart, blstart, tangle);
                let dt = (blstart - tstart) as f64;
                /* Regular spin */
                let togo = blangle - self.traj.get(n).angle;
                let spin = self.spinrate(kind, t, 0.0, dt, theight, theight / 2, togo);
                self.traj.get_mut(n).spin = spin;
                self.match_spins_on_catch(t, n);
                let dx = (blx - tx) / dt;
                let dy = (bly - ty) / dt - self.gr / 2.0 * dt;
                {
                    let tr = self.traj.get_mut(t);
                    tr.dx = dx;
                    tr.dy = dy;
                }
                let gr = self.gr;
                make_parabola(self.traj.get_mut(n), tx, dx, ty, dy, gr);
            }

            self.traj.get_mut(t).status = Status::BPredictor;
            t = self.traj.next(t);
        }
    }

    /// Turn abstract hand motions into cubic splines: a double spline takes a
    /// hand from a throw, through the catch, to the next throw.
    fn hands(&mut self) {
        let mut t = self.traj.next(self.head);
        while t != self.head {
            if self.traj.get(t).status != Status::BPredictor {
                t = self.traj.next(t);
                continue;
            }
            let Some(u) = self.traj.get(t).handlink else {
                t = self.traj.next(t);
                continue; /* no next catch */
            };
            let Some(v) = self.traj.get(u).handlink else {
                t = self.traj.next(t);
                continue; /* no next throw */
            };

            let (tx, ty, tdx, tdy, tstart, tangle, thand) = {
                let tr = self.traj.get(t);
                (tr.x, tr.y, tr.dx, tr.dy, tr.start, tr.angle, tr.hand)
            };
            let (ux, uy, ustart, uangle) = {
                let ur = self.traj.get(u);
                (ur.x, ur.y, ur.start, ur.angle)
            };
            let (vx, vy, vdx, vdy, vstart, vangle, vhand, vposn) = {
                let vr = self.traj.get(v);
                (
                    vr.x, vr.y, vr.dx, vr.dy, vr.start, vr.angle, vr.hand, vr.posn,
                )
            };

            // Make sure an empty hand's spin matches the object it threw, in
            // case that had a handle.
            let tspin = if thand == Hand::Left { -1.0 } else { 1.0 }
                * ((uangle - tangle) / (ustart - tstart) as f64).abs();
            let uspin = if (vhand == Hand::Left) ^ (vposn == '+') {
                -1.0
            } else {
                1.0
            } * ((vangle - uangle) / (vstart - ustart) as f64).abs();

            let (txp, uxp) = make_spline_pair(tx, tdx, tstart, ux, ustart, vx, vdx, vstart);
            let (typ, uyp) = make_spline_pair(ty, tdy, tstart, uy, ustart, vy, vdy, vstart);

            {
                let tr = self.traj.get_mut(t);
                tr.finish = ustart;
                tr.status = Status::Predictor;
                tr.spin = tspin;
                tr.xp = txp;
                tr.yp = typ;
            }
            {
                let ur = self.traj.get_mut(u);
                ur.finish = vstart;
                ur.status = Status::Predictor;
                ur.spin = uspin;
                ur.xp = uxp;
                ur.yp = uyp;
            }

            t = self.traj.next(t);
        }
    }

    /// Put the hand at the target if it can reach, otherwise point at it.
    fn reach_arm(&mut self, side: Hand, p: DPoint) -> DPoint {
        let s = self.arm[1][side as usize][SHOULDER];
        let (h, e) = find_elbow(40.0, p, s, 25.0);
        self.arm[1][side as usize][HAND] = h;
        self.arm[1][side as usize][ELBOW] = e;
        h
    }
}

/// Compute a single spline from `x0` with velocity `dx0` at time `t0` to `x1`
/// with velocity `dx1` at time `t1`.
fn make_spline(x0: f64, dx0: f64, t0: i64, x1: f64, dx1: f64, t1: i64) -> Spline {
    let x10 = x1 - x0;
    let t10 = (t1 - t0) as f64;
    let t0 = t0 as f64;
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

/// A pair of splines meeting at `x1`, `t1` with a shared velocity there, so the
/// hand does not jerk as it passes through the catch.
#[allow(clippy::too_many_arguments)]
fn make_spline_pair(
    x0: f64,
    dx0: f64,
    t0: i64,
    x1: f64,
    t1: i64,
    x2: f64,
    dx2: f64,
    t2: i64,
) -> (Spline, Spline) {
    let x10 = x1 - x0;
    let x21 = x2 - x1;
    let t21 = (t2 - t1) as f64;
    let t10 = (t1 - t0) as f64;
    let t20 = (t2 - t0) as f64;
    let dx1 = (3.0 * x10 * t21 * t21 + 3.0 * x21 * t10 * t10 + 3.0 * dx0 * t10 * t21 * t21
        - dx2 * t10 * t10 * t21
        - 4.0 * dx0 * t10 * t21 * t21)
        / (2.0 * t10 * t21 * t20);
    (
        make_spline(x0, dx0, t0, x1, dx1, t1),
        make_spline(x1, dx1, t1, x2, dx2, t2),
    )
}

/// A ballistic path as a pair of degenerate splines: constant velocity across,
/// constant acceleration down.
fn make_parabola(n: &mut Trajectory, x: f64, dx: f64, y: f64, dy: f64, g: f64) {
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

/// Where to put the elbow so the hand lands on the target, or as near as the
/// arm will reach.
fn find_elbow(armlength: f64, p: DPoint, s: DPoint, z: f64) -> (DPoint, DPoint) {
    let x = p.x - s.x;
    let y = p.y - s.y;
    let h2 = x * x + y * y + z * z;
    if h2 > 4.0 * armlength * armlength {
        let t = armlength / h2.sqrt();
        (
            DPoint {
                x: 2.0 * t * x + s.x,
                y: 2.0 * t * y + s.y,
            },
            DPoint {
                x: t * x + s.x,
                y: t * y + s.y,
            },
        )
    } else {
        let r = (x * x + z * z).sqrt();
        let t = (4.0 * armlength * armlength / h2 - 1.0).sqrt();
        (
            DPoint {
                x: x + s.x,
                y: y + s.y,
            },
            DPoint {
                x: x * (1.0 + y * t / r) / 2.0 + s.x,
                y: (y - r * t) / 2.0 + s.y,
            },
        )
    }
}

/// The number of balls a pattern needs, which in Adam notation is the largest
/// number in it.
fn get_num_balls(j: &str) -> i32 {
    let mut balls = 0;
    let mut h = 0;
    for c in j.chars() {
        if c.is_ascii_digit() {
            h = 10 * h + (c as i32 - '0' as i32);
        } else {
            if h > balls {
                balls = h;
            }
            h = 0;
        }
    }
    balls
}

/// Popular patterns, in any order. They are given in Adam notation so the
/// generator can concatenate them safely; the height notation a juggler
/// would read is worked out and displayed by [`Juggle::name`].
fn portfolio() -> Vec<PatternEntry> {
    vec![
        PatternEntry {
            pattern: "[+2 1]",
            name: "Typical 2 ball juggler",
        },
        PatternEntry {
            pattern: "[2 0]",
            name: "2 in 1 hand",
        },
        PatternEntry {
            pattern: "[2 0 1]",
            name: "",
        },
        PatternEntry {
            pattern: "[+2 0 +2 0 0]",
            name: "",
        },
        PatternEntry {
            pattern: "[+2 0 1 2 2]",
            name: "",
        },
        PatternEntry {
            pattern: "[2 0 1 1]",
            name: "",
        },
        PatternEntry {
            pattern: "[3]",
            name: "3 cascade",
        },
        PatternEntry {
            pattern: "[+3]",
            name: "reverse 3 cascade",
        },
        PatternEntry {
            pattern: "[=3]",
            name: "cascade 3 under arm",
        },
        PatternEntry {
            pattern: "[&3]",
            name: "cascade 3 catching under arm",
        },
        PatternEntry {
            pattern: "[_3]",
            name: "bouncing 3 cascade",
        },
        PatternEntry {
            pattern: "[+3 x3 =3]",
            name: "Mill's mess",
        },
        PatternEntry {
            pattern: "[3 2 1]",
            name: "",
        },
        PatternEntry {
            pattern: "[3 3 1]",
            name: "",
        },
        PatternEntry {
            pattern: "[3 1 2]",
            name: "See-saw",
        },
        PatternEntry {
            pattern: "[=3 3 1 2]",
            name: "",
        },
        PatternEntry {
            pattern: "[=3 2 2 3 1 2]",
            name: "=4 5 1 2 stretched",
        },
        PatternEntry {
            pattern: "[+3 3 1 3]",
            name: "anemic shower box",
        },
        PatternEntry {
            pattern: "[3 3 1]",
            name: "",
        },
        PatternEntry {
            pattern: "[+3 2 3]",
            name: "",
        },
        PatternEntry {
            pattern: "[+3 1]",
            name: "3 shower",
        },
        PatternEntry {
            pattern: "[_3 1]",
            name: "bouncing 3 shower",
        },
        PatternEntry {
            pattern: "[3 0 3 0 3]",
            name: "shake 3 out of 5",
        },
        PatternEntry {
            pattern: "[3 3 3 0 0]",
            name: "flash 3 out of 5",
        },
        PatternEntry {
            pattern: "[3 3 0]",
            name: "complete waste of a 5 ball juggler",
        },
        PatternEntry {
            pattern: "[3 3 3 0 0 0 0]",
            name: "3 flash",
        },
        PatternEntry {
            pattern: "[+3 0 +3 0 +3 0 0]",
            name: "",
        },
        PatternEntry {
            pattern: "[3 2 2 0 3 2 0 2 3 0 2 2 0]",
            name: "",
        },
        PatternEntry {
            pattern: "[3 0 2 0]",
            name: "",
        },
        PatternEntry {
            pattern: "[_3 2 1]",
            name: "",
        },
        PatternEntry {
            pattern: "[_3 0 1]",
            name: "",
        },
        PatternEntry {
            pattern: "[1 _3 1 _3 0 1 _3 0]",
            name: "",
        },
        PatternEntry {
            pattern: "[_3 2 1 _3 1 2 1]",
            name: "",
        },
        PatternEntry {
            pattern: "[4]",
            name: "4 cascade",
        },
        PatternEntry {
            pattern: "[+4 3]",
            name: "4 ball half shower",
        },
        PatternEntry {
            pattern: "[4 4 2]",
            name: "",
        },
        PatternEntry {
            pattern: "[+4 4 4 +4]",
            name: "4 columns",
        },
        PatternEntry {
            pattern: "[+4 3 +4]",
            name: "",
        },
        PatternEntry {
            pattern: "[4 3 4 4]",
            name: "",
        },
        PatternEntry {
            pattern: "[4 3 3 4]",
            name: "",
        },
        PatternEntry {
            pattern: "[4 3 2 4",
            name: "",
        },
        PatternEntry {
            pattern: "[+4 1]",
            name: "4 shower",
        },
        PatternEntry {
            pattern: "[4 4 4 4 0]",
            name: "learning 5",
        },
        PatternEntry {
            pattern: "[+4 x4 =4]",
            name: "Mill's mess for 4",
        },
        PatternEntry {
            pattern: "[+4 2 1 3]",
            name: "",
        },
        PatternEntry {
            pattern: "[4 4 1 4 1 4]",
            name: "",
        },
        PatternEntry {
            pattern: "[_4 _4 _4 1 _4 1]",
            name: "",
        },
        PatternEntry {
            pattern: "[_4 3 3]",
            name: "",
        },
        PatternEntry {
            pattern: "[_4 3 1]",
            name: "",
        },
        PatternEntry {
            pattern: "[_4 2 1]",
            name: "",
        },
        PatternEntry {
            pattern: "[_4 3 3 3 0]",
            name: "",
        },
        PatternEntry {
            pattern: "[_4 1 3 1]",
            name: "",
        },
        PatternEntry {
            pattern: "[_4 1 3 1 2]",
            name: "",
        },
        PatternEntry {
            pattern: "[5]",
            name: "5 cascade",
        },
        PatternEntry {
            pattern: "[_5 _5 _5 _5 _5 5 5 5 5 5]",
            name: "",
        },
        PatternEntry {
            pattern: "[+5 x5 =5]",
            name: "Mill's mess for 5",
        },
        PatternEntry {
            pattern: "[5 4 4]",
            name: "",
        },
        PatternEntry {
            pattern: "[_5 4 4]",
            name: "",
        },
        PatternEntry {
            pattern: "[1 2 3 4 5 5 5 5 5]",
            name: "5 ramp",
        },
        PatternEntry {
            pattern: "[5 4 5 3 1]",
            name: "",
        },
        PatternEntry {
            pattern: "[_5 4 1 +4]",
            name: "",
        },
        PatternEntry {
            pattern: "[_5 4 +4 +4]",
            name: "",
        },
        PatternEntry {
            pattern: "[_5 4 4 4 1]",
            name: "",
        },
        PatternEntry {
            pattern: "[_5 4 4 5 1]",
            name: "",
        },
        PatternEntry {
            pattern: "[_5 4 4 +4 4 0]",
            name: "",
        },
        PatternEntry {
            pattern: "[6]",
            name: "6 cascade",
        },
        PatternEntry {
            pattern: "[+6 5]",
            name: "",
        },
        PatternEntry {
            pattern: "[6 4]",
            name: "",
        },
        PatternEntry {
            pattern: "[+6 3]",
            name: "",
        },
        PatternEntry {
            pattern: "[6 5 4 4]",
            name: "",
        },
        PatternEntry {
            pattern: "[+6 5 5 5]",
            name: "",
        },
        PatternEntry {
            pattern: "[6 0 6]",
            name: "",
        },
        PatternEntry {
            pattern: "[_6 0 _6]",
            name: "",
        },
        PatternEntry {
            pattern: "[_7]",
            name: "bouncing 7 cascade",
        },
        PatternEntry {
            pattern: "[7]",
            name: "7 cascade",
        },
        PatternEntry {
            pattern: "[7 6 6 6 6]",
            name: "Gatto's High Throw",
        },
    ]
}

/**************************************************************************
 *                        Rendering Functions                             *
 **************************************************************************/

impl Juggle {
    fn show_arms(&mut self, d: &mut Dpy, color: Pixel) {
        let j = usize::from(color != self.mi.black);
        let w = self.armwidth();
        self.mi.gc.set_line_width(w);
        self.mi.gc.set_foreground(color);
        for side in [Hand::Left, Hand::Right] {
            let mut a = [XPoint::default(); 3];
            for (i, p) in a.iter_mut().enumerate() {
                let j0 = self.arm[j][side as usize][i];
                *p = XPoint {
                    x: (f64::from(self.mi.width) / 2.0 + j0.x * self.scale) as i32,
                    y: (f64::from(self.mi.height) - j0.y * self.scale) as i32,
                };
                // Drawing the arms in the foreground is also what records
                // where they are, so the next frame can erase them there.
                if j == 1 {
                    self.arm[0][side as usize][i] = self.arm[1][side as usize][i];
                }
            }
            d.win().draw_lines(&self.mi.gc, &a);
        }
    }

    fn show_figure(&mut self, d: &mut Dpy, color: Pixel, init: bool) {
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
                |     |
                |     |
                |     |
              7 +     + 8
        */
        let sx = SX;
        let figure: [DPoint; 11] = [
            DPoint { x: 15.0, y: 70.0 },   /* 0  Left Hip */
            DPoint { x: 0.0, y: 90.0 },    /* 1  Waist */
            DPoint { x: sx, y: 130.0 },    /* 2  Left Shoulder */
            DPoint { x: -sx, y: 130.0 },   /* 3  Right Shoulder */
            DPoint { x: -15.0, y: 70.0 },  /* 4  Right Hip */
            DPoint { x: 0.0, y: 130.0 },   /* 5  Neck */
            DPoint { x: 0.0, y: 140.0 },   /* 6  Chin */
            DPoint { x: sx, y: 0.0 },      /* 7  Left Foot */
            DPoint { x: -sx, y: 0.0 },     /* 8  Right Foot */
            DPoint { x: -17.0, y: 174.0 }, /* 9  Head1 */
            DPoint { x: 17.0, y: 140.0 },  /* 10 Head2 */
        ];

        let mut a = [XPoint::default(); 11];
        for (i, f) in figure.iter().enumerate() {
            a[i] = XPoint {
                x: (f64::from(self.mi.width) / 2.0 + (self.cx + f.x) * self.scale) as i32,
                y: (f64::from(self.mi.height) - f.y * self.scale) as i32,
            };
        }

        let w = self.armwidth();
        self.mi.gc.set_line_width(w);
        self.mi.gc.set_foreground(color);

        let body = [a[0], a[1], a[2], a[3], a[1], a[4]];
        d.win().draw_lines(&self.mi.gc, &body);
        let legs = [a[7], a[0], a[4], a[8]];
        d.win().draw_lines(&self.mi.gc, &legs);
        let neck = [a[5], a[6]];
        d.win().draw_lines(&self.mi.gc, &neck);
        d.win().draw_arc(
            &self.mi.gc,
            a[9].x,
            a[9].y,
            a[10].x - a[9].x,
            a[10].y - a[9].y,
            0,
            64 * 360,
        );

        self.arm[1][Hand::Left as usize][SHOULDER].x = self.cx + figure[2].x;
        self.arm[1][Hand::Right as usize][SHOULDER].x = self.cx + figure[3].x;
        if init {
            for i in 0..2 {
                self.arm[i][Hand::Left as usize][SHOULDER].y = figure[2].y;
                self.arm[i][Hand::Left as usize][ELBOW].x = figure[2].x;
                self.arm[i][Hand::Left as usize][ELBOW].y = figure[1].y;
                self.arm[i][Hand::Left as usize][HAND].x = figure[0].x;
                self.arm[i][Hand::Left as usize][HAND].y = figure[1].y;
                self.arm[i][Hand::Right as usize][SHOULDER].y = figure[3].y;
                self.arm[i][Hand::Right as usize][ELBOW].x = figure[3].x;
                self.arm[i][Hand::Right as usize][ELBOW].y = figure[1].y;
                self.arm[i][Hand::Right as usize][HAND].x = figure[4].x;
                self.arm[i][Hand::Right as usize][HAND].y = figure[1].y;
            }
        }
    }

    /// Draw one object at one of its recorded positions. Passing the background
    /// colour is how the same code erases it again.
    fn draw_object(&mut self, d: &mut Dpy, kind: ObjType, color: Pixel, s: usize) {
        match kind {
            ObjType::Ball => self.show_ball(d, color, s),
            ObjType::Club => self.show_europeanclub(d, color, s),
            ObjType::Torch => self.show_torch(d, color, s),
            ObjType::Knife => self.show_knife(d, color, s),
            ObjType::Ring => self.show_ring(d, color, s),
            ObjType::BBall => self.show_bball(d, color, s),
        }
    }

    /// True if this position is so far off the top that drawing it would wrap.
    fn offscreen(&self, y: f64) -> bool {
        y * self.scale > f64::from(self.mi.height) * 2.0
    }

    fn show_ball(&mut self, d: &mut Dpy, color: Pixel, s: usize) {
        let t = *self.traces.get(s);
        let offset = (t.angle * 64.0 * 180.0 / std::f64::consts::PI) as i32;
        let x = (f64::from(self.mi.width) / 2.0 + t.x * self.scale) as i32;
        let y = (f64::from(self.mi.height) - t.y * self.scale) as i32;
        if self.offscreen(t.y) {
            return;
        }
        let r = self.ballradius();

        self.mi.gc.set_foreground(color);
        if t.divisions == 0 || color == self.mi.black {
            d.win()
                .fill_arc(&self.mi.gc, x - r, y - r, 2 * r, 2 * r, 0, 23040);
        } else if t.divisions == 4 {
            /* 90 degree divisions */
            d.win().fill_arc(
                &self.mi.gc,
                x - r,
                y - r,
                2 * r,
                2 * r,
                offset % 23040,
                5760,
            );
            d.win().fill_arc(
                &self.mi.gc,
                x - r,
                y - r,
                2 * r,
                2 * r,
                (offset + 11520) % 23040,
                5760,
            );
            let white = self.mi.white;
            self.mi.gc.set_foreground(white);
            d.win().fill_arc(
                &self.mi.gc,
                x - r,
                y - r,
                2 * r,
                2 * r,
                (offset + 5760) % 23040,
                5760,
            );
            d.win().fill_arc(
                &self.mi.gc,
                x - r,
                y - r,
                2 * r,
                2 * r,
                (offset + 17280) % 23040,
                5760,
            );
        } else if t.divisions == 2 {
            /* 180 degree divisions */
            d.win().fill_arc(
                &self.mi.gc,
                x - r,
                y - r,
                2 * r,
                2 * r,
                offset % 23040,
                11520,
            );
            let white = self.mi.white;
            self.mi.gc.set_foreground(white);
            d.win().fill_arc(
                &self.mi.gc,
                x - r,
                y - r,
                2 * r,
                2 * r,
                (offset + 11520) % 23040,
                11520,
            );
        }
    }

    fn show_europeanclub(&mut self, d: &mut Dpy, color: Pixel, s: usize) {
        let t = *self.traces.get(s);
        let sa = t.angle.sin();
        let ca = t.angle.cos();

        /*  6   6
              +-+
             /   \
          4 +-----+ 7
           ////////\
        3 +---------+ 8
        2 +---------+ 9
           |///////|
         1 +-------+ 10
            |     |
            |     |
             |   |
             |   |
              | |
              | |
              +-+
             0  11 	*/
        let club: [(f64, f64); 13] = [
            (-24.0, 2.0),
            (-10.0, 3.0),
            (1.0, 6.0),
            (8.0, 6.0),
            (14.0, 4.0),
            (16.0, 3.0),
            (16.0, -3.0),
            (14.0, -4.0),
            (8.0, -6.0),
            (1.0, -6.0),
            (-10.0, -3.0),
            (-24.0, -2.0),
            (-24.0, 2.0), /* close boundary */
        ];

        if self.offscreen(t.y) {
            return;
        }

        /* Translate and fake perspective */
        let rs = self.scale.sqrt();
        let mut a = [XPoint::default(); 13];
        for (i, c) in club.iter().enumerate() {
            a[i] = XPoint {
                x: (f64::from(self.mi.width) / 2.0 + (t.x + c.0 * PERSPEC * sa) * self.scale
                    - c.1 * rs * ca) as i32,
                y: (f64::from(self.mi.height) - (t.y - c.0 * ca) * self.scale + c.1 * sa * rs)
                    as i32,
            };
        }

        if color != self.mi.black {
            /* Outline in black */
            let black = self.mi.black;
            self.mi.gc.set_foreground(black);
            self.mi.gc.set_line_width(2);
            d.win().draw_lines(&self.mi.gc, &a);
        }

        self.mi.gc.set_foreground(color);

        // Upstream's note: don't be tempted to optimize the erase by drawing
        // all the black in one operation. It must use the same operations as
        // the colours to guarantee a clean erase.
        let stripe1 = [a[1], a[2], a[9], a[10]];
        d.win().fill_polygon(&self.mi.gc, &stripe1);
        let stripe2 = [a[3], a[4], a[7], a[8]];
        d.win().fill_polygon(&self.mi.gc, &stripe2);

        if color != self.mi.black {
            let white = self.mi.white;
            self.mi.gc.set_foreground(white);
        }
        let band = [a[2], a[3], a[8], a[9]];
        d.win().fill_polygon(&self.mi.gc, &band);
        let handle = [a[0], a[1], a[10], a[11]];
        d.win().fill_polygon(&self.mi.gc, &handle);
        let tip = [a[4], a[5], a[6], a[7]];
        d.win().fill_polygon(&self.mi.gc, &tip);
    }

    fn show_torch(&mut self, d: &mut Dpy, color: Pixel, s: usize) {
        let t = *self.traces.get(s);
        let sa = t.angle.sin();
        let ca = t.angle.cos();

        let tail_len = -24.0;
        let head_len = 16.0;
        let width = (5.0 * self.scale.sqrt()) as i32;

        /*
          +///+ head
        last  |
              |
              + tail
        */
        let dhead = DPoint {
            x: t.x + head_len * PERSPEC * sa,
            y: t.y - head_len * ca,
        };

        let dlast = if color == self.mi.black {
            t.dlast /* Use 'last' when erasing */
        } else {
            // Store 'last' so it can be used later, when the previous trace
            // has gone.
            let prev = self.traces.prev(s);
            let dlast = if prev != self.traces.next(s) {
                let p = *self.traces.get(prev);
                DPoint {
                    x: p.x + head_len * PERSPEC * p.angle.sin(),
                    y: p.y - head_len * p.angle.cos(),
                }
            } else {
                dhead
            };
            self.traces.get_mut(s).dlast = dlast;
            dlast
        };

        /* Avoid wrapping (after last is stored) */
        if self.offscreen(t.y) {
            return;
        }

        let head = XPoint {
            x: (f64::from(self.mi.width) / 2.0 + dhead.x * self.scale) as i32,
            y: (f64::from(self.mi.height) - dhead.y * self.scale) as i32,
        };
        let last = XPoint {
            x: (f64::from(self.mi.width) / 2.0 + dlast.x * self.scale) as i32,
            y: (f64::from(self.mi.height) - dlast.y * self.scale) as i32,
        };
        let tail = XPoint {
            x: (f64::from(self.mi.width) / 2.0 + (t.x + tail_len * PERSPEC * sa) * self.scale)
                as i32,
            y: (f64::from(self.mi.height) - (t.y - tail_len * ca) * self.scale) as i32,
        };

        if color != self.mi.black {
            let black = self.mi.black;
            self.mi.gc.set_foreground(black);
            self.mi.gc.set_line_width(width);
            d.win()
                .draw_line(&self.mi.gc, head.x, head.y, tail.x, tail.y);
        }
        self.mi.gc.set_foreground(color);
        self.mi.gc.set_line_width(width * 2);
        d.win()
            .draw_line(&self.mi.gc, head.x, head.y, last.x, last.y);
    }

    fn show_knife(&mut self, d: &mut Dpy, color: Pixel, s: usize) {
        let t = *self.traces.get(s);
        let sa = t.angle.sin();
        let ca = t.angle.cos();

        /*
            2 +
              |+ 3
              ||
            1 +++ 5
              |4|
              | |
               + 0
        */
        let knife: [(f64, f64); 6] = [
            (-24.0, 0.0),
            (-5.0, -3.0),
            (16.0, -3.0),
            (12.0, 0.0),
            (-5.0, 0.0),
            (-5.0, 3.0),
        ];

        if self.offscreen(t.y) {
            return;
        }

        let rs = self.scale.sqrt();
        let mut a = [XPoint::default(); 6];
        for (i, k) in knife.iter().enumerate() {
            a[i] = XPoint {
                x: (f64::from(self.mi.width) / 2.0 + (t.x + k.0 * PERSPEC * sa) * self.scale
                    - k.1 * rs * ca * PERSPEC) as i32,
                y: (f64::from(self.mi.height) - (t.y - k.0 * ca) * self.scale + k.1 * sa * rs)
                    as i32,
            };
        }

        /* Handle */
        self.mi.gc.set_foreground(color);
        self.mi.gc.set_line_width((4.0 * rs) as i32);
        d.win()
            .draw_line(&self.mi.gc, a[0].x, a[0].y, a[4].x, a[4].y);

        /* Blade */
        if color != self.mi.black {
            let white = self.mi.white;
            self.mi.gc.set_foreground(white);
        }
        let blade = [a[1], a[2], a[3], a[5]];
        d.win().fill_polygon(&self.mi.gc, &blade);
    }

    fn show_ring(&mut self, d: &mut Dpy, color: Pixel, s: usize) {
        let t = *self.traces.get(s);
        let x = (f64::from(self.mi.width) / 2.0 + t.x * self.scale) as i32;
        let y = (f64::from(self.mi.height) - t.y * self.scale) as i32;
        let radius = 15.0 * self.scale;
        let thickness = (8.0 * self.scale.sqrt()) as i32;

        if self.offscreen(t.y) {
            return;
        }

        self.mi.gc.set_foreground(color);
        self.mi.gc.set_line_width(thickness);
        d.win().draw_arc(
            &self.mi.gc,
            (f64::from(x) - radius * PERSPEC) as i32,
            (f64::from(y) - radius) as i32,
            (2.0 * radius * PERSPEC) as i32,
            (2.0 * radius) as i32,
            0,
            23040,
        );
    }

    fn show_bball(&mut self, d: &mut Dpy, color: Pixel, s: usize) {
        let t = *self.traces.get(s);
        let x = (f64::from(self.mi.width) / 2.0 + t.x * self.scale) as i32;
        let y = (f64::from(self.mi.height) - t.y * self.scale) as i32;
        let radius = 12.0 * self.scale;
        let offset = (t.angle * 64.0 * 180.0 / std::f64::consts::PI) as i32;
        let holesize = (3.0 * self.scale.sqrt()) as i32;

        if self.offscreen(t.y) {
            return;
        }

        let black = self.mi.black;
        self.mi.gc.set_foreground(black);
        d.win().fill_arc(
            &self.mi.gc,
            (f64::from(x) - radius) as i32,
            (f64::from(y) - radius) as i32,
            (2.0 * radius) as i32,
            (2.0 * radius) as i32,
            0,
            23040,
        );
        self.mi.gc.set_foreground(color);
        self.mi.gc.set_line_width(2);
        d.win().draw_arc(
            &self.mi.gc,
            (f64::from(x) - radius) as i32,
            (f64::from(y) - radius) as i32,
            (2.0 * radius) as i32,
            (2.0 * radius) as i32,
            0,
            23040,
        );

        /* Draw finger holes. A zero-length arc is a dot. */
        self.mi.gc.set_line_width(holesize);
        for (scale, extra) in [(0.5, 960), (0.7, 1920), (0.7, 0)] {
            d.win().draw_arc(
                &self.mi.gc,
                (f64::from(x) - radius * scale) as i32,
                (f64::from(y) - radius * scale) as i32,
                (2.0 * radius * scale) as i32,
                (2.0 * radius * scale) as i32,
                (offset + extra) % 23040,
                0,
            );
        }
    }
}

/**************************************************************************
 *                    Public Functions                                    *
 **************************************************************************/

const MAXPAT: i32 = 10;
const MAXREPEAT: i32 = 300;
/// Larger makes num_ball changes less likely.
const CHANGE_BIAS: i32 = 8;
/// Larger makes hand movements less likely.
const POSITION_BIAS: i32 = 20;

impl Juggle {
    /// Add more pattern to the end of the programme, then run the whole
    /// pipeline over whatever is not finished yet.
    fn refill(&mut self) {
        let cycles = self.mi.cycles;
        let mut count = 0;
        while count < cycles {
            let l = nrand(MAXPAT) + 1;
            let t = nrand(MAXREPEAT.min(cycles - count)) + 1;

            {
                /* vary number of balls */
                let mut new_balls = self.num_balls;
                let change = if new_balls == 2 {
                    /* Do not juggle 2 that often */
                    nrand(2 + CHANGE_BIAS / 4)
                } else {
                    nrand(2 + CHANGE_BIAS)
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
                    self.program("[*]", None, 1); /* lose ball */
                }
                self.num_balls = new_balls;
            }

            count += t;
            let nb = self.num_balls.clamp(0, self.index.len() as i32 - 1) as usize;
            if nrand(2) != 0 && self.index[nb].number != 0 {
                /* Pick from the portfolio */
                let p = self.index[nb].start + nrand(self.index[nb].number) as usize;
                let (pat, name) = (self.portfolio[p].pattern, self.portfolio[p].name);
                let name = if name.is_empty() { None } else { Some(name) };
                self.program(pat, name, t);
            } else {
                /* Invent a new pattern */
                let mut b = String::from("[");
                let mut maxseen = false;
                for _ in 0..l {
                    let (mut n, mut m);
                    loop {
                        /* Triangular Distribution => high values more likely */
                        m = nrand(self.num_balls + 1);
                        n = nrand(self.num_balls + 1);
                        if m < n {
                            break;
                        }
                    }
                    if n == self.num_balls {
                        maxseen = true;
                    }
                    match nrand(5 + POSITION_BIAS) {
                        0 => b.push('+'), /* Outside throw */
                        1 => b.push('='), /* Cross throw */
                        2 => b.push('&'), /* Cross catch */
                        3 => b.push('x'), /* Cross throw and catch */
                        4 => b.push('_'), /* Bounce */
                        _ => {}           /* Inside throw (default) */
                    }
                    b.push(char::from(b'0' + n as u8));
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
        self.lob();
        self.clap();
        self.positions();
        self.projectile();
        self.hands();
    }

    /// Pick a new object theme and reprogramme, keeping whatever is in the air.
    fn change(&mut self, d: &mut Dpy) {
        /* Strip pending trajectories */
        let mut t = self.traj.next(self.head);
        while t != self.head {
            let tr = self.traj.get(t);
            let stale = tr.start > self.time || tr.finish < self.time;
            let next = self.traj.next(t);
            if stale {
                self.trajectory_destroy(t);
            }
            t = next;
        }

        self.objtypes = self.choose_object();
        self.refill();

        // Clean up the screen. Not the usual clear, since we don't want the
        // fade effects that come with it.
        d.clear_window();
        let white = self.mi.white;
        self.show_figure(d, white, true);
    }

    /// `init_juggle`: everything that has to happen once, plus everything that
    /// has to happen again on a resize.
    fn reset(&mut self, d: &mut Dpy) {
        if !self.started {
            self.started = true;

            // Draw the figure, which is also what discovers his proportions.
            let white = self.mi.white;
            self.show_figure(d, white, true);

            // "7" should be about three times the height of the juggler's
            // shoulders.
            let h = 3.0 * self.arm[0][Hand::Right as usize][SHOULDER].y;
            let tt = 7.0 * self.throw_catch_interval();
            self.gr = -(4.0 * h / (tt * tt));

            self.wander(0); /* Initialize wander */
            self.build_index();
            if self.maxballs > 0 {
                self.num_balls = self.minballs + nrand(self.maxballs - self.minballs);
            }
        }

        self.change(d);
        d.clear_window();

        // MIN so that odd window shapes still work: narrow windows for tall
        // patterns, and so on.
        self.scale = (f64::from(self.mi.height) / 480.0).min(f64::from(self.mi.width) / 160.0);
    }

    /// Sort the pattern list by ball count and record where each group starts,
    /// so a pattern for a given number of balls can be picked at random.
    fn build_index(&mut self) {
        self.portfolio = portfolio();
        // Upstream qsorts, which is not stable; this is, so the members of a
        // group come out in a different order. The groups themselves, which is
        // all the index records, are the same either way.
        self.portfolio.sort_by_key(|p| get_num_balls(p.pattern));

        let nelements = self.portfolio.len();
        self.index = vec![PatternIndex::default(); nelements];
        let mut numpat = 0;
        self.maxballs = 1;
        for i in 0..nelements {
            let b = get_num_balls(self.portfolio[i].pattern);
            if b > self.maxballs {
                let m = self.maxballs as usize;
                self.index[m].number = numpat;
                if numpat == 0 {
                    self.minballs = b;
                }
                self.maxballs = b;
                numpat = 1;
                let m = self.maxballs as usize;
                self.index[m].start = i;
            } else {
                numpat += 1;
            }
        }
        let m = self.maxballs as usize;
        self.index[m].number = numpat;
    }
}

impl Screenhack for Juggle {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        let mut future = 0i64;
        let mut pattern: Option<String> = None;

        /* Update timer */
        if self.real {
            self.time = (d.time * 1000.0) as i64;
        } else {
            self.time += i64::from(self.mi.delay) / 1000;
        }

        /* First pass: Move arms and strip out expired elements */
        let mut traj = self.traj.next(self.head);
        while traj != self.head {
            let next = self.traj.next(traj);
            let tr = self.traj.get(traj);
            if tr.status != Status::Predictor {
                // Skip anything that needs further processing. Removing it
                // would be wrong: the refiller still wants it.
                traj = next;
                continue;
            }
            if tr.start > future {
                future = tr.start; /* Lookahead to the end of the show */
            }
            if self.time < tr.start {
                traj = next;
                continue; /* early */
            }
            if self.time >= tr.finish {
                /* expired */
                self.trajectory_destroy(traj);
                traj = next;
                continue;
            }

            /* working */
            if let Some(p) = &self.traj.get(traj).pattern {
                pattern = Some(p.clone());
            }

            let kind = self.traj.get(traj).kind;
            if kind == Throwable::Empty || kind == Throwable::Full {
                /* Only interested in hands on this pass */
                let tr = self.traj.get(traj);
                let angle = tr.angle + tr.spin * (self.time - tr.start) as f64;
                let (mut xd, mut yd) = (0.0, 0.0);
                /* Find the catching offset */
                if let Some(o) = tr.object {
                    let handle = self.objects.get(o).kind.handle();
                    if handle > 0.0 {
                        /* Handles need to be oriented */
                        xd = handle * PERSPEC * angle.sin();
                        yd = handle * angle.cos();
                    } else {
                        /* Balls are always caught at the bottom */
                        xd = 0.0;
                        yd = -4.0;
                    }
                }
                let p = DPoint {
                    x: tr.xp.at(self.time as f64) - xd,
                    y: tr.yp.at(self.time as f64) + yd,
                };
                let hand = tr.hand;
                let p = self.reach_arm(hand, p);

                /* Store updated hand position */
                let tr = self.traj.get_mut(traj);
                tr.x = p.x + xd;
                tr.y = p.y - yd;
            }

            let kind = self.traj.get(traj).kind;
            if kind == Throwable::Ball || kind == Throwable::Full {
                /* Only interested in objects on this pass */
                let tr = self.traj.get(traj);
                let (x, y) = if kind == Throwable::Full {
                    /* Adjusted these in the first pass */
                    (tr.x, tr.y)
                } else {
                    (tr.xp.at(self.time as f64), tr.yp.at(self.time as f64))
                };
                let angle = tr.angle + tr.spin * (self.time - tr.start) as f64;
                let divisions = tr.divisions;
                if let Some(o) = tr.object {
                    let head = self.objects.get(o).trace;
                    let at = self.traces.prev(head);
                    self.traces.add_after(
                        at,
                        Trace {
                            x,
                            y,
                            angle,
                            divisions,
                            dlast: DPoint::default(),
                        },
                    );
                    let ob = self.objects.get_mut(o);
                    ob.tracelen += 1;
                    ob.active = true;
                }
            }

            traj = next;
        }

        /* Erase end of trails */
        let mut o = self.objects.next(self.objects_head);
        while o != self.objects_head {
            let next = self.objects.next(o);
            let mut cur = o;
            loop {
                let ob = self.objects.get(cur);
                let head = ob.trace;
                if self.traces.next(head) == head || !(ob.count == 0 || ob.tracelen > ob.tail) {
                    break;
                }
                let s = self.traces.next(head);
                let (kind, black) = (ob.kind, self.mi.black);
                self.draw_object(d, kind, black, s);
                self.traces.remove(s);
                self.objects.get_mut(cur).tracelen -= 1;
                let ob = self.objects.get(cur);
                if ob.count <= 0 && ob.tracelen <= 0 {
                    /* Object no longer in use and trail gone */
                    let n = cur;
                    cur = self.objects.prev(cur);
                    self.object_destroy(n);
                }
                if self.objects.get(cur).count <= 0 {
                    break; /* Allow loop for catch-up, but not clean-up */
                }
            }
            o = next;
        }

        let black = self.mi.black;
        self.show_arms(d, black);
        let cx = self.wander(self.time);
        /* Reduce flicker by only permitting movements of more than a pixel */
        if (self.cx - cx).abs() * self.scale >= 2.0 {
            self.show_figure(d, black, false);
            self.cx = cx;
        }
        let white = self.mi.white;
        self.show_figure(d, white, false);
        self.show_arms(d, white);

        /* Draw Objects */
        let mut o = self.objects.next(self.objects_head);
        while o != self.objects_head {
            let next = self.objects.next(o);
            let ob = self.objects.get(o);
            if ob.active {
                let (kind, color) = (ob.kind, self.mi.pixel(ob.color as usize));
                let head = ob.trace;
                let last = self.traces.prev(head);
                self.draw_object(d, kind, color, last);
                self.objects.get_mut(o).active = false;
            }
            o = next;
        }

        // Save the pattern name so it can be erased when it changes. Upstream
        // draws the name here; with no font it clears the strip and stops.
        if let Some(p) = pattern
            && self.pattern != p
        {
            let black = self.mi.black;
            self.mi.gc.set_foreground(black);
            let (w, h) = (self.mi.width, 25);
            d.win().fill_rectangle(&self.mi.gc, 0, 0, w, h);
            self.pattern = p;
        }

        if future < self.time + 100 * self.throw_catch_interval() as i64 {
            self.refill();
        } else if self.time > 1 << 30 {
            /* Hard Reset before the clock wraps */
            self.free_performance();
            self.started = false;
            self.time = 0;
            self.reset(d);
        }

        self.mi.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.mi.reshape(width, height);
        self.reset(d);
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:   10000 ",
    "*count:   200   ",
    "*cycles:  1000  ",
    "*ncolors: 32    ",
    "*font:    -*-helvetica-bold-r-normal-*-180-*",
    "*fpsSolid: true",
    "*pattern: random",
    "*tail: 1",
    "*real: True",
    "*describe: True",
    "*balls: True",
    "*clubs: True",
    "*torches: True",
    "*knives: True",
    "*rings: True",
    "*bballs: True",
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
    Opt::slider("tail", "Trail length", 0.0, 100.0, 1.0, 0, "1"),
    Opt::boolean("balls", "Balls", "True"),
    Opt::boolean("clubs", "Clubs", "True"),
    Opt::boolean("rings", "Rings", "True"),
    Opt::boolean("knives", "Knives", "True"),
    Opt::boolean("torches", "Flaming torches", "True"),
    Opt::boolean("bballs", "Bowling balls", "True"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "juggle",
    label: "Juggle",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Tim Auckland",
        year: "2002",
        video: Some("https://www.youtube.com/watch?v=E3Ae7uQtWP0"),
        blurb: "A stick figure juggling, in patterns it invents as it goes.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
