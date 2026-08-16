//! The navigator: what the drone knows, and what it decides to do about it.
//!
//! Once every [`PERIOD_MILLIS`] the navigator reads the sensors into a set of
//! flags, dead-reckons its position from the flight controller's velocity and
//! gyro, marks up its map, and then runs one step of a state machine that
//! keeps it flying along walls and turning at corners.
//!
//! It never knows where it really is. Position comes from integrating a
//! velocity, so it drifts, and the map drifts with it. That is the honest
//! result of the sensor set the drone carries, and watching the drift build up
//! is a large part of what the simulator is for.

use crate::fc::Fc;
use crate::fix16::Fix16;
use crate::io::{APin, DPin, Tx};
use crate::ir::Ir;
use crate::laser::{LASER_MAX_DISTANCE_CM, Laser};
use crate::log::{Log, Sender};
use crate::map::{CENTIMETERS_PR_FIELD, FieldState, MAP_HEIGHT, MAP_WIDTH, Map, MapCoord};
use crate::search::{Search, align_to_map};
use crate::sonar::Sonar;
use crate::{Firmware, ONE_MS};

/// How much further the laser has to read than the sonar before the drone
/// decides it is looking through glass, in centimetres.
pub const WINDOW_RECON_THRESHOLD: u16 = 20;
/// How close something has to be to count as being in the way, in centimetres.
/// Also the distance the drone tries to hold from a wall it is following.
pub const MIN_SENSOR_RANGE: u16 = 60;
/// How close something has to be before it is drawn on the map, in
/// centimetres. Further off than this, a reading is not worth recording.
pub const MIN_DRAW_RANGE: u16 = 100;
/// How far a sonar reading may sit from the distance worked out from the laser
/// before the sonar is disbelieved, in centimetres.
pub const SENSOR_DEVIATION: i32 = 5;
/// How long between navigator runs, in milliseconds.
pub const PERIOD_MILLIS: u32 = 100;
/// How long between navigator runs, in seconds.
pub const PERIOD_SECONDS: f32 = PERIOD_MILLIS as f32 / 1000.0;
/// The middle of the map, in fields. The drone assumes it starts there,
/// because it has nothing better to assume.
pub const MAP_MIDDLE: u16 = (MAP_HEIGHT as u16 + MAP_WIDTH as u16) / 4;

/// The ratio between the distance to a wall straight ahead and the distance to
/// it along the edge of the sonar's cone.
///
/// The C header calls this sin(15°)/sin(75°) and gives it as the word `0x5290`.
/// Those disagree: the ratio is 0.268 and the word is 0.3225, the sine of about
/// 18.8 degrees over the sine of its complement. The word is what the drone
/// flew with, so the word is what is kept.
pub const SONAR_RELIABLE_CONSTANT: Fix16 = Fix16::from_raw(0x5290);
/// 270 degrees, in radians: where the drone's right-hand laser points.
pub const DRONE_RIGHT_SIDE: Fix16 = Fix16::from_raw(0x0004_b65f);
/// 90 degrees, in radians: where the drone's left-hand laser points.
pub const DRONE_LEFT_SIDE: Fix16 = Fix16::from_raw(0x0001_9220);
/// 90 degrees, in radians: how far the drone turns at a corner.
pub const FULL_TURN: Fix16 = Fix16::from_raw(0x0001_9220);

/// The one thing the drone is doing at any moment.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Task {
    /// Deciding what to do next.
    #[default]
    Idle,
    /// Turning on the spot until [`Nav::val`] radians have gone by.
    Turning,
    /// Flying forward until [`Nav::val`] centimetres have gone by.
    MoveForward,
    /// Climbing until the ceiling is close.
    MoveUp,
    /// Descending until the floor is close.
    MoveDown,
    /// Flying along a wall.
    FollowForward,
    /// Carrying on past where the wall ended, in case it has not.
    FollowFurther,
    /// Turning to point the sonar at what the laser stopped seeing, to find
    /// out whether the wall became a window or a doorway.
    FollowCheck,
    /// Turning at a corner.
    FollowTurn,
    /// Looking for somewhere unvisited to go.
    Searching,
}

/// What the drone believes about what is around it.
///
/// A bitfield of fifteen single-bit flags in the C, which is a byte and a half
/// on a board with 2 KiB of RAM. Nothing here is short of memory.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct State {
    /// A wall within [`MIN_SENSOR_RANGE`] in front.
    pub wall_front: bool,
    /// A wall within [`MIN_SENSOR_RANGE`] to the right.
    pub wall_right: bool,
    /// A wall within [`MIN_SENSOR_RANGE`] to the left.
    pub wall_left: bool,
    /// A window within [`MIN_SENSOR_RANGE`] in front.
    pub win_front: bool,
    /// A window within [`MIN_SENSOR_RANGE`] to the right.
    pub win_right: bool,
    /// A window within [`MIN_SENSOR_RANGE`] to the left.
    pub win_left: bool,
    /// A window check is under way.
    pub win_check: bool,
    /// The floor is close below.
    pub ground: bool,
    /// The ceiling is close above.
    pub ceiling: bool,
    /// Either a wall or a window in front.
    pub blocked_front: bool,
    /// Either a wall or a window to the left.
    pub blocked_left: bool,
    /// Either a wall or a window to the right.
    pub blocked_right: bool,
    /// Following the wall on the left.
    pub follow_left: bool,
    /// Following the wall on the right.
    pub follow_right: bool,
    /// Following a wall on either side.
    pub follow: bool,
}

