//! Port of `hacks/penrose.c`.
//!
//! ```text
//! penrose --- quasiperiodic tilings
//!
//! Copyright (c) 1996 by Timo Korvola <tkorvola@dopey.hut.fi>
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
//! See Onoda, Steinhardt, DiVincenzo and Socolar in
//! Phys. Rev. Lett. 60, #25, 1988 or
//! Strandburg in Computers in Physics, Sep/Oct 1991.
//!
//! This implementation uses the simpler version of the growth
//! algorithm, i.e., if there are no forced vertices, a randomly chosen
//! tile is added to a randomly chosen vertex (no preference for those
//! 108 degree angles).
//!
//! There are two essential differences to the algorithm presented in
//! the literature: First, we do not allow the tiling to enclose an
//! untiled area.  Whenever this is in danger of happening, we just
//! do not add the tile, hoping for a better random choice the next
//! time.  Second, when choosing a vertex randomly, we will take
//! one that lies within the viewport if available.  If this seems to
//! cause enclosures in the forced rule case, we will allow invisible
//! vertices to be chosen.
//!
//! Tiling is restarted whenever one of the following happens: there
//! are no incomplete vertices within the viewport or the tiling has
//! extended a window's length beyond the edge of the window
//! horizontally or vertically or forced rule choice has failed 100
//! times due to areas about to become enclosed.
//! ```
//!
//! Two rhombs, one fat and one thin, that tile the plane and can never tile it
//! the same way twice. The tiling grows outwards one tile per frame from a
//! single edge, and the thing that makes it quasiperiodic rather than periodic
//! is that a tile may only be laid where a short list of rules permits.
//!
//! The rules are eight vertex figures: the eight ways rhombs are allowed to
//! meet at a point, each written as the cycle of tile corners around it. A
//! vertex on the growing edge remembers which of the eight it could still turn
//! into, and every tile laid next to it strikes out the ones that no longer
//! match. Matching is done by packing the corners into the bits of an integer
//! and rotating the rule to every offset, so a partial vertex matches a rule if
//! its bits are a substring of the rule's, wrapped.
//!
//! What makes this grow a plane rather than a thicket is that vertices whose
//! remaining rules all agree about the next tile are kept in a separate pool
//! and served first. A tile laid there is not a choice at all, it is the only
//! legal move, and laying it usually forces its neighbours in turn. Only when
//! nothing is forced does the algorithm pick a vertex and a legal tile at
//! random, which is the step that can go wrong.
//!
//! It goes wrong in two ways, and both are handled by giving up rather than by
//! backtracking. A tile can close a ring around untiled ground, which the
//! growth rules can never fill in afterwards, so before laying one the code
//! works out which neighbouring vertices it would join and refuses the move if
//! it would swallow a hole; a hundred refusals in a row and the tiling starts
//! over. Or the random tile can produce a vertex whose rule set is empty, a
//! dislocation, which is a real failure of the simple algorithm and is rare
//! enough that the author left it in. Both pause on screen for a moment before
//! the restart, which is upstream celebrating.
//!
//! Only the growing edge is kept. Vertices swallowed by the tiling are freed as
//! soon as they are enclosed, so the memory is proportional to the perimeter
//! and not the area, and nothing can be redrawn: a resize or a completed
//! tiling starts a new one.
//!
//! Coordinates are five integers, not a point. Every edge is a unit step along
//! one of five directions, so a vertex is an exact integer combination of them
//! and two vertices coincide exactly when their five integers do. Rounding to
//! pixels happens only when a vertex is drawn, and never feeds back into the
//! tiling.
//!
//! One thing here is not upstream's: with two colours or fewer it draws the
//! Ammann lines dashed, and there is no dash support in this runtime, so they
//! come out solid. Every other path is the same.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, lrand, nrand};
use crate::runtime::{About, Dpy, Opt, Runner, SaverDef, Screenhack, StartArgs, XPoint};

const MINSIZE: i32 = 5;
/// Frames of pause after a dislocation, which upstream calls celebrating.
const CELEBRATE: i32 = 31415;
/// Frames of pause once the tiling has filled the screen.
const COMPLETION: i32 = 3141;
const MAX_TILES_PER_VERTEX: usize = 7;
const N_VERTEX_RULES: usize = 8;
/// The most legal tiles a vertex can offer on one side.
const MAX_COMPL: usize = 2;

/// Which side of a vertex, looking at the untiled region from it.
const S_LEFT: u32 = 1;
const S_RIGHT: u32 = 2;

/// A vertex type packs the tile type and which corner of it is here.
const VT_CORNER_MASK: u8 = 0x3;
const VT_TYPE_MASK: u8 = 0x4;
const VT_THIN: u8 = 0;
const VT_THICK: u8 = 0x4;
const VT_BITS: u32 = 3;
const VT_TOTAL_MASK: u8 = 0x7;

/// Standing at a vertex looking at the middle of its tile, the corner on the
/// left, on the right, and across.
fn vt_left(vt: u8) -> u8 {
    (vt.wrapping_sub(1) & VT_CORNER_MASK) | (vt & VT_TYPE_MASK)
}
fn vt_right(vt: u8) -> u8 {
    (vt.wrapping_add(1) & VT_CORNER_MASK) | (vt & VT_TYPE_MASK)
}
fn vt_far(vt: u8) -> u8 {
    vt ^ 2
}

