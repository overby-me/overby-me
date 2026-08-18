//! Port of `hacks/pacman.c`, `hacks/pacman_ai.c` and `hacks/pacman_level.c`.
//!
//! ```text
//! pacman --- Mr. Pacman and his ghost friends
//!
//! Copyright (c) 2002 by Edwin de Jong <mauddib@gmx.net>.
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
//! Revision History:
//! 25-Feb-2005: Added bonus dots. I am using a recursive back track algorithm
//!              to help the ghost find there way home. This also means that
//!              they do not know the shorts path.
//!              Jeremy English jhe@jeremyenglish.org
//! 15-Aug-2004: Added support for pixmap pacman.
//!              Jeremy English jhe@jeremyenglish.org
//! 11-Aug-2004: Added support for pixmap ghost.
//!              Jeremy English jhe@jeremyenglish.org
//! 13-May-2002: Added -trackmouse feature thanks to code from 'maze.c'.
//!              splitted up code into several files.  Retouched AI code,
//!              cleaned up code.
//!  3-May-2002: Added AI to pacman and ghosts, slowed down ghosts.
//! 26-Nov-2001: Random level generator added
//! 01-Nov-2000: Allocation checks
//! 04-Jun-1997: Compatible with xscreensaver
//! ```
//!
//! Nobody is playing. Pac-Man and all four ghosts are programs, and watching
//! them lose is the point.
//!
//! Half the time the level is the one from the arcade cabinet, and half the
//! time it is generated: a jail in the middle, then eleven five-by-five tiles
//! (a corridor, a corner, a T, a crossroads) laid down by recursive
//! backtracking from the jail outward, each mirrored to the other half of the
//! screen so the result is symmetric the way a real one is. Whatever is left
//! over becomes wall, and a second pass rounds the corners by looking at each
//! wall cell's eight neighbours.
//!
//! Pac-Man has four states. Eating steers by a vector summed from every dot on
//! the board, each pulling in inverse proportion to the square of its distance,
//! so a clump of dots outweighs a nearer single one. Hiding is the same sum
//! with the direction of the closest ghost struck off the list. Chasing runs at
//! them, and dying just stands there for eight frames. He also keeps the last
//! forty vectors he steered by, and if one comes round again a dozen times he
//! decides he is going in circles and moves at random for a while.
//!
//! The ghosts pick randomly, or toward him, or away when he has eaten a bonus
//! dot and they are frightened. Getting home once eaten is a recursive
//! backtrack through the maze that stores the way it came, which is why a ghost
//! goes home by a route nobody would choose.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo};
use crate::runtime::{
    About, Dpy, Fb, Gc, Opt, Pixmap, Runner, SaverDef, Screenhack, StartArgs, XEvent, png,
    random_below,
};
use std::rc::Rc;

const LEVHEIGHT: i32 = 32;
const LEVWIDTH: i32 = 40;
const TILEWIDTH: usize = 5;
const TILEHEIGHT: usize = 5;

const GHOSTS: usize = 4;
const MAXMOUTH: usize = 3;
const MAXGDIR: usize = 4;
const MAXGWAG: usize = 2;
const MAXGFLASH: usize = 2;
const MINGRIDSIZE: i32 = 4;
const MINSIZE: i32 = 3;
/// The "no position yet" marker, which is a coordinate no maze reaches.
const NOWHERE: i32 = 16383;
const MINDOTPERC: i32 = 10;

const JAILHEIGHT: i32 = 7;
const JAILWIDTH: i32 = 8;
const TRACEVECS: usize = 40;
const PAC_DEATH_FRAMES: usize = 8;
/// The most cells a ghost can have walked on its way home.
const GHOST_TRACE: usize = (LEVWIDTH * LEVHEIGHT) as usize;
const DIRVECS: usize = 4;
const NUM_BONUS_DOTS: usize = 4;

// The characters the level is made of. Two stages: the generator works in
// walls and dots, then `frmtlevel` rewrites the walls as the pieces of line
// and corner that get drawn.
const BLOCK_EMPTY: u8 = b' ';
/// A dot in the generator's alphabet, and a top-right corner in the drawn one.
const BLOCK_DOT_1: u8 = b'`';
const BLOCK_DOT_2: u8 = b'.';
const BLOCK_WALL: u8 = b'#';
const BLOCK_GHOST_ONLY: u8 = b'=';
const BLOCK_WALL_TL: u8 = b'\'';
const BLOCK_WALL_TR: u8 = b'`';
const BLOCK_WALL_BR: u8 = b',';
const BLOCK_WALL_BL: u8 = b'_';
const BLOCK_WALL_HO: u8 = b'-';
const BLOCK_WALL_VE: u8 = b'|';
const BLOCK_DOT_BONUS: u8 = b'o';

/// The standard level, without the left-right tunnel.
const STDLEVEL: [&[u8; 40]; 32] = [
    b"########################################",
    b"########################################",
    b"#######````````````##````````````#######",
    b"#######`####`#####`##`#####`####`#######",
    b"#######`####`#####`##`#####`####`#######",
    b"#######`####`#####`##`#####`####`#######",
    b"#######``````````````````````````#######",
    b"#######`####`##`########`##`####`#######",
    b"#######`####`##`########`##`####`#######",
    b"#######``````##````##````##``````#######",
    b"############`#####`##`#####`############",
    b"############`#####`##`#####`############",
    b"############`##``````````##`############",
    b"############`##`###==###`##`############",
    b"############`##`########`##`############",
    b"############````########````############",
    b"############`##`########`##`############",
    b"############`##`########`##`############",
    b"############`##``````````##`############",
    b"############`##`########`##`############",
    b"############`##`########`##`############",
    b"#######````````````##````````````#######",
    b"#######`####`#####`##`#####`####`#######",
    b"#######`####`#####`##`#####`####`#######",
    b"#######```##````````````````##```#######",
    b"#########`##`##`########`##`##`#########",
    b"#########`##`##`########`##`##`#########",
    b"#######``````##````##````##``````#######",
    b"#######`##########`##`##########`#######",
    b"#######`##########`##`##########`#######",
    b"#######``````````````````````````#######",
    b"########################################",
];

const GO_UP: u32 = 0x0001;
const GO_LEFT: u32 = 0x0002;
const GO_RIGHT: u32 = 0x0004;
const GO_DOWN: u32 = 0x0008;

/// One of the pieces the generator lays down: what it looks like, which ways it
/// leads, and which other tiles it is close enough to that trying them after it
/// failed would be a waste.
struct Tile {
    block: &'static [u8; TILEWIDTH * TILEHEIGHT],
    dirvec: [u32; 4],
    ndirs: usize,
    similar_to: u32,
}

/// ' ' is don't care, '#' is wall, '`' is clear. The middle is always clear.
const TILES: [Tile; 11] = [
    Tile {
        block: b"  #    #   ```   #    #  ",
        dirvec: [GO_LEFT, GO_RIGHT, 0, 0],
        ndirs: 2,
        similar_to: 1 << 0 | 1 << 6 | 1 << 8 | 1 << 10,
    },
    Tile {
        block: b"       `  ##`##  `       ",
        dirvec: [GO_UP, GO_DOWN, 0, 0],
        ndirs: 2,
        similar_to: 1 << 1 | 1 << 7 | 1 << 9 | 1 << 10,
    },
    Tile {
        block: b"   ####`####`` #### #### ",
        dirvec: [GO_UP, GO_RIGHT, 0, 0],
        ndirs: 2,
        similar_to: 1 << 2 | 1 << 6 | 1 << 7 | 1 << 10,
    },
    Tile {
        block: b"#### #### ##`` ##`##   ##",
        dirvec: [GO_RIGHT, GO_DOWN, 0, 0],
        ndirs: 2,
        similar_to: 1 << 3 | 1 << 7 | 1 << 8 | 1 << 10,
    },
    Tile {
        block: b"  ###  ### ``####`  ##   ",
        dirvec: [GO_LEFT, GO_DOWN, 0, 0],
        ndirs: 2,
        similar_to: 1 << 4 | 1 << 8 | 1 << 9 | 1 << 10,
    },
    Tile {
        block: b"##   ##`## ``## #### ####",
        dirvec: [GO_LEFT, GO_UP, 0, 0],
        ndirs: 2,
        similar_to: 1 << 5 | 1 << 6 | 1 << 9 | 1 << 10,
    },
    Tile {
        block: b"##`####`##````` ###  ### ",
        dirvec: [GO_LEFT, GO_UP, GO_RIGHT, 0],
        ndirs: 3,
        similar_to: 1 << 6,
    },
    Tile {
        block: b"  `####`####```##`##  `##",
        dirvec: [GO_UP, GO_RIGHT, GO_DOWN, 0],
        ndirs: 3,
        similar_to: 1 << 7,
    },
    Tile {
        block: b" ###  ### `````##`####`##",
        dirvec: [GO_LEFT, GO_RIGHT, GO_DOWN, 0],
        ndirs: 3,
        similar_to: 1 << 8,
    },
    Tile {
        block: b"##`  ##`##```####`####`  ",
        dirvec: [GO_UP, GO_DOWN, GO_LEFT, 0],
        ndirs: 3,
        similar_to: 1 << 9,
    },
    Tile {
        block: b"##`####`##`````##`####`##",
        dirvec: [GO_UP, GO_DOWN, GO_LEFT, GO_RIGHT],
        ndirs: 4,
        similar_to: 1 << 10,
    },
];