/// The drone's world representation: the flight controller and every sensor.
///
/// The C held five pointers here, into structs the caller owned. Owning them
/// outright says the same thing without the aliasing.
#[derive(Clone, Debug)]
pub struct Rep {
    /// The flight controller.
    pub fc: Fc,
    /// The laser range finder.
    pub laser: Laser,
    /// The sonar.
    pub sonar: Sonar,
    /// The infrared sensor pointing at the ceiling.
    pub ir_top: Ir,
    /// The infrared sensor pointing at the floor.
    pub ir_bottom: Ir,
}

impl Rep {
    /// The sensors, wired as they are on the drone.
    #[must_use]
    pub fn new() -> Self {
        Self {
            fc: Fc::new(Tx::Tx1, ONE_MS),
            laser: Laser::new(Tx::UsbTx),
            sonar: Sonar::new(DPin::P2, DPin::P3),
            ir_top: Ir::new(APin::A1),
            ir_bottom: Ir::new(APin::A0),
        }
    }
}

impl Default for Rep {
    fn default() -> Self {
        Self::new()
    }
}

/// Where the drone thinks it is, and what it is doing.
#[derive(Clone, Debug)]
pub struct Nav {
    /// What it believes about its surroundings.
    pub state: State,
    /// What it is doing.
    pub task: Task,
    /// Its heading, in radians, anticlockwise from east.
    pub angle: Fix16,
    /// Its x position, in centimetres from the map's left edge.
    pub posx: u16,
    /// Its y position, in centimetres from the map's bottom edge.
    pub posy: u16,
    /// The last distance measured to the wall being followed.
    pub prev_dist_wall: i16,
    /// How much of the current task is left: radians to turn, or centimetres
    /// to fly.
    pub val: Fix16,
    /// A path search, for finding a way to somewhere unvisited.
    pub search_data: Search,
}

impl Nav {
    /// A navigator that has not moved yet, in the middle of its map.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: State::default(),
            task: Task::Idle,
            angle: Fix16::ZERO,
            posx: MAP_MIDDLE * CENTIMETERS_PR_FIELD,
            posy: MAP_MIDDLE * CENTIMETERS_PR_FIELD,
            prev_dist_wall: 0,
            val: Fix16::ZERO,
            search_data: Search::new(),
        }
    }

    /// Where the drone thinks it is, as a field on its map.
    #[must_use]
    pub fn position(&self) -> MapCoord {
        align_to_map(self.posx, self.posy)
    }
}

impl Default for Nav {
    fn default() -> Self {
        Self::new()
    }
}

/// How far along the y-axis something is, at `angle` and `distance` away.
#[must_use]
pub fn calc_y_dist(angle: Fix16, distance: Fix16) -> Fix16 {
    angle.sin() * distance
}

/// How far along the x-axis something is, at `angle` and `distance` away.
#[must_use]
pub fn calc_x_dist(angle: Fix16, distance: Fix16) -> Fix16 {
    angle.cos() * distance
}

/// Whether there is a wall within range in front.
///
/// Reads the laser but insists the sonar had something to say, which is how
/// the drone avoids calling a window a wall. The C also passed the current
/// state in, for a reliability check that is commented out there.
#[must_use]
pub fn check_wall_front(rep: &Rep) -> bool {
    rep.sonar.valid && rep.laser.val_front <= MIN_SENSOR_RANGE
}

/// Whether there is a wall within range to the left.
#[must_use]
pub fn check_wall_left(rep: &Rep) -> bool {
    rep.laser.val_left <= MIN_SENSOR_RANGE
}

/// Whether there is a wall within range to the right.
#[must_use]
pub fn check_wall_right(rep: &Rep) -> bool {
    rep.laser.val_right <= MIN_SENSOR_RANGE
}

/// Whether the floor is close below.
#[must_use]
pub fn check_ground(rep: &Rep) -> bool {
    u16::from(rep.ir_bottom.value) <= MIN_SENSOR_RANGE
}

/// Whether the ceiling is close above.
#[must_use]
pub fn check_ceiling(rep: &Rep) -> bool {
    u16::from(rep.ir_top.value) <= MIN_SENSOR_RANGE
}

/// Whether there is a window within range in front: the sonar sees something
/// the laser did not stop at.
#[must_use]
pub fn check_win_front(rep: &Rep) -> bool {
    rep.sonar.valid && rep.sonar.value <= MIN_SENSOR_RANGE
}

