//! Port of `hacks/glx/endgame.c`.
//!
//! ```text
//! endgame.c
//! plays through a chess game ending.  enjoy.
//!
//! version 1.0 - June 6, 2002
//!
//! Copyright (C) 2002-2008 Blair Tennessy (tennessb@unbc.ca)
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
//! Thirty-four famous games, played through a move at a time on a board that
//! reflects, with the players and the year named in the corner. Nothing here
//! plays chess: each game is a list of moves in coordinate notation and a
//! starting position in FEN, and the saver's whole job is to work out what
//! that move means in pieces and to slide the right one there.
//!
//! Most of the reading of a move is bookkeeping. A capture is a move onto an
//! occupied square, unless it is a pawn moving sideways onto an empty one, in
//! which case it is en passant and the piece taken is beside the destination
//! rather than on it. Castling is spotted by the king's own move, `e1g1` and
//! its three siblings, and starts a second move for the rook at the same time,
//! which is why two moves can be in flight at once.
//!
//! Pieces mostly slide. A knight hops instead, but only when something is in
//! its way: the two squares it passes over are checked, and if they are empty
//! it slides like everything else, since nothing would have been in the way to
//! justify the leap. A castling rook swings out in an arc so that it reads as
//! going round the king rather than through him. A promoting pawn fades out
//! and its replacement fades in over the same move.
//!
//! Two of the three passes over the pieces are tricks with the stencil buffer.
//! The reflection is masked to the board's tiles, the same way `queens` does
//! it. The shadows are the other way round: every piece is flattened onto the
//! board through a projection matrix built from the light's position, marked
//! into the stencil with `GL_INCR` rather than drawn, and then one dark quad
//! is washed over the whole board wherever the mark is not zero. Drawing the
//! flattened pieces directly would darken the board twice where two shadows
//! crossed; marking them and washing once gives their union.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::chessmodels::Piece;
use crate::runtime::easing::{Ease, ease};
use crate::runtime::gl::{Blend, Mat4, Shape, Stencil, StencilFunc, StencilOp};
use crate::runtime::texfont::TexFont;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, random, random_below,
    screenhack_event_helper,
};

use super::endgame_games::{GAMES, Game};

const BOARDSIZE: usize = 8;
const WHITES: usize = 5;
const CHECK_IMAGE_WIDTH: i32 = 16;
const CHECK_IMAGE_HEIGHT: i32 = 16;
const CONCURRENT_MOVES: usize = 2;
const TICKS_BET_MOVES: i32 = 50;
const FADE_FACTOR: i32 = 2;
const END_FACTOR: i32 = 5;
const NUM_STEPS: i32 = 80;
const NUM_STEPS_F: f64 = NUM_STEPS as f64;

/// Upstream's piece numbering, which is also what a board square holds: a
/// white piece is one of these, a black one is seven more.
const PIECES: i32 = 7;
const NONE: i32 = 0;
const KING: i32 = 1;
const QUEEN: i32 = 2;
const BISHOP: i32 = 3;
const KNIGHT: i32 = 4;
const ROOK: i32 = 5;
const PAWN: i32 = 6;
const BKING: i32 = 8;
const BQUEEN: i32 = 9;
const BBISHOP: i32 = 10;
const BKNIGHT: i32 = 11;
const BROOK: i32 = 12;
const BPAWN: i32 = 13;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    ChoseGame,
    FadeIn,
    DoMove,
    WaitForNextMove,
    WaitForNextGame,
    FadeOut,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Color {
    White,
    Black,
}

const MATERIAL_SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.3];

/* i prefer silvertip */
const WHITES_COLORS: [[f32; 3]; WHITES] = [
    [1.0, 0.55, 0.1],
    [0.8, 0.52, 0.8],
    [0.43, 0.54, 0.76],
    [0.7, 0.7, 0.7],
    [0.35, 0.60, 0.35],
];

const DIFFUSE2: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const SHININESS: f32 = 60.0;
const SPECULAR: [f32; 4] = [0.4, 0.4, 0.4, 1.0];

#[derive(Clone, Copy)]
struct MoveState {
    /// Is a piece moving.
    active: bool,
    /// The moving piece.
    mpiece: i32,
    /// Piece taken by this move, if any.
    tpiece: i32,
    /// Promotion piece.
    promotion: i32,
    /// Origin case.
    from: [i32; 2],
    /// Destination case.
    to: [i32; 2],
    /// Is this an en passant capture.
    en_passant: [i32; 2],
    /// White or black move.
    color: Color,
    /// Delta x.
    dx: f64,
    /// Delta z.
    dz: f64,
    /// Are we castling.
    castling: bool,
}

impl MoveState {
    const fn new() -> MoveState {
        MoveState {
            active: false,
            mpiece: NONE,
            tpiece: NONE,
            promotion: NONE,
            from: [0, 0],
            to: [0, 0],
            en_passant: [-1, -1],
            color: Color::White,
            dx: 0.0,
            dz: 0.0,
            castling: false,
        }
    }
}

struct Endgame {
    trackball: Trackball,
    game: &'static Game,
    /// The game's own description, then the move being played, on their own
    /// lines. Upstream keeps this as one fixed buffer it patches in place.
    game_desc: String,
    oldwhite: i32,

    /// Definition of white/black (orange/gray) colors.
    colors: [[f32; 3]; 2],

    piecetexture: u32,
    boardtexture: u32,

    board: [[i32; BOARDSIZE]; BOARDSIZE],
    /// If castling, two pieces can move at the same time.
    moves: [MoveState; CONCURRENT_MOVES],
    steps: i32,
    stage: Stage,
    mc: usize,
    ticks: i32,
    abort: bool,
    theta: f64,

    position: [f32; 4],
    position2: [f32; 4],

    modulus: f32,
    ground: [f32; 4],

    cur_game_idx: i32,

    fonts: [TexFont; 3],
    /// Upstream compiles one display list per piece; here they are meshes,
    /// indexed by the piece number less one.
    pieces: [Piece; 6],
    labels: bool,
    rotate: bool,
    reflections: bool,
    shadows: bool,
    /// `GL_CONSTANT_ATTENUATION` of both lights, which is upstream's fade: a
    /// hundred is dark and one is fully lit. Its own note says the trick does
    /// nothing in `queens`; here it is the whole of how a game arrives and
    /// leaves.
    constant_att: f32,
}

