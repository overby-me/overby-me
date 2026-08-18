//! Turning world positions into map fields, and finding a way between them.
//!
//! [`align_to_map`] is the load-bearing half: the navigator calls it every
//! round to place itself and whatever its sensors found onto the map.
//!
//! The path search is the other half, and the C version of it does not run. It
//! dereferences two uninitialized pointers (`*current = lowestf(search)` with
//! `current` never pointed anywhere), builds its four neighbours by writing
//! through `coords->` four times, which is `coords[0]` every time, so three of
//! them stay where they started, scans its open set with a loop that decrements
//! the index it tests against a growing bound, and refers to a `search_data`
//! that is not in scope. Its one caller, `on_searching`, is an empty function,
//! which is presumably why none of that was ever noticed.
//!
//! So this is the A* the module describes rather than a transcription of the
//! code: the same sliced budget, sets and scores, with the neighbour walk,
//! the heuristic and the open-set scan written to work. Three specifics differ
//! and are called out where they are made: neighbours are the four orthogonal
//! fields, the estimate is a Manhattan distance, and a field the drone believes
//! is solid is not walked through.

use alloc::vec::Vec;

use crate::map::{CENTIMETERS_PR_FIELD, FieldState, MAP_HEIGHT, MAP_WIDTH, Map, MapCoord};

/// The lowest world y-coordinate the map covers, in centimetres.
pub const LOWEST_Y_ORG: u16 = 0;
/// One past the highest world y-coordinate the map covers, in centimetres.
pub const HIGHEST_Y_ORG: u16 = MAP_HEIGHT as u16 * CENTIMETERS_PR_FIELD;
/// The lowest world x-coordinate the map covers, in centimetres.
pub const LOWEST_X_ORG: u16 = 0;
/// One past the highest world x-coordinate the map covers, in centimetres.
pub const HIGHEST_X_ORG: u16 = MAP_WIDTH as u16 * CENTIMETERS_PR_FIELD;

/// How many fields the search may expand per scheduled slice. The navigator
/// runs on a 100 ms cycle and cannot spend all of it here.
pub const ALLOWED_SEARCH_ITERATIONS: usize = 10;

/// Rounds a world position in centimetres onto the field that holds it.
///
/// The C computed the field index in `int` and then stored it in a `uint8_t`,
/// so a position more than 6375 cm out wrapped into a field index that looked
/// perfectly valid and let the drone write over a part of the map it was
/// nowhere near. The check happens before the narrowing here.
#[must_use]
pub fn align_to_map(x_coord: u16, y_coord: u16) -> MapCoord {
    let (x, x_valid) = align_axis(x_coord, LOWEST_X_ORG, MAP_WIDTH);
    let (y, y_valid) = align_axis(y_coord, LOWEST_Y_ORG, MAP_HEIGHT);

    MapCoord {
        x,
        y,
        valid: x_valid && y_valid,
    }
}

/// Rounds one axis to the nearest field, and reports whether it landed on the
/// map. Half a field or more into the next one rounds up, as in the C.
fn align_axis(coord: u16, lowest: u16, fields: u8) -> (u8, bool) {
    let offset = coord.wrapping_sub(lowest);
    let mut field = offset / CENTIMETERS_PR_FIELD;

    if offset % CENTIMETERS_PR_FIELD >= CENTIMETERS_PR_FIELD / 2 {
        field += 1;
    }

    if field >= u16::from(fields) {
        return (fields, false);
    }

    (field as u8, true)
}

/// Which set a node is in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Set {
    /// Still worth expanding.
    Open,
    /// Expanded already.
    Closed,
}

/// A field the search has reached.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SearchNode {
    /// Where on the map.
    pub pos: MapCoord,
    /// Which closed node this was reached from, if any.
    pub parent: Option<usize>,
    /// Fields walked to get here.
    ///
    /// The C held the scores in a `uint8_t`, which a path across a 64x64 map
    /// can overrun; they are 16 bits here.
    pub gscore: u16,
    /// `gscore` plus the estimate of what is left.
    pub fscore: u16,
}

/// How a slice of searching ended.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SearchStep {
    /// The budget ran out with the goal not yet reached. Call again.
    Working,
    /// A path, from the start to the goal inclusive.
    Found(Vec<MapCoord>),
    /// Everything reachable has been expanded and the goal was not among it.
    Unreachable,
}

