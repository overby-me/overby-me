//! The running simulation, and everything the page needs to draw and steer it.
//!
//! The simulation itself is not a Dioxus signal. It changes sixty times a
//! second and is read by canvas code that has no business re-rendering the DOM,
//! so it lives behind an `Rc<RefCell<_>>` that the animation loop owns, and the
//! panel samples it a few times a second instead.

use std::cell::RefCell;
use std::rc::Rc;

use randie_firmware::laser::LASER_MAX_DISTANCE_CM;
use randie_firmware::nav::Task;
use randie_sim::{BlockType, DEFAULT_MAP, Simulator, Vec2};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

use crate::camera::Camera;
use crate::panel::Readout;
use crate::render;

/// How much room is left around the room when the view is fitted, in pixels.
const FIT_MARGIN: f64 = 40.0;

/// How often the frame rate is recomputed, in milliseconds.
const FPS_WINDOW: f64 = 500.0;

/// How far the arrow keys pan, in blocks.
const PAN_BLOCKS: f64 = 1.0;

/// The simulation and the view onto it.
pub struct View {
    /// The room, the drone and the firmware.
    pub sim: Simulator,
    /// Where the view is looking.
    pub camera: Camera,
    /// Whether the clock is running.
    pub running: bool,
    /// How many ticks are run per drawn frame.
    pub speed: u32,
    /// What a drag draws.
    pub kind: BlockType,

    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    width: f64,
    height: f64,
    device_pixel_ratio: f64,

    /// Whether a drag is drawing or rubbing out.
    painting: bool,
    erasing: bool,
    /// Whether a drag is moving the view.
    panning: bool,
    pan_from: (f64, f64),
    /// Where the cursor last was, in the room.
    mouse: Vec2,
    /// Whether the view still reframes itself when the canvas changes size.
    /// Set aside the moment the view is moved by hand.
    auto_fit: bool,

    fps: f64,
    frames: u32,
    fps_since: f64,
}

impl View {
    /// Sets up on a canvas, with the default room loaded and framed.
    #[must_use]
    pub fn new(canvas: HtmlCanvasElement) -> Option<Self> {
        let ctx = canvas
            .get_context("2d")
            .ok()??
            .dyn_into::<CanvasRenderingContext2d>()
            .ok()?;

        let mut view = Self {
            sim: Simulator::with_map(DEFAULT_MAP).ok()?,
            camera: Camera::default(),
            running: true,
            speed: 1,
            kind: BlockType::Wall,
            canvas,
            ctx,
            width: 0.0,
            height: 0.0,
            device_pixel_ratio: 1.0,
            painting: false,
            erasing: false,
            panning: false,
            pan_from: (0.0, 0.0),
            mouse: Vec2::ZERO,
            auto_fit: true,
            fps: 0.0,
            frames: 0,
            fps_since: 0.0,
        };

        view.resize();
        view.fit();

        Some(view)
    }