/// Upstream's `glColorMaterial (GL_FRONT, GL_DIFFUSE)` is not a form the
/// recorder has, so a colour goes into three places at once: the vertex
/// colour, which is what an unlit pass draws with, and the front diffuse,
/// which is what a lit one does, and then the back diffuse goes back to the
/// near-black upstream leaves it at. The back matters: a reflection is drawn
/// mirrored, so its outsides face away from the camera and take that colour.
fn set_color(g: &mut Gl, c: [f32; 4]) {
    g.glx.color4f(c[0], c[1], c[2], c[3]);
    g.glx.material_diffuse(c);
    g.glx.material_back_ambient_diffuse(MATERIAL_SHADOW);
}

fn get_piece_color(piece: i32) -> Color {
    if piece < BKING {
        Color::White
    } else {
        Color::Black
    }
}

/* Helpers */
fn char_to_piece(piece_char: u8) -> i32 {
    match piece_char {
        b'p' => BPAWN,
        b'P' => PAWN,
        b'q' => BQUEEN,
        b'Q' => QUEEN,
        b'k' => BKING,
        b'K' => KING,
        b'b' => BBISHOP,
        b'B' => BISHOP,
        b'n' => BKNIGHT,
        b'N' => KNIGHT,
        b'r' => BROOK,
        b'R' => ROOK,
        _ => NONE,
    }
}

fn get_digit(c: u8) -> usize {
    match c {
        b'1'..=b'8' => (c - b'0') as usize,
        _ => 0,
    }
}

fn char_to_rowcol(c: u8) -> i32 {
    match c {
        b'a' | b'8' => 0,
        b'b' | b'7' => 1,
        b'c' | b'6' => 2,
        b'd' | b'5' => 3,
        b'e' | b'4' => 4,
        b'f' | b'3' => 5,
        b'g' | b'2' => 6,
        b'h' | b'1' => 7,
        _ => -1,
    }
}

fn coord_to_case(uci: &[u8]) -> [i32; 2] {
    [char_to_rowcol(uci[1]), char_to_rowcol(uci[0])]
}

fn color_piece(piece: i32, color: Color) -> i32 {
    if piece == NONE {
        return piece;
    }
    if color == Color::White {
        piece % PIECES
    } else {
        (piece % PIECES) + PIECES
    }
}

fn ease_move(x: f64, max: f64) -> f64 {
    if max == 0.0 {
        return x;
    }
    let max = max.abs();
    if x > 0.0 {
        max * ease(Ease::InOutCubic, x / max)
    } else {
        max * -ease(Ease::InOutCubic, -x / max)
    }
}

/// Create a matrix that will project the desired shadow.
fn shadowmatrix(groundplane: [f32; 4], lightpos: [f32; 4]) -> Mat4 {
    /* find dot product between light position vector and ground plane normal */
    let dot: f32 = (0..4).map(|i| groundplane[i] * lightpos[i]).sum();

    let mut m = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            m[col * 4 + row] =
                if col == row { dot } else { 0.0 } - lightpos[row] * groundplane[col];
        }
    }
    Mat4(m)
}

impl Endgame {
    fn build_colors(&mut self) {
        /* find new white */
        let mut newwhite = self.oldwhite;
        while newwhite == self.oldwhite {
            newwhite = random_below(WHITES as i32);
        }
        self.oldwhite = newwhite;
        self.colors[0] = WHITES_COLORS[self.oldwhite as usize];
    }

    /// Build piece texture, and the board's. Both are sixteen squares of
    /// noise: the pieces get a fine grid of it, since a turned surface shows
    /// its lathe marks, and the board an even scatter.
    fn make_textures(&mut self, g: &mut Gl) {
        for piece in [true, false] {
            let mut image = Vec::with_capacity(16 * 16 * 4);
            for i in 0..CHECK_IMAGE_WIDTH {
                for j in 0..CHECK_IMAGE_HEIGHT {
                    let c = if piece {
                        if j % 2 == 0 || i % 2 == 0 {
                            240
                        } else {
                            180 + random() % 16
                        }
                    } else {
                        /* uniform noise in [180,180+50] */
                        180 + random() % 51
                    } as u8;
                    image.extend_from_slice(&[c, c, c, 255]);
                }
            }
            let id = g.glx.gen_texture();
            g.glx.bind_texture(id);
            g.glx
                .tex_image_2d(CHECK_IMAGE_WIDTH, CHECK_IMAGE_HEIGHT, image);
            if piece {
                self.piecetexture = id;
            } else {
                self.boardtexture = id;
            }
        }
    }

    /// Configure lighting.
    fn setup_lights(&self, g: &mut Gl) {
        g.glx.lighting(true);
        g.glx
            .light_position(0, self.position[0], self.position[1], self.position[2], 1.0);
        g.glx.light_diffuse(0, DIFFUSE2);
        g.glx.light_enable(0, true);

        g.glx.material_shininess(SHININESS);
        g.glx.material_specular(SPECULAR);

        g.glx.light_specular(1, DIFFUSE2);
        g.glx.light_diffuse(1, DIFFUSE2);
        g.glx.light_enable(1, true);
    }

    fn piece_mesh(&self, piece: i32) -> Option<&Piece> {
        let n = piece % PIECES;
        if n == NONE {
            None
        } else {
            self.pieces.get(n as usize - 1)
        }
    }

    /// A piece stands facing the camera except for the two that do not look
    /// the same from every side: the knight faces along the board, and the
    /// bishop's slit turns with whose side he is on.
    fn draw_turned(&self, g: &mut Gl, board_value: i32, piece: i32, side: f32, turn: bool) {
        let knight = board_value == KNIGHT;
        let bishop = piece % PIECES == BISHOP;
        if turn {
            if knight {
                g.glx.rotate(180.0, 0.0, 1.0, 0.0);
            } else if bishop {
                g.glx.rotate(90.0 * side, 0.0, 1.0, 0.0);
            }
        }
        if let Some(mesh) = self.piece_mesh(piece) {
            mesh.draw(&mut g.glx);
        }
        if turn {
            if knight {
                g.glx.rotate(180.0, 0.0, 1.0, 0.0);
            } else if bishop {
                g.glx.rotate(-90.0 * side, 0.0, 1.0, 0.0);
            }
        }
    }

