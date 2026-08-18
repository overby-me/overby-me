use glam::{Mat4, Vec3, Vec4};

use super::data::GraphData;
use super::simulation::Simulation;

pub struct HitResult {
    pub node_index: usize,
}

/// Unproject screen coordinates into a world-space ray (origin, direction).
pub fn screen_ray(
    screen_x: f32,
    screen_y: f32,
    canvas_width: f32,
    canvas_height: f32,
    view: &Mat4,
    proj: &Mat4,
) -> (Vec3, Vec3) {
    // Convert screen coords to NDC
    let ndc_x = (2.0 * screen_x / canvas_width) - 1.0;
    let ndc_y = 1.0 - (2.0 * screen_y / canvas_height);

    // Inverse projection * view
    let inv_vp = (*proj * *view).inverse();

    let near_point = inv_vp * Vec4::new(ndc_x, ndc_y, -1.0, 1.0);
    let far_point = inv_vp * Vec4::new(ndc_x, ndc_y, 1.0, 1.0);

    let near = Vec3::new(
        near_point.x / near_point.w,
        near_point.y / near_point.w,
        near_point.z / near_point.w,
    );
    let far = Vec3::new(
        far_point.x / far_point.w,
        far_point.y / far_point.w,
        far_point.z / far_point.w,
    );

    (near, (far - near).normalize())
}

/// Intersect a ray with a plane defined by a point and normal. Returns the hit
/// point, or None if the ray is parallel to (or points away from) the plane.
pub fn intersect_plane(
    origin: Vec3,
    dir: Vec3,
    plane_point: Vec3,
    plane_normal: Vec3,
) -> Option<Vec3> {
    let denom = dir.dot(plane_normal);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (plane_point - origin).dot(plane_normal) / denom;
    if t < 0.0 {
        return None;
    }
    Some(origin + dir * t)
}

/// Unproject screen coordinates to a ray in world space, test against node spheres.
#[allow(clippy::too_many_arguments)]
pub fn pick_node(
    screen_x: f32,
    screen_y: f32,
    canvas_width: f32,
    canvas_height: f32,
    view: &Mat4,
    proj: &Mat4,
    simulation: &Simulation,
    data: &GraphData,
) -> Option<HitResult> {
    let (ray_origin, ray_dir) =
        screen_ray(screen_x, screen_y, canvas_width, canvas_height, view, proj);

    let mut closest: Option<(usize, f32)> = None;

    for (i, node) in data.nodes.iter().enumerate() {
        let center = simulation.positions[i];
        let radius = if node.center {
            20.0
        } else if node.color.is_some() {
            15.0
        } else {
            9.0
        };

        if let Some(t) = ray_sphere_intersect(ray_origin, ray_dir, center, radius)
            && closest.is_none_or(|(_, ct)| t < ct)
        {
            closest = Some((i, t));
        }
    }

    closest.map(|(idx, _)| HitResult { node_index: idx })
}

fn ray_sphere_intersect(origin: Vec3, dir: Vec3, center: Vec3, radius: f32) -> Option<f32> {
    let oc = origin - center;
    let a = dir.dot(dir);
    let b = 2.0 * oc.dot(dir);
    let c = oc.dot(oc) - radius * radius;
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return None;
    }
    let t = (-b - discriminant.sqrt()) / (2.0 * a);
    if t > 0.0 { Some(t) } else { None }
}

/// Project a 3D world position to 2D screen coordinates.
pub fn project_to_screen(
    world_pos: Vec3,
    view: &Mat4,
    proj: &Mat4,
    canvas_width: f32,
    canvas_height: f32,
) -> (f32, f32) {
    let clip = *proj * *view * Vec4::new(world_pos.x, world_pos.y, world_pos.z, 1.0);
    let ndc_x = clip.x / clip.w;
    let ndc_y = clip.y / clip.w;
    let screen_x = (ndc_x + 1.0) * 0.5 * canvas_width;
    let screen_y = (1.0 - ndc_y) * 0.5 * canvas_height;
    (screen_x, screen_y)
}