    /// Whether the canvas is still on the page.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.canvas.is_connected()
    }

    /// Matches the drawing buffer to the canvas's size on the page, at the
    /// display's pixel density.
    pub fn resize(&mut self) {
        let ratio = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio());

        self.width = f64::from(self.canvas.client_width());
        self.height = f64::from(self.canvas.client_height());
        self.device_pixel_ratio = ratio;

        self.canvas.set_width((self.width * ratio) as u32);
        self.canvas.set_height((self.height * ratio) as u32);

        // Setting the size resets the transform, so it goes back on here
        // rather than being scaled once at start-up.
        let _ = self.ctx.set_transform(ratio, 0.0, 0.0, ratio, 0.0, 0.0);
    }

    /// Frames the room, or the drone if the room is empty, and goes back to
    /// reframing it whenever the window changes size.
    pub fn fit(&mut self) {
        let (min, max) = self.bounds();
        self.camera.fit(min, max, self.free_area(), FIT_MARGIN);
        self.auto_fit = true;
    }

    /// The part of the canvas the panel is not sitting over.
    ///
    /// The canvas is the whole window and the panel floats above it, so
    /// framing the room against the canvas would tuck a third of it out of
    /// sight. The panel is down one side on a wide window and across the top on
    /// a narrow one, which is why this measures rather than assumes.
    fn free_area(&self) -> (f64, f64, f64, f64) {
        let whole = (0.0, 0.0, self.width, self.height);

        let Some(panel) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.query_selector(".panel").ok().flatten())
        else {
            return whole;
        };

        let panel = panel.get_bounding_client_rect();
        let canvas = self.canvas.get_bounding_client_rect();

        if panel.width() >= canvas.width() * 0.9 {
            let top = (panel.bottom() - canvas.top()).clamp(0.0, self.height);
            (0.0, top, self.width, (self.height - top).max(1.0))
        } else {
            let left = (panel.right() - canvas.left()).clamp(0.0, self.width);
            (left, 0.0, (self.width - left).max(1.0), self.height)
        }
    }

    /// The corners of everything worth looking at.
    fn bounds(&self) -> (Vec2, Vec2) {
        let mut min = self.sim.drone.pos;
        let mut max = self.sim.drone.pos;

        for block in &self.sim.blocks {
            min = Vec2::new(min.x.min(block.min.x), min.y.min(block.min.y));
            max = Vec2::new(max.x.max(block.max.x), max.y.max(block.max.y));
        }

        (min, max)
    }

    /// Matches the drawing buffer to the canvas whenever the page has changed
    /// its size, and reframes the room while the view is still automatic.
    ///
    /// Checked every frame rather than on a resize event, because the first
    /// time it matters is before any resize has happened: a canvas that has
    /// been mounted but not yet laid out measures 300 by 150, the intrinsic
    /// size the HTML specification gives one, and a buffer that size stretched
    /// over a window is a blurry mess.
    fn sync_size(&mut self) {
        let width = f64::from(self.canvas.client_width());
        let height = f64::from(self.canvas.client_height());
        let ratio = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio());

        let unchanged = (width - self.width).abs() < 0.5
            && (height - self.height).abs() < 0.5
            && (ratio - self.device_pixel_ratio).abs() < 0.001;

        if unchanged || width <= 0.0 || height <= 0.0 {
            return;
        }

        self.resize();

        if self.auto_fit {
            self.fit();
        }
    }

    /// One drawn frame, and however many simulated ticks go with it.
    pub fn frame(&mut self, now: f64) {
        self.sync_size();

        if self.running {
            for _ in 0..self.speed {
                self.sim.step();
            }
        }

        render::draw(&self.ctx, &self.sim, &self.camera, self.width, self.height);

        self.frames += 1;
        if now - self.fps_since >= FPS_WINDOW {
            self.fps = f64::from(self.frames) * 1000.0 / (now - self.fps_since);
            self.frames = 0;
            self.fps_since = now;
        }
    }

    /// Runs one tick without starting the clock.
    pub fn step_once(&mut self) {
        self.sim.step();
    }

    /// Puts the drone back at the start with a blank map, leaving the room as
    /// it is.
    pub fn reset(&mut self) {
        self.sim.reset();
    }

    /// Puts the default room back.
    pub fn reload_room(&mut self) {
        if self.sim.load_map(DEFAULT_MAP).is_ok() {
            self.sim.reset();
            self.fit();
        }
    }

    /// Takes every block away, so a room can be drawn from nothing.
    pub fn clear_room(&mut self) {
        self.sim.blocks.clear();
        self.sim.reset();
    }

    /// Where a pointer event happened, in the room.
    #[must_use]
    pub fn to_room(&self, client_x: f64, client_y: f64) -> Vec2 {
        let rect = self.canvas.get_bounding_client_rect();
        self.camera
            .to_world(client_x - rect.left(), client_y - rect.top())
    }

    /// Remembers where the cursor is, for the readout.
    pub fn track_cursor(&mut self, point: Vec2) {
        self.mouse = point;
    }

    /// Starts a drag.
    ///
    /// Whether the drag draws or rubs out is decided by what is under the
    /// cursor when it starts, as in the C: press on empty floor and the drag
    /// draws, press on a block and it rubs out. That way a wall can be dragged
    /// out and dragged away without a mode to switch.
    pub fn begin_paint(&mut self, point: Vec2) {
        self.painting = true;
        self.erasing = self.sim.block_at(point).is_some();
        self.paint(point);
    }

    /// Carries a drag on.
    pub fn paint(&mut self, point: Vec2) {
        if !self.painting {
            return;
        }

        if self.erasing {
            self.sim.remove_block(point);
        } else {
            self.sim.place_block(point, self.kind);
        }
    }

    /// Ends a drag.
    pub fn end_paint(&mut self) {
        self.painting = false;
        self.erasing = false;
    }

    /// Starts moving the view.
    pub fn begin_pan(&mut self, x: f64, y: f64) {
        self.panning = true;
        self.pan_from = (x, y);
    }

    /// Carries a view move on.
    pub fn pan(&mut self, x: f64, y: f64) {
        if !self.panning {
            return;
        }

        self.camera
            .pan_by_pixels(x - self.pan_from.0, y - self.pan_from.1);
        self.pan_from = (x, y);
        self.auto_fit = false;
    }

    /// Ends a view move.
    pub fn end_pan(&mut self) {
        self.panning = false;
    }

    /// Moves the view a block at a time, as the arrow keys do.
    ///
    /// At less than one pixel to the centimetre a block is a very small step,
    /// so the step grows as the view pulls out, which is what the C's rather
    /// more cryptic version of this worked out to.
    pub fn nudge(&mut self, dx: f64, dy: f64) {
        let blocks = (1.0 / self.camera.zoom).trunc().max(PAN_BLOCKS);
        let step = blocks * randie_sim::block::Block::SIZE;

        self.camera.pan_by_room(dx * step, dy * step);
        self.auto_fit = false;
    }

    /// Zooms about a point on the canvas.
    pub fn zoom_at(&mut self, client_x: f64, client_y: f64, factor: f64) {
        let rect = self.canvas.get_bounding_client_rect();
        self.camera
            .zoom_at(client_x - rect.left(), client_y - rect.top(), factor);
        self.auto_fit = false;
    }

    /// What the panel shows.
    #[must_use]
    pub fn readout(&self) -> Readout {
        let drone = &self.sim.drone;
        let nav = &drone.firmware.nav;
        let rep = &drone.firmware.rep;

        Readout {
            time_ms: self.sim.time,
            fps: self.fps.round() as u32,
            blocks: self.sim.blocks.len(),
            mouse: (self.mouse.x, self.mouse.y),

            pos: (drone.pos.x, drone.pos.y),
            height: drone.height,
            angle: normalize_degrees(drone.angle.to_degrees()),

            believed: (nav.posx, nav.posy),
            believed_angle: normalize_degrees(nav.angle.to_f64().to_degrees()),
            task: task_name(nav.task).to_string(),

            pitch: rep.fc.vel.y.to_f64(),
            roll: rep.fc.vel.x.to_f64(),
            throttle: rep.fc.vel.z.to_f64(),
            yaw: rep.fc.gyro.to_f64().to_degrees(),

            laser: (
                rep.laser.val_left.min(LASER_MAX_DISTANCE_CM),
                rep.laser.val_front.min(LASER_MAX_DISTANCE_CM),
                rep.laser.val_right.min(LASER_MAX_DISTANCE_CM),
            ),
            sonar: (rep.sonar.value, rep.sonar.valid),
            ir: (rep.ir_bottom.value, rep.ir_top.value),
        }
    }
}