    /// Draw pieces.
    fn draw_pieces_standing(&self, g: &mut Gl, shadow: bool) {
        for i in 0..BOARDSIZE {
            for j in 0..BOARDSIZE {
                let v = self.board[i][j];
                if v != NONE {
                    if shadow {
                        set_color(g, [0.0, 0.0, 0.0, 0.4]);
                        if let Some(mesh) = self.piece_mesh(v) {
                            mesh.draw(&mut g.glx);
                        }
                    } else {
                        let c = self.colors[(v / PIECES) as usize];
                        set_color(g, [c[0], c[1], c[2], 1.0]);
                        let side = if v == v % PIECES { 1.0 } else { -1.0 };
                        self.draw_turned(g, v, v, side, true);
                    }
                }
                g.glx.translate(1.0, 0.0, 0.0);
            }
            g.glx.translate(-(BOARDSIZE as f32), 0.0, 1.0);
        }
        g.glx.translate(0.0, 0.0, -(BOARDSIZE as f32));
    }

    fn are_moves_active(&self) -> bool {
        self.moves.iter().any(|m| m.active)
    }

    fn is_a_piece_taken(&self) -> bool {
        self.moves.iter().any(|m| m.tpiece != NONE)
    }

    fn init_moves(&mut self) {
        self.moves = [MoveState::new(); CONCURRENT_MOVES];
    }

    /// Draw a moving piece.
    fn draw_moving_piece(&mut self, g: &mut Gl, shadow: bool) {
        for i in 0..CONCURRENT_MOVES {
            let m = self.moves[i];
            if !m.active {
                continue;
            }
            let mut piece = m.mpiece % PIECES;
            let side = if m.color == Color::White { 1.0 } else { -1.0 };
            if piece == NONE {
                continue;
            }
            let promotion_piece = m.promotion;
            let c = self.colors[(m.mpiece / PIECES) as usize];

            g.glx.push_matrix();

            if shadow {
                set_color(g, MATERIAL_SHADOW);
            } else {
                set_color(g, [c[0], c[1], c[2], 1.0]);
            }

            if (m.mpiece == PAWN && m.to[0] == 0) || (m.mpiece == BPAWN && m.to[0] == 7) {
                // A promotion. The pawn fades out over the first half of the
                // move and whatever it becomes fades in over the second.
                g.glx.translate(
                    m.from[1] as f32 + self.steps as f32 * m.dx as f32,
                    0.0,
                    m.from[0] as f32 + self.steps as f32 * m.dz as f32,
                );
                let a = ((NUM_STEPS_F / 2.0) - self.steps as f64).abs() / (NUM_STEPS_F / 2.0);
                let rgb = if shadow {
                    MATERIAL_SHADOW
                } else {
                    [c[0], c[1], c[2], 0.0]
                };
                set_color(g, [rgb[0], rgb[1], rgb[2], a as f32]);

                piece = if self.steps < NUM_STEPS / 2 {
                    PAWN
                } else {
                    promotion_piece % PIECES
                };

                /* what a kludge */
                if self.steps == NUM_STEPS - 1 {
                    self.moves[i].mpiece = promotion_piece;
                }
            } else if m.mpiece % PIECES == KNIGHT && self.knight_is_blocked(&m) {
                /* Move by hopping. */
                let y = 1.5 * (std::f64::consts::PI * self.steps as f64 / NUM_STEPS_F).sin();
                g.glx.translate(
                    m.from[1] as f32
                        + ease_move(self.steps as f64 * m.dx, (m.to[1] - m.from[1]) as f64) as f32,
                    y as f32,
                    m.from[0] as f32
                        + ease_move(self.steps as f64 * m.dz, (m.to[0] - m.from[0]) as f64) as f32,
                );
            } else if m.mpiece % PIECES == ROOK && m.castling {
                /* Move z in an arc */
                let offset = 1.5 * (std::f64::consts::PI * self.steps as f64 / NUM_STEPS_F).sin();
                let offset = if m.color == Color::White {
                    offset
                } else {
                    -offset
                };
                g.glx.translate(
                    m.from[1] as f32
                        + ease_move(self.steps as f64 * m.dx, (m.to[1] - m.from[1]) as f64) as f32,
                    0.0,
                    m.from[0] as f32
                        + ease_move(
                            self.steps as f64 * m.dz + offset,
                            (m.to[0] - m.from[0]) as f64,
                        ) as f32,
                );
            } else {
                /* Move by sliding. */
                g.glx.translate(
                    m.from[1] as f32
                        + ease_move(self.steps as f64 * m.dx, (m.to[1] - m.from[1]) as f64) as f32,
                    0.0,
                    m.from[0] as f32
                        + ease_move(self.steps as f64 * m.dz, (m.to[0] - m.from[0]) as f64) as f32,
                );
            }

            g.glx.blend(Blend::Alpha);

            let half = self.steps >= NUM_STEPS / 2;
            let knight = m.mpiece == KNIGHT || (promotion_piece == KNIGHT && half);
            let bishop = piece == BISHOP || (promotion_piece == BISHOP && half);
            if knight {
                g.glx.rotate(180.0, 0.0, 1.0, 0.0);
            }
            if bishop {
                g.glx.rotate(90.0 * side, 0.0, 1.0, 0.0);
            }
            if let Some(mesh) = self.piece_mesh(piece) {
                mesh.draw(&mut g.glx);
            }
            if knight {
                g.glx.rotate(180.0, 0.0, 1.0, 0.0);
            }
            if bishop {
                g.glx.rotate(-90.0 * side, 0.0, 1.0, 0.0);
            }

            g.glx.material_shininess(SHININESS);
            g.glx.material_specular(SPECULAR);

            g.glx.pop_matrix();

            g.glx.blend(Blend::Off);
        }
    }

