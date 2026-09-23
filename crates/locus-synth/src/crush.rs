//! A vehicle's front corner for the volumetric crush tool: an undamaged reference and a copy
//! with a dent of exactly known volume, each placed in the scene by its own pose.

use crate::Pose;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub seed: u64,
    /// Paraboloid dent in the front face: radius and depth (m).
    pub radius: f64,
    pub depth: f64,
    /// Range noise, 1σ (m).
    pub noise: f64,
    /// Where each vehicle stands: translation (m) and heading (°).
    pub reference_at: [f64; 3],
    pub reference_heading_deg: f64,
    pub damaged_at: [f64; 3],
    pub damaged_heading_deg: f64,
    /// The symmetric vehicle (placed where the damaged one is): how far its left front stands
    /// proud of a mirror image of its right (m).
    pub asymmetry: f64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            seed: 1,
            radius: 0.25,
            depth: 0.08,
            noise: 0.002,
            reference_at: [4.0, 4.0, 0.0],
            reference_heading_deg: 15.0,
            damaged_at: [4.0, -1.5, 0.0],
            damaged_heading_deg: -10.0,
            asymmetry: 0.001,
        }
    }
}

/// The dent's centre on the front face, in the vehicle's frame.
pub const DENT_CENTRE: [f64; 3] = [0.0, 0.8, 0.5];

/// Undamaged features to pick on both vehicles (vehicle frame): on the side, the bonnet and
/// the front's far end, well away from the dent.
pub const FEATURES: [[f64; 3]; 4] = [
    [0.5, 0.0, 0.5],
    [0.4, 0.3, 1.0],
    [0.4, 1.3, 1.0],
    [0.0, 1.5, 0.15],
];

/// The dent's exact volume, π R² D / 2.
pub fn dent_volume(radius: f64, depth: f64) -> f64 {
    std::f64::consts::PI * radius * radius * depth / 2.0
}