/// The eight ways tiles are allowed to meet at a vertex, counterclockwise.
struct VertexRule {
    tiles: [u8; MAX_TILES_PER_VERTEX],
    n_tiles: usize,
}

const VERTEX_RULES: [VertexRule; N_VERTEX_RULES] = [
    VertexRule {
        tiles: [
            VT_THICK | 2,
            VT_THICK | 2,
            VT_THICK | 2,
            VT_THICK | 2,
            VT_THICK | 2,
            0,
            0,
        ],
        n_tiles: 5,
    },
    VertexRule {
        tiles: [VT_THICK, VT_THICK, VT_THICK, VT_THICK, VT_THICK, 0, 0],
        n_tiles: 5,
    },
    VertexRule {
        tiles: [VT_THICK, VT_THICK, VT_THICK, VT_THIN, 0, 0, 0],
        n_tiles: 4,
    },
    VertexRule {
        tiles: [
            VT_THICK | 2,
            VT_THICK | 2,
            VT_THIN | 1,
            VT_THIN | 3,
            VT_THICK | 2,
            VT_THIN | 1,
            VT_THIN | 3,
        ],
        n_tiles: 7,
    },
    VertexRule {
        tiles: [
            VT_THICK | 2,
            VT_THICK | 2,
            VT_THICK | 2,
            VT_THICK | 2,
            VT_THIN | 1,
            VT_THIN | 3,
            0,
        ],
        n_tiles: 6,
    },
    VertexRule {
        tiles: [VT_THICK | 1, VT_THICK | 3, VT_THIN | 2, 0, 0, 0, 0],
        n_tiles: 3,
    },
    VertexRule {
        tiles: [VT_THICK, VT_THIN, VT_THIN, 0, 0, 0, 0],
        n_tiles: 3,
    },
    VertexRule {
        tiles: [
            VT_THICK | 2,
            VT_THIN | 1,
            VT_THICK | 3,
            VT_THICK | 1,
            VT_THIN | 3,
            0,
            0,
        ],
        n_tiles: 5,
    },
];

/// All angles are in multiples of thirty-six degrees.
const VTYPE_ANGLES: [i32; 8] = [4, 1, 4, 1, 2, 3, 2, 3];

fn vtype_angle(v: u8) -> i32 {
    VTYPE_ANGLES[v as usize]
}

/// Where a rule matched, and at what offset into it.
#[derive(Clone, Copy, Default)]
struct RuleMatch {
    rule: usize,
    pos: usize,
}

/// What adding a tile would do to the growing edge.
const FC_BAG: u32 = 1;
const FC_NEW_RIGHT: u32 = 2;
const FC_NEW_FAR: u32 = 4;
const FC_NEW_LEFT: u32 = 8;
const FC_CUT_THIS: u32 = 0x10;
const FC_CUT_RIGHT: u32 = 0x20;
const FC_CUT_FAR: u32 = 0x40;
const FC_CUT_LEFT: u32 = 0x80;

/// An arena index. Upstream's pointers are indices here, and its null is this.
type Id = usize;
const NIL: Id = usize::MAX;

/// A vertex of the growing edge. Nothing inside the tiling is kept.
#[derive(Clone)]
struct Fringe {
    prev: Id,
    next: Id,
    /// The tiles around this vertex, counterclockwise. Any gap lies between
    /// the last and the first.
    tiles: [u8; MAX_TILES_PER_VERTEX],
    n_tiles: usize,
    /// Which of the eight vertex rules this vertex could still become.
    rule_mask: u32,
    /// The forced-pool entry for this vertex, if it has one.
    forced: Id,
    loc: XPoint,
    /// The exact position, in whole steps along the five edge directions.
    fived: [i32; 5],
    off_screen: bool,
}

impl Default for Fringe {
    fn default() -> Self {
        Self {
            prev: NIL,
            next: NIL,
            tiles: [0; MAX_TILES_PER_VERTEX],
            n_tiles: 0,
            rule_mask: 0,
            forced: NIL,
            loc: XPoint { x: 0, y: 0 },
            fived: [0; 5],
            off_screen: false,
        }
    }
}

/// A vertex where at least one side can only be extended one way.
#[derive(Clone, Copy, Default)]
struct Forced {
    vertex: Id,
    forced_sides: u32,
    prev: Id,
    next: Id,
}

struct Penrose {
    mi: ModeInfo,
    width: i32,
    height: i32,
    origin: XPoint,
    edge_length: i32,
    line_width: i32,

    fringe: Vec<Fringe>,
    free_fringe: Vec<Id>,
    /// Some node of the ring, which is how the whole edge is reached.
    nodes: Id,
    /// On-screen vertices only.
    n_nodes: i32,

    forced: Vec<Forced>,
    free_forced: Vec<Id>,
    forced_first: Id,
    forced_n_nodes: i32,
    forced_n_visible: i32,

    done: bool,
    failures: i32,
    thick_color: usize,
    thin_color: usize,
    busy_loop: i32,
    ammann: bool,
    ammann_r: f32,
    fived_table: [(f32, f32); 5],
}