    /// If there is nothing in the path of a knight, move it by sliding, just
    /// like the other pieces. But if there are any pieces on the middle two
    /// squares in its path, the knight would intersect them, so in that case,
    /// move it in an airborne arc.
    fn knight_is_blocked(&self, m: &MoveState) -> bool {
        let mut fromx = m.from[1].min(m.to[1]);
        let mut fromy = m.from[0].min(m.to[0]);
        let mut tox = m.from[1].max(m.to[1]);
        let mut toy = m.from[0].max(m.to[0]);
        if fromx == tox - 2 {
            fromx += 1;
            tox = fromx;
        }
        if fromy == toy - 2 {
            fromy += 1;
            toy = fromy;
        }
        for i in fromy..=toy {
            for j in fromx..=tox {
                if self.board[i as usize][j as usize] != NONE {
                    return true;
                }
            }
        }
        false
    }

    /// Code to squish a taken piece.
    ///
    /// Upstream leaves out the push and pop around each move here, so the
    /// translate and the scale carry over into the next one. Only the first
    /// move can take a piece, so the second never draws anything and the leak
    /// never shows; it is kept because taking it out would change where the
    /// second move's geometry would land if one ever did.
    fn draw_take_piece(&self, g: &mut Gl, shadow: bool) {
        g.glx.blend(Blend::Alpha);

        for m in &self.moves {
            let side = if m.color == Color::Black { 1.0 } else { -1.0 };
            if !m.active {
                continue;
            }

            let c = self.colors[(m.tpiece / PIECES) as usize];
            let a = (NUM_STEPS_F - 1.6 * self.steps as f64) / NUM_STEPS_F;
            if shadow {
                set_color(
                    g,
                    [
                        MATERIAL_SHADOW[0],
                        MATERIAL_SHADOW[1],
                        MATERIAL_SHADOW[2],
                        a as f32,
                    ],
                );
            } else {
                set_color(g, [c[0], c[1], c[2], a as f32]);
            }

            if m.en_passant[0] != -1 {
                g.glx
                    .translate(m.en_passant[1] as f32, 0.0, m.en_passant[0] as f32);
            } else {
                g.glx.translate(m.to[1] as f32, 0.0, m.to[0] as f32);
            }

            if m.tpiece % PIECES == KNIGHT {
                let s = 1.0 + self.steps as f32 / NUM_STEPS as f32;
                g.glx.scale(s, 1.0, s);
            } else {
                let s = self.steps as f32 / NUM_STEPS as f32;
                g.glx
                    .scale(1.0, if 1.0 - s / 2.0 > 0.01 { 1.0 - s } else { 0.01 }, 1.0);
            }

            self.draw_turned(g, m.tpiece, m.tpiece, side, true);
        }

        g.glx.blend(Blend::Off);
    }

    /// Draw board.
    fn draw_board(&self, g: &mut Gl) {
        // Upstream colours each square inside one block and lets
        // GL_COLOR_MATERIAL carry it; there are only two colours, so the
        // squares go out a colour at a time instead.
        for par in 0..2 {
            let c = self.colors[par];
            set_color(g, [c[0], c[1], c[2], 0.65]);
            g.glx.begin(Shape::Quads);
            for i in 0..BOARDSIZE {
                for j in 0..BOARDSIZE {
                    if (i + j) % 2 != par {
                        continue;
                    }
                    let m = self.modulus;
                    let even = (i + j) % 2 == 0;
                    let (i, j) = (i as f32, j as f32);
                    let ma1 = if even { m * i } else { 0.0 };
                    let mb1 = if even { m * j } else { 0.0 };
                    let ma2 = if even { m * (i + 1.0) } else { 0.01 };
                    let mb2 = if even { m * (j + 1.0) } else { 0.01 };

                    g.glx.normal3f(0.0, 1.0, 0.0);
                    g.glx.tex_coord2f(ma1, mb2);
                    g.glx.vertex3f(i, 0.0, j + 1.0);
                    g.glx.tex_coord2f(ma2, mb2);
                    g.glx.vertex3f(i + 1.0, 0.0, j + 1.0);
                    g.glx.tex_coord2f(ma2, mb1);
                    g.glx.vertex3f(i + 1.0, 0.0, j);
                    g.glx.tex_coord2f(ma1, mb1);
                    g.glx.vertex3f(i, 0.0, j);
                }
            }
            g.glx.end();
        }

        {
            let off = 0.01;
            let w = BOARDSIZE as f32;
            let h = 0.1;

            /* Give the board a slight lip. */
            /* #### oops, normals are wrong here, but you can't tell */

            set_color(g, [0.3, 0.3, 0.3, 1.0]);
            box_sides(g, w, h);

            /* Fill in the underside of the board with an invisible black box
               to hide the reflections that are not on tiles.  Probably there's
               a way to do this with stencils instead.
            */
            let w = w - off * 2.0;
            let h = 5.0;

            g.glx.push_matrix();
            g.glx.translate(off, 0.0, off);
            g.glx.lighting(false);
            g.glx.color3f(0.0, 0.0, 0.0);
            box_sides(g, w, h);
            g.glx.pop_matrix();
            g.glx.lighting(true);
        }
    }

    /// The three passes over the pieces that draw them for real: standing,
    /// moving, and being taken.
    fn draw_pieces(&mut self, g: &mut Gl) {
        g.glx.texturing(true);
        g.glx.bind_texture(self.piecetexture);
        set_color(g, [0.5, 0.5, 0.5, 1.0]);

        self.draw_pieces_standing(g, false);
        if self.are_moves_active() {
            self.draw_moving_piece(g, false);
        }
        if self.is_a_piece_taken() {
            self.draw_take_piece(g, false);
        }
        g.glx.texturing(false);
    }