/// An angle in the half-open range a compass would use.
fn normalize_degrees(degrees: f64) -> f64 {
    let wrapped = degrees % 360.0;

    if wrapped < 0.0 {
        wrapped + 360.0
    } else {
        wrapped
    }
}

/// What the drone is doing, in words rather than in the enum's spelling.
fn task_name(task: Task) -> &'static str {
    match task {
        Task::Idle => "Idle",
        Task::Turning => "Turning",
        Task::MoveForward => "Flying forward",
        Task::MoveUp => "Climbing",
        Task::MoveDown => "Descending",
        Task::FollowForward => "Following a wall",
        Task::FollowFurther => "Carrying on past it",
        Task::FollowCheck => "Checking for glass",
        Task::FollowTurn => "Turning a corner",
        Task::Searching => "Searching",
    }
}

/// Keeps drawing frames until the canvas leaves the page.
pub fn start_animation_loop(view: Rc<RefCell<View>>) {
    type Frame = Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>>;

    let next: Frame = Rc::new(RefCell::new(None));
    let first = Rc::clone(&next);

    *first.borrow_mut() = Some(Closure::wrap(Box::new(move |now: f64| {
        let live = {
            let mut view = view.borrow_mut();
            if view.is_live() {
                view.frame(now);
                true
            } else {
                false
            }
        };

        if live
            && let Some(window) = web_sys::window()
            && let Some(frame) = next.borrow().as_ref()
        {
            let _ = window.request_animation_frame(frame.as_ref().unchecked_ref());
        }
    }) as Box<dyn FnMut(f64)>));

    if let Some(window) = web_sys::window()
        && let Some(frame) = first.borrow().as_ref()
    {
        let _ = window.request_animation_frame(frame.as_ref().unchecked_ref());
    }
}
