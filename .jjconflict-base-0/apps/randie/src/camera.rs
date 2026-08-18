//! Where the view is looking.
//!
//! The transforms are the C renderer's: an offset in room coordinates and a
//! zoom, with the y-axis flipped because the room's y grows north and a
//! canvas's grows down.

use randie_sim::Vec2;

/// How far out the view can be pulled.
pub const MIN_ZOOM: f64 = 0.2;
/// How far in the view can be pushed.
pub const MAX_ZOOM: f64 = 10.0;

/// The view onto the room.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Camera {
    /// Canvas pixels to the centimetre.
    pub zoom: f64,
    /// The room coordinate at the canvas's top-left corner.
    pub offset: Vec2,
}

impl Default for Camera {
    /// The C renderer's opening view: no magnification, with the room's origin
    /// five metres in from the corner.
    fn default() -> Self {
        Self {
            zoom: 1.0,
            offset: Vec2::new(-500.0, 500.0),
        }
    }
}

impl Camera {
    /// A length in centimetres, in canvas pixels.
    #[must_use]
    pub fn scale(&self, centimetres: f64) -> f64 {
        centimetres * self.zoom
    }

    /// A room coordinate, on the canvas.
    #[must_use]
    pub fn to_screen(self, world: Vec2) -> (f64, f64) {
        (
            (world.x - self.offset.x) * self.zoom,
            -(world.y - self.offset.y) * self.zoom,
        )
    }

    /// A canvas coordinate, in the room.
    #[must_use]
    pub fn to_world(self, x: f64, y: f64) -> Vec2 {
        Vec2::new(
            x / self.zoom + self.offset.x,
            -(y / self.zoom - self.offset.y),
        )
    }

    /// Zooms by a factor about a point on the canvas, so whatever is under the
    /// cursor stays under it.
    ///
    /// The C's wheel handler changed the zoom on its own and let the view slide
    /// out from under the pointer.
    pub fn zoom_at(&mut self, x: f64, y: f64, factor: f64) {
        let before = self.to_world(x, y);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let after = self.to_world(x, y);

        self.offset += before - after;
    }

    /// Slides the view by a distance in canvas pixels.
    pub fn pan_by_pixels(&mut self, dx: f64, dy: f64) {
        self.offset += Vec2::new(-dx / self.zoom, dy / self.zoom);
    }

    /// Slides the view by a distance in centimetres.
    pub fn pan_by_room(&mut self, dx: f64, dy: f64) {
        self.offset += Vec2::new(dx, dy);
    }

    /// Frames a region of the room inside a rectangle of the canvas.
    ///
    /// The rectangle is `(left, top, width, height)` in canvas pixels, so the
    /// room can be centred on the part of the canvas the panel is not sitting
    /// over.
    ///
    /// The C opened on a fixed offset and a zoom of one, which put a room the
    /// size of the default one mostly off the bottom-right of a window it
    /// assumed was a thousand pixels square. The canvas here is whatever size
    /// the browser window is, so the view is worked out instead.
    pub fn fit(&mut self, min: Vec2, max: Vec2, area: (f64, f64, f64, f64), margin: f64) {
        let (left, top, width, height) = area;

        let room_width = (max.x - min.x).max(1.0);
        let room_height = (max.y - min.y).max(1.0);

        let usable_width = (width - margin * 2.0).max(1.0);
        let usable_height = (height - margin * 2.0).max(1.0);

        self.zoom = (usable_width / room_width)
            .min(usable_height / room_height)
            .clamp(MIN_ZOOM, MAX_ZOOM);

        // Put the region's middle at the middle of the rectangle.
        let centre = Vec2::new((min.x + max.x) / 2.0, (min.y + max.y) / 2.0);
        self.offset = Vec2::new(
            centre.x - (left + width / 2.0) / self.zoom,
            centre.y + (top + height / 2.0) / self.zoom,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_and_room_coordinates_are_inverses() {
        let camera = Camera::default();
        let world = Vec2::new(123.0, -456.0);
        let (x, y) = camera.to_screen(world);
        let back = camera.to_world(x, y);

        assert!((back.x - world.x).abs() < 1e-9);
        assert!((back.y - world.y).abs() < 1e-9);
    }

    #[test]
    fn north_is_up() {
        let camera = Camera::default();
        let (_, high) = camera.to_screen(Vec2::new(0.0, 100.0));
        let (_, low) = camera.to_screen(Vec2::new(0.0, 0.0));

        assert!(high < low);
    }

    #[test]
    fn zooming_keeps_the_point_under_the_cursor() {
        let mut camera = Camera::default();
        let before = camera.to_world(300.0, 200.0);

        camera.zoom_at(300.0, 200.0, 2.0);
        let after = camera.to_world(300.0, 200.0);

        assert!((after.x - before.x).abs() < 1e-9);
        assert!((after.y - before.y).abs() < 1e-9);
        assert!((camera.zoom - 2.0).abs() < 1e-9);
    }

    #[test]
    fn zooming_stops_at_the_ends_of_its_range() {
        let mut camera = Camera::default();

        for _ in 0..50 {
            camera.zoom_at(0.0, 0.0, 2.0);
        }
        assert_eq!(camera.zoom, MAX_ZOOM);

        for _ in 0..100 {
            camera.zoom_at(0.0, 0.0, 0.5);
        }
        assert_eq!(camera.zoom, MIN_ZOOM);
    }

    #[test]
    fn fitting_a_room_puts_it_in_the_middle_of_the_canvas() {
        let mut camera = Camera::default();
        let min = Vec2::new(0.0, -600.0);
        let max = Vec2::new(600.0, 0.0);

        camera.fit(min, max, (0.0, 0.0, 1000.0, 800.0), 20.0);

        let (x, y) = camera.to_screen(Vec2::new(300.0, -300.0));
        assert!((x - 500.0).abs() < 1e-6, "{x}");
        assert!((y - 400.0).abs() < 1e-6, "{y}");

        // And the whole room is on the canvas.
        let (left, top) = camera.to_screen(Vec2::new(min.x, max.y));
        let (right, bottom) = camera.to_screen(Vec2::new(max.x, min.y));
        assert!(left >= 0.0 && top >= 0.0);
        assert!(right <= 1000.0 && bottom <= 800.0);
    }

    #[test]
    fn fitting_into_a_rectangle_keeps_clear_of_the_rest() {
        let mut camera = Camera::default();
        let min = Vec2::new(0.0, -600.0);
        let max = Vec2::new(600.0, 0.0);

        // A 320-pixel panel down the left-hand side.
        camera.fit(min, max, (320.0, 0.0, 680.0, 800.0), 20.0);

        let (left, _) = camera.to_screen(Vec2::new(min.x, max.y));
        let (right, _) = camera.to_screen(Vec2::new(max.x, min.y));

        assert!(left >= 320.0, "the room runs under the panel: {left}");
        assert!(right <= 1000.0, "the room runs off the canvas: {right}");
        // And it is centred on what is left.
        assert!(((left + right) / 2.0 - 660.0).abs() < 1e-6);
    }

    #[test]
    fn panning_by_pixels_moves_the_view_the_other_way() {
        let mut camera = Camera::default();
        let before = camera.to_world(500.0, 500.0);

        // Dragging the room to the right shows what was to its left.
        camera.pan_by_pixels(100.0, 0.0);
        let after = camera.to_world(500.0, 500.0);

        assert!((after.x - (before.x - 100.0)).abs() < 1e-9);
    }
}