/// Whether there is a window to the left.
///
/// Never implemented. Telling glass from open air off to one side needs the
/// drone to turn and point its sonar at it, which is what the follow-check
/// state does instead; the C left this function with its body commented out
/// and its call site commented out with it.
#[must_use]
pub fn check_win_left(_state: &State) -> bool {
    false
}

/// Whether there is a window to the right. Never implemented; see
/// [`check_win_left`].
#[must_use]
pub fn check_win_right(_state: &State) -> bool {
    false
}

/// Whether anything at all is in the way in front.
#[must_use]
pub fn check_blocked_front(state: &State) -> bool {
    state.wall_front || state.win_front
}

/// Whether anything at all is in the way to the right.
#[must_use]
pub fn check_blocked_right(state: &State) -> bool {
    state.wall_right || state.win_right
}

/// Whether anything at all is in the way to the left.
#[must_use]
pub fn check_blocked_left(state: &State) -> bool {
    state.wall_left || state.win_left
}

/// Whether a wall is being followed on either side.
#[must_use]
pub fn check_follow_wall(state: &State) -> bool {
    state.follow_left || state.follow_right
}

/// Reads every sensor into the flags the state machine runs on.
pub fn update_state(state: &mut State, rep: &Rep) {
    state.ceiling = check_ceiling(rep);
    state.ground = check_ground(rep);
    state.wall_front = check_wall_front(rep);
    state.wall_left = check_wall_left(rep);
    state.wall_right = check_wall_right(rep);
    state.win_front = check_win_front(rep);
    // The C leaves the two side-window checks commented out here, since
    // neither is implemented.
    state.blocked_front = check_blocked_front(state);
    state.blocked_left = check_blocked_left(state);
    state.blocked_right = check_blocked_right(state);
    state.follow = check_follow_wall(state);
}

/// Whether the sonar's reading agrees with the laser's, given the angle
/// between them.
///
/// The drone following a wall on one side can work out how far a beam 15
/// degrees off centre should have travelled to reach that same wall. If the
/// sonar reports something much shorter, it is seeing the wall beside the
/// drone rather than whatever is in front, and should not be believed.
///
/// Never called: the two call sites in the C are commented out. Its condition
/// there also closes the absolute-value bracket after the comparison rather
/// than before it, so it takes the absolute value of a boolean -- which
/// happens to come out the same, since neither 0 nor 1 changes under `abs`.
#[must_use]
pub fn is_sonar_reliable(rep: &Rep, state: &State) -> bool {
    // The wall being followed is on the left unless something blocks the right.
    let dist_to_wall = Fix16::from_int(i32::from(if state.blocked_right {
        rep.laser.val_left
    } else {
        rep.laser.val_right
    }));

    // b = a · (sin B / sin A)
    let expected = dist_to_wall * SONAR_RELIABLE_CONSTANT;
    let measured = Fix16::from_int(i32::from(rep.sonar.value));

    (expected - measured).abs() <= Fix16::from_int(SENSOR_DEVIATION) && rep.sonar.valid
}

/// Counts a task's remaining distance or angle down by one period's worth of
/// travel, stopping at zero. Velocity is signed; either direction counts.
pub fn update_nav_value(nav_val: &mut Fix16, velocity: Fix16) {
    let travelled = velocity * Fix16::from_f32(PERIOD_SECONDS);

    let remaining = if velocity > Fix16::ZERO {
        *nav_val - travelled
    } else {
        *nav_val + travelled
    };

    *nav_val = if remaining <= Fix16::ZERO {
        Fix16::ZERO
    } else {
        remaining
    };
}

impl Firmware {
    /// One navigator run: read the sensors, work out where that puts the
    /// drone, mark up the map, and take one step of whatever it is doing.
    pub fn navigation(&mut self) {
        update_state(&mut self.nav.state, &self.rep);
        self.update_angle();
        self.update_pos();
        self.update_map();

        match self.nav.task {
            Task::Idle => self.on_idle(),
            Task::Turning => self.on_turning(),
            Task::MoveForward => self.on_move_forward(),
            Task::MoveUp => self.on_move_up(),
            Task::MoveDown => self.on_move_down(),
            Task::FollowForward => self.on_follow_forward(),
            Task::FollowFurther => self.on_follow_further(),
            Task::FollowCheck => self.on_follow_check(),
            Task::FollowTurn => self.on_follow_turn(),
            Task::Searching => self.on_searching(),
        }

        // Remember how far the followed wall was, for the alignment check.
        if self.nav.state.blocked_left {
            self.nav.prev_dist_wall = self.rep.laser.val_left as i16;
        } else if self.nav.state.blocked_right {
            self.nav.prev_dist_wall = self.rep.laser.val_right as i16;
        }
    }