impl Penrose {
    fn new(d: &mut Dpy) -> Self {
        let mi = ModeInfo::new(d, ColorScheme::Random);
        let fifth = 8.0 * 1.0f64.atan() / 5.0;
        let mut fived_table = [(0.0, 0.0); 5];
        for (i, e) in fived_table.iter_mut().enumerate() {
            *e = (
                (fifth * i as f64).cos() as f32,
                (fifth * i as f64).sin() as f32,
            );
        }
        // 1 - sin(pi/10) / (2 sin(3 pi/10)): where an Ammann line crosses the
        // long diagonal of a fat rhomb.
        let pi10 = 2.0 * 1.0f64.atan() / 5.0;
        let ammann_r = (1.0 - pi10.sin() / (2.0 * (3.0 * pi10).sin())) as f32;

        let mut st = Self {
            width: mi.width,
            height: mi.height,
            mi,
            origin: XPoint { x: 0, y: 0 },
            edge_length: 1,
            line_width: 1,
            fringe: Vec::new(),
            free_fringe: Vec::new(),
            nodes: NIL,
            n_nodes: 0,
            forced: Vec::new(),
            free_forced: Vec::new(),
            forced_first: NIL,
            forced_n_nodes: 0,
            forced_n_visible: 0,
            done: false,
            failures: 0,
            thick_color: 0,
            thin_color: 0,
            busy_loop: 0,
            ammann: d.res.bool("ammann"),
            ammann_r,
            fived_table,
        };
        st.restart(d);
        st
    }

    // ---- the arena --------------------------------------------------------

    fn new_fringe(&mut self) -> Id {
        match self.free_fringe.pop() {
            Some(i) => {
                self.fringe[i] = Fringe::default();
                i
            }
            None => {
                self.fringe.push(Fringe::default());
                self.fringe.len() - 1
            }
        }
    }

    fn free_all(&mut self) {
        self.fringe.clear();
        self.free_fringe.clear();
        self.forced.clear();
        self.free_forced.clear();
        self.nodes = NIL;
        self.forced_first = NIL;
        self.forced_n_nodes = 0;
        self.forced_n_visible = 0;
    }

    /// `init_penrose`: start a new tiling from a single edge.
    fn restart(&mut self, d: &mut Dpy) {
        self.done = false;
        self.busy_loop = 0;
        self.failures = 0;
        self.width = self.mi.width;
        self.height = self.mi.height;
        let npixels = self.mi.npixels();
        if npixels > 2 {
            self.thick_color = nrand(npixels) as usize;
            // Insure good contrast.
            self.thin_color = ((nrand(2 * npixels / 3) + self.thick_color as i32 + npixels / 6)
                % npixels) as usize;
        }

        let mut size = self.mi.size;
        self.line_width = 1;
        if self.mi.width > 2560 || self.mi.height > 2560 {
            // Retina displays.
            size *= 3;
            self.line_width *= 3;
        }

        let half = MINSIZE.max(self.width.min(self.height) / 2);
        self.edge_length = if size < -MINSIZE {
            nrand((-size).min(half) - MINSIZE + 1) + MINSIZE
        } else if size < MINSIZE {
            if size == 0 { half } else { MINSIZE }
        } else {
            size.min(half)
        };
        self.origin = XPoint {
            x: (self.width / 2 + nrand(self.width)) / 2,
            y: (self.height / 2 + nrand(self.height)) / 2,
        };

        self.free_all();
        self.n_nodes = 2;

        // First vertex, at the origin.
        let a = self.new_fringe();
        let b = self.new_fringe();
        self.fringe[a] = Fringe {
            prev: b,
            next: b,
            rule_mask: (1 << N_VERTEX_RULES) - 1,
            loc: self.origin,
            ..Fringe::default()
        };
        // Second vertex, one edge away in a random direction. That is the
        // whole starting position.
        let mut second = self.fringe[a].clone();
        second.prev = a;
        second.next = a;
        second.fived[nrand(5) as usize] = 2 * nrand(2) - 1;
        second.loc = self.fived_to_loc(&second.fived);
        self.fringe[b] = second;
        self.nodes = a;

        self.mi.clear_window(d);
    }

    // ---- geometry ---------------------------------------------------------

    /// The direction of the edge from a vertex to its neighbour on one side.
    fn vertex_dir(&mut self, vertex: Id, side: u32) -> i32 {
        let v2 = if side == S_LEFT {
            self.fringe[vertex].next
        } else {
            self.fringe[vertex].prev
        };
        for i in 0..5 {
            match self.fringe[v2].fived[i] - self.fringe[vertex].fived[i] {
                1 => return 2 * i as i32,
                -1 => return (2 * i as i32 + 5) % 10,
                _ => {}
            }
        }
        // Upstream reports this as weirdness and celebrates it.
        self.done = true;
        self.busy_loop = CELEBRATE;
        0
    }

    /// One step in a given direction, in five-dimensional coordinates.
    fn add_unit_vec(dir: i32, fived: &mut [i32; 5]) {
        const DIR2I: [usize; 5] = [0, 3, 1, 4, 2];
        let dir = dir.rem_euclid(10);
        fived[DIR2I[(dir % 5) as usize]] += if dir % 2 != 0 { -1 } else { 1 };
    }

