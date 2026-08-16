//! The drone's internal map, packed four fields to the byte.
//!
//! A field is one of four states, so it fits in two bits, and a 64x64 map fills
//! the 328p's 1 KiB EEPROM exactly. The port keeps the packing rather than
//! spending a byte a field, because the packing is the reason the map is the
//! size it is.
//!
//! Two guards are tighter here than in the C. `map_write` there checked only
//! that the byte address was inside the EEPROM, so a field one column past the
//! right-hand edge silently overwrote the first column of the row above; and
//! `map_read` checked nothing at all. Both bounds-check the coordinate now. No
//! caller relied on the wrap: the navigator only writes through
//! [`align_to_map`](crate::search::align_to_map), which already refuses a
//! coordinate off the map.

use alloc::string::String;

use crate::io::EEPROM_SIZE;
use crate::log::{Log, Sender};

/// The map's height, in fields.
pub const MAP_HEIGHT: u8 = 64;
/// The map's width, in fields.
pub const MAP_WIDTH: u8 = 64;
/// How many centimetres of room one field covers, on a side.
pub const CENTIMETERS_PR_FIELD: u16 = 25;
/// The map's height in centimetres.
pub const MAP_CENTI_HEIGHT: u16 = MAP_HEIGHT as u16 * CENTIMETERS_PR_FIELD;
/// The map's width in centimetres.
pub const MAP_CENTI_WIDTH: u16 = MAP_WIDTH as u16 * CENTIMETERS_PR_FIELD;
/// How many fields are packed into one byte.
pub const FIELDS_PER_BYTE: usize = 4;
/// The largest map the board can hold.
pub const MAX_MAP_SIZE: usize = EEPROM_SIZE * FIELDS_PER_BYTE;
/// The bits one field occupies.
const FIELD_SIZE: u32 = 2;
/// A field's worth of set bits.
const FULL_FIELD: u8 = 0b11;

/// The character an unvisited field is written as in a text map.
pub const CHAR_UNVISITED: char = ' ';
/// The character a visited field is written as in a text map.
pub const CHAR_VISITED: char = '\'';
/// The character a wall is written as in a text map.
pub const CHAR_WALL: char = '#';
/// The character a window is written as in a text map.
pub const CHAR_TRANSPARENT: char = '&';

/// What the drone believes is at a field.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FieldState {
    /// Never been there, never seen it.
    #[default]
    Unvisited = 0,
    /// Flown through.
    Visited = 1,
    /// Something solid: the laser came back off it.
    Wall = 2,
    /// Something the laser goes through but the sonar does not.
    Window = 3,
}

impl FieldState {
    /// Reads back the two bits of a packed field.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        match bits & FULL_FIELD {
            0 => Self::Unvisited,
            1 => Self::Visited,
            2 => Self::Wall,
            _ => Self::Window,
        }
    }

    /// The character used for this state in a text map.
    #[must_use]
    pub const fn to_char(self) -> char {
        match self {
            Self::Unvisited => CHAR_UNVISITED,
            Self::Visited => CHAR_VISITED,
            Self::Wall => CHAR_WALL,
            Self::Window => CHAR_TRANSPARENT,
        }
    }

    /// The state a text map's character stands for.
    #[must_use]
    pub const fn from_char(c: char) -> Option<Self> {
        match c {
            CHAR_UNVISITED => Some(Self::Unvisited),
            CHAR_VISITED => Some(Self::Visited),
            CHAR_WALL => Some(Self::Wall),
            CHAR_TRANSPARENT => Some(Self::Window),
            _ => None,
        }
    }
}

/// A field on the map, and whether it is on the map at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct MapCoord {
    pub x: u8,
    pub y: u8,
    /// False when the world position it came from is off the map. The caller
    /// is expected to check this before writing.
    pub valid: bool,
}

/// The map, and the EEPROM it lives in.
#[derive(Clone)]
pub struct Map {
    width: u8,
    height: u8,
    eeprom: [u8; EEPROM_SIZE],
}