    /// Turns the gyro's rotational velocity into a new heading, wrapped into
    /// one turn.
    pub fn update_angle(&mut self) {
        let turned = self.rep.fc.gyro * Fix16::from_f32(PERIOD_SECONDS);
        let full_circle = Fix16::from_f64(core::f64::consts::TAU);

        self.nav.angle += turned;

        if self.nav.angle > full_circle {
            self.nav.angle = self.nav.angle % full_circle;
        } else if self.nav.angle < Fix16::ZERO {
            self.nav.angle = full_circle + self.nav.angle;
        }
    }

    /// Dead-reckons the position forward by one period of forward velocity.
    pub fn update_pos(&mut self) {
        let distance = self.rep.fc.vel.y * Fix16::from_f32(PERIOD_SECONDS);

        // The C lets these wrap through a `uint16_t`, and so does this: a
        // position that has drifted off the map is off the map either way.
        self.nav.posx =
            (i32::from(self.nav.posx) + calc_x_dist(self.nav.angle, distance).to_int()) as u16;
        self.nav.posy =
            (i32::from(self.nav.posy) + calc_y_dist(self.nav.angle, distance).to_int()) as u16;
    }

    // ---- What to do while a task is running ------------------------------

    /// Nothing to do: pick something.
    fn on_idle(&mut self) {
        let state = self.nav.state;

        if !(state.blocked_front || state.follow) {
            self.nav_move_forward(Fix16::from_int(i32::from(CENTIMETERS_PR_FIELD)));
            return;
        }

        if state.follow {
            self.nav_follow_forward();
            return;
        }

        self.nav_turn_around();
    }

    /// Turning on the spot until the angle has been covered.
    fn on_turning(&mut self) {
        update_nav_value(&mut self.nav.val, self.rep.fc.gyro);

        if self.nav.val == Fix16::ZERO {
            self.rep.fc.move_stop();
            self.nav.task = Task::Idle;
        }
    }

    /// Flying forward across open floor.
    fn on_move_forward(&mut self) {
        update_nav_value(&mut self.nav.val, self.rep.fc.vel.y);

        // Whatever wall turns up first is followed on the left.
        self.nav.state.follow_left = true;

        if self.nav.state.blocked_front {
            self.rep.fc.move_stop();
            self.nav_follow_turn();
        } else if self.nav.val == Fix16::ZERO {
            self.nav_move_forward(Fix16::from_int(25));
        }
    }

    /// Climbing until the ceiling is close.
    fn on_move_up(&mut self) {
        if self.nav.state.ceiling {
            self.nav_idle();
        }
    }

    /// Descending until the floor is close.
    fn on_move_down(&mut self) {
        if self.nav.state.ground {
            self.nav_idle();
        }
    }

    /// Flying along a wall. If the wall falls away, carry on a little in case
    /// it comes back.
    fn on_follow_forward(&mut self) {
        update_nav_value(&mut self.nav.val, self.rep.fc.vel.y);

        if self.nav.state.blocked_front {
            self.rep.fc.move_stop();
            self.nav_follow_turn();
            return;
        }

        if self.nav.state.follow_left && !self.nav.state.blocked_left {
            self.nav_follow_further(Fix16::from_int(50));
            return;
        }

        if self.nav.state.follow_right && !self.nav.state.blocked_right {
            self.nav_follow_further(Fix16::from_int(50));
        }
    }

    /// Carrying on past where the wall ended. Either it comes back, or the
    /// drone stops and looks.
    fn on_follow_further(&mut self) {
        update_nav_value(&mut self.nav.val, self.rep.fc.vel.y);

        if self.nav.state.blocked_front {
            self.rep.fc.move_stop();
            if self.nav.state.blocked_left {
                self.nav_follow_turn();
            } else {
                self.nav_follow_check();
            }
            return;
        }

        if self.nav.state.wall_left && self.nav.state.follow_left {
            self.nav_follow_forward();
            self.nav.state.win_left = false;
            return;
        }

        if self.nav.val == Fix16::ZERO {
            self.rep.fc.move_stop();
            self.nav_follow_check();
        }
    }

    /// Turned to look at what the wall became. If the sonar sees something the
    /// laser did not, it is glass; note it and turn back.
    fn on_follow_check(&mut self) {
        update_nav_value(&mut self.nav.val, self.rep.fc.gyro);

        if self.nav.val != Fix16::ZERO {
            return;
        }

        if self.nav.state.win_front && !self.nav.state.win_check {
            if self.nav.state.follow_left {
                self.nav.state.win_left = true;
            } else {
                self.nav.state.win_right = true;
            }

            self.nav.state.win_check = true;
            self.nav_follow_check();
        } else {
            self.nav.state.win_check = false;
            self.rep.fc.move_stop();
            self.nav_follow_further(Fix16::from_int(65));
        }
    }

    /// Turning at a corner.
    fn on_follow_turn(&mut self) {
        update_nav_value(&mut self.nav.val, self.rep.fc.gyro);

        if self.nav.val == Fix16::ZERO {
            self.rep.fc.move_stop();
            self.nav_follow_forward();
        }
    }

