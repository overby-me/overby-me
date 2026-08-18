//! Randsim: the room the Randie firmware is flown in.
//!
//! A port of the C++ simulator at <https://github.com/prozum/randie>, minus its
//! SDL front end. The room is a grid of 25 cm blocks, the drone is a point with
//! a heading and a height, and the sensors are fans of rays cast against the
//! blocks. Every tick the sensors are read into the firmware's own structs, the
//! navigator is run on its 100 ms period, and the drone is moved by whatever
//! the flight controller was left set to.
//!
//! Nothing here draws anything. The frontend reads [`Simulator::blocks`],
//! [`Simulator::drone`] and the firmware's map, and puts them on a canvas.
//!
//! ```
//! use randie_sim::{DEFAULT_MAP, Simulator};
//!
//! let mut sim = Simulator::with_map(DEFAULT_MAP).unwrap();
//! for _ in 0..1000 {
//!     sim.step();
//! }
//! // Ten seconds of simulated time later, the drone has been somewhere.
//! assert!(sim.time >= 10_000);
//! ```

pub mod block;
pub mod drone;
pub mod ray;
pub mod sensors;
pub mod vector;

pub use block::{Block, BlockType};
pub use drone::Drone;
pub use ray::Ray;
pub use vector::Vec2;

/// The room the simulator opens in: two rooms and a corridor, with a run of
/// windows down the right-hand wall.
pub const DEFAULT_MAP: &str = include_str!("../maps/default.txt");

/// Where the drone starts, in centimetres.
pub const DRONE_START: Vec2 = Vec2::new(80.0, -90.0);

/// How much simulated time one tick covers, in milliseconds.
pub const DELTA_TIME: u32 = 10;

/// The room, the drone, and the clock.
#[derive(Clone, Debug)]
pub struct Simulator {
    /// Everything the drone can run into.
    pub blocks: Vec<Block>,
    /// The drone, and the firmware flying it.
    pub drone: Drone,
    /// How long the simulation has been running, in milliseconds.
    pub time: u32,
    /// How much time one tick covers, in milliseconds.
    pub delta_time: u32,
}

impl Simulator {
    /// An empty room with the drone in it.
    #[must_use]
    pub fn new() -> Self {
        Self {
            blocks: Vec::new(),
            drone: Drone::new(DRONE_START, drone::DRONE_SIZE),
            time: 0,
            delta_time: DELTA_TIME,
        }
    }

    /// A room laid out from a text map. Returns the offending character if the
    /// text holds one that is not a block.
    pub fn with_map(text: &str) -> Result<Self, char> {
        let mut sim = Self::new();
        sim.load_map(text)?;
        Ok(sim)
    }

    /// Lays the room out from a text map, replacing whatever was there.
    ///
    /// The first line is the top row. A `#` is a wall, a `&` a window, a space
    /// nothing at all; the first block's centre sits half a block in from the
    /// origin, and the room extends east and south from there.
    pub fn load_map(&mut self, text: &str) -> Result<(), char> {
        let half = Block::SIZE / 2.0;
        let mut blocks = Vec::new();
        let mut x = half;
        let mut y = -half;

        for c in text.chars() {
            match c {
                '#' => {
                    blocks.push(Block::new(Vec2::new(x, y), BlockType::Wall));
                    x += Block::SIZE;
                }
                '&' => {
                    blocks.push(Block::new(Vec2::new(x, y), BlockType::Window));
                    x += Block::SIZE;
                }
                ' ' => x += Block::SIZE,
                '\n' => {
                    y -= Block::SIZE;
                    x = half;
                }
                // A text map written on Windows, which the C would have
                // rejected outright.
                '\r' => {}
                other => return Err(other),
            }
        }

        self.blocks = blocks;
        Ok(())
    }

    /// One tick: move everything, then advance the clock.
    pub fn step(&mut self) {
        self.drone.update(&self.blocks, self.time, self.delta_time);
        self.time = self.time.wrapping_add(self.delta_time);
    }

    /// Runs `ticks` ticks.
    pub fn run(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.step();
        }
    }

    /// Puts the drone back where it started with a blank map and a firmware
    /// that has forgotten everything, and restarts the clock. The room is left
    /// as it is.
    pub fn reset(&mut self) {
        self.drone.reset(DRONE_START);
        self.time = 0;
    }

    /// Which block covers a point, if any.
    #[must_use]
    pub fn block_at(&self, point: Vec2) -> Option<usize> {
        self.blocks.iter().position(|block| block.contains(point))
    }

    /// The centre of the grid square a point falls in.
    ///
    /// Rounds towards the origin, as the C does, so the four squares around
    /// the origin each keep their own corner of it.
    #[must_use]
    pub fn snap_to_grid(point: Vec2) -> Vec2 {
        let half = Block::SIZE / 2.0;

        Vec2::new(
            (point.x / Block::SIZE).trunc() * Block::SIZE
                + if point.x >= 0.0 { half } else { -half },
            (point.y / Block::SIZE).trunc() * Block::SIZE
                + if point.y >= 0.0 { half } else { -half },
        )
    }

    /// Puts a block on the square a point falls in, unless one is there
    /// already. Returns whether anything was added.
    pub fn place_block(&mut self, point: Vec2, kind: BlockType) -> bool {
        if self.block_at(point).is_some() {
            return false;
        }

        self.blocks
            .push(Block::new(Self::snap_to_grid(point), kind));
        true
    }

    /// Takes away whatever block covers a point. Returns whether anything was
    /// taken away.
    pub fn remove_block(&mut self, point: Vec2) -> bool {
        let before = self.blocks.len();
        self.blocks.retain(|block| !block.contains(point));
        self.blocks.len() != before
    }
}