    /// Mark every piece's silhouette into the stencil, then wash one dark
    /// quad over the board wherever the mark took.
    fn draw_shadow_pieces(&mut self, g: &mut Gl) {
        /* use the stencil */
        g.glx.lighting(false);
        g.glx.depth_test(false);
        g.glx.texturing(false);
        g.glx.blend(Blend::Off);

        g.glx.clear_stencil();
        g.glx.color_mask(false);
        g.glx.stencil(Some(Stencil {
            func: StencilFunc::Always,
            reference: 1,
            pass: StencilOp::Incr,
        }));

        g.glx.push_matrix();
        g.glx.translate(0.0, 0.001, 0.0);

        /* draw the pieces */
        self.draw_pieces_standing(g, true);
        if self.are_moves_active() {
            self.draw_moving_piece(g, self.shadows);
        }
        if self.is_a_piece_taken() {
            self.draw_take_piece(g, self.shadows);
        }

        g.glx.pop_matrix();

        /* turn on drawing into colour buffer */
        g.glx.color_mask(true);

        /* now draw the union of the shadows */
        g.glx.stencil(Some(Stencil {
            func: StencilFunc::NotEqual,
            reference: 0,
            pass: StencilOp::Replace,
        }));

        g.glx.blend(Blend::Alpha);
        g.glx.color4f(
            MATERIAL_SHADOW[0],
            MATERIAL_SHADOW[1],
            MATERIAL_SHADOW[2],
            MATERIAL_SHADOW[3],
        );

        /* draw the board generously to fill the shadows */
        let w = BOARDSIZE as f32;
        g.glx.begin(Shape::Quads);
        g.glx.vertex3f(-1.0, 0.0, -1.0);
        g.glx.vertex3f(-1.0, 0.0, w + 1.0);
        g.glx.vertex3f(1.0 + w, 0.0, w + 1.0);
        g.glx.vertex3f(1.0 + w, 0.0, -1.0);
        g.glx.end();

        g.glx.stencil(None);

        /* "pop" attributes */
        g.glx.lighting(true);
        g.glx.cull_face(true);
    }

    /// Reflectionboard.
    fn draw_reflections(&mut self, g: &mut Gl) {
        g.glx.stencil(Some(Stencil {
            func: StencilFunc::Always,
            reference: 1,
            pass: StencilOp::Replace,
        }));
        g.glx.color_mask(false);
        g.glx.cull_face(false);

        g.glx.depth_test(false);
        g.glx.begin(Shape::Quads);

        /* only draw white squares */
        for i in 0..BOARDSIZE {
            for j in (((BOARDSIZE + i) % 2)..BOARDSIZE).step_by(2) {
                let (i, j) = (i as f32, j as f32);
                g.glx.vertex3f(i, 0.0, j + 1.0);
                g.glx.vertex3f(i + 1.0, 0.0, j + 1.0);
                g.glx.vertex3f(i + 1.0, 0.0, j);
                g.glx.vertex3f(i, 0.0, j);
            }
        }
        g.glx.end();
        g.glx.depth_test(true);

        g.glx.color_mask(true);
        g.glx.stencil(Some(Stencil {
            func: StencilFunc::Equal,
            reference: 1,
            pass: StencilOp::Keep,
        }));

        g.glx.push_matrix();
        g.glx.scale(1.0, -1.0, 1.0);
        g.glx.translate(0.5, 0.0, 0.5);

        g.glx
            .light_position(0, self.position[0], self.position[1], self.position[2], 1.0);
        self.draw_pieces(g);
        g.glx.pop_matrix();

        g.glx.stencil(None);
        g.glx
            .light_position(0, self.position[0], self.position[1], self.position[2], 1.0);

        g.glx.cull_face(true);
    }

    /// Draws the scene.
    fn display(&mut self, g: &mut Gl) {
        g.glx.clear();

        g.glx.load_identity();

        /* setup perspective */
        g.glx.translate(0.0, 0.0, -1.5 * BOARDSIZE as f32);
        g.glx.rotate(30.0, 1.0, 0.0, 0.0);
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        if self.rotate {
            g.glx.rotate(self.theta as f32 * 100.0, 0.0, 1.0, 0.0);
        }
        g.glx
            .translate(-0.5 * BOARDSIZE as f32, 0.0, -0.5 * BOARDSIZE as f32);

        /* this is the lone light that the shadow matrix is generated from */
        self.position[0] = 1.0;
        self.position[2] = 1.0;
        self.position[1] = 16.0;

        let a = self.theta * 100.0 * std::f64::consts::PI / 180.0;
        self.position2[0] = (4.0 + 8.0 * -a.sin()) as f32;
        self.position2[2] = (4.0 + 8.0 * a.cos()) as f32;

        g.glx.lighting(true);
        g.glx
            .light_position(0, self.position[0], self.position[1], self.position[2], 1.0);
        g.glx.light_position(
            1,
            self.position2[0],
            self.position2[1],
            self.position2[2],
            1.0,
        );
        g.glx.light_enable(0, true);

        /* draw board, pieces */
        if self.reflections {
            self.draw_reflections(g);
        }
        g.glx.blend(Blend::Alpha);

        g.glx.texturing(true);
        g.glx.bind_texture(self.boardtexture);
        self.draw_board(g);
        g.glx.texturing(false);

        if self.shadows {
            /* render shadows */
            let m = shadowmatrix(self.ground, self.position);

            g.glx.material_diffuse(MATERIAL_SHADOW);
            g.glx.blend(Blend::Alpha);
            g.glx.lighting(false);
            g.glx.depth_test(false);

            /* display shadow */
            g.glx.push_matrix();
            g.glx.translate(0.0, 0.001, 0.0);
            g.glx.mult_matrix(m);
            g.glx.translate(0.5, 0.01, 0.5);
            self.draw_shadow_pieces(g);
            g.glx.pop_matrix();

            g.glx.lighting(true);
            g.glx.blend(Blend::Off);
            g.glx.depth_test(true);
        }

        g.glx.blend(Blend::Off);

        g.glx.translate(0.5, 0.0, 0.5);
        self.draw_pieces(g);

        g.glx.lighting(false);

        if !self.trackball.button_down() {
            self.theta += 0.002;
        }
    }

    fn apply_fen(&mut self) {
        self.board = [[NONE; BOARDSIZE]; BOARDSIZE];
        for (row_num, row) in self.game.fen.split('/').take(BOARDSIZE).enumerate() {
            let mut col_num = 0;
            for &c in row.as_bytes() {
                let d = get_digit(c);
                for _ in 0..d {
                    if col_num < BOARDSIZE {
                        self.board[row_num][col_num] = NONE;
                    }
                    col_num += 1;
                }
                let p = char_to_piece(c);
                if p != NONE {
                    if col_num < BOARDSIZE {
                        self.board[row_num][col_num] = p;
                    }
                    col_num += 1;
                }
            }
        }
    }