    /// Looking for somewhere unvisited to fly to.
    ///
    /// Never written. The C's body is empty, and the path search it would have
    /// used does not run; see [`crate::search`]. Nothing puts the drone into
    /// this state, so it is unreachable rather than merely idle.
    fn on_searching(&mut self) {}

    // ---- Entering a task -------------------------------------------------

    /// Stops and waits.
    pub fn nav_idle(&mut self) {
        self.rep.fc.move_stop();
        self.nav.task = Task::Idle;
    }

    /// Turns anticlockwise through `angle` radians.
    pub fn nav_turn_left(&mut self, angle: Fix16) {
        self.rep.fc.rotate_left();
        self.nav.val = angle;
        self.nav.task = Task::Turning;
    }

    /// Turns clockwise through `angle` radians.
    pub fn nav_turn_right(&mut self, angle: Fix16) {
        self.rep.fc.rotate_right();
        self.nav.val = angle;
        self.nav.task = Task::Turning;
    }

    /// Turns to face the other way.
    pub fn nav_turn_around(&mut self) {
        self.rep.fc.rotate_right();
        self.nav.val = Fix16::PI;
        self.nav.task = Task::Turning;
    }

    /// Flies forward `distance` centimetres.
    pub fn nav_move_forward(&mut self, distance: Fix16) {
        self.rep.fc.move_forward();
        self.nav.val = distance;
        self.nav.task = Task::MoveForward;
    }

    /// Climbs.
    pub fn nav_move_up(&mut self) {
        self.rep.fc.move_up();
        self.nav.task = Task::MoveUp;
    }

    /// Descends.
    pub fn nav_move_down(&mut self) {
        self.rep.fc.move_down();
        self.nav.task = Task::MoveDown;
    }

    /// Flies along the wall.
    pub fn nav_follow_forward(&mut self) {
        self.rep.fc.move_forward();
        self.nav.task = Task::FollowForward;
    }

    /// Carries on `distance` centimetres past where the wall ended.
    pub fn nav_follow_further(&mut self, distance: Fix16) {
        self.rep.fc.move_forward();
        self.nav.val = distance;
        self.nav.task = Task::FollowFurther;
    }

    /// Turns a quarter circle to point the sonar at the wall, or back again if
    /// it has already looked.
    pub fn nav_follow_check(&mut self) {
        if self.nav.state.win_check {
            self.rep.fc.rotate_right();
        } else {
            self.rep.fc.rotate_left();
        }

        self.nav.val = FULL_TURN;
        self.nav.task = Task::FollowCheck;
    }

    /// Turns a quarter circle away from the wall being followed.
    pub fn nav_follow_turn(&mut self) {
        if self.nav.state.follow_left {
            self.nav.state.win_left = false;
            self.rep.fc.rotate_right();
        } else {
            self.nav.state.win_right = false;
            self.rep.fc.rotate_left();
        }

        self.nav.val = FULL_TURN;
        self.nav.task = Task::FollowTurn;
    }

    // ---- The map ---------------------------------------------------------

    /// Turns towards or away from the wall to hold a constant distance from it.
    ///
    /// Never called: its one call site, in the forward-flying state, is
    /// commented out, and the C's own comment marks the angle it turns through
    /// as a placeholder ("todo: Insert proper calculation") -- it turns through
    /// the *change in distance to the wall*, in centimetres, read as radians.
    /// Kept as it stands rather than guessed at.
    // The four cases are the C's four cases. Two pairs of them turn the same
    // way, and collapsing those would hide which wall each is about.
    #[allow(clippy::if_same_then_else)]
    pub fn align_to_wall(&mut self) {
        let mut diff_wall = Fix16::ZERO;

        if self.nav.state.blocked_left {
            diff_wall = Fix16::from_int(
                i32::from(self.rep.laser.val_left) - i32::from(self.nav.prev_dist_wall),
            );
            self.nav.prev_dist_wall = self.rep.laser.val_left as i16;
        } else if self.nav.state.blocked_right {
            diff_wall = Fix16::from_int(
                i32::from(self.rep.laser.val_right) - i32::from(self.nav.prev_dist_wall),
            );
            self.nav.prev_dist_wall = self.rep.laser.val_right as i16;
        }

        let degrees_to_turn = Fix16::from_int(diff_wall.to_int().abs());
        let closing = diff_wall < Fix16::ZERO;
        let left_near = self.rep.laser.val_left < MIN_SENSOR_RANGE;
        let right_near = self.rep.laser.val_right < MIN_SENSOR_RANGE;

        if closing && left_near {
            self.nav_turn_right(degrees_to_turn);
        } else if closing && right_near {
            self.nav_turn_left(degrees_to_turn);
        } else if diff_wall > Fix16::ZERO && left_near {
            self.nav_turn_left(degrees_to_turn);
        } else if diff_wall > Fix16::ZERO && right_near {
            self.nav_turn_right(degrees_to_turn);
        }
    }