/// A path search in progress.
///
/// The sets are kept as flat vectors, as the C kept them, rather than as a heap
/// and a hash set: the map is 4096 fields and the board has 2 KiB of RAM, so a
/// linear scan of a short list is the cheaper structure.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Search {
    /// Nodes still to expand.
    pub open_set: Vec<SearchNode>,
    /// Nodes already expanded. Parent indices point in here.
    pub closed_set: Vec<SearchNode>,
    /// Where the search is trying to get to.
    pub goal: MapCoord,
    /// Whether a search is under way.
    pub active: bool,
}

impl Search {
    /// An idle search.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts looking for a way from `start` to `goal`, throwing away whatever
    /// was under way before.
    pub fn begin(&mut self, start: MapCoord, goal: MapCoord) {
        self.open_set.clear();
        self.closed_set.clear();
        self.goal = goal;
        self.active = true;

        self.open_set.push(SearchNode {
            pos: start,
            parent: None,
            gscore: 0,
            fscore: estimate(start, goal),
        });
    }

    /// Expands up to [`ALLOWED_SEARCH_ITERATIONS`] fields.
    ///
    /// Fields the drone believes are solid are not expanded, so a path never
    /// runs through a wall or a window. The C never consulted the map at all,
    /// which would have let it plan straight through the room's outer wall.
    pub fn step(&mut self, map: &Map) -> SearchStep {
        if !self.active {
            return SearchStep::Unreachable;
        }

        for _ in 0..ALLOWED_SEARCH_ITERATIONS {
            let Some(best) = self.lowest_f() else {
                self.active = false;
                return SearchStep::Unreachable;
            };

            let current = self.open_set.swap_remove(best);

            if current.pos.x == self.goal.x && current.pos.y == self.goal.y {
                self.closed_set.push(current);
                self.active = false;
                return SearchStep::Found(self.reconstruct(self.closed_set.len() - 1));
            }

            self.closed_set.push(current);
            self.add_neighbours(map, self.closed_set.len() - 1);
        }

        SearchStep::Working
    }

    /// The index of the open node with the lowest estimated total cost.
    fn lowest_f(&self) -> Option<usize> {
        self.open_set
            .iter()
            .enumerate()
            .min_by_key(|(_, node)| node.fscore)
            .map(|(index, _)| index)
    }

    /// Opens the four fields orthogonally next to a closed one.
    ///
    /// The C meant to take the four diagonals, as far as its dead code shows.
    /// Orthogonal neighbours are what the heuristic below measures, and what a
    /// drone that flies along walls actually moves between.
    fn add_neighbours(&mut self, map: &Map, parent: usize) {
        let node = self.closed_set[parent];
        let gscore = node.gscore + 1;

        for (dx, dy) in [(0i16, 1i16), (1, 0), (0, -1), (-1, 0)] {
            let x = i16::from(node.pos.x) + dx;
            let y = i16::from(node.pos.y) + dy;

            if x < 0 || y < 0 || x >= i16::from(MAP_WIDTH) || y >= i16::from(MAP_HEIGHT) {
                continue;
            }

            let pos = MapCoord {
                x: x as u8,
                y: y as u8,
                valid: true,
            };

            if matches!(
                map.read(pos.x, pos.y),
                FieldState::Wall | FieldState::Window
            ) {
                continue;
            }

            if self.contains(pos, Set::Closed) {
                continue;
            }

            match self
                .open_set
                .iter_mut()
                .find(|open| open.pos.x == pos.x && open.pos.y == pos.y)
            {
                // Already open, and no better this way round.
                Some(open) if gscore >= open.gscore => {}
                Some(open) => {
                    open.gscore = gscore;
                    open.fscore = gscore + estimate(pos, self.goal);
                    open.parent = Some(parent);
                }
                None => self.open_set.push(SearchNode {
                    pos,
                    parent: Some(parent),
                    gscore,
                    fscore: gscore + estimate(pos, self.goal),
                }),
            }
        }
    }

    /// Whether a field is in one of the sets.
    #[must_use]
    pub fn contains(&self, pos: MapCoord, set: Set) -> bool {
        let nodes = match set {
            Set::Open => &self.open_set,
            Set::Closed => &self.closed_set,
        };

        nodes
            .iter()
            .any(|node| node.pos.x == pos.x && node.pos.y == pos.y)
    }

    /// Walks the parent chain back from a closed node to the start.
    fn reconstruct(&self, from: usize) -> Vec<MapCoord> {
        let mut path = Vec::new();
        let mut cursor = Some(from);

        while let Some(index) = cursor {
            path.push(self.closed_set[index].pos);
            cursor = self.closed_set[index].parent;
        }

        path.reverse();
        path
    }
}

