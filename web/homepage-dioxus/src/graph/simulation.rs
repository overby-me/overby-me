use glam::Vec3;

use super::data::{LINKS, NODES};

// Faithful port of d3-force-3d as configured by react-force-graph-3d, whose
// defaults the reference homepage (web/homepage) relies on. The simulation runs
// three forces every tick in insertion order (link, charge, center), then
// integrates positions with velocity decay, matching d3's `tick()`.

// Simulation schedule (d3-force defaults; force-graph sets the same values).
const ALPHA_MIN: f32 = 0.001;
// alphaDecay = 1 - alphaMin^(1/300) ≈ 0.0228, so the layout settles in ~300 ticks.
const ALPHA_DECAY: f32 = 0.022_807_765;
// velocityDecay 0.4 in d3's API stores (1 - 0.4) = 0.6 as the per-tick multiplier.
const VELOCITY_DECAY: f32 = 0.6;

// forceManyBody defaults.
const CHARGE_STRENGTH: f32 = -30.0;
const DISTANCE_MIN_SQ: f32 = 1.0;

// forceLink defaults.
const LINK_DISTANCE: f32 = 30.0;

// forceCenter default strength.
const CENTER_STRENGTH: f32 = 1.0;

// d3's phyllotaxis seeding for initial node positions (deterministic).
const INITIAL_RADIUS: f32 = 10.0;

pub struct Simulation {
    pub positions: Vec<Vec3>,
    pub velocities: Vec<Vec3>,
    alpha: f32,
    // Where alpha decays toward. 0 while resting; raised to reheat during a drag.
    alpha_target: f32,
    // Pinned nodes (d3's fx/fy/fz): held at a fixed position, still exerting
    // forces on others but not integrated themselves. Set while dragging.
    fixed: Vec<Option<Vec3>>,
    // Precomputed link topology (indices into NODES) and d3's degree-derived
    // per-link strength and bias.
    link_source: Vec<usize>,
    link_target: Vec<usize>,
    link_strength: Vec<f32>,
    link_bias: Vec<f32>,
    // Deterministic RNG for d3's `jiggle` (only used for coincident nodes).
    rng: u32,
}

impl Simulation {
    pub fn new() -> Self {
        let n = NODES.len();

        // d3's initializeNodes: 3D phyllotaxis spiral, no randomness.
        let initial_angle_roll = std::f32::consts::PI * (3.0 - 5.0_f32.sqrt());
        let initial_angle_yaw = std::f32::consts::PI * 20.0 / (9.0 + 221.0_f32.sqrt());
        let mut positions = Vec::with_capacity(n);
        for i in 0..n {
            let radius = INITIAL_RADIUS * (0.5 + i as f32).cbrt();
            let roll = i as f32 * initial_angle_roll;
            let yaw = i as f32 * initial_angle_yaw;
            positions.push(Vec3::new(
                radius * roll.sin() * yaw.cos(),
                radius * roll.cos(),
                radius * roll.sin() * yaw.sin(),
            ));
        }
        let velocities = vec![Vec3::ZERO; n];

        // Node degrees drive forceLink's strength and bias.
        let mut degree = vec![0u32; n];
        let mut link_source = Vec::with_capacity(LINKS.len());
        let mut link_target = Vec::with_capacity(LINKS.len());
        for link in LINKS {
            if let (Some(s), Some(t)) = (node_index(link.source), node_index(link.target)) {
                degree[s] += 1;
                degree[t] += 1;
                link_source.push(s);
                link_target.push(t);
            }
        }
        let mut link_strength = Vec::with_capacity(link_source.len());
        let mut link_bias = Vec::with_capacity(link_source.len());
        for k in 0..link_source.len() {
            let ds = degree[link_source[k]];
            let dt = degree[link_target[k]];
            link_strength.push(1.0 / ds.min(dt) as f32);
            link_bias.push(ds as f32 / (ds + dt) as f32);
        }

        Self {
            positions,
            velocities,
            alpha: 1.0,
            alpha_target: 0.0,
            fixed: vec![None; n],
            link_source,
            link_target,
            link_strength,
            link_bias,
            rng: 0x9e37_79b9,
        }
    }

    pub fn is_active(&self) -> bool {
        // Keep ticking while cooling down, or while a reheat target holds it warm
        // (e.g. during a node drag).
        self.alpha > ALPHA_MIN || self.alpha_target > ALPHA_MIN
    }

    /// Raise the reheat target (d3's `alphaTarget`). Set to 0.3 while dragging
    /// so neighbours keep relaxing, and back to 0.0 on release.
    pub fn set_alpha_target(&mut self, target: f32) {
        self.alpha_target = target;
    }

    /// Pin a node to a fixed world position (d3's fx/fy/fz) and reset its
    /// velocity, so forces read its position but never move it.
    pub fn pin(&mut self, index: usize, pos: Vec3) {
        self.fixed[index] = Some(pos);
        self.positions[index] = pos;
        self.velocities[index] = Vec3::ZERO;
    }

    /// Release a previously pinned node back into the simulation.
    pub fn unpin(&mut self, index: usize) {
        self.fixed[index] = None;
    }

