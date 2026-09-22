//! Synthetic multi-scan E57 scenes: a warehouse-sized room (floor, ceiling, walls, pillars,
//! crates) scanned from several stations, like a terrestrial laser scanner would. Each
//! station casts rays in random directions, keeps the first hit, adds range noise, and
//! stores points in its own frame with its pose, so the import path sees what real
//! multi-station exports look like: density falling off with range, occlusion, and posed
//! scans. Deterministic for a given seed.

use e57::{
    E57Writer, Quaternion, Record, RecordDataType, RecordName, RecordValue, Transform, Translation,
};
use std::path::Path;

struct Rng(u64);

impl Rng {
    fn unit(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Standard normal (Box–Muller).
    fn normal(&mut self) -> f64 {
        let (u, v) = (self.unit().max(1e-300), self.unit());
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

/// Axis-aligned box with a surface colour.
struct Solid {
    lo: [f64; 3],
    hi: [f64; 3],
    rgb: [i64; 3],
}

/// The room: 40 m × 30 m × 5 m interior with 0.3 m walls, pillars and crates.
fn room() -> Vec<Solid> {
    let s = |lo: [f64; 3], hi: [f64; 3], rgb: [i64; 3]| Solid { lo, hi, rgb };
    let mut v = vec![
        s([-0.3, -0.3, -0.3], [40.3, 30.3, 0.0], [120, 115, 105]), // floor
        s([-0.3, -0.3, 5.0], [40.3, 30.3, 5.3], [200, 200, 205]),  // ceiling
        s([-0.3, -0.3, 0.0], [0.0, 30.3, 5.0], [180, 170, 150]),   // walls
        s([40.0, -0.3, 0.0], [40.3, 30.3, 5.0], [180, 170, 150]),
        s([0.0, -0.3, 0.0], [40.0, 0.0, 5.0], [170, 175, 160]),
        s([0.0, 30.0, 0.0], [40.0, 30.3, 5.0], [170, 175, 160]),
    ];
    for i in 0..4 {
        for j in 0..3 {
            let (x, y) = (8.0 + i as f64 * 8.0, 7.5 + j as f64 * 7.5);
            v.push(s(
                [x - 0.25, y - 0.25, 0.0],
                [x + 0.25, y + 0.25, 5.0],
                [140, 140, 150],
            ));
        }
    }
    let mut r = Rng(0x51_7cc1_b727_220a);
    for _ in 0..30 {
        let (x, y) = (2.0 + r.unit() * 35.0, 2.0 + r.unit() * 25.0);
        let (w, d, h) = (
            0.4 + r.unit() * 1.5,
            0.4 + r.unit() * 1.5,
            0.3 + r.unit() * 1.8,
        );
        let c = [
            90 + (r.unit() * 120.0) as i64,
            60 + (r.unit() * 100.0) as i64,
            40 + (r.unit() * 80.0) as i64,
        ];
        v.push(s([x, y, 0.0], [x + w, y + d, h], c));
    }
    v
}

/// Nearest hit of a ray on any solid: distance, solid index, and axis of the face hit.
fn cast(o: [f64; 3], d: [f64; 3], solids: &[Solid]) -> Option<(f64, usize, usize)> {
    let mut best: Option<(f64, usize, usize)> = None;
    for (i, s) in solids.iter().enumerate() {
        let (mut t0, mut t1, mut axis) = (0.0f64, f64::INFINITY, 0);
        let mut miss = false;
        for a in 0..3 {
            if d[a].abs() < 1e-12 {
                if o[a] < s.lo[a] || o[a] > s.hi[a] {
                    miss = true;
                    break;
                }
                continue;
            }
            let (mut near, mut far) = ((s.lo[a] - o[a]) / d[a], (s.hi[a] - o[a]) / d[a]);
            if near > far {
                std::mem::swap(&mut near, &mut far);
            }
            if near > t0 {
                t0 = near;
                axis = a;
            }
            t1 = t1.min(far);
            if t0 > t1 {
                miss = true;
                break;
            }
        }
        if !miss && t0 > 1e-6 && best.is_none_or(|b| t0 < b.0) {
            best = Some((t0, i, axis));
        }
    }
    best
}

pub struct Options {
    pub scans: usize,
    pub points_per_scan: u64,
    pub seed: u64,
}

/// Write the scene to `out`. Returns total points written.
pub fn generate(
    out: &Path,
    opts: &Options,
    progress: &mut dyn FnMut(usize, u64),
) -> e57::Result<u64> {
    let solids = room();
    let mut w = E57Writer::from_file(out, "locus-synthetic-room")?;
    let coord = |name| Record {
        name,
        data_type: RecordDataType::ScaledInteger {
            min: -700_000,
            max: 700_000,
            scale: 0.0001,
            offset: 0.0,
        },
    };
    let int = |name, max| Record {
        name,
        data_type: RecordDataType::Integer { min: 0, max },
    };
    let proto = vec![
        coord(RecordName::CartesianX),
        coord(RecordName::CartesianY),
        coord(RecordName::CartesianZ),
        int(RecordName::CartesianInvalidState, 2),
        int(RecordName::Intensity, 2047),
        int(RecordName::ColorRed, 255),
        int(RecordName::ColorGreen, 255),
        int(RecordName::ColorBlue, 255),
    ];
    let mut rng = Rng(opts.seed | 1);
    let mut total = 0;
    for s in 0..opts.scans {
        // Stations on a jittered grid across the room, 1.5 m high, each turned differently.
        let cols = (opts.scans as f64).sqrt().ceil() as usize;
        let rows = opts.scans.div_ceil(cols);
        let station = [
            (s % cols) as f64 * 36.0 / cols as f64 + 4.0 + rng.unit() * 2.0,
            (s / cols) as f64 * 26.0 / rows as f64 + 3.0 + rng.unit() * 2.0,
            1.5,
        ];
        let yaw = rng.unit() * std::f64::consts::TAU;
        let (sy, cy) = (yaw.sin(), yaw.cos());
        let mut pc = w.add_pointcloud(&format!("station-{s}"), proto.clone())?;
        pc.set_name(Some(format!("Station {}", s + 1)));
        let half = yaw / 2.0;
        pc.set_transform(Some(Transform {
            rotation: Quaternion {
                w: half.cos(),
                x: 0.0,
                y: 0.0,
                z: half.sin(),
            },
            translation: Translation {
                x: station[0],
                y: station[1],
                z: station[2],
            },
        }));
        for i in 0..opts.points_per_scan {
            // Uniform direction, skipping the scanner's own footprint below it.
            let (d, hit) = loop {
                let z = rng.unit() * 2.0 - 1.0;
                let a = rng.unit() * std::f64::consts::TAU;
                let r = (1.0 - z * z).sqrt();
                let d = [r * a.cos(), r * a.sin(), z];
                if z > -0.95 {
                    break (d, cast(station, d, &solids));
                }
            };
            // About 0.3% of beams get no return (glass, dark surfaces, out of range).
            let no_return = rng.unit() < 0.003;
            let values = match hit {
                Some((t, si, axis)) if !no_return => {
                    let range = t + rng.normal() * 0.001; // 1 mm range noise, 1σ
                    let world = [d[0] * range, d[1] * range, d[2] * range];
                    // Scanner frame: rotate by −yaw about z (the pose rotates it back).
                    let local = [
                        cy * world[0] + sy * world[1],
                        -sy * world[0] + cy * world[1],
                        world[2],
                    ];
                    let incidence = d[axis].abs();
                    let intensity =
                        ((incidence * 0.8 + 0.2) * (1.0 / (1.0 + t * 0.03)) * 2047.0) as i64;
                    let c = solids[si].rgb;
                    let shade = |v: i64| (v as f64 * (0.6 + 0.4 * incidence)) as i64;
                    vec![
                        RecordValue::ScaledInteger((local[0] / 0.0001).round() as i64),
                        RecordValue::ScaledInteger((local[1] / 0.0001).round() as i64),
                        RecordValue::ScaledInteger((local[2] / 0.0001).round() as i64),
                        RecordValue::Integer(0),
                        RecordValue::Integer(intensity.clamp(0, 2047)),
                        RecordValue::Integer(shade(c[0]).clamp(0, 255)),
                        RecordValue::Integer(shade(c[1]).clamp(0, 255)),
                        RecordValue::Integer(shade(c[2]).clamp(0, 255)),
                    ]
                }
                _ => vec![
                    RecordValue::ScaledInteger(0),
                    RecordValue::ScaledInteger(0),
                    RecordValue::ScaledInteger(0),
                    RecordValue::Integer(2),
                    RecordValue::Integer(0),
                    RecordValue::Integer(0),
                    RecordValue::Integer(0),
                    RecordValue::Integer(0),
                ],
            };
            pc.add_point(values)?;
            if i % 1_000_000 == 0 {
                progress(s, i);
            }
        }
        pc.finalize()?;
        total += opts.points_per_scan;
    }
    w.finalize()?;
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rays_hit_the_nearest_face() {
        let solids = room();
        // Straight down from 1.5 m: the floor at z = 0.
        let (t, i, axis) = cast([20.0, 3.0, 1.5], [0.0, 0.0, -1.0], &solids).unwrap();
        assert!((t - 1.5).abs() < 1e-12);
        assert_eq!((i, axis), (0, 2));
        // Along +x from x = 1 at y = 0.5: the west... east wall at x = 40.
        let (t, _, axis) = cast([1.0, 0.5, 4.9], [1.0, 0.0, 0.0], &solids).unwrap();
        assert!((t - 39.0).abs() < 1e-12);
        assert_eq!(axis, 0);
    }
}
