//! Port of `hacks/glx/klondike.c` and `hacks/glx/klondike-game.c`.
//!
//! ```text
//! klondike, Copyright (c) 2024  Joshua Timmons <josh@developerx.com>
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
//! A game of Klondike solitaire, playing itself, with the camera drifting
//! around the table. When the game runs out of moves the cards gather
//! themselves back into the deck and it deals again.
//!
//! The two upstream files are one module here, because they are not really
//! separable: every move in the game logic also sets up the animation that
//! shows it, writing where a card starts, where it is going, which frames it
//! travels between and how far it turns over on the way. The strategy is
//! greedy and tries, in this order: turn over a face-down card at the foot of
//! a pile, play from a pile to a foundation, move a king into an empty
//! column, move a run between columns, uncover a card a foundation wants,
//! play the waste to a foundation, play the waste to a column, deal from the
//! stock, and finally turn the waste back into the stock.
//!
//! Upstream reads one past the start of the foundation, the waste and a
//! tableau column whenever one of them is empty. In C those reads land on the
//! end of the previous row of the same array and quietly answer a question
//! nobody asked; here they are guarded, which is the same behaviour for every
//! case that matters and defined behaviour for the rest.
//!
//! Two of its quirks are kept because the picture depends on them: the
//! perspective is multiplied into the modelview rather than the projection,
//! which leaves the projection an identity, and the scale factors it applies
//! in `reshape` are wiped by the `glLoadIdentity` at the top of every frame.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::opts::SelectItem;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};
use std::f32::consts::PI;

const NUM_CARDS: usize = 52;
const MAX_WASTE: usize = 24;
const MAX_FOUNDATION: usize = 13;
const MAX_TABLEAU: usize = 20;

/// Upstream's suit order is diamonds, clubs, hearts, spades, which is also
/// the order the card pictures are in. Red and black alternate in it, which
/// is why a colour test is `suit % 2`.
const ACE: u8 = 1;
const KING: u8 = 13;

/// One card, and where it is on its way to.
#[derive(Clone, Copy, Default, PartialEq)]
struct Card {
    suit: u8,
    rank: u8,
    face_up: bool,

    x: f32,
    y: f32,
    z: f32,
    start_x: f32,
    start_y: f32,
    dest_x: f32,
    dest_y: f32,
    start_frame: f32,
    end_frame: f32,
    angle: f32,
    start_angle: f32,
    end_angle: f32,
    start_z: f32,
}

/// Where every card is: the stock, the seven columns, the waste and the four
/// foundations.
#[derive(Clone)]
struct Game {
    deck: [Card; NUM_CARDS],
    tableau: [[Card; MAX_TABLEAU]; 7],
    tableau_size: [usize; 7],
    waste: [Card; MAX_WASTE],
    waste_size: usize,
    foundation: [[Card; MAX_FOUNDATION]; 4],
    foundation_size: [usize; 4],
    moves: i32,
    moves_since_waste_flip: i32,
}

impl Default for Game {
    fn default() -> Self {
        Game {
            deck: [Card::default(); NUM_CARDS],
            tableau: [[Card::default(); MAX_TABLEAU]; 7],
            tableau_size: [0; 7],
            waste: [Card::default(); MAX_WASTE],
            waste_size: 0,
            foundation: [[Card::default(); MAX_FOUNDATION]; 4],
            foundation_size: [0; 4],
            moves: 0,
            moves_since_waste_flip: 0,
        }
    }
}

impl Game {
    /// `klondike_deck_size`: what is left in the stock, counted by what is
    /// not anywhere else.
    fn deck_size(&self) -> usize {
        let t: usize = self.tableau_size.iter().sum();
        let f: usize = self.foundation_size.iter().sum();
        NUM_CARDS - t - f - self.waste_size
    }

    /// The top card of a foundation, if it has one.
    fn foundation_top(&self, suit: u8) -> Option<Card> {
        let n = self.foundation_size[suit as usize];
        (n > 0).then(|| self.foundation[suit as usize][n - 1])
    }

    /// The face-up card at the head of the waste, if there is one.
    fn waste_top(&self) -> Option<Card> {
        (self.waste_size > 0).then(|| self.waste[self.waste_size - 1])
    }

    /// The card at the foot of a column, if it has one.
    fn tableau_top(&self, pile: usize) -> Option<Card> {
        let n = self.tableau_size[pile];
        (n > 0).then(|| self.tableau[pile][n - 1])
    }

    /// How many face-down cards a column is hiding under its foot.
    fn hidden(&self, pile: usize) -> usize {
        (0..self.tableau_size[pile].saturating_sub(1))
            .filter(|&j| !self.tableau[pile][j].face_up)
            .count()
    }

