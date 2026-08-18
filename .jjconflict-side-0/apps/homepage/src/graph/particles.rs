use glam::Vec3;

use super::data::GraphData;
use super::simulation::Simulation;

pub struct ParticleSystem {
    /// For each link: the progress [0..1) of each of 2 particles
    t_values: Vec<[f32; 2]>,
    speed: f32,
}

impl ParticleSystem {
    pub fn new(data: &GraphData) -> Self {
        let mut t_values = Vec::with_capacity(data.links.len());
        for _ in &data.links {
            t_values.push([0.0, 0.5]); // Two particles per link, offset by 0.5
        }
        Self {
            t_values,
            // react-force-graph's default linkDirectionalParticleSpeed.
            speed: 0.01,
        }
    }

    pub fn tick(&mut self) {
        for t in &mut self.t_values {
            t[0] = (t[0] + self.speed) % 1.0;
            t[1] = (t[1] + self.speed) % 1.0;
        }
    }

    /// Returns positions of particle pairs for each link
    pub fn positions(&self, simulation: &Simulation, data: &GraphData) -> Vec<(Vec3, Vec3)> {
        let mut result = Vec::with_capacity(data.links.len());
        for (i, link) in data.links.iter().enumerate() {
            let si = data.node_index(&link.source);
            let ti = data.node_index(&link.target);
            if let (Some(si), Some(ti)) = (si, ti) {
                let s = simulation.positions[si];
                let t = simulation.positions[ti];
                let p1 = s.lerp(t, self.t_values[i][0]);
                let p2 = s.lerp(t, self.t_values[i][1]);
                result.push((p1, p2));
            }
        }
        result
    }
}