    /// Screen coordinates from the five-dimensional ones. X has y increasing
    /// downwards, hence the subtraction.
    fn fived_to_loc(&self, fived: &[i32; 5]) -> XPoint {
        let mut ox = 0.0f32;
        let mut oy = 0.0f32;
        for (f, dir) in fived.iter().zip(self.fived_table.iter()) {
            let r = (f * self.edge_length) as f32;
            ox += r * dir.0;
            oy -= r * dir.1;
        }
        XPoint {
            x: self.origin.x + (ox + 0.5) as i32,
            y: self.origin.y + (oy + 0.5) as i32,
        }
    }

    // ---- the rules --------------------------------------------------------

    /// Match a vertex against the eight rules, striking out the ones that can
    /// no longer apply. Only strict subsequences match, wrapped.
    fn match_rules(&mut self, vertex: Id, matches: &mut [RuleMatch], first_only: bool) -> usize {
        let n_tiles = self.fringe[vertex].n_tiles;
        let mut hits = 0;
        let mut good_rules = [0usize; N_VERTEX_RULES];
        let mut n_good = 0;
        // Every live vertex has at least one tile by the time it is matched.
        let lower_bits_mask: u32 =
            !((VT_TOTAL_MASK as u32) << (VT_BITS * (n_tiles.max(1) as u32 - 1)));
        let mut new_rule_mask = 0;

        for (i, rule) in VERTEX_RULES.iter().enumerate() {
            if n_tiles >= rule.n_tiles {
                self.fringe[vertex].rule_mask &= !(1 << i);
            } else if self.fringe[vertex].rule_mask & (1 << i) != 0 {
                good_rules[n_good] = i;
                n_good += 1;
            }
        }
        let mut vertex_hash: u32 = 0;
        for i in 0..n_tiles {
            vertex_hash |= (self.fringe[vertex].tiles[i] as u32) << (VT_BITS * i as u32);
        }

        for &g in good_rules.iter().take(n_good) {
            let vr = &VERTEX_RULES[g];
            let mut rule_hash: u32 = 0;
            for i in 0..n_tiles {
                rule_hash |= (vr.tiles[i] as u32) << (VT_BITS * i as u32);
            }
            if rule_hash == vertex_hash {
                if let Some(m) = matches.get_mut(hits) {
                    *m = RuleMatch { rule: g, pos: 0 };
                }
                hits += 1;
                if first_only {
                    return hits;
                }
                new_rule_mask |= 1 << g;
            }
            for i in (1..vr.n_tiles).rev() {
                rule_hash = vr.tiles[i] as u32 | ((rule_hash & lower_bits_mask) << VT_BITS);
                if vertex_hash == rule_hash {
                    if let Some(m) = matches.get_mut(hits) {
                        *m = RuleMatch { rule: g, pos: i };
                    }
                    hits += 1;
                    if first_only {
                        return hits;
                    }
                    new_rule_mask |= 1 << g;
                }
            }
        }
        self.fringe[vertex].rule_mask = new_rule_mask;
        hits
    }

    /// The distinct tiles that could be added to one side of a vertex.
    fn find_completions(
        &self,
        vertex: Id,
        matches: &[RuleMatch],
        n_matches: usize,
        side: u32,
        results: &mut [u8; MAX_COMPL],
    ) -> usize {
        let mut n_res = 0;
        for m in matches.iter().take(n_matches) {
            let rule = &VERTEX_RULES[m.rule];
            let pos = (m.pos
                + if side == S_RIGHT {
                    self.fringe[vertex].n_tiles
                } else {
                    rule.n_tiles - 1
                })
                % rule.n_tiles;
            let vtype = rule.tiles[pos];
            if !results[..n_res].contains(&vtype) && n_res < MAX_COMPL {
                results[n_res] = vtype;
                n_res += 1;
            }
        }
        n_res
    }

    /// Whether a tile of this type would completely fill the gap at a vertex.
    fn fills_vertex(&mut self, vtype: u8, vertex: Id) -> bool {
        (self.vertex_dir(vertex, S_LEFT) - self.vertex_dir(vertex, S_RIGHT) - vtype_angle(vtype))
            % 10
            == 0
    }