    /// `remove_card_from_deck`: take it out and close the gap.
    fn remove_from_deck(&mut self, card: Card) {
        if let Some(i) = (0..NUM_CARDS)
            .find(|&i| self.deck[i].suit == card.suit && self.deck[i].rank == card.rank)
        {
            for j in i..NUM_CARDS - 1 {
                self.deck[j] = self.deck[j + 1];
            }
            self.deck[NUM_CARDS - 1] = Card::default();
        }
    }
}

/// Everything the game needs to know about the table to animate a move.
struct Board {
    foundation_x: [f32; 4],
    foundation_y: [f32; 4],
    tableau_x: [f32; 7],
    tableau_y: [f32; 7],
    waste_x: f32,
    waste_y: f32,
    deck_x: f32,
    deck_y: f32,
    tick: f32,
    animation_ticks: f32,
    draw_count: usize,
    sloppy: bool,
}

impl Board {
    /// `RANDOM_POSITION_OFFSET`: a little scatter, so a pile does not look
    /// like it was stacked by a machine.
    fn jitter(&self) -> f32 {
        if self.sloppy {
            (frand(1.0) as f32 - 0.5) * 0.0125
        } else {
            0.0
        }
    }

    /// Set a card travelling from where it is to somewhere else.
    fn send(&self, card: &mut Card, dest: (f32, f32), start_angle: f32, end_angle: f32) {
        card.start_frame = self.tick;
        card.end_frame = self.tick + self.animation_ticks;
        card.start_x = card.x;
        card.start_y = card.y;
        card.dest_x = dest.0;
        card.dest_y = dest.1;
        card.start_angle = start_angle;
        card.end_angle = end_angle;
        card.start_z = card.z;
    }
}

/* ------------------------------------------------------------------ */
/* The moves                                                          */
/* ------------------------------------------------------------------ */

/// `move_card_to_foundation`: play one named card, from wherever it is, onto
/// its foundation.
fn move_card_to_foundation(b: &Board, g: &Game, card: Card) -> Option<Game> {
    if !card.face_up {
        return None;
    }
    let suit = card.suit as usize;
    let plays = card.rank == ACE
        || g.foundation_top(card.suit)
            .is_some_and(|t| t.rank + 1 == card.rank);
    if !plays {
        return None;
    }

    let mut ret = g.clone();
    let land = |ret: &mut Game, mut c: Card| {
        c.start_frame = b.tick;
        c.end_frame = b.tick + b.animation_ticks;
        c.start_x = c.x;
        c.start_y = c.y;
        c.dest_x = b.foundation_x[suit];
        c.dest_y = b.foundation_y[suit];
        c.start_angle = c.end_angle;
        c.start_z = c.z + 3.0;
        let n = ret.foundation_size[suit];
        ret.foundation[suit][n] = c;
        ret.foundation_size[suit] = n + 1;
    };

    for i in 0..7 {
        if let Some(top) = ret.tableau_top(i)
            && top.rank == card.rank
            && top.suit == card.suit
        {
            let j = ret.tableau_size[i] - 1;
            let moved = ret.tableau[i][j];
            ret.tableau_size[i] -= 1;
            land(&mut ret, moved);
            return Some(ret);
        }
    }

    if let Some(top) = ret.waste_top()
        && top.rank == card.rank
        && top.suit == card.suit
    {
        land(&mut ret, top);
        ret.waste_size -= 1;
        return Some(ret);
    }
    None
}

/// `move_king_to_empty_tableau`: free a column by moving a king that is
/// sitting on face-down cards into an empty one.
fn move_king_to_empty_tableau(b: &Board, g: &Game) -> Option<Game> {
    let mut min_hidden = 20;
    let mut min_pile = None;
    for i in 0..7 {
        if g.tableau_top(i).is_some_and(|t| t.rank == KING) {
            let hidden = g.hidden(i);
            if hidden < min_hidden && hidden > 0 {
                min_hidden = hidden;
                min_pile = Some(i);
            }
        }
    }
    let from = min_pile?;
    let to = (0..7).find(|&i| g.tableau_size[i] == 0)?;

    let mut ret = g.clone();
    let mut card = ret.tableau[from][ret.tableau_size[from] - 1];
    ret.tableau_size[from] -= 1;
    b.send(
        &mut card,
        (b.tableau_x[to] + b.jitter(), b.tableau_y[to] + b.jitter()),
        180.0,
        180.0,
    );
    ret.tableau[to][0] = card;
    ret.tableau_size[to] = 1;
    Some(ret)
}

/// `can_move_to_tableau`: may this card go on the foot of that column?
fn can_move_to_tableau(g: &Game, card: Card, to_pile: usize) -> bool {
    let pile = (0..7).find(|&i| {
        (0..g.tableau_size[i])
            .any(|j| g.tableau[i][j].rank == card.rank && g.tableau[i][j].suit == card.suit)
    });
    let hidden = pile.map_or(0, |p| g.hidden(p));

    // A king may take an empty column, but only if moving it uncovers
    // something.
    if hidden > 0 && g.tableau_size[to_pile] == 0 && card.rank == KING && card.face_up {
        return true;
    }
    let Some(top) = g.tableau_top(to_pile) else {
        return false;
    };
    card.face_up && top.face_up && card.rank + 1 == top.rank && card.suit % 2 != top.suit % 2
}