    fn setup_move(&mut self, uci: &str, move_index: usize) {
        let uci = uci.as_bytes();
        let mut m = MoveState::new();
        m.from = coord_to_case(uci);
        m.to = coord_to_case(&uci[2..]);
        m.mpiece = self.board[m.from[0] as usize][m.from[1] as usize];
        m.color = get_piece_color(m.mpiece);
        if uci.len() == 5 {
            m.promotion = color_piece(char_to_piece(uci[4]), m.color);
        }
        m.dz = (m.to[0] - m.from[0]) as f64 / NUM_STEPS_F;
        m.dx = (m.to[1] - m.from[1]) as f64 / NUM_STEPS_F;
        /* Remove moving piece from board */
        self.board[m.from[0] as usize][m.from[1] as usize] = NONE;

        m.tpiece = self.board[m.to[0] as usize][m.to[1] as usize];
        /* Capture ? */
        if m.tpiece != NONE {
            /* Remove captured piece from board */
            self.board[m.to[0] as usize][m.to[1] as usize] = NONE;
            /* Not en passant */
            m.en_passant = [-1, -1];
        } else {
            /* Destination case is empty. Is this an en passant capture ? */
            /* White en passant ? */
            if m.mpiece == PAWN && m.from[0] == 3 && m.from[1] != m.to[1] {
                m.tpiece = BPAWN;
                m.en_passant = [3, m.to[1]];
            }
            /* Black en passant ? */
            else if m.mpiece == BPAWN && m.from[0] == 4 && m.from[1] != m.to[1] {
                m.tpiece = PAWN;
                m.en_passant = [4, m.to[1]];
            }
            if m.en_passant[0] != -1 {
                /* Remove captured piece */
                self.board[m.en_passant[0] as usize][m.en_passant[1] as usize] = NONE;
            }
        }
        m.active = true;
        self.moves[move_index] = m;
    }

    fn setup_moves(&mut self, uci: &str) {
        self.setup_move(uci, 0); /* General case */
        let rook = match &uci[..4] {
            "e1g1" => Some("h1f1"), /* White king castling */
            "e1c1" => Some("a1d1"), /* White queen castling */
            "e8g8" => Some("h8f8"), /* Black king castling */
            "e8c8" => Some("a8d8"), /* Black queen castling */
            _ => None,
        };
        if let Some(rook) = rook {
            /* rook will move at the same time */
            self.setup_move(rook, 1);
            self.moves[1].castling = true;
        }
    }

    fn set_description(&mut self) {
        if !self.labels {
            return;
        }
        self.game_desc = format!("{}\n", self.game.desc);
    }

    fn manage_labels(&mut self, g: &mut Gl) {
        if !matches!(
            self.stage,
            Stage::DoMove | Stage::FadeIn | Stage::WaitForNextMove | Stage::WaitForNextGame
        ) || !self.labels
        {
            return;
        }

        if self.stage != Stage::FadeIn {
            // Upstream indexes one before the first move when there is no
            // previous one to name, and gets an empty string out of the
            // bytes it lands in; here the line is simply left blank.
            let san = if self.stage == Stage::DoMove {
                self.game.moves.get(self.mc)
            } else if self.mc > 0 {
                self.game.moves.get(self.mc - 1)
            } else {
                None
            };
            let head = self.game_desc.rsplit_once('\n').map_or("", |(a, _)| a);
            self.game_desc = format!("{head}\n{}", san.map_or("", |m| m.1));
        }

        let f = if g.width() >= 500 && g.height() >= 375 {
            0
        } else if g.width() >= 350 && g.height() >= 260 {
            1
        } else {
            2
        };
        let (w, h) = (g.width(), g.height());
        self.fonts[f].print_label(&mut g.glx, &self.game_desc, w, h, 1, [0.8, 0.8, 0.0, 1.0]);
    }
}

/// The four walls and the floor of a box `w` square and `h` deep hanging below
/// the origin, which is both the board's lip and the black box under it.
fn box_sides(g: &mut Gl, w: f32, h: f32) {
    g.glx.begin(Shape::Quads);
    for [a, b, c, d] in [
        [[0.0, 0.0, 0.0], [0.0, -h, 0.0], [0.0, -h, w], [0.0, 0.0, w]],
        [[0.0, 0.0, w], [0.0, -h, w], [w, -h, w], [w, 0.0, w]],
        [[w, 0.0, w], [w, -h, w], [w, -h, 0.0], [w, 0.0, 0.0]],
        [[w, 0.0, 0.0], [w, -h, 0.0], [0.0, -h, 0.0], [0.0, 0.0, 0.0]],
        [[0.0, -h, 0.0], [w, -h, 0.0], [w, -h, w], [0.0, -h, w]],
    ] {
        for v in [a, b, c, d] {
            g.glx.vertex3f(v[0], v[1], v[2]);
        }
    }
    g.glx.end();
}