    /// Which vertices a new tile would attach to, and which the tiling would
    /// swallow. A returned `NIL` means the vertex would have to be created.
    fn fringe_changes(&mut self, vertex: Id, side: u32, vtype: u8) -> (u32, Id, Id, Id) {
        let (mut right, mut left) = (NIL, NIL);
        let mut far = NIL;
        let mut f = NIL;
        let mut result = FC_NEW_FAR;

        if self.fills_vertex(vtype, vertex) {
            result |= FC_CUT_THIS;
        } else if side == S_LEFT {
            result |= FC_NEW_RIGHT;
        } else {
            result |= FC_NEW_LEFT;
        }

        if result & FC_NEW_LEFT == 0 {
            let v = self.fringe[vertex].next;
            left = v;
            if self.fills_vertex(vt_left(vtype), v) {
                result = (result & !FC_NEW_FAR) | FC_CUT_LEFT;
                f = self.fringe[v].next;
                far = f;
            }
        }
        if result & FC_NEW_RIGHT == 0 {
            let v = self.fringe[vertex].prev;
            right = v;
            if self.fills_vertex(vt_right(vtype), v) {
                result = (result & !FC_NEW_FAR) | FC_CUT_RIGHT;
                f = self.fringe[v].prev;
                far = f;
            }
        }
        if result & FC_NEW_FAR == 0 && f != NIL && self.fills_vertex(vt_far(vtype), f) {
            result |= FC_CUT_FAR;
            result &= !FC_NEW_LEFT & !FC_NEW_RIGHT;
            if result & FC_CUT_LEFT != 0 {
                right = self.fringe[f].next;
            }
            if result & FC_CUT_RIGHT != 0 {
                left = self.fringe[f].prev;
            }
        }
        if ((result & FC_CUT_LEFT != 0) && (result & FC_CUT_RIGHT != 0))
            || ((result & FC_CUT_THIS != 0) && (result & FC_CUT_FAR != 0))
        {
            result |= FC_BAG;
        }
        (result, right, far, left)
    }

    // ---- the forced pool --------------------------------------------------

    fn unlink_forced(&mut self, vertex: Id) {
        let node = self.fringe[vertex].forced;
        if node == NIL {
            return;
        }
        let (prev, next) = (self.forced[node].prev, self.forced[node].next);
        if prev == NIL {
            self.forced_first = next;
        } else {
            self.forced[prev].next = next;
        }
        if next != NIL {
            self.forced[next].prev = prev;
        }
        self.free_forced.push(node);
        self.forced_n_nodes -= 1;
        if !self.fringe[vertex].off_screen {
            self.forced_n_visible -= 1;
        }
        self.fringe[vertex].forced = NIL;
    }

    /// Move a vertex on or off the forced pool according to how many ways it
    /// can still be extended. A vertex with no rules left is a dislocation.
    fn check_vertex(&mut self, vertex: Id) {
        let mut hits = [RuleMatch::default(); MAX_TILES_PER_VERTEX * N_VERTEX_RULES];
        let n_hits = self.match_rules(vertex, &mut hits, false);
        let mut forced_sides = 0;

        if self.fringe[vertex].rule_mask == 0 {
            self.done = true;
            self.busy_loop = CELEBRATE;
        }
        let mut buf = [0u8; MAX_COMPL];
        if self.find_completions(vertex, &hits, n_hits, S_LEFT, &mut buf) == 1 {
            forced_sides |= S_LEFT;
        }
        if self.find_completions(vertex, &hits, n_hits, S_RIGHT, &mut buf) == 1 {
            forced_sides |= S_RIGHT;
        }

        if forced_sides == 0 {
            self.unlink_forced(vertex);
        } else if self.fringe[vertex].forced == NIL {
            let node = match self.free_forced.pop() {
                Some(i) => i,
                None => {
                    self.forced.push(Forced::default());
                    self.forced.len() - 1
                }
            };
            self.forced[node] = Forced {
                vertex,
                forced_sides,
                prev: NIL,
                next: self.forced_first,
            };
            if self.forced_first != NIL {
                self.forced[self.forced_first].prev = node;
            }
            self.forced_first = node;
            self.fringe[vertex].forced = node;
            self.forced_n_nodes += 1;
            if !self.fringe[vertex].off_screen {
                self.forced_n_visible += 1;
            }
        } else {
            let node = self.fringe[vertex].forced;
            self.forced[node].forced_sides = forced_sides;
        }
    }

    /// Drop a vertex that the tiling has swallowed. It must already be
    /// unlinked from the ring.
    fn delete_vertex(&mut self, vertex: Id) {
        if self.nodes == vertex {
            self.done = true;
            self.busy_loop = CELEBRATE;
        }
        self.unlink_forced(vertex);
        if !self.fringe[vertex].off_screen {
            self.n_nodes -= 1;
        }
        self.free_fringe.push(vertex);
    }

    /// A new vertex one step from an existing one, inheriting its ring links.
    fn alloc_vertex(&mut self, dir: i32, from: Id) -> Id {
        let v = self.new_fringe();
        self.fringe[v] = self.fringe[from].clone();
        let mut fived = self.fringe[v].fived;
        Self::add_unit_vec(dir, &mut fived);
        let loc = self.fived_to_loc(&fived);
        let node = &mut self.fringe[v];
        node.fived = fived;
        node.loc = loc;
        if loc.x < 0 || loc.y < 0 || loc.x >= self.width || loc.y >= self.height {
            let ww = self.width.max(200);
            let hh = self.height.max(200);
            node.off_screen = true;
            if loc.x < -ww || loc.y < -hh || loc.x >= 2 * ww || loc.y >= 2 * hh {
                self.done = true;
            }
        } else {
            node.off_screen = false;
            self.n_nodes += 1;
        }
        let node = &mut self.fringe[v];
        node.n_tiles = 0;
        node.rule_mask = (1 << N_VERTEX_RULES) - 1;
        node.forced = NIL;
        v
    }

    // ---- drawing ----------------------------------------------------------