/// `move_tableau_base_card_to_tableau`: move the whole face-up run at the
/// foot of one column onto another, preferring high cards and the columns
/// hiding the most.
fn move_run_between_columns(b: &Board, g: &Game) -> Option<Game> {
    for preferred_rank in (1..=13u8).rev() {
        for preferred_hidden in (0..=6usize).rev() {
            for i in 0..7 {
                if g.hidden(i) != preferred_hidden {
                    continue;
                }
                let Some(base) = (0..g.tableau_size[i]).find(|&k| g.tableau[i][k].face_up) else {
                    continue;
                };
                if g.tableau[i][base].rank != preferred_rank {
                    continue;
                }
                for j in 0..7 {
                    if i == j || !can_move_to_tableau(g, g.tableau[i][base], j) {
                        continue;
                    }
                    let mut ret = g.clone();
                    let face_up = (0..ret.tableau_size[j])
                        .filter(|&l| ret.tableau[j][l].face_up)
                        .count();
                    for k in base..ret.tableau_size[i] {
                        let mut card = ret.tableau[i][k];
                        let n = ret.tableau_size[j];
                        let dest = (
                            b.tableau_x[j] + b.jitter(),
                            b.tableau_y[j] - (face_up + k - base) as f32 * 0.05 + b.jitter(),
                        );
                        let angle = card.end_angle;
                        b.send(&mut card, dest, angle, angle);
                        ret.tableau[j][n] = card;
                        ret.tableau_size[j] = n + 1;
                    }
                    ret.tableau_size[i] = base;
                    return Some(ret);
                }
            }
        }
    }
    None
}

/// `reveal_foundation_move`: if a foundation wants a card that is buried one
/// deep, play whatever is on top of it.
fn reveal_foundation_move(b: &Board, g: &Game) -> Option<Game> {
    for i in 0..4 {
        let Some(top) = g.foundation_top(i as u8) else {
            continue;
        };
        for j in 0..7 {
            for k in 0..g.tableau_size[j] {
                let c = g.tableau[j][k];
                if c.rank == top.rank + 1
                    && c.face_up
                    && top.suit == c.suit
                    && g.tableau_size[j] > k + 1
                    && let Some(ret) = move_card_to_foundation(b, g, g.tableau[j][k + 1])
                {
                    return Some(ret);
                }
            }
        }
    }
    None
}

/// `move_tableau_base_card_to_foundation`: play the foot of a column.
fn play_column_foot(b: &Board, g: &Game) -> Option<Game> {
    for i in 0..7 {
        let Some(card) = g.tableau_top(i) else {
            continue;
        };
        for j in 0..4 {
            let wanted = match g.foundation_top(j as u8) {
                None => card.rank == ACE,
                Some(t) => t.rank + 1 == card.rank && t.suit == card.suit && card.face_up,
            };
            if wanted && let Some(ret) = move_card_to_foundation(b, g, card) {
                return Some(ret);
            }
        }
    }
    None
}

/// `move_waste_to_foundation`.
fn move_waste_to_foundation(b: &Board, g: &Game) -> Option<Game> {
    let top = g.waste_top()?;
    for i in 0..4u8 {
        let wanted = match g.foundation_top(i) {
            Some(t) => t.rank + 1 == top.rank,
            None => top.rank == ACE && top.suit == i,
        };
        if wanted && let Some(ret) = move_card_to_foundation(b, g, top) {
            return Some(ret);
        }
    }
    None
}

/// `move_waste_to_tableau`.
fn move_waste_to_tableau(b: &Board, g: &Game) -> Option<Game> {
    let top = g.waste_top()?;
    for i in 0..7 {
        let onto = g
            .tableau_top(i)
            .is_some_and(|t| t.rank == top.rank + 1 && t.suit % 2 != top.suit % 2);
        let empty = g.tableau_size[i] == 0 && top.rank == KING;
        if !onto && !empty {
            continue;
        }
        let mut ret = g.clone();
        let face_up = (0..ret.tableau_size[i])
            .filter(|&k| ret.tableau[i][k].face_up)
            .count();
        let mut card = top;
        b.send(
            &mut card,
            (
                b.tableau_x[i] + b.jitter(),
                b.tableau_y[i] - face_up as f32 * 0.05 + b.jitter(),
            ),
            180.0,
            180.0,
        );
        let n = ret.tableau_size[i];
        ret.tableau[i][n] = card;
        ret.tableau_size[i] = n + 1;
        ret.waste_size -= 1;
        return Some(ret);
    }
    None
}