impl Default for Simulator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use randie_firmware::map::FieldState;
    use randie_firmware::nav::Task;

    use super::*;

    #[test]
    fn the_default_room_is_laid_out_from_its_top_left_corner() {
        let sim = Simulator::with_map(DEFAULT_MAP).unwrap();

        // The first character of the first line is the top-left block.
        assert!(sim.block_at(Vec2::new(1.0, -1.0)).is_some());
        // The room is open in the middle.
        assert!(sim.block_at(Vec2::new(100.0, -100.0)).is_none());
        // And there are windows down the right-hand side of the lower room.
        assert!(
            sim.blocks
                .iter()
                .any(|block| block.kind == BlockType::Window)
        );
    }

    #[test]
    fn a_map_with_a_stray_character_is_refused() {
        assert_eq!(Simulator::with_map("##x##").err(), Some('x'));
    }

    #[test]
    fn a_block_can_be_placed_and_taken_away() {
        let mut sim = Simulator::new();
        let point = Vec2::new(60.0, -60.0);

        assert!(sim.place_block(point, BlockType::Wall));
        assert_eq!(sim.blocks.len(), 1);
        // The square is taken now.
        assert!(!sim.place_block(point, BlockType::Wall));

        assert!(sim.remove_block(point));
        assert!(sim.blocks.is_empty());
        assert!(!sim.remove_block(point));
    }

    #[test]
    fn a_placed_block_lands_on_the_grid() {
        assert_eq!(
            Simulator::snap_to_grid(Vec2::new(60.0, -60.0)),
            Vec2::new(62.5, -62.5)
        );
        assert_eq!(
            Simulator::snap_to_grid(Vec2::new(1.0, 1.0)),
            Vec2::new(12.5, 12.5)
        );
        assert_eq!(
            Simulator::snap_to_grid(Vec2::new(-1.0, -1.0)),
            Vec2::new(-12.5, -12.5)
        );
    }

    #[test]
    fn the_clock_runs_at_the_stated_rate() {
        let mut sim = Simulator::new();
        sim.run(100);
        assert_eq!(sim.time, 1000);
    }

    #[test]
    fn the_drone_sets_off_and_finds_the_wall_in_front_of_it() {
        let mut sim = Simulator::with_map(DEFAULT_MAP).unwrap();

        // It starts idle and facing east, with nothing close in front. The
        // navigator first runs a tenth of a second in, and sets it going.
        assert_eq!(sim.drone.firmware.nav.task, Task::Idle);
        sim.run(10);
        assert_eq!(sim.drone.firmware.nav.task, Task::Idle);
        sim.run(1);
        assert_eq!(sim.drone.firmware.nav.task, Task::MoveForward);

        // A hundred seconds is long enough to cross the room and start
        // working along a wall.
        sim.run(10_000);
        assert!(
            matches!(
                sim.drone.firmware.nav.task,
                Task::FollowForward | Task::FollowFurther | Task::FollowCheck | Task::FollowTurn
            ),
            "{:?}",
            sim.drone.firmware.nav.task
        );
    }

    #[test]
    fn the_drone_maps_the_walls_it_flies_past() {
        let mut sim = Simulator::with_map(DEFAULT_MAP).unwrap();
        sim.run(10_000);

        let mut walls = 0;
        let mut visited = 0;
        for y in 0..64 {
            for x in 0..64 {
                match sim.drone.firmware.map.read(x, y) {
                    FieldState::Wall => walls += 1,
                    FieldState::Visited => visited += 1,
                    _ => {}
                }
            }
        }

        assert!(walls > 0, "nothing was mapped");
        assert!(visited > 0, "the drone never recorded where it had been");
    }

    #[test]
    fn the_drone_stays_inside_the_room() {
        let mut sim = Simulator::with_map(DEFAULT_MAP).unwrap();

        for _ in 0..20_000 {
            sim.step();
            assert!(
                sim.drone.pos.x > -Block::SIZE && sim.drone.pos.x < 700.0,
                "flew out sideways at {:?} after {} ms",
                sim.drone.pos,
                sim.time
            );
            assert!(
                sim.drone.pos.y < Block::SIZE && sim.drone.pos.y > -700.0,
                "flew out lengthways at {:?} after {} ms",
                sim.drone.pos,
                sim.time
            );
        }
    }

    #[test]
    fn resetting_puts_everything_back_but_the_room() {
        let mut sim = Simulator::with_map(DEFAULT_MAP).unwrap();
        let blocks = sim.blocks.len();
        sim.run(5000);

        sim.reset();

        assert_eq!(sim.time, 0);
        assert_eq!(sim.drone.pos, DRONE_START);
        assert_eq!(sim.drone.firmware.nav.task, Task::Idle);
        assert_eq!(sim.blocks.len(), blocks);
    }
}