impl Hack3d for Endgame {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        match self.stage {
            Stage::ChoseGame => {
                let mut new_game_idx = self.cur_game_idx;
                while new_game_idx == self.cur_game_idx {
                    new_game_idx = random_below(GAMES.len() as i32);
                }

                /* mod the mod */
                self.modulus = 0.6 + (random() % 20) as f32 / 10.0;

                self.cur_game_idx = new_game_idx;
                self.game = &GAMES[self.cur_game_idx as usize];

                self.stage = Stage::FadeIn;
                self.ticks = 1;
                self.mc = 0;
                self.build_colors();
                self.apply_fen();
                self.set_description();
                self.init_moves();
                self.constant_att = (FADE_FACTOR * TICKS_BET_MOVES) as f32;
                self.abort = false;
            }
            Stage::FadeIn => {
                self.constant_att = (FADE_FACTOR * TICKS_BET_MOVES) as f32 / self.ticks as f32;
                if self.abort {
                    self.abort = false;
                    self.stage = Stage::FadeOut;
                } else {
                    self.ticks += 1;
                    if self.ticks - 1 == FADE_FACTOR * TICKS_BET_MOVES {
                        self.stage = Stage::WaitForNextMove;
                    }
                }
            }
            Stage::FadeOut => {
                self.constant_att = (FADE_FACTOR * TICKS_BET_MOVES) as f32 / self.ticks as f32;
                self.ticks -= 1;
                if self.ticks + 1 == 0 {
                    self.stage = Stage::ChoseGame;
                }
                self.abort = false;
            }
            Stage::DoMove => {
                if self.abort {
                    self.abort = false;
                    self.ticks = FADE_FACTOR * TICKS_BET_MOVES;
                    self.stage = Stage::FadeOut;
                } else if self.are_moves_active() {
                    self.steps += 1;
                    if self.steps == NUM_STEPS {
                        /* Update board with moved piece(s) */
                        for m in self.moves {
                            if m.active {
                                self.board[m.to[0] as usize][m.to[1] as usize] = m.mpiece;
                            }
                        }
                        /* Reinit stuff for next move */
                        self.ticks = 0;
                        self.steps = 0;
                        self.init_moves();
                        self.mc += 1;
                        if self.mc == self.game.moves.len() {
                            self.ticks = 0;
                            self.stage = Stage::WaitForNextGame;
                            self.mc = 0;
                        } else {
                            self.stage = Stage::WaitForNextMove;
                        }
                    }
                }
            }
            Stage::WaitForNextMove => {
                if self.abort {
                    self.abort = false;
                    self.ticks = FADE_FACTOR * TICKS_BET_MOVES;
                    self.stage = Stage::FadeOut;
                } else {
                    self.ticks += 1;
                    if self.ticks > TICKS_BET_MOVES {
                        /* Wait before processing next move */
                        let uci = self.game.moves[self.mc].0;
                        self.setup_moves(uci);
                        self.steps = 0;
                        self.ticks = 0;
                        self.stage = Stage::DoMove;
                    }
                }
            }
            Stage::WaitForNextGame => {
                /* Wait before moving on */
                if self.abort {
                    self.abort = false;
                    self.ticks = FADE_FACTOR * TICKS_BET_MOVES;
                    self.stage = Stage::FadeOut;
                } else {
                    self.ticks += 1;
                    if self.ticks > END_FACTOR * TICKS_BET_MOVES {
                        self.ticks = FADE_FACTOR * TICKS_BET_MOVES;
                        self.stage = Stage::FadeOut;
                    }
                }
            }
        }
        g.glx.light_attenuation(0, self.constant_att, 0.14, 0.0);
        g.glx.light_attenuation(1, self.constant_att, 0.14, 0.0);

        self.display(g);
        self.manage_labels(g);

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let mut height = height.max(1);
        let mut h = height as f32 / width as f32;
        let mut y = 0;