/// `move_deck_to_waste`: turn over the next one or three from the stock.
fn move_deck_to_waste(b: &Board, g: &Game) -> Option<Game> {
    let on_table: usize = g.tableau_size.iter().sum::<usize>()
        + g.foundation_size.iter().sum::<usize>()
        + g.waste_size;
    if on_table == NUM_CARDS {
        return None;
    }
    let mut ret = g.clone();
    for i in 0..b.draw_count {
        if on_table + i < NUM_CARDS {
            let mut card = ret.deck[0];
            card.face_up = true;
            card.start_frame = b.tick + b.animation_ticks / 4.0 * i as f32;
            card.end_frame = card.start_frame + b.animation_ticks;
            card.start_x = b.deck_x;
            card.start_y = b.deck_y;
            card.dest_x = b.waste_x + 0.025 * ret.waste_size as f32 + b.jitter();
            card.dest_y = b.waste_y + b.jitter();
            card.start_angle = 0.0;
            card.end_angle = 180.0;
            card.start_z = card.z;
            let n = ret.waste_size;
            ret.waste[n] = card;
            ret.waste_size = n + 1;
            let taken = ret.deck[0];
            ret.remove_from_deck(taken);
        }
    }
    Some(ret)
}

/// `turn_over_last_tableau_card`.
fn turn_over_last_tableau_card(b: &Board, g: &Game) -> Option<Game> {
    for i in 0..7 {
        if g.tableau_top(i).is_some_and(|t| !t.face_up) {
            let mut ret = g.clone();
            let j = ret.tableau_size[i] - 1;
            let card = &mut ret.tableau[i][j];
            card.face_up = true;
            card.start_frame = b.tick;
            card.end_frame = b.tick + b.animation_ticks;
            card.start_x = card.x;
            card.start_y = card.y;
            card.dest_x = card.x;
            card.dest_y = card.y;
            card.start_angle = 0.0;
            card.end_angle = 180.0;
            return Some(ret);
        }
    }
    None
}

/// `reset_waste`: the stock is empty, so the waste becomes the stock again.
fn reset_waste(b: &Board, g: &mut Game) {
    for i in 0..g.waste_size {
        let mut card = g.waste[i];
        let start = b.tick + (i + 5) as f32 * b.animation_ticks / 3.0;
        card.start_frame = start;
        card.end_frame = start + b.animation_ticks;
        card.start_x = card.x;
        card.start_y = card.y;
        card.dest_x = b.deck_x + b.jitter();
        card.dest_y = b.deck_y + b.jitter();
        card.start_angle = 180.0;
        card.end_angle = 360.0;
        card.start_z = card.z;
        g.deck[i] = card;
        g.waste[i] = Card::default();
    }
    g.waste_size = 0;
    g.moves_since_waste_flip = 0;
}

/// `klondike_next_move`: the whole strategy, in the order it tries things.
fn next_move(b: &Board, g: &Game) -> Option<Game> {
    let tries: [fn(&Board, &Game) -> Option<Game>; 7] = [
        turn_over_last_tableau_card,
        play_column_foot,
        move_king_to_empty_tableau,
        move_run_between_columns,
        reveal_foundation_move,
        move_waste_to_foundation,
        move_waste_to_tableau,
    ];
    let mut ret = None;
    for f in tries {
        if let Some(mut r) = f(b, g) {
            r.moves_since_waste_flip += 1;
            ret = Some(r);
            break;
        }
    }
    if ret.is_none() {
        ret = move_deck_to_waste(b, g);
    }
    if ret.is_none() && g.moves_since_waste_flip > 0 {
        let mut r = g.clone();
        reset_waste(b, &mut r);
        ret = Some(r);
    }

    let mut r = ret?;
    r.moves += 1;
    // Wipe what is past the end of each pile, so a stale card cannot be
    // mistaken for a real one.
    for i in 0..4 {
        for j in r.foundation_size[i]..MAX_FOUNDATION {
            r.foundation[i][j] = Card::default();
        }
    }
    for i in 0..7 {
        for j in r.tableau_size[i]..MAX_TABLEAU {
            r.tableau[i][j] = Card::default();
        }
    }
    for i in r.waste_size..MAX_WASTE {
        r.waste[i] = Card::default();
    }
    Some(r)
}

/* ------------------------------------------------------------------ */
/* The table                                                          */
/* ------------------------------------------------------------------ */

fn ease_in_out_quart(x: f32) -> f32 {
    if x < 0.5 {
        8.0 * x * x * x * x
    } else {
        1.0 - (-2.0 * x + 2.0).powi(4) / 2.0
    }
}

fn ease_out_quart(x: f32) -> f32 {
    1.0 - (1.0 - x).powi(4)
}

struct Klondike {
    trackball: Trackball,
    board: Board,
    game: Game,
    camera_speed: f32,
    camera_phase: f32,
    tick: f32,
    universe_tick: f32,
    final_animation: f32,
    redeal: bool,
    /// One texture per card front, in upstream's order, and the back.
    fronts: [u32; 52],
    back: u32,
}