    /// One rhomb. The vertices are counterclockwise and `vtype` says which
    /// corner of which tile the first of them is.
    fn draw_tile(&mut self, d: &mut Dpy, v1: Id, v2: Id, v3: Id, v4: Id, vtype: u8) {
        if self.fringe[v1].off_screen
            && self.fringe[v2].off_screen
            && self.fringe[v3].off_screen
            && self.fringe[v4].off_screen
        {
            return;
        }
        let corner = (vtype & VT_CORNER_MASK) as usize;
        let mut pts = [XPoint { x: 0, y: 0 }; 5];
        pts[corner] = self.fringe[v1].loc;
        pts[vt_right(corner as u8) as usize] = self.fringe[v2].loc;
        pts[vt_far(corner as u8) as usize] = self.fringe[v3].loc;
        pts[vt_left(corner as u8) as usize] = self.fringe[v4].loc;
        pts[4] = pts[0];

        let thick = (vtype & VT_TYPE_MASK) == VT_THICK;
        let fill = if self.mi.npixels() > 2 {
            self.mi.pixel(if thick {
                self.thick_color
            } else {
                self.thin_color
            })
        } else {
            self.mi.white
        };
        self.mi.gc.set_foreground(fill);
        d.win().fill_polygon(&self.mi.gc, &pts[..4]);
        self.mi.gc.set_foreground(self.mi.black);
        self.mi.gc.set_line_width(self.line_width);
        d.win().draw_lines(&self.mi.gc, &pts);

        if self.ammann {
            // The Ammann bars: the lines that cross every tile and, taken
            // together, are themselves a one-dimensional quasiperiodic
            // sequence. Upstream draws them dashed when there are no colours;
            // there are no dashes here, so they are drawn solid.
            let r = self.ammann_r;
            if thick {
                let colour = if self.mi.npixels() > 2 {
                    self.mi.pixel(self.thin_color)
                } else {
                    self.mi.black
                };
                self.mi.gc.set_foreground(colour);
                d.win().draw_line(
                    &self.mi.gc,
                    (r * pts[3].x as f32 + (1.0 - r) * pts[0].x as f32 + 0.5) as i32,
                    (r * pts[3].y as f32 + (1.0 - r) * pts[0].y as f32 + 0.5) as i32,
                    (r * pts[1].x as f32 + (1.0 - r) * pts[0].x as f32 + 0.5) as i32,
                    (r * pts[1].y as f32 + (1.0 - r) * pts[0].y as f32 + 0.5) as i32,
                );
            } else {
                let colour = if self.mi.npixels() > 2 {
                    self.mi.pixel(self.thick_color)
                } else {
                    self.mi.black
                };
                self.mi.gc.set_foreground(colour);
                d.win().draw_line(
                    &self.mi.gc,
                    ((pts[3].x + pts[2].x) as f32 / 2.0 + 0.5) as i32,
                    ((pts[3].y + pts[2].y) as f32 / 2.0 + 0.5) as i32,
                    ((pts[1].x + pts[2].x) as f32 / 2.0 + 0.5) as i32,
                    ((pts[1].y + pts[2].y) as f32 / 2.0 + 0.5) as i32,
                );
            }
        }
    }

    // ---- growth -----------------------------------------------------------

    fn add_vtype(&mut self, vertex: Id, side: u32, vtype: u8) {
        let v = &mut self.fringe[vertex];
        if v.n_tiles >= MAX_TILES_PER_VERTEX {
            return;
        }
        if side == S_RIGHT {
            v.tiles[v.n_tiles] = vtype;
        } else {
            for i in (1..=v.n_tiles).rev() {
                v.tiles[i] = v.tiles[i - 1];
            }
            v.tiles[0] = vtype;
        }
        v.n_tiles += 1;
    }