/// Which tile to try next, weighted: the plain corridors come up more often
/// than the crossroads.
const TILEPROB: [usize; 22] = [
    0, 0, 0, 1, 1, 2, 3, 4, 5, 6, 6, 6, 7, 7, 8, 8, 8, 9, 9, 10, 10, 10,
];

const DIRS: [(i32, i32); DIRVECS] = [(-1, 0), (0, 1), (1, 0), (0, -1)];
const POS_LEFT: usize = 0;
const POS_UP: usize = 1;
const POS_RIGHT: usize = 2;
const POS_DOWN: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum GhostState {
    InBox,
    GoingOut,
    RandDir,
    Chasing,
    Hiding,
    GoingIn,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PacState {
    Eating,
    Chasing,
    Hiding,
    Random,
    Dying,
}

#[derive(Clone)]
struct Ghost {
    col: i32,
    row: i32,
    lastbox: i32,
    nextcol: i32,
    nextrow: i32,
    dead: bool,
    cfactor: i32,
    rfactor: i32,
    cf: i32,
    rf: i32,
    oldcf: i32,
    oldrf: i32,
    timeleft: i32,
    aistate: GhostState,
    speed: i32,
    delta: (i32, i32),
    err: (i32, i32),
    flash_scared: bool,
    /// The way back to the jail, stored backwards by the walk that found it.
    way_home: Vec<(i32, i32)>,
    home_idx: i32,
    home_count: i32,
    /// Set when the ghost has just been eaten and has yet to work out its way
    /// home, which happens once rather than every step.
    wait_pos: bool,
}

impl Default for Ghost {
    fn default() -> Self {
        Ghost {
            col: 0,
            row: 0,
            lastbox: NOWHERE,
            nextcol: NOWHERE,
            nextrow: NOWHERE,
            dead: false,
            cfactor: 0,
            rfactor: 0,
            cf: NOWHERE,
            rf: NOWHERE,
            oldcf: NOWHERE,
            oldrf: NOWHERE,
            timeleft: 0,
            aistate: GhostState::InBox,
            speed: 3,
            delta: (0, 0),
            err: (0, 0),
            flash_scared: false,
            way_home: vec![(-1, -1); GHOST_TRACE],
            home_idx: 0,
            home_count: 0,
            wait_pos: false,
        }
    }
}

struct Pac {
    col: i32,
    row: i32,
    lastbox: i32,
    nextcol: i32,
    nextrow: i32,
    cfactor: i32,
    rfactor: i32,
    cf: i32,
    rf: i32,
    oldcf: i32,
    oldrf: i32,
    justate: bool,
    aistate: PacState,
    /// The last forty steering vectors, for spotting a loop.
    trace: [(i32, i32); TRACEVECS],
    cur_trace: usize,
    state_change: bool,
    roundscore: i32,
    speed: i32,
    delta: (i32, i32),
    err: (i32, i32),
    deaths: i32,
    init_row: i32,
}

impl Default for Pac {
    fn default() -> Self {
        Pac {
            col: 0,
            row: 0,
            lastbox: NOWHERE,
            nextcol: NOWHERE,
            nextrow: NOWHERE,
            cfactor: 0,
            rfactor: 0,
            cf: NOWHERE,
            rf: NOWHERE,
            oldcf: NOWHERE,
            oldrf: NOWHERE,
            justate: false,
            aistate: PacState::Eating,
            trace: [(NOWHERE, NOWHERE); TRACEVECS],
            cur_trace: 0,
            state_change: false,
            roundscore: 0,
            speed: 4,
            delta: (0, 0),
            err: (0, 0),
            deaths: 0,
            init_row: 0,
        }
    }
}

/// One cell of the sprite sheet: the picture and, where it matters, the
/// silhouette to clip it through.
#[derive(Clone)]
struct Sprite {
    pix: Pixmap,
    mask: Option<Rc<Fb>>,
}

struct Pacman {
    mi: ModeInfo,
    /// Cell size, and where the board sits in the window.
    xs: i32,
    ys: i32,
    xb: i32,
    yb: i32,
    incx: i32,
    incy: i32,
    wallwidth: i32,
    spritexs: i32,
    spriteys: i32,
    spritedx: i32,
    spritedy: i32,

    gc: Gc,
    level: Vec<u8>,
    dotsleft: i32,
    bonus_dots: [(i32, i32, bool); NUM_BONUS_DOTS],

    pacman: Pac,
    ghosts: Vec<Ghost>,

    ghost_pixmap: Vec<Sprite>,
    ghost_mask: Option<Rc<Fb>>,
    scared_ghost: Vec<Sprite>,
    ghost_eyes: Vec<Sprite>,
    pacman_pixmap: Vec<Sprite>,
    pacman_death: Vec<Sprite>,

    /* draw_pacman_sprite */
    pm_mouth: i32,
    pm_mouth_delay: i32,
    pm_open_mouth: bool,
    pm_death_frame: usize,
    pm_death_delay: i32,

    /* draw_ghost_sprite */
    gh_wag: usize,
    gh_wag_count: i32,

    /* flash_bonus_dots */
    bd_flash_count: i32,
    bd_on: bool,

    /* pacman_tick */
    ghost_scared_timer: i32,
    flash_timer: i32,
    old_pac_state: PacState,

    delay: u32,
}

// ---------------------------------------------------------------------------
// The level generator (`pacman_level.c`)
// ---------------------------------------------------------------------------

/// The grid the generator works in, before it is formatted for drawing.
type Lev = Vec<u8>;

fn lev_at(level: &Lev, x: i32, y: i32) -> u8 {
    level[(y * LEVWIDTH + x) as usize]
}

fn setblockto(level: &mut Lev, x: i32, y: i32, c: u8) {
    if (0..LEVWIDTH).contains(&x) && (0..LEVHEIGHT).contains(&y) {
        level[(y * LEVWIDTH + x) as usize] = c;
    }
}

/// True when the cell is wall, ghost-gate, or off the board.
fn checkset(level: &Lev, x: i32, y: i32) -> bool {
    if !((0..LEVWIDTH).contains(&x) && (0..LEVHEIGHT).contains(&y)) {
        return true;
    }
    let c = lev_at(level, x, y);
    c == BLOCK_WALL || c == BLOCK_GHOST_ONLY
}

/// True when the cell already holds a dot, so a wall may not go there.
fn checkunsetdef(level: &Lev, x: i32, y: i32) -> bool {
    if !((0..LEVWIDTH).contains(&x) && (0..LEVHEIGHT).contains(&y)) {
        return false;
    }
    lev_at(level, x, y) == BLOCK_DOT_1
}

fn createjail(level: &mut Lev, width: i32, height: i32) {
    let xstart = LEVWIDTH / 2 - width / 2;
    let xend = LEVWIDTH / 2 + width / 2;
    let ystart = LEVHEIGHT / 2 - height / 2;
    let yend = LEVHEIGHT / 2 + height / 2;

    for y in ystart - 1..yend + 1 {
        for x in xstart - 1..xend + 1 {
            setblockto(level, x, y, BLOCK_DOT_1);
        }
    }
    for y in ystart..yend {
        for x in xstart..xend {
            setblockto(level, x, y, BLOCK_WALL);
        }
    }
}

fn jail_opening() -> (i32, i32) {
    let xstart = LEVWIDTH / 2 - JAILWIDTH / 2;
    let ystart = LEVHEIGHT / 2 - JAILHEIGHT / 2;
    (xstart + JAILWIDTH / 2, ystart)
}

/// Hollow the jail out and put the gate the ghosts come through on top.
fn finishjail(level: &mut Lev, width: i32, height: i32) {
    let xstart = LEVWIDTH / 2 - width / 2;
    let xend = LEVWIDTH / 2 + width / 2;
    let ystart = LEVHEIGHT / 2 - height / 2;
    let yend = LEVHEIGHT / 2 + height / 2;

    for y in ystart + 1..yend - 1 {
        for x in xstart + 1..xend - 1 {
            setblockto(level, x, y, BLOCK_EMPTY);
        }
    }
    for x in xstart - 1..xend + 1 {
        setblockto(level, x, ystart - 1, BLOCK_EMPTY);
        setblockto(level, x, yend, BLOCK_EMPTY);
    }
    for y in ystart - 1..yend + 1 {
        setblockto(level, xstart - 1, y, BLOCK_EMPTY);
        setblockto(level, xend, y, BLOCK_EMPTY);
    }
    setblockto(level, xstart + width / 2 - 1, ystart, BLOCK_GHOST_ONLY);
    setblockto(level, xstart + width / 2, ystart, BLOCK_GHOST_ONLY);
}

/// Try to lay a tile down centred on a cell. Returns false, and leaves the
/// level in an unusable state, when it will not fit.
fn tryset(level: &mut Lev, xpos: i32, ypos: i32, block: &[u8]) -> bool {
    if lev_at(level, xpos, ypos) == BLOCK_DOT_1 {
        return false;
    }
    let xstart = xpos - 2;
    let ystart = ypos - 2;

    for y in 0..TILEHEIGHT as i32 {
        for x in 0..TILEWIDTH as i32 {
            let locchar = block[(y * TILEWIDTH as i32 + x) as usize];
            if locchar == BLOCK_EMPTY {
                continue;
            }
            // A clear cell may not land on the border or on a wall; a wall may
            // not land on a cell that is already open.
            let clear_blocked = locchar == BLOCK_DOT_1
                && (xstart + x < 1
                    || xstart + x >= LEVWIDTH - 1
                    || ystart + y < 1
                    || ystart + y >= LEVHEIGHT - 1
                    || checkset(level, xstart + x, ystart + y));
            let wall_blocked = locchar == BLOCK_WALL
                && (xstart + x > 1
                    && xstart + x < LEVWIDTH
                    && ystart + y > 1
                    && ystart + y < LEVHEIGHT - 1)
                && checkunsetdef(level, xstart + x, ystart + y);
            if clear_blocked || wall_blocked {
                return false;
            }
        }
    }

    let xend = if xstart + (TILEWIDTH as i32) < LEVWIDTH - 1 {
        TILEWIDTH as i32
    } else {
        LEVWIDTH - xstart - 2
    };
    let yend = if ystart + (TILEHEIGHT as i32) < LEVHEIGHT - 1 {
        TILEHEIGHT as i32
    } else {
        LEVHEIGHT - ystart - 2
    };

    for y in (1 - ystart).max(0)..yend {
        for x in (1 - xstart).max(0)..xend {
            let locchar = block[(y * TILEWIDTH as i32 + x) as usize];
            if locchar == BLOCK_WALL && lev_at(level, xstart + x, ystart + y) == BLOCK_EMPTY {
                // Everything is mirrored into the other half, which is what
                // makes the level symmetric.
                setblockto(level, xstart + x, ystart + y, BLOCK_WALL);
                setblockto(level, LEVWIDTH - (xstart + x + 1), ystart + y, BLOCK_WALL);
            }
        }
    }

    setblockto(level, xpos, ypos, BLOCK_DOT_1);
    setblockto(level, LEVWIDTH - xpos - 1, ypos, BLOCK_DOT_1);
    true
}

/// Walk outward from a tile, laying more tiles in each direction it opens on.
fn nextstep(level: &mut Lev, x: i32, y: i32, dirvec: &[u32; 4], ndirs: usize) -> u32 {
    let mut dirvec = *dirvec;
    let mut ndirs = ndirs;
    let mut inc = 0u32;

    while ndirs > 0 {
        ndirs -= 1;
        let curdir = if ndirs == 0 {
            dirvec[0]
        } else {
            let dirpos = random_below(ndirs as i32) as usize;
            dirvec.swap(dirpos, ndirs);
            dirvec[ndirs]
        };

        let ret = match curdir {
            GO_UP if y >= 1 => creatlevelblock(level, x, y - 1),
            GO_RIGHT if x <= LEVWIDTH - 2 => creatlevelblock(level, x + 1, y),
            GO_DOWN if y <= LEVHEIGHT - 2 => creatlevelblock(level, x, y + 1),
            GO_LEFT if x >= 1 => creatlevelblock(level, x - 1, y),
            _ => return 0,
        };
        if ret == 0 {
            return 0;
        }
        if ret != -1 {
            inc += ret as u32;
        }
    }
    if inc == 0 { 1 } else { inc }
}

/// Try tiles at a cell until one fits and everything downstream of it fits
/// too. Returns how many cells were opened, or zero if the cell is a dead end.
///
/// The saved copy is a heap allocation rather than a stack array, which
/// matters: this recurses once per cell of the maze, and upstreams
/// stack-allocated copy of the whole level would be more than a browser gives
/// a wasm module.
fn creatlevelblock(level: &mut Lev, x: i32, y: i32) -> i32 {
    let mut tried = (1u32 << TILES.len()) - 1;

    if !((0..LEVWIDTH).contains(&x) && (0..LEVHEIGHT).contains(&y)) {
        return 0;
    }
    if checkunsetdef(level, x, y) {
        return -1;
    }

    // Tiles that would run off an edge are struck off before we start.
    if x == 0 {
        tried &= !(1 << 0);
    } else if x == 1 {
        tried &= !(1 << 4 | 1 << 5 | 1 << 6 | 1 << 8 | 1 << 9 | 1 << 10);
    } else if x == LEVWIDTH - 1 {
        tried &= !(1 << 0);
    } else if x == LEVWIDTH - 2 {
        tried &= !(1 << 2 | 1 << 3 | 1 << 6 | 1 << 7 | 1 << 8 | 1 << 10);
    }
    if y == 1 {
        tried &= !(1 << 2 | 1 << 5 | 1 << 6 | 1 << 7 | 1 << 9 | 1 << 10);
    } else if y == 0 || y == LEVHEIGHT - 1 {
        tried &= !(1 << 1);
    } else if y == LEVHEIGHT - 2 {
        tried &= !(1 << 3 | 1 << 4 | 1 << 7 | 1 << 8 | 1 << 9 | 1 << 10);
    }

    let savedlev = level.clone();

    while tried != 0 {
        let tilenr = TILEPROB[random_below(TILEPROB.len() as i32) as usize];
        if tried & (1 << tilenr) == 0 {
            continue;
        }
        if tryset(level, x, y, TILES[tilenr].block) {
            let ret = nextstep(level, x, y, &TILES[tilenr].dirvec, TILES[tilenr].ndirs);
            if ret != 0 {
                return ret as i32 + 1;
            }
            level.copy_from_slice(&savedlev);
        }
        tried &= !TILES[tilenr].similar_to;
    }
    0
}

/// Everything still empty becomes wall.
fn filllevel(level: &mut Lev) {
    for c in level.iter_mut() {
        if *c == BLOCK_EMPTY {
            *c = BLOCK_WALL;
        }
    }
}

/// The first open cell scanning from each corner, which is where the four
/// bonus dots go.
fn find_corner(level: &Lev, from_bottom: bool, from_right: bool) -> (i32, i32) {
    let ys: Vec<i32> = if from_bottom {
        (0..LEVHEIGHT).rev().collect()
    } else {
        (0..LEVHEIGHT).collect()
    };
    let xs: Vec<i32> = if from_right {
        (0..LEVWIDTH).rev().collect()
    } else {
        (0..LEVWIDTH).collect()
    };
    for y in ys {
        for x in &xs {
            if !checkset(level, *x, y) {
                return (*x, y);
            }
        }
    }
    (0, 0)
}

/// Turn the wall/dot grid into the pieces of line and corner that get drawn,
/// by looking at each wall cells eight neighbours. Upstream: "Stupid
/// algorithm, could be done better!"
fn frmtlevel(level: &mut Lev, bonus: &mut [(i32, i32, bool); NUM_BONUS_DOTS]) {
    let mut out: Lev = vec![BLOCK_EMPTY; level.len()];

    bonus[0] = (
        find_corner(level, false, false).0,
        find_corner(level, false, false).1,
        false,
    );
    let tr = find_corner(level, false, true);
    bonus[1] = (tr.0, tr.1, false);
    let bl = find_corner(level, true, false);
    bonus[2] = (bl.0, bl.1, false);
    let br = find_corner(level, true, true);
    bonus[3] = (br.0, br.1, false);

    for y in 0..LEVHEIGHT {
        for x in 0..LEVWIDTH {
            let at = (y * LEVWIDTH + x) as usize;
            if !checkset(level, x, y) {
                out[at] = if bonus.iter().any(|b| b.0 == x && b.1 == y) {
                    BLOCK_DOT_BONUS
                } else {
                    BLOCK_DOT_2
                };
                continue;
            }
            if lev_at(level, x, y) == BLOCK_GHOST_ONLY {
                out[at] = BLOCK_GHOST_ONLY;
                continue;
            }

            // The four diagonal neighbours, then the four orthogonal ones.
            let poscond = u32::from(checkset(level, x - 1, y - 1))
                | u32::from(checkset(level, x + 1, y - 1)) << 1
                | u32::from(checkset(level, x + 1, y + 1)) << 2
                | u32::from(checkset(level, x - 1, y + 1)) << 3;
            let poscond2 = u32::from(checkset(level, x - 1, y))
                | u32::from(checkset(level, x, y - 1)) << 1
                | u32::from(checkset(level, x + 1, y)) << 2
                | u32::from(checkset(level, x, y + 1)) << 3;

            out[at] = match poscond {
                0xF => BLOCK_EMPTY,
                0x1 => BLOCK_WALL_TL,
                0x2 => BLOCK_WALL_TR,
                0x4 => BLOCK_WALL_BR,
                0x8 => BLOCK_WALL_BL,
                _ => match poscond2 {
                    0x5 | 0xD | 0x7 => BLOCK_WALL_HO,
                    0xA | 0xB | 0xE => BLOCK_WALL_VE,
                    0x3 => BLOCK_WALL_TL,
                    0x6 => BLOCK_WALL_TR,
                    0xC => BLOCK_WALL_BR,
                    0x9 => BLOCK_WALL_BL,
                    _ => match poscond {
                        0xE => BLOCK_WALL_TL,
                        0xD => BLOCK_WALL_TR,
                        0xB => BLOCK_WALL_BR,
                        0x7 => BLOCK_WALL_BL,
                        _ => BLOCK_EMPTY,
                    },
                },
            };
        }
    }
    *level = out;
}

impl Pacman {
    /// A new level: half the time the arcade one, half the time generated.
    fn createnewlevel(&mut self) -> i32 {
        let mut level: Lev = vec![BLOCK_EMPTY; (LEVWIDTH * LEVHEIGHT) as usize];
        let mut i = 0;

        if random_below(2) == 0 {
            loop {
                level.fill(BLOCK_EMPTY);
                createjail(&mut level, JAILWIDTH, JAILHEIGHT);
                let ret = nextstep(
                    &mut level,
                    LEVWIDTH / 2 - 1,
                    LEVHEIGHT / 2 - JAILHEIGHT / 2 - 3,
                    &[GO_UP, 0, 0, 0],
                    1,
                );
                if ret == 0 {
                    self.level = level;
                    return i;
                }
                if ret as i32 * 100 >= LEVWIDTH * LEVHEIGHT * MINDOTPERC {
                    break;
                }
            }
            filllevel(&mut level);
            frmtlevel(&mut level, &mut self.bonus_dots);
            finishjail(&mut level, JAILWIDTH, JAILHEIGHT);
        } else {
            for (y, row) in STDLEVEL.iter().enumerate() {
                let at = y * LEVWIDTH as usize;
                level[at..at + LEVWIDTH as usize].copy_from_slice(*row);
            }
            frmtlevel(&mut level, &mut self.bonus_dots);
            i = 1;
        }

        self.level = level;
        self.dotsleft = self
            .level
            .iter()
            .filter(|c| **c == BLOCK_DOT_2 || **c == BLOCK_DOT_BONUS)
            .count() as i32;
        i
    }

    /// Can a ghost or Pac-Man stand here?
    fn check_pos(&self, y: i32, x: i32, ghostpass: bool) -> bool {
        if !((0..LEVWIDTH).contains(&x) && (0..LEVHEIGHT).contains(&y)) {
            return false;
        }
        let c = self.level[(y * LEVWIDTH + x) as usize];
        c == BLOCK_DOT_2
            || c == BLOCK_EMPTY
            || c == BLOCK_DOT_BONUS
            || (c == BLOCK_GHOST_ONLY && ghostpass)
    }

    fn check_dot(&self, x: i32, y: i32) -> bool {
        if !((0..LEVWIDTH).contains(&x) && (0..LEVHEIGHT).contains(&y)) {
            return false;
        }
        self.level[(y * LEVWIDTH + x) as usize] == BLOCK_DOT_2
    }

    fn is_bonus_dot(&self, x: i32, y: i32) -> Option<usize> {
        self.bonus_dots.iter().position(|b| b.0 == x && b.1 == y)
    }
}

// -------------------------------------------------------------------------
// The AI (`pacman_ai.c`)
// -------------------------------------------------------------------------

impl Pacman {
    /// Which ways a ghost may go, with the way it came struck off.
    fn ghost_get_posdirs(&self, g: &Ghost, posdirs: &mut [bool; DIRVECS]) -> i32 {
        let mut nrdirs = 0;
        for i in 0..DIRVECS {
            posdirs[i] = false;
            // Remove the opposite of the way it came.
            if g.lastbox != NOWHERE
                && i as i32 == (g.lastbox + 2) % DIRVECS as i32
                && g.aistate != GhostState::GoingOut
            {
                continue;
            }
            if g.aistate == GhostState::GoingOut && i == 1 {
                continue;
            }
            // Only a ghost on its way in or out may cross the jail gate.
            let can_go_in = g.aistate == GhostState::GoingOut || g.aistate == GhostState::GoingIn;
            posdirs[i] = self.check_pos(g.row + DIRS[i].1, g.col + DIRS[i].0, can_go_in);
            if posdirs[i] {
                nrdirs += 1;
            }
        }
        nrdirs
    }

    /// Somewhere at random, but not back the way it came unless there is
    /// nowhere else.
    fn ghost_random(&self, g: &mut Ghost) {
        let mut posdirs = [false; DIRVECS];
        let nrdirs = self.ghost_get_posdirs(g, &mut posdirs);
        let mut dir = 0;
        for (i, ok) in posdirs.iter().enumerate() {
            if *ok {
                dir = i;
            }
        }
        if nrdirs == 0 {
            dir = ((g.lastbox + 2) % DIRVECS as i32) as usize;
        } else if nrdirs > 1 {
            for (i, ok) in posdirs.iter().enumerate() {
                if *ok && random_below(nrdirs) == 0 {
                    dir = i;
                    break;
                }
            }
        }
        g.nextrow = g.row + DIRS[dir].1;
        g.nextcol = g.col + DIRS[dir].0;
        g.lastbox = dir as i32;
    }

    /// Whichever open direction points most nearly at Pac-Man, or away from
    /// him when the ghost is frightened.
    fn ghost_toward(&self, g: &mut Ghost, away: bool) {
        let mut posdirs = [false; DIRVECS];
        let nrdirs = self.ghost_get_posdirs(g, &mut posdirs);
        let mut dir = 0;
        let mut highest = -100_000;
        for (i, ok) in posdirs.iter().enumerate() {
            if *ok {
                dir = i;
            }
        }
        if nrdirs == 0 {
            dir = ((g.lastbox + 2) % DIRVECS as i32) as usize;
        } else if nrdirs > 1 {
            for (i, ok) in posdirs.iter().enumerate() {
                if !*ok {
                    continue;
                }
                let (dc, dr) = if away {
                    (g.col - self.pacman.col, g.row - self.pacman.row)
                } else {
                    (self.pacman.col - g.col, self.pacman.row - g.row)
                };
                let thisvec = dc * DIRS[i].0 + dr * DIRS[i].1;
                if thisvec >= highest {
                    dir = i;
                    // Upstream only records the new best score when chasing,
                    // so a hiding ghost keeps taking the last open direction.
                    if !away {
                        highest = thisvec;
                    }
                }
            }
        }
        g.nextrow = g.row + DIRS[dir].1;
        g.nextcol = g.col + DIRS[dir].0;
        g.lastbox = dir as i32;
    }

    /// One step of the recursive walk home, which stores the way it came so
    /// the ghost can replay it. Not a shortest path, and upstream says so.
    ///
    /// Upstream applies the x component of a direction to the row and the y to
    /// the column here, and again when replaying, with a comment reading
    /// "row == vx ? wtf... Dont Ask". The two halves agree, so the ghost does
    /// get home; it is kept as written.
    fn recur_back_track(
        &self,
        row: i32,
        col: i32,
        trace: &mut Vec<(i32, i32)>,
        way_home: &mut Vec<(i32, i32)>,
    ) -> bool {
        if trace.contains(&(col, row)) {
            return false;
        }
        let (cx, cy) = jail_opening();
        if row == cy + 1 && col == cx {
            return true;
        }
        if trace.len() >= GHOST_TRACE {
            return false;
        }
        trace.push((col, row));

        for ps in [POS_LEFT, POS_UP, POS_DOWN, POS_RIGHT] {
            let tr = row + DIRS[ps].0;
            let tc = col + DIRS[ps].1;
            if self.check_pos(tr, tc, true) && self.recur_back_track(tr, tc, trace, way_home) {
                way_home.push(DIRS[ps]);
                return true;
            }
        }
        false
    }

    fn find_home(&self, g: &mut Ghost) {
        let mut trace: Vec<(i32, i32)> = Vec::new();
        let mut way_home: Vec<(i32, i32)> = Vec::new();
        let (r, c) = (g.row, g.col);
        self.recur_back_track(r, c, &mut trace, &mut way_home);
        g.way_home.fill((-1, -1));
        for (i, w) in way_home.iter().take(GHOST_TRACE).enumerate() {
            g.way_home[i] = *w;
        }
        g.home_count = way_home.len().min(GHOST_TRACE) as i32;
        g.home_idx = g.home_count;
    }

    fn ghost_goingin(&self, g: &mut Ghost) {
        g.home_idx -= 1;
        if g.home_idx < 0 {
            g.aistate = GhostState::GoingOut;
            return;
        }
        let w = g.way_home[g.home_idx as usize];
        g.nextrow = g.row + w.0;
        g.nextcol = g.col + w.1;
    }

    /// A vector from Pac-Man towards every dot on the board, each pulling in
    /// inverse proportion to the square of its distance.
    fn pac_dot_vec(&self) -> (i64, i64) {
        let (mut vx, mut vy) = (0i64, 0i64);
        for y in 0..LEVHEIGHT {
            for x in 0..LEVWIDTH {
                if !self.check_dot(x, y) {
                    continue;
                }
                let dx = i64::from(x - self.pacman.col);
                let dy = i64::from(y - self.pacman.row);
                let dist = dx * dx + dy * dy;
                if dist == 0 {
                    continue;
                }
                vx += (dx * i64::from(LEVWIDTH) * i64::from(LEVHEIGHT)) / dist;
                vy += (dy * i64::from(LEVWIDTH) * i64::from(LEVHEIGHT)) / dist;
            }
        }
        (vx, vy)
    }

    /// How close the nearest ghost is, and which way it lies.
    fn pac_ghost_prox_and_vector(&self) -> (i32, i32, i32) {
        let mut closest = 100_000;
        let (mut vx, mut vy) = (0, 0);
        for g in &self.ghosts {
            if g.dead || g.aistate == GhostState::InBox || g.aistate == GhostState::GoingOut {
                continue;
            }
            let dx = g.col - self.pacman.col;
            let dy = g.row - self.pacman.row;
            let dist = dx * dx + dy * dy;
            if dist < closest {
                closest = dist;
                vx = dx;
                vy = dy;
            }
        }
        (closest, vx, vy)
    }

    fn pac_get_posdirs(&mut self, posdirs: &mut [bool; DIRVECS]) -> i32 {
        let mut nrdirs = 0;
        for i in 0..DIRVECS {
            // Having just eaten, or just changed state, he may turn round.
            if !self.pacman.justate
                && !self.pacman.state_change
                && self.pacman.lastbox != NOWHERE
                && i as i32 == (self.pacman.lastbox + 2) % DIRVECS as i32
            {
                posdirs[i] = false;
            } else {
                posdirs[i] = self.check_pos(
                    self.pacman.row + DIRS[i].1,
                    self.pacman.col + DIRS[i].0,
                    false,
                );
                if posdirs[i] {
                    nrdirs += 1;
                }
            }
        }
        self.pacman.state_change = false;
        nrdirs
    }

    fn clear_trace(&mut self) {
        self.pacman.trace = [(NOWHERE, NOWHERE); TRACEVECS];
        self.pacman.cur_trace = 0;
    }

    fn pac_save_trace(&mut self, vx: i32, vy: i32) {
        if !(vx == NOWHERE && vy == NOWHERE) {
            let at = self.pacman.cur_trace;
            self.pacman.trace[at] = (vx, vy);
            self.pacman.cur_trace = (at + 1) % TRACEVECS;
        }
    }

    fn pac_check_trace(&self, vx: i32, vy: i32) -> bool {
        for i in 1..TRACEVECS {
            let curel = (self.pacman.cur_trace + TRACEVECS - i) % TRACEVECS;
            let t = self.pacman.trace[curel];
            if t == (NOWHERE, NOWHERE) {
                continue;
            }
            if t == (vx, vy) {
                return true;
            }
        }
        false
    }
}

impl Pacman {
    /// Eat as many dots as possible, and hide when a ghost gets close. In
    /// hiding, the direction the nearest ghost lies in is struck off first.
    fn pac_eating(&mut self) {
        let (prox, gvx, gvy) = self.pac_ghost_prox_and_vector();

        if prox < 4 * 4 && self.pacman.aistate == PacState::Eating {
            self.pacman.aistate = PacState::Hiding;
            self.pacman.state_change = true;
            self.clear_trace();
        }
        if prox > 6 * 6 && self.pacman.aistate == PacState::Hiding {
            self.pacman.aistate = PacState::Eating;
            if !self.pacman.justate {
                self.pacman.state_change = true;
            }
            self.clear_trace();
        }
        if prox < 3 * 3 {
            self.pacman.state_change = true;
        }

        let mut posdirs = [false; DIRVECS];
        self.pac_get_posdirs(&mut posdirs);

        let mut dir = 0;
        if self.pacman.aistate == PacState::Hiding {
            let mut highest = -(1 << 16);
            let mut worst = 0;
            for i in 0..DIRVECS {
                if !posdirs[i] {
                    continue;
                }
                let score = gvx * DIRS[i].0 + gvy * DIRS[i].1;
                if score > highest {
                    worst = i;
                    highest = score;
                }
                dir = i;
            }
            posdirs[worst] = false;
        }

        // The last open direction, if everything else fails.
        for (i, ok) in posdirs.iter().enumerate() {
            if *ok {
                dir = i;
            }
        }

        let (lvx, lvy) = self.pac_dot_vec();
        let (vx, vy) = (lvx as i32, lvy as i32);

        if vx != NOWHERE && vy != NOWHERE && self.pac_check_trace(vx, vy) {
            self.pacman.roundscore += 1;
            if self.pacman.roundscore >= 12 {
                self.pacman.roundscore = 0;
                self.pacman.aistate = PacState::Random;
                self.clear_trace();
            }
        } else {
            self.pacman.roundscore = 0;
        }

        if !self.pacman.justate {
            self.pac_save_trace(vx, vy);
        }

        let mut highest = -(1 << 16);
        let mut dotfound = false;
        for i in 0..DIRVECS {
            if !posdirs[i] {
                continue;
            }
            let score = DIRS[i].0 * vx + DIRS[i].1 * vy;
            if self.check_dot(self.pacman.col + DIRS[i].0, self.pacman.row + DIRS[i].1) {
                if !dotfound || score > highest {
                    highest = score;
                    dir = i;
                    dotfound = true;
                }
            } else if score > highest && !dotfound {
                dir = i;
                highest = score;
            }
        }

        self.pacman.nextrow = self.pacman.row + DIRS[dir].1;
        self.pacman.nextcol = self.pacman.col + DIRS[dir].0;
        self.pacman.lastbox = dir as i32;
    }

    /// Go after the ghosts. Upstream never fills in the ghost vector here, so
    /// this comes to "any open direction", which is why a chasing Pac-Man
    /// looks more hopeful than effective.
    fn pac_chasing(&mut self) {
        let mut posdirs = [false; DIRVECS];
        self.pac_get_posdirs(&mut posdirs);

        let (vx, vy) = (0, 0);
        let mut dir = 0;
        let mut highest = -(1 << 16);
        let mut worst = 0;
        for i in 0..DIRVECS {
            if !posdirs[i] {
                continue;
            }
            let score = vx * DIRS[i].0 + vy * DIRS[i].1;
            if score < highest {
                worst = i;
                highest = score;
            }
            dir = i;
        }
        posdirs[worst] = false;

        for (i, ok) in posdirs.iter().enumerate() {
            if *ok {
                dir = i;
            }
        }

        let (lvx, lvy) = self.pac_dot_vec();
        let (vx, vy) = (lvx as i32, lvy as i32);
        if vx != NOWHERE && vy != NOWHERE && self.pac_check_trace(vx, vy) {
            self.pacman.roundscore += 1;
            if self.pacman.roundscore >= 12 {
                self.pacman.roundscore = 0;
                self.pacman.aistate = PacState::Random;
                self.clear_trace();
            }
        } else {
            self.pacman.roundscore = 0;
        }

        self.pacman.nextrow = self.pacman.row + DIRS[dir].1;
        self.pacman.nextcol = self.pacman.col + DIRS[dir].0;
        self.pacman.lastbox = dir as i32;
    }

    /// Wander, which is what he does once he has noticed he is going in
    /// circles. A dot in reach snaps him out of it.
    fn pac_random(&mut self) {
        let (prox, _, _) = self.pac_ghost_prox_and_vector();
        if prox < 5 * 5 {
            self.pacman.aistate = PacState::Hiding;
            self.pacman.state_change = true;
        }
        if random_below(20) == 0 {
            self.pacman.aistate = PacState::Eating;
            self.pacman.state_change = true;
            self.clear_trace();
        }

        let mut posdirs = [false; DIRVECS];
        let nrdirs = self.pac_get_posdirs(&mut posdirs).max(1);

        let mut dir: Option<usize> = None;
        let mut lastdir = 0;
        for i in 0..DIRVECS {
            if !posdirs[i] {
                continue;
            }
            lastdir = i;
            if self.check_dot(self.pacman.col + DIRS[i].0, self.pacman.row + DIRS[i].1) {
                dir = Some(i);
                self.pacman.aistate = PacState::Eating;
                self.pacman.state_change = true;
                self.clear_trace();
                break;
            } else if random_below(nrdirs) == 0 {
                dir = Some(i);
            }
        }
        let dir = dir.unwrap_or(lastdir);

        self.pacman.nextrow = self.pacman.row + DIRS[dir].1;
        self.pacman.nextcol = self.pacman.col + DIRS[dir].0;
        self.pacman.lastbox = dir as i32;
    }

    /// One step of a ghost, including the changes of mind between states.
    fn ghost_update(&mut self, i: usize) {
        {
            let g = &mut self.ghosts[i];
            if !(g.nextrow == NOWHERE && g.nextcol == NOWHERE) {
                g.row = g.nextrow;
                g.col = g.nextcol;
            }

            if (g.aistate == GhostState::RandDir || g.aistate == GhostState::Chasing)
                && random_below(10) == 0
            {
                g.aistate = if random_below(3) == 0 {
                    GhostState::RandDir
                } else {
                    GhostState::Chasing
                };
            } else if g.aistate == GhostState::InBox {
                if g.timeleft < 0 {
                    g.aistate = GhostState::GoingOut;
                } else {
                    g.timeleft -= 1;
                }
            } else if g.aistate == GhostState::GoingOut
                && (g.col < LEVWIDTH / 2 - JAILWIDTH / 2
                    || g.col > LEVWIDTH / 2 + JAILWIDTH / 2
                    || g.row < LEVHEIGHT / 2 - JAILHEIGHT / 2
                    || g.row > LEVHEIGHT / 2 + JAILHEIGHT / 2)
            {
                g.aistate = GhostState::RandDir;
            }
        }

        let mut g = std::mem::take(&mut self.ghosts[i]);
        match g.aistate {
            GhostState::InBox | GhostState::GoingOut | GhostState::RandDir => {
                self.ghost_random(&mut g);
            }
            GhostState::Chasing => self.ghost_toward(&mut g, false),
            GhostState::Hiding => self.ghost_toward(&mut g, true),
            GhostState::GoingIn => {
                if g.wait_pos {
                    self.find_home(&mut g);
                    g.wait_pos = false;
                }
                self.ghost_goingin(&mut g);
            }
        }
        g.cfactor = getfactor(g.nextcol, g.col);
        g.rfactor = getfactor(g.nextrow, g.row);
        self.ghosts[i] = g;
    }

    /// One step of Pac-Man, including eating whatever he is standing on.
    fn pac_update(&mut self) {
        if !(self.pacman.nextrow == NOWHERE && self.pacman.nextcol == NOWHERE) {
            self.pacman.row = self.pacman.nextrow;
            self.pacman.col = self.pacman.nextcol;
        }

        let at = (self.pacman.row * LEVWIDTH + self.pacman.col) as usize;
        if self.level[at] == BLOCK_DOT_2 || self.level[at] == BLOCK_DOT_BONUS {
            self.level[at] = BLOCK_EMPTY;
            self.pacman.justate = true;
            self.dotsleft -= 1;
        } else {
            self.pacman.justate = false;
        }

        match self.pacman.aistate {
            PacState::Eating | PacState::Hiding => self.pac_eating(),
            PacState::Random => self.pac_random(),
            PacState::Chasing => self.pac_chasing(),
            PacState::Dying => {} /* Dont move */
        }

        self.pacman.cfactor = getfactor(self.pacman.nextcol, self.pacman.col);
        self.pacman.rfactor = getfactor(self.pacman.nextrow, self.pacman.row);
    }
}

fn getfactor(a: i32, b: i32) -> i32 {
    match a.cmp(&b) {
        std::cmp::Ordering::Greater => 1,
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
    }
}

// -------------------------------------------------------------------------
// Drawing and the game loop (`pacman.c`)
// -------------------------------------------------------------------------

/// Upstreams `scale_pixmap`: nearest neighbour, a column and then a row at a
/// time. It is what the sprites were always scaled with, and at these sizes it
/// is what makes them look like a cabinet rather than like a photograph.
fn scale_pixmap(src: &Fb, dw: i32, dh: i32) -> Fb {
    let mut out = if src.depth() == 1 {
        Fb::new_bitmap(dw.max(1), dh.max(1))
    } else {
        Fb::new(dw.max(1), dh.max(1))
    };
    for y in 0..dh {
        let sy = (y * src.height() / dh.max(1)).min(src.height() - 1);
        for x in 0..dw {
            let sx = (x * src.width() / dw.max(1)).min(src.width() - 1);
            out.put_pixel(x, y, src.get_pixel(sx, sy));
        }
    }
    out
}

impl Pacman {
    fn setwallcolor(&mut self) {
        self.gc.set_foreground(if self.mi.npixels() > 2 {
            // BLUE, as upstream indexes it out of the uniform colormap.
            self.mi.pixel((45 * self.mi.npixels() / 64) as usize)
        } else {
            self.mi.white
        });
    }

    fn setdotcolor(&mut self) {
        let white = self.mi.white;
        self.gc.set_foreground(white);
    }

    fn cleardotcolor(&mut self) {
        let black = self.mi.black;
        self.gc.set_foreground(black);
    }

    /// Row and column to the middle of a cell in pixels.
    fn dot_rc_to_pixel(&self, x: i32, y: i32) -> (i32, i32) {
        (
            self.xs * x + self.xs / 2 - if self.xs > 32 { self.xs / 16 } else { 1 } + self.xb,
            self.ys * y + self.ys / 2 - if self.ys > 32 { self.ys / 16 } else { 1 } + self.yb,
        )
    }

    fn dot_width_height(&self) -> i32 {
        if self.xs > 32 { self.xs / 16 } else { 1 }
    }

    fn bonus_dot_width_height(&self, d: &Dpy) -> i32 {
        d.height() / 65
    }

    fn draw_bonus_dot(&mut self, d: &mut Dpy, x: i32, y: i32) {
        self.setdotcolor();
        let (px, py) = self.dot_rc_to_pixel(x, y);
        let w = self.bonus_dot_width_height(d);
        let gc = self.gc.clone();
        d.win().fill_arc(&gc, px, py, w, w, 0, 23040);
    }

    fn clear_bonus_dot(&mut self, d: &mut Dpy, x: i32, y: i32) {
        self.cleardotcolor();
        let (px, py) = self.dot_rc_to_pixel(x, y);
        let w = self.bonus_dot_width_height(d);
        let gc = self.gc.clone();
        d.win().fill_arc(&gc, px, py, w, w, 0, 23040);
    }

    fn draw_regular_dot(&mut self, d: &mut Dpy, x: i32, y: i32) {
        self.setdotcolor();
        let (px, py) = self.dot_rc_to_pixel(x, y);
        let w = self.dot_width_height();
        let gc = self.gc.clone();
        d.win().draw_arc(&gc, px, py, w, w, 0, 23040);
    }

    /// One cell of the level: a dot, or the piece of wall the formatting pass
    /// decided this cell is.
    fn drawlevelblock(&mut self, d: &mut Dpy, x: i32, y: i32) {
        let dx = if self.xs % 2 == 1 { -1 } else { 0 };
        let dy = if self.ys % 2 == 1 { -1 } else { 0 };
        let width = self.wallwidth;
        self.gc.set_line_width(width);
        let (xs, ys, xb, yb) = (self.xs, self.ys, self.xb, self.yb);
        let c = self.level[(y * LEVWIDTH + x) as usize];

        if xs < 2 || ys < 2 {
            match c {
                BLOCK_EMPTY | BLOCK_GHOST_ONLY => {}
                BLOCK_DOT_2 => {
                    self.setdotcolor();
                    let gc = self.gc.clone();
                    d.win().draw_point(&gc, x * xs + xb, y * ys + yb);
                }
                _ => {
                    self.setwallcolor();
                    let gc = self.gc.clone();
                    d.win().draw_point(&gc, x * xs + xb, y * ys + yb);
                }
            }
            return;
        }

        match c {
            BLOCK_EMPTY | BLOCK_GHOST_ONLY => {}
            BLOCK_DOT_2 => {
                if xs < 8 || ys < 8 {
                    self.setdotcolor();
                    let gc = self.gc.clone();
                    d.win()
                        .draw_point(&gc, x * xs + xb + xs / 2, y * ys + yb + ys / 2);
                } else {
                    self.draw_regular_dot(d, x, y);
                }
            }
            BLOCK_DOT_BONUS => self.draw_bonus_dot(d, x, y),
            BLOCK_WALL_HO => {
                self.setwallcolor();
                let gc = self.gc.clone();
                d.win().draw_line(
                    &gc,
                    xs * x + xb,
                    ys * y + ys / 2 + yb,
                    xs * (x + 1) + xb,
                    ys * y + ys / 2 + yb,
                );
            }
            BLOCK_WALL_VE => {
                self.setwallcolor();
                let gc = self.gc.clone();
                d.win().draw_line(
                    &gc,
                    xs * x + xs / 2 + xb,
                    ys * y + yb,
                    xs * x + xs / 2 + xb,
                    ys * (y + 1) + yb,
                );
            }
            BLOCK_WALL_BL => {
                self.setwallcolor();
                let gc = self.gc.clone();
                d.win().draw_arc(
                    &gc,
                    xs * x - ys / 2 + xb + dx,
                    ys * y + ys / 2 + yb,
                    xs,
                    ys,
                    0,
                    90 * 64,
                );
            }
            BLOCK_WALL_BR => {
                self.setwallcolor();
                let gc = self.gc.clone();
                d.win().draw_arc(
                    &gc,
                    xs * x + ys / 2 + xb,
                    ys * y + ys / 2 + yb,
                    xs,
                    ys,
                    90 * 64,
                    90 * 64,
                );
            }
            BLOCK_WALL_TR => {
                self.setwallcolor();
                let gc = self.gc.clone();
                d.win().draw_arc(
                    &gc,
                    xs * x + ys / 2 + xb,
                    ys * y - ys / 2 + yb + dy,
                    xs,
                    ys,
                    180 * 64,
                    90 * 64,
                );
            }
            BLOCK_WALL_TL => {
                self.setwallcolor();
                let gc = self.gc.clone();
                d.win().draw_arc(
                    &gc,
                    xs * x - ys / 2 + xb + dx,
                    ys * y - ys / 2 + yb + dy,
                    xs,
                    ys,
                    270 * 64,
                    90 * 64,
                );
            }
            _ => {}
        }
    }

    fn drawlevel(&mut self, d: &mut Dpy) {
        for y in 0..LEVHEIGHT {
            for x in 0..LEVWIDTH {
                self.drawlevelblock(d, x, y);
            }
        }
    }
}

const MAX_MOUTH_DELAY: i32 = 2;
const MAX_DEATH_DELAY: i32 = 20;
const MAX_WAG_COUNT: i32 = 50;
const MAX_FLASH_COUNT: i32 = 25;
const DEFAULT_SCARED_TIME: i32 = 500;
const START_FLASH: i32 = 200;
const FLASH_COUNT: i32 = 25;

impl Pacman {
    fn draw_pacman_sprite(&mut self, d: &mut Dpy) {
        if self.pacman.aistate == PacState::Dying {
            self.pacman.cf = self.pacman.oldcf;
            self.pacman.rf = self.pacman.oldrf;
        } else {
            self.pacman.cf = self.pacman.col * self.xs
                + self.pacman.delta.0 * self.pacman.cfactor
                + self.xb
                + self.spritedx;
            self.pacman.rf = self.pacman.row * self.ys
                + self.pacman.delta.1 * self.pacman.rfactor
                + self.yb
                + self.spritedy;
        }

        let dir = ((self.pacman.cfactor.abs() * (2 - self.pacman.cfactor)
            + self.pacman.rfactor.abs() * (1 + self.pacman.rfactor))
            % 4) as usize;

        if self.pm_mouth_delay == MAX_MOUTH_DELAY {
            if self.pm_mouth == MAXMOUTH as i32 - 1 || self.pm_mouth == 0 {
                self.pm_open_mouth = !self.pm_open_mouth;
            }
            if self.pm_open_mouth {
                self.pm_mouth += 1;
            } else {
                self.pm_mouth -= 1;
            }
            self.pm_mouth_delay = 0;
        } else {
            self.pm_mouth_delay += 1;
        }

        let sprite;
        if self.pacman.aistate == PacState::Dying {
            if self.pm_death_frame >= PAC_DEATH_FRAMES {
                self.pacman.aistate = PacState::Eating;
                self.pm_death_frame = 0;
                self.pm_death_delay = 0;
                self.reset_level(d, 0, false);
                return;
            }
            sprite = self.pacman_death[self.pm_death_frame].clone();
            if self.pm_death_delay == MAX_DEATH_DELAY {
                self.pm_death_frame += 1;
                self.pm_death_delay = 0;
            } else {
                self.pm_death_delay += 1;
            }
        } else {
            let m = self.pm_mouth.clamp(0, MAXMOUTH as i32 - 1) as usize;
            sprite = self.pacman_pixmap[dir * MAXMOUTH + m].clone();
        }

        // Upstream erases through the mask of the sprite facing right with its
        // mouth shut, whichever way he is actually facing.
        let old = Sprite {
            pix: sprite.pix.clone(),
            mask: self.pacman_pixmap[0].mask.clone(),
        };
        let (ocf, orf) = (self.pacman.oldcf, self.pacman.oldrf);
        let (cf, rf) = (self.pacman.cf, self.pacman.rf);
        let (w, h) = (self.spritexs, self.spriteys);
        let black = self.mi.black;
        let mut gc = self.gc.clone();
        gc.set_foreground(black);
        if let Some(m) = &old.mask {
            gc.set_clip_mask(Rc::clone(m));
        }
        gc.set_clip_origin(ocf, orf);
        d.win().fill_rectangle(&gc, ocf, orf, w, h);

        let mut gc = self.gc.clone();
        gc.set_foreground(black);
        if let Some(m) = &sprite.mask {
            gc.set_clip_mask(Rc::clone(m));
        }
        gc.set_clip_origin(cf, rf);
        d.win().copy_area(&gc, &sprite.pix, 0, 0, w, h, cf, rf);

        if self.pacman.aistate != PacState::Dying {
            self.pacman.oldcf = self.pacman.cf;
            self.pacman.oldrf = self.pacman.rf;
        }
    }

    fn draw_ghost_sprite(&mut self, d: &mut Dpy, ghost: usize) {
        let g = self.ghosts[ghost].clone();
        let dir =
            ((g.cfactor.abs() * (2 - g.cfactor) + g.rfactor.abs() * (1 + g.rfactor)) % 4) as usize;
        let fs = usize::from(g.flash_scared);

        let sprite = match g.aistate {
            GhostState::Hiding => self.scared_ghost[fs * MAXGWAG + self.gh_wag].clone(),
            GhostState::GoingIn => self.ghost_eyes[dir].clone(),
            _ => self.ghost_pixmap[(ghost * MAXGDIR + dir) * MAXGWAG + self.gh_wag].clone(),
        };

        let cf = g.col * self.xs + g.delta.0 * g.cfactor + self.xb + self.spritedx;
        let rf = g.row * self.ys + g.delta.1 * g.rfactor + self.yb + self.spritedy;
        self.ghosts[ghost].cf = cf;
        self.ghosts[ghost].rf = rf;

        let (w, h) = (self.spritexs, self.spriteys);
        let black = self.mi.black;
        let mut gc = self.gc.clone();
        gc.set_foreground(black);
        if let Some(m) = &self.ghost_mask {
            gc.set_clip_mask(Rc::clone(m));
        }
        gc.set_clip_origin(g.oldcf, g.oldrf);
        d.win().fill_rectangle(&gc, g.oldcf, g.oldrf, w, h);

        if self.pacman.aistate != PacState::Dying {
            // Put back the piece of level the ghost was standing on.
            self.drawlevelblock(d, g.col, g.row);

            let mut gc = self.gc.clone();
            gc.set_foreground(black);
            if let Some(m) = &self.ghost_mask {
                gc.set_clip_mask(Rc::clone(m));
            }
            gc.set_clip_origin(cf, rf);
            d.win().copy_area(&gc, &sprite.pix, 0, 0, w, h, cf, rf);

            self.ghosts[ghost].oldcf = cf;
            self.ghosts[ghost].oldrf = rf;
            self.gh_wag_count += 1;
            if self.gh_wag_count == MAX_WAG_COUNT {
                self.gh_wag = 1 - self.gh_wag;
                self.gh_wag_count = 0;
            }
        }
    }

    fn ghost_over(&self, x: i32, y: i32) -> bool {
        let (px, py) = self.dot_rc_to_pixel(x, y);
        self.ghosts.iter().any(|g| {
            g.cf <= px && px <= g.cf + self.spritexs && g.rf <= py && py <= g.rf + self.spriteys
        })
    }

    fn flash_bonus_dots(&mut self, d: &mut Dpy) {
        for i in 0..NUM_BONUS_DOTS {
            let (x, y, eaten) = self.bonus_dots[i];
            if eaten || self.ghost_over(x, y) {
                continue;
            }
            if self.bd_on {
                self.draw_bonus_dot(d, x, y);
            } else {
                self.clear_bonus_dot(d, x, y);
            }
        }
        if self.bd_flash_count == 0 {
            self.bd_flash_count = MAX_FLASH_COUNT;
            self.bd_on = !self.bd_on;
        } else {
            self.bd_flash_count -= 1;
        }
    }

    fn ate_bonus_dot(&mut self) -> bool {
        let Some(i) = self.is_bonus_dot(self.pacman.col, self.pacman.row) else {
            return false;
        };
        let was = self.bonus_dots[i].2;
        self.bonus_dots[i].2 = true;
        !was
    }

    fn ghost_scared(&mut self) {
        for g in self.ghosts.iter_mut() {
            if matches!(
                g.aistate,
                GhostState::GoingIn | GhostState::GoingOut | GhostState::InBox
            ) {
                continue;
            }
            g.aistate = GhostState::Hiding;
            g.flash_scared = false;
            if self.pacman.aistate != PacState::Dying {
                self.pacman.aistate = PacState::Chasing;
            }
        }
    }

    fn ghost_not_scared(&mut self) {
        for g in self.ghosts.iter_mut() {
            if matches!(
                g.aistate,
                GhostState::GoingIn | GhostState::GoingOut | GhostState::InBox
            ) {
                continue;
            }
            g.aistate = GhostState::Chasing;
        }
        if self.pacman.aistate != PacState::Dying {
            self.pacman.aistate = PacState::Eating;
        }
    }

    /// Everything that moves, once.
    fn tick(&mut self, d: &mut Dpy) {
        for ghost in 0..self.ghosts.len() {
            self.draw_ghost_sprite(d, ghost);
        }
        self.draw_pacman_sprite(d);
        self.flash_bonus_dots(d);
        if self.ate_bonus_dot() {
            self.ghost_scared_timer = random_below(100) + DEFAULT_SCARED_TIME;
            self.ghost_scared();
        }

        if self.ghost_scared_timer > 0 {
            self.ghost_scared_timer -= 1;
            if self.ghost_scared_timer == 0 {
                self.ghost_not_scared();
            } else if self.ghost_scared_timer <= START_FLASH {
                if self.flash_timer <= 0 {
                    self.flash_timer = FLASH_COUNT;
                    for g in self.ghosts.iter_mut() {
                        g.flash_scared = !g.flash_scared;
                    }
                }
                self.flash_timer -= 1;
            }
        }

        // Wait for the last death to finish playing before starting again, so
        // it is not cut off.
        let died_out = self.pacman.deaths >= 3
            && self.old_pac_state == PacState::Dying
            && self.pacman.aistate != PacState::Dying;
        if self.dotsleft == 0 || died_out {
            self.repopulate(d);
        }
        self.old_pac_state = self.pacman.aistate;
    }
}

impl Pacman {
    /// Put everyone back at the start without building a new level.
    fn reset_level(&mut self, d: &mut Dpy, n: i32, pac_init: bool) {
        d.clear_window();
        self.drawlevel(d);

        if pac_init {
            self.pacman.row = (LEVHEIGHT + JAILHEIGHT) / 2 - n;
            self.pacman.init_row = self.pacman.row;
        } else {
            self.pacman.row = self.pacman.init_row;
        }
        self.pacman.col = LEVWIDTH / 2;
        self.pacman.nextrow = NOWHERE;
        self.pacman.nextcol = NOWHERE;
        self.pacman.cf = NOWHERE;
        self.pacman.rf = NOWHERE;
        self.pacman.oldcf = NOWHERE;
        self.pacman.oldrf = NOWHERE;
        self.pacman.aistate = PacState::Eating;
        self.pacman.cur_trace = 0;
        self.pacman.roundscore = 0;
        self.pacman.speed = 4;
        self.pacman.delta = (0, 0);

        for i in 0..self.ghosts.len() {
            let g = &mut self.ghosts[i];
            g.col = LEVWIDTH / 2;
            g.row = LEVHEIGHT / 2;
            g.nextcol = NOWHERE;
            g.nextrow = NOWHERE;
            g.dead = false;
            g.lastbox = if random_below(2) == 0 { 1 } else { 3 };
            g.cf = NOWHERE;
            g.rf = NOWHERE;
            g.oldcf = NOWHERE;
            g.oldrf = NOWHERE;
            g.aistate = GhostState::InBox;
            g.timeleft = i as i32 * 20;
            g.speed = 3;
            g.delta = (0, 0);
            g.flash_scared = false;
            g.wait_pos = false;
            self.ghost_update(i);
        }
        self.pac_update();
    }

    fn repopulate(&mut self, d: &mut Dpy) {
        self.pacman.deaths = 0;
        let n = self.createnewlevel();
        self.reset_level(d, n, true);
        self.check_death(d);
    }

    fn ghost_collision(&self, ghost: usize) -> bool {
        let g = &self.ghosts[ghost];
        let p = &self.pacman;
        (g.nextrow == p.nextrow && g.nextcol == p.nextcol)
            || (g.nextrow == p.row
                && g.nextcol == p.col
                && g.row == p.nextrow
                && g.col == p.nextcol)
    }

    /// Somebody has run into somebody. A frightened ghost dies, and otherwise
    /// Pac-Man does.
    fn check_death(&mut self, d: &mut Dpy) {
        if self.pacman.aistate == PacState::Dying {
            return;
        }
        for ghost in 0..self.ghosts.len() {
            if !self.ghost_collision(ghost) {
                continue;
            }
            if self.ghosts[ghost].aistate == GhostState::GoingIn {
                continue;
            }
            if self.ghosts[ghost].aistate == GhostState::Hiding {
                self.ghosts[ghost].dead = true;
                self.ghosts[ghost].aistate = GhostState::GoingIn;
                self.ghosts[ghost].wait_pos = true;
                let black = self.mi.black;
                self.gc.set_foreground(black);
                let (cf, rf) = (self.ghosts[ghost].cf, self.ghosts[ghost].rf);
                let (w, h) = (self.spritexs, self.spriteys);
                let gc = self.gc.clone();
                d.win().fill_rectangle(&gc, cf, rf, w, h);
            } else {
                self.pacman.deaths += 1;
                self.pacman.aistate = PacState::Dying;
            }
        }
    }
}

impl Screenhack for Pacman {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        self.pacman.err.0 = (self.pacman.err.0 + 1) % self.pacman.speed;
        self.pacman.err.1 = (self.pacman.err.1 + 1) % self.pacman.speed;
        if self.pacman.err.0 != 0 {
            self.pacman.delta.0 += self.incx;
        }
        if self.pacman.err.1 != 0 {
            self.pacman.delta.1 += self.incy;
        }

        if self.pacman.delta.0 >= self.xs && self.pacman.delta.1 >= self.ys {
            self.pac_update();
            self.check_death(d);
            self.pacman.delta = (self.incx, self.incy);
        }
        self.pacman.delta.0 = self.pacman.delta.0.min(self.xs + self.incx);
        self.pacman.delta.1 = self.pacman.delta.1.min(self.ys + self.incy);

        for g in 0..self.ghosts.len() {
            let speed = self.ghosts[g].speed;
            self.ghosts[g].err.0 = (self.ghosts[g].err.0 + 1) % speed;
            self.ghosts[g].err.1 = (self.ghosts[g].err.1 + 1) % speed;
            if self.ghosts[g].err.0 != 0 {
                self.ghosts[g].delta.0 += self.incx;
            }
            if self.ghosts[g].err.1 != 0 {
                self.ghosts[g].delta.1 += self.incy;
            }
            if self.ghosts[g].delta.0 >= self.xs && self.ghosts[g].delta.1 >= self.ys {
                self.ghost_update(g);
                self.ghosts[g].delta = (self.incx, self.incy);
            }
            self.ghosts[g].delta.0 = self.ghosts[g].delta.0.min(self.xs + self.incx);
            self.ghosts[g].delta.1 = self.ghosts[g].delta.1.min(self.ys + self.incy);
        }

        self.tick(d);
        self.delay
    }

    fn reshape(&mut self, d: &mut Dpy, width: i32, height: i32) {
        self.mi.reshape(width, height);
        self.xb = (width - LEVWIDTH * self.xs) >> 1;
        self.yb = (height - LEVHEIGHT * self.ys) >> 1;
        d.clear_window();
        self.drawlevel(d);
    }

    fn event(&mut self, _d: &mut Dpy, _event: &XEvent) -> bool {
        false
    }
}