impl Klondike {
    /// `initialize_placeholders`: where the piles sit on the table.
    fn place(&mut self, width: i32, height: i32) {
        let (w, h) = (width as f32, height as f32);
        let scale = 1.1;
        let mut xscale = if w > h { 0.53 * h / w } else { 0.53 };
        if w < h {
            xscale *= w / 1280.0;
        }
        for i in 0..4 {
            self.board.foundation_x[i] =
                0.15 + -0.4 + (0.075 + 0.15 * i as f32 * xscale / 0.3) * scale;
            self.board.foundation_y[i] = 0.7 * scale;
        }
        for i in 0..7 {
            self.board.tableau_x[i] = 0.15 + -0.55 + 0.15 * i as f32 * xscale / 0.3 * scale;
            self.board.tableau_y[i] = 0.3 * scale;
        }
        self.board.waste_x = self.board.tableau_x[0];
        self.board.waste_y = -0.65 * scale;
        self.board.deck_x = self.board.tableau_x[6];
        self.board.deck_y = -0.65 * scale;
    }

    /// `klondike_initialize_deck` and `klondike_shuffle_deck`.
    fn new_deck(&mut self) {
        let b = &self.board;
        let mut i = 0;
        for suit in 0..4u8 {
            for rank in ACE..=KING {
                self.game.deck[i] = Card {
                    suit,
                    rank,
                    face_up: false,
                    x: b.deck_x,
                    y: b.deck_y,
                    start_x: b.deck_x + b.jitter(),
                    start_y: b.deck_y + b.jitter(),
                    dest_x: b.deck_x + b.jitter(),
                    dest_y: b.deck_y + b.jitter(),
                    ..Card::default()
                };
                i += 1;
            }
        }
        for i in (1..NUM_CARDS).rev() {
            let j = random() as usize % (i + 1);
            self.game.deck.swap(i, j);
        }
    }

    /// `klondike_deal_cards`: seven columns, one more card each.
    fn deal(&mut self) {
        for i in 0..7 {
            self.game.tableau_size[i] = 0;
            for j in 0..=i {
                let card = self.game.deck[0];
                self.game.remove_from_deck(card);
                self.game.tableau[i][j] = card;
                if j == i {
                    self.game.tableau[i][j].face_up = true;
                }
                self.game.tableau_size[i] += 1;
            }
        }
        self.game.waste_size = 0;
        self.game.foundation_size = [0; 4];
        self.game.moves = 0;
        self.game.moves_since_waste_flip = 0;
    }

    /// `animate_initial_board`: deal them out one at a time, across then down.
    fn animate_deal(&mut self) {
        let mut n = 0;
        for i in 0..7 {
            for j in 0..7 {
                if i < self.game.tableau_size[j] {
                    let face_up = i == self.game.tableau_size[j] - 1;
                    let dest = (
                        self.board.tableau_x[j] + self.board.jitter(),
                        self.board.tableau_y[j] + self.board.jitter(),
                    );
                    let card = &mut self.game.tableau[j][i];
                    card.start_frame = 10.0 + n as f32 * self.board.animation_ticks / 4.0;
                    card.end_frame = card.start_frame + self.board.animation_ticks;
                    card.start_x = self.board.deck_x;
                    card.start_y = self.board.deck_y;
                    card.dest_x = dest.0;
                    card.dest_y = dest.1;
                    card.angle = 0.0;
                    card.start_angle = 0.0;
                    card.face_up = face_up;
                    card.end_angle = if face_up { 180.0 } else { 0.0 };
                    n += 1;
                }
            }
        }
    }

    /// `animate_board_to_deck`: the game is over, so gather everything up.
    fn animate_gather(&mut self) {
        let mut n = 0;
        let gather = |card: &mut Card, b: &Board, n: &mut i32| {
            card.start_frame = b.tick + *n as f32 * b.animation_ticks / 3.0;
            card.end_frame = card.start_frame + b.animation_ticks;
            card.start_x = card.x;
            card.start_y = card.y;
            card.dest_x = b.deck_x + b.jitter();
            card.dest_y = b.deck_y + b.jitter();
            card.start_angle = card.angle;
            card.end_angle = 360.0;
            card.face_up = false;
            *n += 1;
        };
        for i in 0..7 {
            for j in 0..self.game.tableau_size[i] {
                gather(&mut self.game.tableau[i][j], &self.board, &mut n);
            }
        }
        for i in 0..4 {
            for j in 0..self.game.foundation_size[i] {
                gather(&mut self.game.foundation[i][j], &self.board, &mut n);
            }
        }
        for j in 0..self.game.waste_size {
            gather(&mut self.game.waste[j], &self.board, &mut n);
        }
        self.final_animation = self.board.tick
            + n as f32 * self.board.animation_ticks / 3.0
            + self.board.animation_ticks;
    }