        if width > height * 5 {
            /* tiny window: show middle */
            height = width * 9 / 16;
            y = -height / 2;
            h = height as f32 / width as f32;
        }

        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(45.0, 1.0 / h, 2.0, 30.0);
        g.glx.matrix_mode_modelview();
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            true
        } else if screenhack_event_helper(event) {
            self.abort = true;
            true
        } else {
            false
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let classic = g.res.bool("classic");
    let mut st = Endgame {
        trackball: Trackball::new(),
        game: &GAMES[0],
        game_desc: String::new(),
        oldwhite: -1,
        colors: [[1.0, 0.5, 0.0], [0.3, 0.3, 0.3]],
        piecetexture: 0,
        boardtexture: 0,
        board: [[NONE; BOARDSIZE]; BOARDSIZE],
        moves: [MoveState::new(); CONCURRENT_MOVES],
        steps: 0,
        stage: Stage::ChoseGame,
        mc: 0,
        ticks: 0,
        abort: false,
        theta: 0.0,
        position: [0.0, 24.0, 2.0, 1.0],
        position2: [5.0, 5.0, 5.0, 1.0],
        modulus: 1.4,
        ground: [0.0, 1.0, 0.0, -0.00001],
        cur_game_idx: -1,
        fonts: [
            TexFont::load(&mut g.glx, "sans-serif 18"),
            TexFont::load(&mut g.glx, "sans-serif 12"),
            TexFont::load(&mut g.glx, "sans-serif 8"),
        ],
        pieces: Piece::set(classic),
        labels: g.res.bool("labels"),
        rotate: g.res.bool("rotate"),
        reflections: g.res.bool("reflections"),
        shadows: g.res.bool("shadows"),
        constant_att: (FADE_FACTOR * TICKS_BET_MOVES) as f32,
    };

    st.make_textures(g);
    st.setup_lights(g);
    g.glx.depth_func(crate::runtime::gl::DepthFunc::LessEqual);
    g.glx.cull_face(true);
    g.glx.depth_test(true);

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:      20000",
    "*showFPS:    False",
    "*wireframe:  False",
    "*titleFont:  sans-serif 18",
    "*titleFont2: sans-serif 12",
    "*titleFont3: sans-serif 8",
    "*rotate:      True",
    "*reflections: True",
    "*shadows:     True",
    "*smooth:      True",
    "*classic:     False",
    "*labels:      True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::boolean("classic", "Low resolution chess pieces", "false"),
    Opt::boolean("labels", "Game and moves description", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "endgame",
    label: "Endgame",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Blair Tennessy and Jamie Zawinski",
        year: "2002",
        video: Some("https://www.youtube.com/watch?v=QfglC_lvUTA"),
        blurb: "Black slips out of three mating nets, but the fourth one holds him tight! \
                A brilliant composition!",
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

    /// A board back in FEN, so a played-out game can be checked against what
    /// the moves say it should be.
    fn to_fen(board: &[[i32; BOARDSIZE]; BOARDSIZE]) -> String {
        let name = |p: i32| {
            let c = b"  kqbnrp"[(p % PIECES) as usize + 1] as char;
            if p < BKING { c.to_ascii_uppercase() } else { c }
        };
        let mut s = String::new();
        for (n, row) in board.iter().enumerate() {
            if n > 0 {
                s.push('/');
            }
            let mut gap = 0;
            for &p in row {
                if p == NONE {
                    gap += 1;
                    continue;
                }
                if gap > 0 {
                    s.push_str(&gap.to_string());
                    gap = 0;
                }
                s.push(name(p));
            }
            if gap > 0 {
                s.push_str(&gap.to_string());
            }
        }
        s
    }

    fn state() -> Endgame {
        let mut g = Gl::for_test(640, 480);
        let mut st = Endgame {
            trackball: Trackball::new(),
            game: &GAMES[0],
            game_desc: String::new(),
            oldwhite: -1,
            colors: [[1.0, 0.5, 0.0], [0.3, 0.3, 0.3]],
            piecetexture: 0,
            boardtexture: 0,
            board: [[NONE; BOARDSIZE]; BOARDSIZE],
            moves: [MoveState::new(); CONCURRENT_MOVES],
            steps: 0,
            stage: Stage::ChoseGame,
            mc: 0,
            ticks: 0,
            abort: false,
            theta: 0.0,
            position: [0.0, 24.0, 2.0, 1.0],
            position2: [5.0, 5.0, 5.0, 1.0],
            modulus: 1.4,
            ground: [0.0, 1.0, 0.0, -0.00001],
            cur_game_idx: -1,
            fonts: [
                TexFont::load(&mut g.glx, "sans-serif 18"),
                TexFont::load(&mut g.glx, "sans-serif 12"),
                TexFont::load(&mut g.glx, "sans-serif 8"),
            ],
            pieces: Piece::set(false),
            labels: true,
            rotate: true,
            reflections: true,
            shadows: true,
            constant_att: (FADE_FACTOR * TICKS_BET_MOVES) as f32,
        };
        st.apply_fen();
        st
    }

    #[test]
    fn a_fen_lays_the_pieces_out_again() {
        let mut st = state();
        for game in GAMES {
            st.game = game;
            st.apply_fen();
            assert_eq!(to_fen(&st.board), game.fen, "{}", game.desc);
            // Both sides have a king, and nobody has more than sixteen men.
            let count = |lo, hi| {
                st.board
                    .iter()
                    .flatten()
                    .filter(|&&p| p >= lo && p <= hi)
                    .count()
            };
            assert_eq!(count(KING, KING), 1, "{}", game.desc);
            assert_eq!(count(BKING, BKING), 1, "{}", game.desc);
            assert!(count(KING, PAWN) <= 16 && count(BKING, BPAWN) <= 16);
        }
    }

    #[test]
    fn every_game_plays_to_its_last_move() {
        let mut st = state();
        for game in GAMES {
            st.game = game;
            st.apply_fen();
            let mut men = st.board.iter().flatten().filter(|&&p| p != NONE).count();
            for (n, (uci, _)) in game.moves.iter().enumerate() {
                st.init_moves();
                st.setup_moves(uci);
                let m = st.moves[0];
                assert!(
                    m.mpiece != NONE,
                    "{}: move {n} {uci} lifts nothing off {:?}",
                    game.desc,
                    m.from
                );
                // Whoever moves alternates, and a capture takes exactly one
                // man off the board.
                let expect = if n % 2 == 0 {
                    Color::White
                } else {
                    Color::Black
                };
                assert!(
                    m.color == expect,
                    "{}: move {n} {uci} out of turn",
                    game.desc
                );
                if m.tpiece != NONE {
                    men -= 1;
                    assert!(
                        get_piece_color(m.tpiece) != m.color,
                        "{}: move {n} {uci} takes its own",
                        game.desc
                    );
                }
                for m in st.moves {
                    if m.active {
                        st.board[m.to[0] as usize][m.to[1] as usize] = if m.promotion != NONE {
                            m.promotion
                        } else {
                            m.mpiece
                        };
                    }
                }
                assert_eq!(
                    st.board.iter().flatten().filter(|&&p| p != NONE).count(),
                    men,
                    "{}: move {n} {uci}",
                    game.desc
                );
            }
        }
    }

    #[test]
    fn the_shadow_is_the_union_of_the_pieces() {
        crate::runtime::ya_rand_init(20260812);
        let mut g = Gl::for_test(640, 480);
        let mut st = state();
        st.stage = Stage::WaitForNextMove;
        st.ticks = 1;
        st.reshape(&mut g, 640, 480);
        g.glx.start_frame(640, 480);
        st.draw(&mut g);

        let batches = &g.glx.frame().batches;
        // One pass marks the silhouettes with GL_INCR, and exactly one quad
        // washes over the board where the mark is not zero.
        let marks = batches
            .iter()
            .filter(|b| b.stencil.is_some_and(|s| s.pass == StencilOp::Incr))
            .count();
        assert!(marks >= 30, "{marks} of {}", batches.len());
        let wash: Vec<_> = batches
            .iter()
            .filter(|b| b.stencil.is_some_and(|s| s.func == StencilFunc::NotEqual))
            .collect();
        assert_eq!(wash.len(), 1, "{}", wash.len());
        // One quad, which the recorder emits as two triangles.
        assert_eq!(wash[0].count, 6);
        // And the marking pass writes no colour, while the wash does.
        assert!(
            batches.iter().all(|b| b.color_mask == [true; 4]
                || b.stencil.is_some_and(|s| s.pass != StencilOp::Keep))
        );
        assert!(batches.iter().any(|b| b.clear_stencil_first));
    }

    #[test]
    fn a_game_runs_from_end_to_end() {
        crate::runtime::ya_rand_init(20260812);
        let mut g = Gl::for_test(640, 480);
        let mut st = state();
        st.reshape(&mut g, 640, 480);

        // Long enough to choose a game, fade it in, and play a few moves.
        let mut seen = Vec::new();
        for _ in 0..900 {
            g.glx.start_frame(640, 480);
            st.draw(&mut g);
            if !seen.contains(&st.stage) {
                seen.push(st.stage);
            }
            assert!(!g.glx.frame().batches.is_empty());
        }
        assert!(seen.contains(&Stage::FadeIn), "{seen:?}");
        assert!(seen.contains(&Stage::DoMove), "{seen:?}");
        assert!(st.mc > 0 || st.stage == Stage::DoMove, "{:?}", st.stage);
    }
}