    /// Whether the drone is holding station at a constant distance from the
    /// wall it is following.
    #[must_use]
    pub fn check_alignment_wall(&mut self) -> bool {
        if self.nav.state.blocked_right {
            if self.nav.prev_dist_wall == 0 {
                self.nav.prev_dist_wall = self.rep.laser.val_right as i16;
                return false;
            }

            if self.nav.prev_dist_wall != self.rep.laser.val_right as i16
                && self.rep.laser.val_right != LASER_MAX_DISTANCE_CM
            {
                return false;
            }
        } else if self.rep.laser.val_left < MIN_SENSOR_RANGE {
            if self.nav.prev_dist_wall == 0 && self.rep.laser.val_left != LASER_MAX_DISTANCE_CM {
                self.nav.prev_dist_wall = self.rep.laser.val_left as i16;
                return false;
            }

            if self.nav.prev_dist_wall != self.rep.laser.val_left as i16
                && self.rep.laser.val_left != LASER_MAX_DISTANCE_CM
            {
                return false;
            }
        }

        true
    }

    /// Records a field on the map.
    pub fn map_set_point(&mut self, x: u8, y: u8, field: FieldState) {
        self.map.write(x, y, field);
    }

    /// Records what is at the drone's own position.
    pub fn map_set_position(&mut self, field: FieldState) {
        let pixel = self.nav.position();

        if pixel.valid {
            self.map.write(pixel.x, pixel.y, field);
        }
    }

    /// Reads a field.
    #[must_use]
    pub fn map_check_point(&self, x: u8, y: u8) -> FieldState {
        self.map.read(x, y)
    }

    /// Reads the field the drone is on.
    #[must_use]
    pub fn map_check_position(&self) -> FieldState {
        let pixel = self.nav.position();
        self.map.read(pixel.x, pixel.y)
    }

    /// Marks something the sensors found, at `side_offset` radians off the
    /// drone's heading and `val` centimetres away.
    pub fn draw_obstacle(&mut self, val: u16, side_offset: Fix16, state: FieldState) {
        let distance = Fix16::from_int(i32::from(val));
        let bearing = self.nav.angle + side_offset;

        let x_offset = calc_x_dist(bearing, distance).to_int();
        let y_offset = calc_y_dist(bearing, distance).to_int();

        let obstacle = align_to_map(
            (i32::from(self.nav.posx) + x_offset) as u16,
            (i32::from(self.nav.posy) + y_offset) as u16,
        );

        if obstacle.valid {
            self.map.write(obstacle.x, obstacle.y, state);
        }
    }

    /// Marks the map up from this round's readings: where the drone has been,
    /// and what its side-facing lasers found.
    pub fn update_map(&mut self) {
        self.map_set_position(FieldState::Visited);

        // What is in front is worked out and then not drawn. The C computes
        // the difference between the laser and sonar readings to tell a window
        // from a wall, and both branches that would have recorded the answer
        // are commented out. Drawing it would change what the map ends up
        // looking like, which is the one thing a port of a mapping algorithm
        // should not do quietly, so this stays as it stands; the classification
        // itself is available from `obstacle_in_front`.
        let _ = self.obstacle_in_front();

        if self.rep.laser.val_right <= MIN_DRAW_RANGE {
            self.draw_obstacle(self.rep.laser.val_right, DRONE_RIGHT_SIDE, FieldState::Wall);
        }

        if self.rep.laser.val_left <= MIN_DRAW_RANGE {
            self.draw_obstacle(self.rep.laser.val_left, DRONE_LEFT_SIDE, FieldState::Wall);
        }
    }

    /// What the two forward-facing sensors make of whatever is in front, if
    /// anything is close enough to be worth recording.
    ///
    /// The laser goes through glass and the sonar does not, so a laser reading
    /// much longer than the sonar's means a window.
    #[must_use]
    pub fn obstacle_in_front(&self) -> Option<FieldState> {
        let laser = self.rep.laser.val_front;
        let sonar = self.rep.sonar.value;

        let in_range = sonar < MIN_DRAW_RANGE || laser <= MIN_DRAW_RANGE;
        if !in_range || laser == LASER_MAX_DISTANCE_CM || sonar == 0 {
            return None;
        }

        Some(if laser.abs_diff(sonar) > WINDOW_RECON_THRESHOLD {
            FieldState::Window
        } else {
            FieldState::Wall
        })
    }
}

/// The map the navigator writes into, and the log it complains to.
impl Firmware {
    /// Everything the firmware needs, in the state it powers up in.
    #[must_use]
    pub fn new() -> Self {
        let mut log = Log::default();
        let map = Map::new(MAP_WIDTH, MAP_HEIGHT, &mut log);

        Self {
            rep: Rep::new(),
            nav: Nav::new(),
            map,
            log,
        }
    }