    /// Lay one tile against one side of a vertex, creating whatever vertices
    /// it needs. False if it would have swallowed untiled ground.
    fn add_tile(&mut self, d: &mut Dpy, vertex: Id, side: u32, vtype: u8) -> bool {
        let (fc, mut right, mut far, mut left) = self.fringe_changes(vertex, side, vtype);
        let ltype = vt_left(vtype);
        let rtype = vt_right(vtype);
        let ftype = vt_far(vtype);

        // This should never occur.
        if fc & FC_BAG != 0 {
            self.done = true;
        }
        if side == S_LEFT {
            if right == NIL {
                let dir = self.vertex_dir(vertex, S_LEFT) - vtype_angle(vtype);
                right = self.alloc_vertex(dir, vertex);
            }
            if far == NIL {
                let dir = self.vertex_dir(left, S_RIGHT) + vtype_angle(ltype);
                far = self.alloc_vertex(dir, left);
            }
        } else {
            if left == NIL {
                let dir = self.vertex_dir(vertex, S_RIGHT) + vtype_angle(vtype);
                left = self.alloc_vertex(dir, vertex);
            }
            if far == NIL {
                let dir = self.vertex_dir(right, S_LEFT) - vtype_angle(rtype);
                far = self.alloc_vertex(dir, right);
            }
        }

        // The new vertices must not land on top of existing ones. If any does,
        // give up and let the next attempt choose differently.
        let mut node = self.nodes;
        loop {
            let f = self.fringe[node].fived;
            if (fc & FC_NEW_LEFT != 0 && f == self.fringe[left].fived)
                || (fc & FC_NEW_RIGHT != 0 && f == self.fringe[right].fived)
                || (fc & FC_NEW_FAR != 0 && f == self.fringe[far].fived)
            {
                if fc & FC_NEW_LEFT != 0 {
                    self.delete_vertex(left);
                }
                if fc & FC_NEW_RIGHT != 0 {
                    self.delete_vertex(right);
                }
                if fc & FC_NEW_FAR != 0 {
                    self.delete_vertex(far);
                }
                return false;
            }
            node = self.fringe[node].next;
            if node == self.nodes {
                break;
            }
        }

        // Rechain the ring around the new tile.
        if fc & FC_CUT_THIS == 0 {
            if side == S_LEFT {
                self.fringe[vertex].next = right;
                self.fringe[right].prev = vertex;
            } else {
                self.fringe[vertex].prev = left;
                self.fringe[left].next = vertex;
            }
        }
        if fc & FC_CUT_FAR == 0 {
            if fc & FC_CUT_LEFT == 0 {
                self.fringe[far].next = left;
                self.fringe[left].prev = far;
            }
            if fc & FC_CUT_RIGHT == 0 {
                self.fringe[far].prev = right;
                self.fringe[right].next = far;
            }
        }
        self.draw_tile(d, vertex, right, far, left, vtype);

        // Drop the vertices the tile enclosed, and re-examine the rest.
        if fc & FC_CUT_THIS != 0 {
            self.nodes = far;
            self.delete_vertex(vertex);
        } else {
            self.add_vtype(vertex, side, vtype);
            self.check_vertex(vertex);
            self.nodes = vertex;
        }
        if fc & FC_CUT_FAR != 0 {
            self.delete_vertex(far);
        } else {
            let s = if fc & FC_CUT_RIGHT != 0 {
                S_LEFT
            } else {
                S_RIGHT
            };
            self.add_vtype(far, s, ftype);
            self.check_vertex(far);
        }
        if fc & FC_CUT_LEFT != 0 {
            self.delete_vertex(left);
        } else {
            let s = if fc & FC_CUT_FAR != 0 {
                S_LEFT
            } else {
                S_RIGHT
            };
            self.add_vtype(left, s, ltype);
            self.check_vertex(left);
        }
        if fc & FC_CUT_RIGHT != 0 {
            self.delete_vertex(right);
        } else {
            let s = if fc & FC_CUT_FAR != 0 {
                S_RIGHT
            } else {
                S_LEFT
            };
            self.add_vtype(right, s, rtype);
            self.check_vertex(right);
        }
        true
    }

    /// Lay the only tile a forced vertex will accept.
    fn add_forced_tile(&mut self, d: &mut Dpy, node: Id) -> bool {
        let sides = self.forced[node].forced_sides;
        let vertex = self.forced[node].vertex;
        let side = if sides == (S_LEFT | S_RIGHT) {
            if nrand(2) != 0 { S_LEFT } else { S_RIGHT }
        } else {
            sides
        };
        let mut hits = [RuleMatch::default(); MAX_TILES_PER_VERTEX * N_VERTEX_RULES];
        let n = self.match_rules(vertex, &mut hits, true);
        let mut vtype = [0u8; MAX_COMPL];
        let n = self.find_completions(vertex, &hits, n, side, &mut vtype);
        if n == 0 {
            self.done = true;
        }
        self.add_tile(d, vertex, side, vtype[0])
    }

    /// Whether a tile of this type could legally go on this side of a vertex.
    fn legal_move(&mut self, vertex: Id, side: u32, vtype: u8) -> bool {
        let mut hits = [RuleMatch::default(); MAX_TILES_PER_VERTEX * N_VERTEX_RULES];
        let n_hits = self.match_rules(vertex, &mut hits, false);
        let mut legal = [0u8; MAX_COMPL];
        let n = self.find_completions(vertex, &hits, n_hits, side, &mut legal);
        legal[..n].contains(&vtype)
    }