/// Cut the sprite sheet into its sixty cells and scale each to the size the
/// board wants.
fn load_pixmaps(spritexs: i32, spriteys: i32) -> (Vec<Sprite>, Option<Rc<Fb>>) {
    let Some((sheet, mask)) = png::decode(crate::images::PACMAN) else {
        return (Vec::new(), None);
    };
    let sw = sheet.width();
    let cells = (sheet.height() / sw.max(1)) as usize;

    let mut out = Vec::with_capacity(cells);
    for i in 0..cells {
        let y = i as i32 * sw;
        let pix = scale_pixmap(&sheet.sub_image(0, y, sw, sw), spritexs, spriteys);
        let m = mask
            .as_ref()
            .map(|m| Rc::new(scale_pixmap(&m.sub_image(0, y, sw, sw), spritexs, spriteys)));
        out.push(Sprite { pix, mask: m });
    }
    // Every ghost is erased through the first cells outline, which is the one
    // shape they all share.
    let ghost_mask = out.first().and_then(|s| s.mask.clone());
    (out, ghost_mask)
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mi = ModeInfo::new(d, ColorScheme::Uniform);
    let (width, height) = (d.width(), d.height());
    let size = mi.size;

    let ys = if size == 0 || MINGRIDSIZE * size > width || MINGRIDSIZE * size > height {
        let mut scale = f64::from((width / LEVWIDTH).min(height / LEVHEIGHT));
        if width > height * 5 || height > width * 5 {
            // A very odd window shape.
            scale = 0.8
                * if width / height != 0 {
                    f64::from(width) / f64::from(height)
                } else {
                    f64::from(height) / f64::from(width)
                };
        }
        (scale as i32).max(1)
    } else if size < -MINSIZE {
        random_below((-size).min(MINSIZE.max(width.min(height) / MINGRIDSIZE)) - MINSIZE + 1)
            + MINSIZE
    } else if size < MINSIZE {
        MINSIZE
    } else {
        size.min(MINSIZE.max(width.min(height) / MINGRIDSIZE))
    };
    let xs = ys;

    let spritexs = (xs + (xs >> 1) - 1).max(1);
    let spriteys = (ys + (ys >> 1) - 1).max(1);
    let (sprites, ghost_mask) = load_pixmaps(spritexs, spriteys);

    // The sheet is one column of squares: four ghosts by four directions by
    // two leg positions, the scared ghost and its flash, the eyes, Pac-Man,
    // and his death.
    let ng = GHOSTS * MAXGDIR * MAXGWAG;
    let ns = MAXGFLASH * MAXGWAG;
    let mut at = 0;
    let take = |at: &mut usize, n: usize| -> Vec<Sprite> {
        let v: Vec<Sprite> = sprites.iter().skip(*at).take(n).cloned().collect();
        *at += n;
        v
    };
    let ghost_pixmap = take(&mut at, ng);
    let scared_ghost = take(&mut at, ns);
    let ghost_eyes = take(&mut at, MAXGDIR);
    let pacman_pixmap = take(&mut at, 4 * MAXMOUTH);
    let pacman_death = take(&mut at, PAC_DEATH_FRAMES);

    let black = mi.black;
    let mut st = Pacman {
        gc: Gc::new(black, black),
        xs,
        ys,
        xb: (width - LEVWIDTH * xs) >> 1,
        yb: (height - LEVHEIGHT * ys) >> 1,
        incx: (xs >> 3) + 1,
        incy: (ys >> 3) + 1,
        wallwidth: ((xs + ys) >> 4).max(1),
        spritexs,
        spriteys,
        spritedx: (xs - spritexs) >> 1,
        spritedy: (ys - spriteys) >> 1,
        mi,
        level: vec![BLOCK_EMPTY; (LEVWIDTH * LEVHEIGHT) as usize],
        dotsleft: 0,
        bonus_dots: [(0, 0, false); NUM_BONUS_DOTS],
        pacman: Pac {
            lastbox: if random_below(2) == 0 { 1 } else { 3 },
            ..Pac::default()
        },
        ghosts: (0..GHOSTS).map(|_| Ghost::default()).collect(),
        ghost_pixmap,
        ghost_mask,
        scared_ghost,
        ghost_eyes,
        pacman_pixmap,
        pacman_death,
        pm_mouth: 0,
        pm_mouth_delay: 0,
        pm_open_mouth: false,
        pm_death_frame: 0,
        pm_death_delay: 0,
        gh_wag: 0,
        gh_wag_count: 0,
        bd_flash_count: 0,
        bd_on: true,
        ghost_scared_timer: 0,
        flash_timer: 0,
        old_pac_state: PacState::Chasing,
        delay: d.res.int("delay").max(0) as u32,
    };
    st.repopulate(d);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:   10000",
    "*size:    0",
    "*ncolors: 6",
    "*fpsTop: true",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::spin("size", "Player size", 0.0, 200.0, "0"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "pacman",
    label: "Pac-Man",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Edwin de Jong and Jamie Zawinski",
        year: "2004",
        video: Some("https://www.youtube.com/watch?v=G-pdjis0ECw"),
        blurb: "A game of Pac-Man on a randomly-created level, played by nobody.",
    },
};

/// The savers entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