    /// Blanks the map and puts the drone back in the middle of it, without
    /// disturbing how the flight controller is wired up.
    pub fn reset(&mut self) {
        self.map.clean();
        self.nav = Nav::new();
        self.log.clear();
        self.log.message(Sender::Board, "navigation reset");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A firmware with the duty cycles the simulator uses, so that a channel
    /// reads directly as backwards, still or forwards.
    fn firmware() -> Firmware {
        let mut fw = Firmware::new();
        fw.rep.fc.duty.min = 0;
        fw.rep.fc.duty.mid = 1;
        fw.rep.fc.duty.max = 2;
        fw.rep.fc.move_stop();
        fw.rep.fc.rotate_stop();
        fw
    }

    /// Nothing within range in any direction.
    fn clear_of_everything(fw: &mut Firmware) {
        fw.rep.laser.val_front = LASER_MAX_DISTANCE_CM;
        fw.rep.laser.val_left = LASER_MAX_DISTANCE_CM;
        fw.rep.laser.val_right = LASER_MAX_DISTANCE_CM;
        fw.rep.sonar.valid = false;
        fw.rep.sonar.value = 0;
        fw.rep.ir_bottom.value = 80;
        fw.rep.ir_top.value = 80;
    }

    #[test]
    fn a_new_navigator_starts_in_the_middle_of_its_map() {
        let fw = Firmware::new();
        assert_eq!(fw.nav.posx, 800);
        assert_eq!(fw.nav.posy, 800);
        assert_eq!(fw.nav.task, Task::Idle);
        assert_eq!(
            fw.nav.position(),
            MapCoord {
                x: 32,
                y: 32,
                valid: true
            }
        );
    }

    #[test]
    fn an_idle_drone_in_the_open_sets_off() {
        let mut fw = firmware();
        clear_of_everything(&mut fw);

        fw.navigation();

        assert_eq!(fw.nav.task, Task::MoveForward);
        assert_eq!(fw.rep.fc.pitch, fw.rep.fc.duty.max);
        assert_eq!(fw.nav.val, Fix16::from_int(25));
    }

    #[test]
    fn a_wall_in_front_turns_the_drone_at_the_corner() {
        let mut fw = firmware();
        clear_of_everything(&mut fw);
        fw.navigation();

        // A wall turns up within range, with the sonar backing the laser.
        fw.rep.laser.val_front = 40;
        fw.rep.sonar.valid = true;
        fw.rep.sonar.value = 40;
        fw.navigation();

        assert_eq!(fw.nav.task, Task::FollowTurn);
        assert_eq!(fw.nav.val, FULL_TURN);
        // Following on the left, so it turns to the right.
        assert_eq!(fw.rep.fc.yaw, fw.rep.fc.duty.max);
    }

    #[test]
    fn a_turn_ends_once_the_angle_has_been_covered() {
        let mut fw = firmware();
        clear_of_everything(&mut fw);
        fw.nav.task = Task::Turning;
        fw.nav.val = Fix16::from_f64(core::f64::consts::FRAC_PI_2);
        // A quarter turn a second, so two rounds of a tenth of a second each
        // are nowhere near enough.
        fw.rep.fc.gyro = Fix16::from_f64(core::f64::consts::FRAC_PI_4);

        for _ in 0..2 {
            fw.navigation();
            assert_eq!(fw.nav.task, Task::Turning);
        }

        let mut rounds = 2;
        while fw.nav.task == Task::Turning {
            fw.navigation();
            rounds += 1;
            assert!(rounds < 100, "the turn never finished");
        }

        assert_eq!(fw.nav.task, Task::Idle);
        assert_eq!(fw.rep.fc.yaw, fw.rep.fc.duty.mid);
        // A quarter turn at a quarter turn a second is two seconds of
        // hundred-millisecond rounds.
        assert_eq!(rounds, 20);
    }

    #[test]
    fn the_heading_wraps_into_one_turn() {
        let mut fw = firmware();
        clear_of_everything(&mut fw);

        // Turning right at a radian a second for long enough to go round.
        fw.rep.fc.gyro = -Fix16::ONE;
        for _ in 0..100 {
            fw.update_angle();
            assert!(fw.nav.angle >= Fix16::ZERO);
            assert!(fw.nav.angle <= Fix16::from_f64(core::f64::consts::TAU));
        }
    }

    #[test]
    fn flying_east_moves_the_position_east() {
        let mut fw = firmware();
        clear_of_everything(&mut fw);
        fw.nav.angle = Fix16::ZERO;
        fw.rep.fc.vel.y = Fix16::from_int(100);

        let start = fw.nav.posx;
        for _ in 0..10 {
            fw.update_pos();
        }

        // A metre a second for a second.
        assert_eq!(fw.nav.posx, start + 100);
        assert_eq!(fw.nav.posy, 800);
    }

    #[test]
    fn where_the_drone_has_been_is_marked_visited() {
        let mut fw = firmware();
        clear_of_everything(&mut fw);

        fw.update_map();

        assert_eq!(fw.map.read(32, 32), FieldState::Visited);
    }

    #[test]
    fn a_wall_to_the_left_is_drawn_to_the_left() {
        let mut fw = firmware();
        clear_of_everything(&mut fw);
        // Facing east, so the left-hand laser points north.
        fw.nav.angle = Fix16::ZERO;
        fw.rep.laser.val_left = 50;

        fw.update_map();

        // 50 cm north of the middle is two fields up.
        assert_eq!(fw.map.read(32, 34), FieldState::Wall);
        assert_eq!(fw.map.read(32, 30), FieldState::Unvisited);
    }

    #[test]
    fn a_wall_to_the_right_is_drawn_to_the_right() {
        let mut fw = firmware();
        clear_of_everything(&mut fw);
        fw.nav.angle = Fix16::ZERO;
        fw.rep.laser.val_right = 50;

        fw.update_map();

        assert_eq!(fw.map.read(32, 30), FieldState::Wall);
    }

    #[test]
    fn something_too_far_off_is_not_drawn() {
        let mut fw = firmware();
        clear_of_everything(&mut fw);
        fw.rep.laser.val_left = MIN_DRAW_RANGE + 1;

        fw.update_map();

        for y in 0..MAP_HEIGHT {
            for x in 0..MAP_WIDTH {
                assert_ne!(fw.map.read(x, y), FieldState::Wall);
            }
        }
    }

    #[test]
    fn glass_in_front_reads_differently_from_a_wall() {
        let mut fw = firmware();
        clear_of_everything(&mut fw);

        // Both sensors stop at the same place: something solid.
        fw.rep.laser.val_front = 50;
        fw.rep.sonar.value = 50;
        assert_eq!(fw.obstacle_in_front(), Some(FieldState::Wall));

        // The laser carries on through and the sonar does not: glass.
        fw.rep.laser.val_front = 90;
        fw.rep.sonar.value = 50;
        assert_eq!(fw.obstacle_in_front(), Some(FieldState::Window));

        // Nothing close enough to record.
        fw.rep.laser.val_front = LASER_MAX_DISTANCE_CM;
        fw.rep.sonar.value = 0;
        assert_eq!(fw.obstacle_in_front(), None);
    }

    #[test]
    fn the_floor_and_ceiling_stop_a_climb_or_a_descent() {
        let mut fw = firmware();
        clear_of_everything(&mut fw);

        fw.nav_move_up();
        assert_eq!(fw.rep.fc.throttle, fw.rep.fc.duty.max);
        fw.navigation();
        assert_eq!(fw.nav.task, Task::MoveUp);

        fw.rep.ir_top.value = 30;
        fw.navigation();
        assert_eq!(fw.nav.task, Task::Idle);
        assert_eq!(fw.rep.fc.throttle, fw.rep.fc.duty.mid);
    }

    #[test]
    fn a_task_counts_down_by_the_distance_flown() {
        let mut value = Fix16::from_int(50);
        // A metre a second, a tenth of a second at a time. A tenth is not
        // exactly representable in 16.16, so the step is a hair over 10.
        update_nav_value(&mut value, Fix16::from_int(100));
        assert!((value.to_f64() - 40.0).abs() < 0.01);

        for _ in 0..10 {
            update_nav_value(&mut value, Fix16::from_int(100));
        }

        assert_eq!(value, Fix16::ZERO);
    }

    #[test]
    fn a_task_counts_down_when_flying_backwards_too() {
        let mut value = Fix16::from_int(50);
        update_nav_value(&mut value, Fix16::from_int(-100));
        assert!((value.to_f64() - 40.0).abs() < 0.01);
    }

    #[test]
    fn the_window_check_turns_out_and_back() {
        let mut fw = firmware();
        clear_of_everything(&mut fw);
        fw.nav.state.follow_left = true;

        // Turned to look, and there is glass there.
        fw.nav.task = Task::FollowCheck;
        fw.nav.val = Fix16::ZERO;
        fw.rep.sonar.valid = true;
        fw.rep.sonar.value = 40;
        fw.navigation();

        assert!(fw.nav.state.win_left);
        assert!(fw.nav.state.win_check);
        assert_eq!(fw.nav.task, Task::FollowCheck);

        // Turned back; the check is over and the drone carries on.
        fw.nav.val = Fix16::ZERO;
        fw.navigation();

        assert!(!fw.nav.state.win_check);
        assert_eq!(fw.nav.task, Task::FollowFurther);
        assert_eq!(fw.nav.val, Fix16::from_int(65));
    }

    #[test]
    fn resetting_puts_the_drone_back_in_the_middle_of_a_blank_map() {
        let mut fw = firmware();
        clear_of_everything(&mut fw);
        fw.rep.laser.val_left = 50;
        fw.navigation();
        fw.nav.posx = 1000;

        fw.reset();

        assert_eq!(fw.nav.posx, 800);
        assert_eq!(fw.nav.task, Task::Idle);
        assert_eq!(fw.map.read(32, 34), FieldState::Unvisited);
    }
}