    /// Lay a random legal tile, which is what happens when nothing is forced.
    /// Every vertex the tile would touch has to accept it too.
    fn add_random_tile(&mut self, d: &mut Dpy, vertex: Id) {
        let npixels = self.mi.npixels();
        if npixels > 2 {
            self.thick_color = nrand(npixels) as usize;
            // Insure good contrast.
            self.thin_color = ((nrand(2 * npixels / 3) + self.thick_color as i32 + npixels / 6)
                % npixels) as usize;
        } else {
            // Upstream puts the white pixel in both, which nothing reads: with
            // this few colours the drawing uses white directly.
            self.thick_color = 0;
            self.thin_color = 0;
        }

        let mut hits = [RuleMatch::default(); MAX_TILES_PER_VERTEX * N_VERTEX_RULES];
        let n_hits = self.match_rules(vertex, &mut hits, false);
        let side = if nrand(2) != 0 { S_LEFT } else { S_RIGHT };
        let mut vtypes = [0u8; MAX_COMPL];
        let n = self.find_completions(vertex, &hits, n_hits, side, &mut vtypes);
        // One answer would mean a forced tile.
        if n == 0 {
            self.done = true;
        }

        let mut no_good = 0u32;
        let mut n_good = n as i32;
        for (i, &vt) in vtypes.iter().enumerate().take(n) {
            let (fc, right, far, left) = self.fringe_changes(vertex, side, vt);
            if fc & FC_BAG != 0 {
                self.done = true;
            }
            if right != NIL {
                let s = if (fc & FC_CUT_FAR != 0) && (fc & FC_CUT_LEFT != 0) {
                    S_RIGHT
                } else {
                    S_LEFT
                };
                if !self.legal_move(right, s, vt_right(vt)) {
                    no_good |= 1 << i;
                    n_good -= 1;
                    continue;
                }
            }
            if left != NIL {
                let s = if (fc & FC_CUT_FAR != 0) && (fc & FC_CUT_RIGHT != 0) {
                    S_LEFT
                } else {
                    S_RIGHT
                };
                if !self.legal_move(left, s, vt_left(vt)) {
                    no_good |= 1 << i;
                    n_good -= 1;
                    continue;
                }
            }
            if far != NIL {
                let s = if fc & FC_CUT_LEFT != 0 {
                    S_RIGHT
                } else {
                    S_LEFT
                };
                if !self.legal_move(far, s, vt_far(vt)) {
                    no_good |= 1 << i;
                    n_good -= 1;
                }
            }
        }
        if n_good <= 0 {
            // Upstream flags this and carries on into a search that then runs
            // off the end of its own array. Restarting is the same outcome
            // without the overrun.
            self.done = true;
            return;
        }

        // Pick the n_good'th tile that was not struck out.
        let pick = nrand(n_good);
        let mut j = 0;
        for _ in 0..=pick {
            while no_good & (1 << j) != 0 {
                j += 1;
            }
            j += 1;
        }
        if !self.add_tile(d, vertex, side, vtypes[j - 1]) {
            self.done = true;
            self.free_all();
        }
    }
}

impl Screenhack for Penrose {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.nodes == NIL {
            self.restart(d);
            return self.mi.delay;
        }
        if self.busy_loop > 0 {
            self.busy_loop -= 1;
            return self.mi.delay;
        }
        if self.done || self.failures >= 100 {
            self.restart(d);
            return self.mi.delay;
        }

        // The initial two-gon, which is one edge and no tiles.
        let nodes = self.nodes;
        if self.fringe[nodes].prev == self.fringe[nodes].next {
            let vtype = (VT_TOTAL_MASK as u32 & lrand()) as u8;
            if !self.add_tile(d, nodes, S_LEFT, vtype) {
                self.free_all();
            }
            return self.mi.delay;
        }
        // Nothing left on screen to grow from.
        if self.n_nodes == 0 {
            self.done = true;
            self.busy_loop = COMPLETION;
            return self.mi.delay;
        }

        let mut p = self.forced_first;
        if self.forced_n_visible > 0 && self.failures < 10 {
            // Prefer a forced vertex that can actually be seen.
            let n = nrand(self.forced_n_visible);
            let mut i = 0;
            loop {
                while p != NIL && self.fringe[self.forced[p].vertex].off_screen {
                    p = self.forced[p].next;
                }
                if p == NIL {
                    // The counts disagree with the list, which upstream would
                    // walk off the end of.
                    self.done = true;
                    return self.mi.delay;
                }
                if i < n {
                    i += 1;
                    p = self.forced[p].next;
                } else {
                    break;
                }
            }
        } else if self.forced_n_nodes > 0 {
            let n = nrand(self.forced_n_nodes);
            let mut i = 0;
            while i < n && p != NIL {
                i += 1;
                p = self.forced[p].next;
            }
            if p == NIL {
                self.done = true;
                return self.mi.delay;
            }
        } else {
            // Nothing is forced: grow from a random visible vertex instead.
            let mut fringe_p = self.nodes;
            let n = nrand(self.n_nodes);
            let mut guard = self.fringe.len() * (n as usize + 2);
            for _ in 0..=n {
                loop {
                    fringe_p = self.fringe[fringe_p].next;
                    if !self.fringe[fringe_p].off_screen {
                        break;
                    }
                    // The visible count says there is one; if it lies, stop
                    // rather than spinning.
                    guard -= 1;
                    if guard == 0 {
                        self.done = true;
                        return self.mi.delay;
                    }
                }
            }
            self.add_random_tile(d, fringe_p);
            self.failures = 0;
            return self.mi.delay;
        }

        if self.add_forced_tile(d, p) {
            self.failures = 0;
        } else {
            self.failures += 1;
        }
        self.mi.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.mi.reshape(width, height);
        self.width = width;
        self.height = height;
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    Box::new(Penrose::new(d))
}

const DEFAULTS: &[&str] = &[
    "*delay: 10000",
    "*size: 40",
    "*ncolors: 64",
    "*fpsSolid: true",
    "*ignoreRotation: True",
    "*ammann: False",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "64"),
    Opt::slider("size", "Tile size", 1.0, 100.0, 1.0, 0, "40"),
    Opt::boolean("ammann", "Draw ammann lines", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "penrose",
    label: "Penrose",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Timo Korvola",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=atlkrWkbYHk"),
        blurb: "Quasiperiodic tilings.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