impl Map {
    /// A map of the given size. `clean` zeroes the store, as the `CLEAN` flag
    /// does on the board; a map is all zeroes to begin with here either way,
    /// since a fresh array is not a used EEPROM.
    #[must_use]
    pub fn new(width: u8, height: u8, log: &mut Log) -> Self {
        if usize::from(width) * usize::from(height) > MAX_MAP_SIZE {
            log.serious_warning(Sender::Map, "map_init: Map size too big!");
        }

        Self {
            width,
            height,
            eeprom: [0; EEPROM_SIZE],
        }
    }

    /// The map the firmware runs with.
    #[must_use]
    pub fn full(log: &mut Log) -> Self {
        Self::new(MAP_WIDTH, MAP_HEIGHT, log)
    }

    /// The map's width in fields.
    #[must_use]
    pub const fn width(&self) -> u8 {
        self.width
    }

    /// The map's height in fields.
    #[must_use]
    pub const fn height(&self) -> u8 {
        self.height
    }

    /// Where a field sits in the store: byte address, and bit offset in it.
    fn address(&self, x: u8, y: u8) -> Option<(usize, u32)> {
        if x >= self.width || y >= self.height {
            return None;
        }

        let index = usize::from(y) * usize::from(self.width) + usize::from(x);
        let addr = index / FIELDS_PER_BYTE;

        if addr >= EEPROM_SIZE {
            return None;
        }

        Some((addr, (index % FIELDS_PER_BYTE) as u32 * FIELD_SIZE))
    }

    /// Records what is at a field. Off-map coordinates are refused.
    pub fn write(&mut self, x: u8, y: u8, state: FieldState) {
        let Some((addr, offset)) = self.address(x, y) else {
            return;
        };

        let mut byte = self.eeprom[addr];
        byte &= !(FULL_FIELD << offset);
        byte |= (state as u8) << offset;
        self.eeprom[addr] = byte;
    }

    /// Reads a field. Off-map coordinates read as unvisited.
    #[must_use]
    pub fn read(&self, x: u8, y: u8) -> FieldState {
        let Some((addr, offset)) = self.address(x, y) else {
            return FieldState::Unvisited;
        };

        FieldState::from_bits(self.eeprom[addr] >> offset)
    }

    /// Blanks the whole map.
    pub fn clean(&mut self) {
        self.eeprom = [0; EEPROM_SIZE];
    }

    /// Draws a straight run of fields between two coordinates.
    ///
    /// The C did this with Bresenham over `uint8_t` deltas and an unsigned
    /// error term, which cannot represent the negative error the algorithm
    /// needs and walks off in the wrong direction for most slopes. Its one
    /// caller was commented out. This is the same algorithm over signed
    /// arithmetic, which is what it was meant to be.
    pub fn write_line(&mut self, start: MapCoord, end: MapCoord, state: FieldState) {
        let mut x = i32::from(start.x);
        let mut y = i32::from(start.y);
        let target_x = i32::from(end.x);
        let target_y = i32::from(end.y);

        let dx = (target_x - x).abs();
        let dy = -(target_y - y).abs();
        let step_x = if x < target_x { 1 } else { -1 };
        let step_y = if y < target_y { 1 } else { -1 };
        let mut error = dx + dy;

        loop {
            self.write(x as u8, y as u8, state);

            if x == target_x && y == target_y {
                return;
            }

            let doubled = 2 * error;
            if doubled >= dy {
                error += dy;
                x += step_x;
            }
            if doubled <= dx {
                error += dx;
                y += step_y;
            }
        }
    }

    /// The map as text, north up: the first line is the highest row. The C sent
    /// this out of the serial port a row at a time, lowest row first;
    /// [`Map::parse`] and this are inverses of each other, which the C pair
    /// were not.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out =
            String::with_capacity((usize::from(self.width) + 3) * usize::from(self.height));

        for row in (0..self.height).rev() {
            out.push('|');
            for column in 0..self.width {
                out.push(self.read(column, row).to_char());
            }
            out.push('|');
            out.push('\n');
        }