    pub fn tick(&mut self) {
        if !self.is_active() {
            return;
        }

        // d3 tick(): advance alpha, run the forces, then integrate.
        self.alpha += (self.alpha_target - self.alpha) * ALPHA_DECAY;
        let alpha = self.alpha;

        self.apply_link_force(alpha);
        self.apply_charge_force(alpha);
        self.apply_center_force();

        for i in 0..self.positions.len() {
            if let Some(p) = self.fixed[i] {
                // Pinned node: velocity stays zero, position snaps to the pin.
                self.velocities[i] = Vec3::ZERO;
                self.positions[i] = p;
            } else {
                self.velocities[i] *= VELOCITY_DECAY;
                self.positions[i] += self.velocities[i];
            }
        }
    }

    // forceLink: a spring toward LINK_DISTANCE using the anticipated next
    // position (position + velocity), split between endpoints by degree bias.
    fn apply_link_force(&mut self, alpha: f32) {
        let mut rng = self.rng;
        for k in 0..self.link_source.len() {
            let s = self.link_source[k];
            let t = self.link_target[k];

            let ps = self.positions[s] + self.velocities[s];
            let pt = self.positions[t] + self.velocities[t];
            let mut d = pt - ps;
            if d.x == 0.0 {
                d.x = jiggle(&mut rng);
            }
            if d.y == 0.0 {
                d.y = jiggle(&mut rng);
            }
            if d.z == 0.0 {
                d.z = jiggle(&mut rng);
            }

            let len = d.length();
            let scale = (len - LINK_DISTANCE) / len * alpha * self.link_strength[k];
            d *= scale;

            let b = self.link_bias[k];
            self.velocities[t] -= d * b;
            self.velocities[s] += d * (1.0 - b);
        }
        self.rng = rng;
    }

    // forceManyBody: all-pairs charge. Equal per-node strengths make the naive
    // O(n^2) sum symmetric, so each pair is evaluated once. Matches d3's exact
    // (non-approximated) result: acceleration is the raw delta times
    // strength * alpha / distance^2, i.e. magnitude falls off as 1/distance.
    fn apply_charge_force(&mut self, alpha: f32) {
        let n = self.positions.len();
        let mut rng = self.rng;
        for i in 0..n {
            for j in (i + 1)..n {
                let mut d = self.positions[j] - self.positions[i];
                let mut l = d.length_squared();
                if d.x == 0.0 {
                    d.x = jiggle(&mut rng);
                    l += d.x * d.x;
                }
                if d.y == 0.0 {
                    d.y = jiggle(&mut rng);
                    l += d.y * d.y;
                }
                if d.z == 0.0 {
                    d.z = jiggle(&mut rng);
                    l += d.z * d.z;
                }
                if l < DISTANCE_MIN_SQ {
                    l = (DISTANCE_MIN_SQ * l).sqrt();
                }
                let w = CHARGE_STRENGTH * alpha / l;
                let force = d * w;
                self.velocities[i] += force;
                self.velocities[j] -= force;
            }
        }
        self.rng = rng;
    }

    // forceCenter: rigidly translate all nodes so their centroid returns to the
    // origin. This does not add velocity, so it never pulls nodes inward (the
    // previous spring-to-origin was what collapsed the graph).
    fn apply_center_force(&mut self) {
        let n = self.positions.len();
        if n == 0 {
            return;
        }
        let mut sum = Vec3::ZERO;
        for p in &self.positions {
            sum += *p;
        }
        let shift = sum / n as f32 * CENTER_STRENGTH;
        for p in &mut self.positions {
            *p -= shift;
        }
    }
}

fn node_index(id: &str) -> Option<usize> {
    NODES.iter().position(|n| n.id == id)
}

// d3's jiggle: a tiny nudge to break exact coincidences. xorshift32 keeps it
// deterministic (Math.random is unavailable and reproducibility is desirable).
fn jiggle(rng: &mut u32) -> f32 {
    let mut s = *rng;
    s ^= s << 13;
    s ^= s >> 17;
    s ^= s << 5;
    *rng = s;
    (s as f32 / u32::MAX as f32 - 0.5) * 1e-6
}

#[cfg(test)]
mod tests {
    use super::*;

    // Run the layout to rest and report its extent. Guards against the graph
    // collapsing to the center (the bug this port fixes) and pins down the
    // camera framing distance.
    #[test]
    fn layout_settles_without_collapsing() {
        let mut sim = Simulation::new();
        let mut ticks = 0;
        while sim.is_active() {
            sim.tick();
            ticks += 1;
            assert!(ticks < 5000, "simulation never settled");
        }

        let max_radius = sim
            .positions
            .iter()
            .map(|p| p.length())
            .fold(0.0_f32, f32::max);
        let min_radius = sim
            .positions
            .iter()
            .map(|p| p.length())
            .fold(f32::MAX, f32::min);

        println!("settled after {ticks} ticks; radius {min_radius:.1}..{max_radius:.1}");

        // Nodes must spread out, not pile up at the origin.
        assert!(
            max_radius > 40.0,
            "graph collapsed: max radius {max_radius}"
        );
        // And the whole thing must stay finite/bounded (no runaway explosion).
        assert!(
            max_radius < 1000.0,
            "graph exploded: max radius {max_radius}"
        );
    }
}