struct Rng(u64);
impl Rng {
    fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn gauss(&mut self) -> f64 {
        let (u, v) = (self.uniform().max(1e-300), self.uniform());
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

/// The corner in its own frame: the front (x = 0, outward −x), a side (y = 0) and the bonnet
/// (z = 1), 1.6 m wide and 1 m high and deep, sampled about every 8 mm with `noise` 1σ, and
/// the dent (radius, depth) at `DENT_CENTRE` when given.
pub fn vehicle(dent: Option<(f64, f64)>, noise: f64, seed: u64) -> Vec<[f64; 3]> {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ seed.wrapping_add(1));
    let mut out = vec![];
    let s = 0.008;
    for a in 0..(1.6 / s) as usize {
        for b in 0..(1.0 / s) as usize {
            let (p, q) = (
                a as f64 * s + rng.uniform() * s,
                b as f64 * s + rng.uniform() * s,
            );
            let mut x = noise * rng.gauss();
            if let Some((r, d)) = dent {
                let r2 = (p - DENT_CENTRE[1]).powi(2) + (q - DENT_CENTRE[2]).powi(2);
                if r2 < r * r {
                    x += d * (1.0 - r2 / (r * r));
                }
            }
            out.push([x, p, q]);
            if a < (1.0 / s) as usize {
                out.push([p, noise * rng.gauss(), q]);
            }
            out.push([q, p, 1.0 + noise * rng.gauss()]);
        }
    }
    out
}

/// The symmetric vehicle's dent centre (vehicle frame): right of the centre plane y = 0.
pub const SYMMETRIC_DENT: [f64; 3] = [0.0, 0.35, 0.5];

/// Symmetric features to pick (left, right), vehicle frame: on the sides, the bonnet and the
/// front's lower corners.
pub const SYMMETRIC_PAIRS: [([f64; 3], [f64; 3]); 3] = [
    ([0.5, -0.8, 0.5], [0.5, 0.8, 0.5]),
    ([0.4, -0.5, 1.0], [0.4, 0.5, 1.0]),
    ([0.0, -0.7, 0.15], [0.0, 0.7, 0.15]),
];

/// A whole vehicle front, symmetric about y = 0: the front (x = 0, outward −x) from y = −0.8
/// to 0.8, both sides (y = ±0.8) and the bonnet (z = 1), 1 m deep and high, sampled about every
/// 8 mm with `noise` 1σ. The dent (radius, depth) at `SYMMETRIC_DENT`; `asymmetry` moves the
/// left half of the front (y < 0) that far outward, as real vehicles differ side to side.
pub fn symmetric_vehicle(
    dent: Option<(f64, f64)>,
    noise: f64,
    asymmetry: f64,
    seed: u64,
) -> Vec<[f64; 3]> {
    let mut rng = Rng(0x51_7cc1_b727_220a ^ seed.wrapping_add(1));
    let mut out = vec![];
    let s = 0.008;
    for a in 0..(1.6 / s) as usize {
        for b in 0..(1.0 / s) as usize {
            let (y, q) = (
                -0.8 + a as f64 * s + rng.uniform() * s,
                b as f64 * s + rng.uniform() * s,
            );
            let mut x = noise * rng.gauss() - if y < 0.0 { asymmetry } else { 0.0 };
            if let Some((r, d)) = dent {
                let r2 = (y - SYMMETRIC_DENT[1]).powi(2) + (q - SYMMETRIC_DENT[2]).powi(2);
                if r2 < r * r {
                    x += d * (1.0 - r2 / (r * r));
                }
            }
            out.push([x, y, q]);
            out.push([q, y, 1.0 + noise * rng.gauss()]);
            if a < (1.0 / s) as usize {
                let p = a as f64 * s + rng.uniform() * s;
                out.push([p, -0.8 + noise * rng.gauss(), q]);
                out.push([p, 0.8 + noise * rng.gauss(), q]);
            }
        }
    }
    out
}

pub fn pose(at: [f64; 3], heading_deg: f64) -> Pose {
    Pose::from_yaw_pitch_roll(heading_deg.to_radians(), 0.0, 0.0, at)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Truth {
    pub options: Options,
    pub volume: f64,
    /// The features on each vehicle (scene frame), in `FEATURES` order.
    pub reference_features: Vec<[f64; 3]>,
    pub damaged_features: Vec<[f64; 3]>,
    /// A damage region around the dent (scene frame, axis-aligned), as an examiner would set it.
    pub region: [[f64; 3]; 2],
    /// The symmetric vehicle: its symmetric pairs (left, right) and its damage region.
    pub symmetric_pairs: Vec<([f64; 3], [f64; 3])>,
    pub symmetric_region: [[f64; 3]; 2],
}

/// An axis-aligned box (scene frame) around a dent at `c` (vehicle frame) of radius `r` and
/// depth `d`, the vehicle placed by `pose`.
fn dent_region(pose: &Pose, c: [f64; 3], r: f64, d: f64) -> [[f64; 3]; 2] {
    let m = r + 0.1;
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for k in 0..8 {
        let p = pose.apply([
            if k & 1 == 0 { -0.25 } else { d + 0.1 },
            c[1] + if k & 2 == 0 { -m } else { m },
            c[2] + if k & 4 == 0 { -m } else { m },
        ]);
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    [lo, hi]
}

pub fn truth(o: &Options) -> Truth {
    let (rp, dp) = (
        pose(o.reference_at, o.reference_heading_deg),
        pose(o.damaged_at, o.damaged_heading_deg),
    );
    Truth {
        options: o.clone(),
        volume: dent_volume(o.radius, o.depth),
        reference_features: FEATURES.iter().map(|f| rp.apply(*f)).collect(),
        damaged_features: FEATURES.iter().map(|f| dp.apply(*f)).collect(),
        region: dent_region(&dp, DENT_CENTRE, o.radius, o.depth),
        symmetric_pairs: SYMMETRIC_PAIRS
            .iter()
            .map(|(a, b)| (dp.apply(*a), dp.apply(*b)))
            .collect(),
        symmetric_region: dent_region(&dp, SYMMETRIC_DENT, o.radius.min(0.25), o.depth),
    }
}