        out
    }

    /// Fills the map in from text, north up: the first line is the highest row,
    /// as the simulator's minimap loader read it.
    ///
    /// Returns the offending character if the text holds one that is not a
    /// field. Lines longer than the map, and lines past its top, are ignored.
    pub fn parse(&mut self, text: &str) -> Result<(), char> {
        self.clean();

        for (index, line) in text.lines().enumerate() {
            let Some(row) = usize::from(self.height).checked_sub(index + 1) else {
                break;
            };

            for (column, c) in line.chars().enumerate() {
                let state = FieldState::from_char(c).ok_or(c)?;
                if column < usize::from(self.width) {
                    self.write(column as u8, row as u8, state);
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> Map {
        Map::full(&mut Log::default())
    }

    #[test]
    fn a_field_survives_a_round_trip() {
        let mut map = map();
        map.write(3, 4, FieldState::Wall);
        assert_eq!(map.read(3, 4), FieldState::Wall);
        assert_eq!(map.read(4, 4), FieldState::Unvisited);
    }

    #[test]
    fn neighbouring_fields_share_a_byte_without_treading_on_each_other() {
        let mut map = map();
        for (x, state) in [
            FieldState::Unvisited,
            FieldState::Visited,
            FieldState::Wall,
            FieldState::Window,
        ]
        .into_iter()
        .enumerate()
        {
            map.write(x as u8, 0, state);
        }

        assert_eq!(map.read(0, 0), FieldState::Unvisited);
        assert_eq!(map.read(1, 0), FieldState::Visited);
        assert_eq!(map.read(2, 0), FieldState::Wall);
        assert_eq!(map.read(3, 0), FieldState::Window);
    }

    #[test]
    fn a_field_can_be_overwritten() {
        let mut map = map();
        map.write(9, 9, FieldState::Window);
        map.write(9, 9, FieldState::Visited);
        assert_eq!(map.read(9, 9), FieldState::Visited);
    }

    #[test]
    fn a_write_past_the_edge_does_not_land_on_the_next_row() {
        let mut map = map();
        map.write(MAP_WIDTH, 0, FieldState::Wall);
        assert_eq!(map.read(0, 1), FieldState::Unvisited);
        assert_eq!(map.read(MAP_WIDTH, 0), FieldState::Unvisited);
    }

    #[test]
    fn the_map_fills_the_eeprom_exactly() {
        assert_eq!(
            usize::from(MAP_WIDTH) * usize::from(MAP_HEIGHT) / FIELDS_PER_BYTE,
            EEPROM_SIZE
        );
    }

    #[test]
    fn text_round_trips() {
        let mut map = Map::new(4, 3, &mut Log::default());
        map.parse("####\n#  #\n#'&#").unwrap();

        assert_eq!(map.read(0, 2), FieldState::Wall);
        assert_eq!(map.read(1, 1), FieldState::Unvisited);
        assert_eq!(map.read(1, 0), FieldState::Visited);
        assert_eq!(map.read(2, 0), FieldState::Window);
        assert_eq!(map.render(), "|####|\n|#  #|\n|#'&#|\n");
    }

    #[test]
    fn a_stray_character_is_reported() {
        let mut map = map();
        assert_eq!(map.parse("##x##"), Err('x'));
    }

    #[test]
    fn a_line_runs_between_its_ends() {
        let mut map = map();
        map.write_line(
            MapCoord {
                x: 0,
                y: 0,
                valid: true,
            },
            MapCoord {
                x: 4,
                y: 2,
                valid: true,
            },
            FieldState::Visited,
        );

        assert_eq!(map.read(0, 0), FieldState::Visited);
        assert_eq!(map.read(4, 2), FieldState::Visited);
        // Every column between the ends is touched exactly once.
        let touched = (0..=4)
            .filter(|&x| (0..=2).any(|y| map.read(x, y) == FieldState::Visited))
            .count();
        assert_eq!(touched, 5);
    }

    #[test]
    fn cleaning_blanks_everything() {
        let mut map = map();
        map.write(1, 1, FieldState::Wall);
        map.clean();
        assert_eq!(map.read(1, 1), FieldState::Unvisited);
    }
}
