use glam::{Mat4, Vec3};

pub struct Camera {
    pub theta: f32,
    pub phi: f32,
    pub distance: f32,
    pub fov: f32,
    pub near: f32,
    pub far: f32,
    aspect: f32,
    is_dragging: bool,
    last_mouse_x: f32,
    last_mouse_y: f32,
}

impl Camera {
    pub fn new() -> Self {
        Self {
            theta: 0.0,
            phi: std::f32::consts::FRAC_PI_4,
            // Frames the settled d3 layout (max radius ~115) with margin.
            distance: 240.0,
            fov: 75.0_f32.to_radians(),
            near: 0.1,
            far: 10000.0,
            aspect: 1.0,
            is_dragging: false,
            last_mouse_x: 0.0,
            last_mouse_y: 0.0,
        }
    }

    pub fn set_aspect(&mut self, aspect: f32) {
        self.aspect = aspect;
    }

    pub fn eye_position(&self) -> Vec3 {
        let x = self.distance * self.phi.sin() * self.theta.cos();
        let y = self.distance * self.phi.cos();
        let z = self.distance * self.phi.sin() * self.theta.sin();
        Vec3::new(x, y, z)
    }

    pub fn view_matrix(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye_position(), Vec3::ZERO, Vec3::Y)
    }

    pub fn projection_matrix(&self) -> Mat4 {
        Mat4::perspective_rh_gl(self.fov, self.aspect, self.near, self.far)
    }

    pub fn on_mouse_down(&mut self, x: f32, y: f32) {
        self.is_dragging = true;
        self.last_mouse_x = x;
        self.last_mouse_y = y;
    }

    pub fn on_mouse_move(&mut self, x: f32, y: f32) {
        if !self.is_dragging {
            return;
        }
        let dx = x - self.last_mouse_x;
        let dy = y - self.last_mouse_y;
        self.theta += dx * 0.005;
        self.phi = (self.phi - dy * 0.005).clamp(0.1, std::f32::consts::PI - 0.1);
        self.last_mouse_x = x;
        self.last_mouse_y = y;
    }

    pub fn on_mouse_up(&mut self) {
        self.is_dragging = false;
    }

    pub fn on_wheel(&mut self, delta: f32) {
        self.distance = (self.distance + delta * 0.5).clamp(50.0, 2000.0);
    }
}