    /// Everything on the table, in the order upstream collects it.
    fn render_order(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::with_capacity(NUM_CARDS);
        for i in 0..7 {
            for j in 0..self.game.tableau_size[i] {
                out.push((i, j));
            }
        }
        for i in 0..4 {
            for j in 0..self.game.foundation_size[i] {
                out.push((7 + i, j));
            }
        }
        for j in 0..self.game.waste_size {
            out.push((11, j));
        }
        // The stock, top card first.
        for j in (0..self.game.deck_size()).rev() {
            out.push((12, j));
        }
        out
    }

    fn card_at(&mut self, key: (usize, usize)) -> &mut Card {
        match key.0 {
            0..=6 => &mut self.game.tableau[key.0][key.1],
            7..=10 => &mut self.game.foundation[key.0 - 7][key.1],
            11 => &mut self.game.waste[key.1],
            _ => &mut self.game.deck[key.1],
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let mut this = Klondike {
        trackball: Trackball::new(),
        board: Board {
            foundation_x: [0.0; 4],
            foundation_y: [0.0; 4],
            tableau_x: [0.0; 7],
            tableau_y: [0.0; 7],
            waste_x: 0.0,
            waste_y: 0.0,
            deck_x: 0.0,
            deck_y: 0.0,
            tick: 0.0,
            animation_ticks: g.res.int("speed").clamp(15, 200) as f32,
            draw_count: if g.res.int("draw") == 1 { 1 } else { 3 },
            sloppy: g.res.bool("sloppy"),
        },
        game: Game::default(),
        camera_speed: g.res.int("camera_speed").clamp(10, 100) as f32,
        camera_phase: (frand(1.0) as f32) * 0.2 * PI,
        tick: 0.0,
        universe_tick: 0.0,
        final_animation: 0.0,
        redeal: false,
        fronts: [0; 52],
        back: 0,
    };

    // The card pictures, indexed as upstream indexes them: suit times
    // thirteen plus rank, in the order diamonds, clubs, hearts, spades.
    const SUITS: [char; 4] = ['D', 'C', 'H', 'S'];
    const RANKS: [char; 13] = [
        'A', '2', '3', '4', '5', '6', '7', '8', '9', 'T', 'J', 'Q', 'K',
    ];
    let load = |g: &mut Gl, want: &str| -> u32 {
        let Some((_, bytes)) = crate::images::KLONDIKE_CARDS
            .iter()
            .find(|(n, _)| *n == want)
        else {
            return 0;
        };
        let Some((w, h, px)) = crate::runtime::png::decode_rgba(bytes) else {
            return 0;
        };
        let id = g.glx.gen_texture();
        g.glx.bind_texture(id);
        g.glx.tex_image_2d(w, h, px);
        g.glx.tex_clamp(true);
        g.glx.tex_nearest(false);
        id
    };
    for (s, suit) in SUITS.iter().enumerate() {
        for (r, rank) in RANKS.iter().enumerate() {
            this.fronts[s * 13 + r] = load(g, &format!("{suit}{rank}"));
        }
    }
    this.back = load(g, "back");

    let (w, h) = (g.width(), g.height());
    this.place(w, h);
    Box::new(this)
}

impl Hack3d for Klondike {
    fn reshape(&mut self, _g: &mut Gl, width: i32, height: i32) {
        self.place(width, height);
        self.redeal = true;
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        let (width, height) = (g.width(), g.height());

        if self.tick == 0.0 {
            self.place(width, height);
            self.new_deck();
            self.deal();
            self.animate_deal();
        }
        self.tick += 1.0;
        self.universe_tick += 1.0;
        self.board.tick = self.tick;

        // Collect what is on the table, give each card its depth in the
        // stack, and lift the ones that are travelling.
        let order = self.render_order();
        for (i, key) in order.iter().enumerate() {
            let tick = self.tick;
            let card = self.card_at(*key);
            card.z = i as f32 / 10.0;
            if tick >= card.start_frame && tick < card.end_frame {
                let n = (tick - card.start_frame) / (card.end_frame - card.start_frame);
                card.z += card.start_z * (1.0 - ease_in_out_quart(n));
                card.z += 8.0 * (n * PI).sin();
            } else {
                card.start_z = 0.0;
            }
        }

        // Back to front, and within a depth, in the order the moves happened.
        let mut sorted = order.clone();
        sorted.sort_by(|a, b| {
            let (ca, cb) = (self.peek(*a), self.peek(*b));
            ca.z.total_cmp(&cb.z)
                .then(ca.end_frame.total_cmp(&cb.end_frame))
        });

        let speed = self.camera_speed / 100.0;
        let t = (self.universe_tick + self.camera_phase) * speed;
        let theta = PI / 2.0 + (t * 0.0065).sin() * 0.225;
        let phi = -0.55 + (t * 0.008).sin() * 0.25;
        let d = 3.5 + (t * 0.013).sin();

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.front_face_cw(false);
        g.glx.blend(Blend::Alpha);
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        g.glx.color_material(false);
        g.glx.material_ambient([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_diffuse([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_specular([0.0, 0.0, 0.0, 1.0]);
        g.glx.material_shininess(0.0);

        // Upstream puts the perspective into the modelview rather than the
        // projection and leaves the projection an identity. Keeping that is
        // the difference between its picture and a different one.
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx.perspective(30.0, 1.0, 1.0, 200.0);
        g.glx.light_position(0, 0.0, 0.0, 1.0, 0.0);
        g.glx.light_ambient(0, [0.8, 0.8, 0.8, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_specular(0, [0.0, 0.0, 0.0, 1.0]);
        g.glx.look_at(
            [
                d * theta.cos() * phi.sin(),
                d * theta.sin() * phi.sin(),
                d * phi.cos(),
            ],
            [0.1, 0.0, 0.0],
            [0.0, 0.0, 1.0],
        );
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        let (w, h) = (width as f32, height as f32);
        let mut horiz = 0.53 * h / w;
        let mut vert = 0.53f32;
        if w < h {
            horiz *= w / 1280.0;
            vert *= w / 1280.0;
        }

        let mut last_animation = 0.0f32;
        for key in &sorted {
            let tick = self.tick;
            let card = self.card_at(*key);
            last_animation = last_animation.max(card.end_frame);

            if tick >= card.start_frame && tick < card.end_frame {
                let n = (tick - card.start_frame) / (card.end_frame - card.start_frame);
                let eased = ease_out_quart(n);
                card.x = card.start_x + eased * (card.dest_x - card.start_x);
                card.y = card.start_y + eased * (card.dest_y - card.start_y);
                card.angle = card.start_angle + n * (card.end_angle - card.start_angle);
            } else if tick >= card.end_frame {
                card.x = card.dest_x;
                card.y = card.dest_y;
                card.angle = card.end_angle;
            }
            let card = *card;

            // Between ninety and two hundred and seventy degrees the card is
            // showing its face, and the quad is turned inside out to match.
            let showing_front = card.angle > 90.0 && card.angle < 270.0;
            let (tex, s) = if showing_front {
                let n = card.suit as usize * 13 + card.rank.max(1) as usize - 1;
                (self.fronts[n.min(51)], -1.0f32)
            } else {
                (self.back, 1.0f32)
            };

            g.glx.push_matrix();
            g.glx.translate(card.x, card.y, card.z * 0.025);
            g.glx.rotate(card.angle, 0.0, 1.0, 0.0);
            g.glx.scale(s * horiz * 0.45, vert * 0.45, horiz * 0.45);
            g.glx.texturing(true);
            g.glx.bind_texture(tex);
            g.glx.begin(Shape::Quads);
            g.glx.normal3f(0.0, 0.0, 1.0);
            for (u, v, x, y) in [
                (0.0, 0.0, -0.5, -0.75),
                (1.0, 0.0, 0.5, -0.75),
                (1.0, 1.0, 0.5, 0.75),
                (0.0, 1.0, -0.5, 0.75),
            ] {
                g.glx.tex_coord2f(u, v);
                g.glx.vertex3f(x, y, 0.0);
            }
            g.glx.end();
            g.glx.pop_matrix();
        }

        // When everything has come to rest, make the next move.
        if self.tick >= last_animation {
            let mut next = None;
            if self.redeal {
                self.game = Game::default();
                self.tick = 0.0;
                self.final_animation = 0.0;
                self.redeal = false;
            } else if self.final_animation == 0.0 {
                next = next_move(&self.board, &self.game);
            }
            if self.final_animation != 0.0 && self.tick >= self.final_animation {
                self.redeal = true;
            } else if self.final_animation == 0.0 && next.is_none() {
                self.animate_gather();
            }
            if let Some(n) = next {
                self.game = n;
            }
        }

        g.res.int("delay") as u32
    }
}

impl Klondike {
    fn peek(&self, key: (usize, usize)) -> Card {
        match key.0 {
            0..=6 => self.game.tableau[key.0][key.1],
            7..=10 => self.game.foundation[key.0 - 7][key.1],
            11 => self.game.waste[key.1],
            _ => self.game.deck[key.1],
        }
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:       30000",
    "*showFPS:     False",
    "*speed:       50",
    "*cameraSpeed: 50",
    "*camera_speed: 50",
    "*draw:        3",
    "*sloppy:      True",
];

const DRAW_COUNTS: &[SelectItem] = &[
    SelectItem {
        value: "3",
        label: "Deal 3 cards to waste pile",
    },
    SelectItem {
        value: "1",
        label: "Deal 1 card to waste pile",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Animation speed", 15.0, 200.0, 1.0, 0, "50").inverted(),
    Opt::slider("camera_speed", "Camera speed", 10.0, 100.0, 1.0, 0, "50"),
    Opt::select("draw", "Deal", DRAW_COUNTS, "3"),
    Opt::boolean("sloppy", "Sloppy card placement", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "klondike",
    label: "Klondike",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Joshua Timmons",
        year: "2024",
        video: Some("https://www.youtube.com/watch?v=hPpRD51q91s"),
        blurb: "A game of Klondike solitaire, playing itself, with the camera \
                drifting around the table.",
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

    fn table() -> Board {
        Board {
            foundation_x: [0.0; 4],
            foundation_y: [0.0; 4],
            tableau_x: [0.0; 7],
            tableau_y: [0.0; 7],
            waste_x: 0.0,
            waste_y: 0.0,
            deck_x: 0.0,
            deck_y: 0.0,
            tick: 0.0,
            animation_ticks: 50.0,
            draw_count: 3,
            sloppy: false,
        }
    }

    /// A freshly dealt game. The textures need a `Gl` and nothing here
    /// looks at them, so this builds the table and the pack directly.
    fn dealt() -> (Board, Game) {
        let mut k = Klondike {
            trackball: Trackball::new(),
            board: table(),
            game: Game::default(),
            camera_speed: 50.0,
            camera_phase: 0.0,
            tick: 0.0,
            universe_tick: 0.0,
            final_animation: 0.0,
            redeal: false,
            fronts: [0; 52],
            back: 0,
        };
        k.place(320, 240);
        k.new_deck();
        k.deal();
        k.animate_deal();
        (k.board, k.game)
    }

    #[test]
    fn a_deal_puts_twenty_eight_cards_out_and_leaves_twenty_four() {
        let (_, g) = dealt();
        let out: usize = g.tableau_size.iter().sum();
        assert_eq!(out, 28, "one to seven cards in seven columns");
        assert_eq!(g.tableau_size, [1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(g.deck_size(), 24);
        assert_eq!(g.waste_size, 0);
        assert_eq!(g.foundation_size, [0; 4]);

        // Only the foot of each column is face up.
        for i in 0..7 {
            for j in 0..g.tableau_size[i] {
                assert_eq!(
                    g.tableau[i][j].face_up,
                    j == g.tableau_size[i] - 1,
                    "column {i} card {j}"
                );
            }
        }
    }

    #[test]
    fn the_pack_holds_every_card_once() {
        let (_, g) = dealt();
        let mut seen = vec![false; 52];
        let mut count = |c: Card| {
            assert!(
                (1..=13).contains(&c.rank) && c.suit < 4,
                "{}, {}",
                c.suit,
                c.rank
            );
            let i = c.suit as usize * 13 + c.rank as usize - 1;
            assert!(!seen[i], "two of suit {} rank {}", c.suit, c.rank);
            seen[i] = true;
        };
        for i in 0..7 {
            for j in 0..g.tableau_size[i] {
                count(g.tableau[i][j]);
            }
        }
        for j in 0..g.deck_size() {
            count(g.deck[j]);
        }
        assert!(seen.iter().all(|&b| b), "not every card was dealt");
    }

    #[test]
    fn every_move_it_makes_is_a_legal_one_and_the_game_ends() {
        // Play a whole game out, checking after each move that no foundation
        // has gone out of order, that no column has a card on one it may not
        // sit on, and that no card has gone missing.
        let (b, mut g) = dealt();
        let mut moves = 0;
        let mut stuck = false;
        for _ in 0..20000 {
            let Some(next) = next_move(&b, &g) else {
                stuck = true;
                break;
            };
            g = next;
            moves += 1;

            for s in 0..4usize {
                for j in 0..g.foundation_size[s] {
                    assert_eq!(g.foundation[s][j].suit as usize, s, "wrong suit on a pile");
                    assert_eq!(g.foundation[s][j].rank as usize, j + 1, "out of order");
                }
            }
            for i in 0..7 {
                for j in 1..g.tableau_size[i] {
                    let (under, over) = (g.tableau[i][j - 1], g.tableau[i][j]);
                    if under.face_up && over.face_up {
                        assert_eq!(under.rank, over.rank + 1, "column {i} runs wrong");
                        assert_ne!(under.suit % 2, over.suit % 2, "column {i} is one colour");
                    }
                }
            }
            let total: usize = g.tableau_size.iter().sum::<usize>()
                + g.foundation_size.iter().sum::<usize>()
                + g.waste_size
                + g.deck_size();
            assert_eq!(total, 52, "a card went missing after {moves} moves");
        }
        assert!(moves > 20, "the game stopped after {moves} moves");
        // A game that could never run out of moves would never deal again.
        assert!(stuck, "the game never ran out of moves");
    }

    #[test]
    fn every_card_on_the_table_is_drawn() {
        let mut r = start(StartArgs::new(320, 240, "", 20260812));
        r.step();
        let f = r.frame();
        // Fifty-two quads, each its own call because each has its own matrix
        // and its own picture.
        assert_eq!(f.batches.len(), 52);
        for b in &f.batches {
            assert_eq!(b.count, 6, "a card is two triangles");
            assert!(b.texture.is_some(), "a card without a picture");
        }
    }
}