/// How far a field is from the goal, at best: the Manhattan distance, since
/// the search moves one field at a time in one of four directions.
///
/// The C summed the two axis differences and took the absolute value of the
/// sum, so a field two north and two east of the goal estimated as zero and the
/// search would have wandered off along that diagonal.
#[must_use]
pub fn estimate(node: MapCoord, goal: MapCoord) -> u16 {
    let dx = i32::from(node.x) - i32::from(goal.x);
    let dy = i32::from(node.y) - i32::from(goal.y);

    (dx.unsigned_abs() + dy.unsigned_abs()) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::Log;

    fn coord(x: u8, y: u8) -> MapCoord {
        MapCoord { x, y, valid: true }
    }

    #[test]
    fn a_position_lands_on_its_field() {
        assert_eq!(align_to_map(0, 0), coord(0, 0));
        // Under half a field along, so still field zero. Half of 25 is 12,
        // rounded down, so that is where the boundary sits.
        assert_eq!(align_to_map(11, 11), coord(0, 0));
        // Half a field along rounds up.
        assert_eq!(align_to_map(12, 12), coord(1, 1));
        assert_eq!(align_to_map(800, 800), coord(32, 32));
    }

    #[test]
    fn a_position_off_the_map_is_marked_invalid() {
        assert!(!align_to_map(HIGHEST_X_ORG, 0).valid);
        assert!(!align_to_map(0, HIGHEST_Y_ORG).valid);
        assert!(align_to_map(HIGHEST_X_ORG - CENTIMETERS_PR_FIELD, 0).valid);
    }

    #[test]
    fn a_position_far_off_the_map_does_not_wrap_back_onto_it() {
        // 6400 cm is field 256, which is 0 in a byte. The C called that valid.
        assert!(!align_to_map(6400, 800).valid);
        assert!(!align_to_map(65000, 800).valid);
    }

    #[test]
    fn the_estimate_is_a_manhattan_distance() {
        assert_eq!(estimate(coord(0, 0), coord(3, 4)), 7);
        // Equal and opposite offsets: the C's version made this zero.
        assert_eq!(estimate(coord(2, 0), coord(0, 2)), 4);
    }

    #[test]
    fn a_path_is_found_across_an_empty_map() {
        let map = Map::full(&mut Log::default());
        let mut search = Search::new();
        search.begin(coord(1, 1), coord(4, 5));

        let path = loop {
            match search.step(&map) {
                SearchStep::Working => {}
                SearchStep::Found(path) => break path,
                SearchStep::Unreachable => panic!("no path across an empty map"),
            }
        };

        assert_eq!(path.first().map(|c| (c.x, c.y)), Some((1, 1)));
        assert_eq!(path.last().map(|c| (c.x, c.y)), Some((4, 5)));
        // Three east and four north, plus the field it started on.
        assert_eq!(path.len(), 8);
    }

    #[test]
    fn a_path_goes_around_a_wall_rather_than_through_it() {
        let mut map = Map::full(&mut Log::default());
        for y in 0..8 {
            map.write(3, y, FieldState::Wall);
        }

        let mut search = Search::new();
        search.begin(coord(1, 1), coord(5, 1));

        let path = loop {
            match search.step(&map) {
                SearchStep::Working => {}
                SearchStep::Found(path) => break path,
                SearchStep::Unreachable => panic!("the wall does not reach the top"),
            }
        };

        assert!(path.iter().all(|c| map.read(c.x, c.y) != FieldState::Wall));
        // Over the top of the wall, so longer than the four fields across.
        assert!(path.len() > 5);
    }

    #[test]
    fn a_walled_in_goal_is_reported_unreachable() {
        let mut map = Map::full(&mut Log::default());
        for (x, y) in [(9, 10), (11, 10), (10, 9), (10, 11)] {
            map.write(x, y, FieldState::Wall);
        }

        let mut search = Search::new();
        search.begin(coord(1, 1), coord(10, 10));

        loop {
            match search.step(&map) {
                SearchStep::Working => {}
                SearchStep::Found(_) => panic!("there is no way in"),
                SearchStep::Unreachable => break,
            }
        }
    }

    #[test]
    fn searching_is_sliced_across_calls() {
        let map = Map::full(&mut Log::default());
        let mut search = Search::new();
        search.begin(coord(0, 0), coord(40, 40));

        assert_eq!(search.step(&map), SearchStep::Working);
        assert!(search.closed_set.len() <= ALLOWED_SEARCH_ITERATIONS);
        assert!(search.active);
    }
}
