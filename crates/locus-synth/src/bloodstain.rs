//! Bloodstain impacts with known ground truth: droplets thrown from a known origin onto the
//! floor and walls of a room, each stain's true impact and directional angles, and the
//! ellipse an examiner would measure, with noise on its width, length and orientation.
//!
//! Droplets fly either in straight lines (the model the area-of-origin method assumes, to
//! test its arithmetic) or under gravity and air drag (to show how far the straight-line
//! method is off for real flight: it tends to put the origin too high).
//!
//! Measurement noise (the defaults are assumptions, to be checked against casework):
//! each edge of a stain is located with 1σ `edge_fixed + edge_rel × axis`, so a width or
//! length measured edge to edge has √2 times that; the long axis's orientation is off by
//! about the edge error over half the difference of the axes, so it is well defined for a
//! long stain and poorly for a nearly round one.

use crate::{add, cross, dot, scale, sub, Point, Rng};
use serde::{Deserialize, Serialize};

type V3 = [f64; 3];

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Flight {
    Straight,
    /// Gravity (9.81 m/s²) and quadratic drag on a sphere of the droplet's diameter.
    Ballistic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Options {
    pub seed: u64,
    pub origin: V3,
    pub droplets: usize,
    pub flight: Flight,
    /// Launch speed range (m/s), for ballistic flight.
    pub speed: [f64; 2],
    /// Droplet diameter range (m).
    pub diameter: [f64; 2],
    /// Stain width as a multiple of the droplet diameter (spreading on impact).
    pub spread: f64,
    /// Room: floor at z = 0, walls at x = 0, y = 0 and x = `room[0]`, y = `room[1]`.
    pub room: [f64; 2],
    pub edge_fixed: f64,
    pub edge_rel: f64,
    /// Stains narrower than this, or with impact angles outside this range (degrees), are
    /// not kept (an examiner wouldn't select them).
    pub min_width: f64,
    pub angle_range: [f64; 2],
    /// Stain photos: pixels per millimetre.
    pub px_per_mm: f64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            seed: 1,
            origin: [2.2, 1.6, 1.1],
            droplets: 300,
            flight: Flight::Straight,
            speed: [3.0, 8.0],
            diameter: [0.0008, 0.0025],
            spread: 2.2,
            room: [4.0, 3.5],
            edge_fixed: 0.0001,
            edge_rel: 0.015,
            min_width: 0.0015,
            angle_range: [10.0, 80.0],
            px_per_mm: 20.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stain {
    /// Which surface: "floor", "wall x=0", …
    pub surface: String,
    /// Stain centre (m, project frame) and the surface's normal (into the room).
    pub centre: V3,
    pub normal: V3,
    /// Unit direction of travel at impact.
    pub velocity: V3,
    /// True angle between the droplet's path and the surface, degrees.
    pub impact_deg: f64,
    /// Direction of travel within the surface (the tail's direction), unit.
    pub travel: V3,
    /// True width and length (m).
    pub width: f64,
    pub length: f64,
    /// As measured: width, length, and the long axis direction within the surface (unit,
    /// pointing the way the examiner marked the tail).
    pub measured_width: f64,
    pub measured_length: f64,
    pub measured_axis: V3,
    /// Impact angle from the measured ellipse, asin(width / length), degrees.
    pub measured_impact_deg: f64,
    /// Travelling upward at impact (the stains an area-of-origin estimate should use).
    pub upward: bool,
    /// The photo of this stain: file name (when written) and its mapping to the surface.
    pub photo: Photo,
}

/// A stain photo: `px = centre_px + (dot(p − centre, u), dot(p − centre, v)) × scale`,
/// with image rows running down (−v). Three fiducial crosses are drawn at known points.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Photo {
    pub file: String,
    pub size_px: [u32; 2],
    /// Pixels per metre.
    pub scale: f64,
    pub u: V3,
    pub v: V3,
    pub fiducials: Vec<Fiducial>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fiducial {
    pub world: V3,
    pub px: [f64; 2],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Truth {
    pub options: Options,
    pub stains: Vec<Stain>,
    /// Droplets that hit nothing usable (too small, too grazing, out of the room).
    pub discarded: usize,
}

fn unit(a: V3) -> V3 {
    scale(a, 1.0 / dot(a, a).sqrt())
}

struct Surface {
    name: &'static str,
    /// Plane: dot(n, p) = d, n pointing into the room.
    n: V3,
    d: f64,
}

fn surfaces(room: [f64; 2]) -> [Surface; 5] {
    [
        Surface {
            name: "floor",
            n: [0.0, 0.0, 1.0],
            d: 0.0,
        },
        Surface {
            name: "wall x=0",
            n: [1.0, 0.0, 0.0],
            d: 0.0,
        },
        Surface {
            name: "wall y=0",
            n: [0.0, 1.0, 0.0],
            d: 0.0,
        },
        Surface {
            name: "wall x=max",
            n: [-1.0, 0.0, 0.0],
            d: -room[0],
        },
        Surface {
            name: "wall y=max",
            n: [0.0, -1.0, 0.0],
            d: -room[1],
        },
    ]
}

/// First surface the segment a→b crosses (going from inside to outside).
fn hit(ss: &[Surface], a: V3, b: V3) -> Option<(usize, V3)> {
    let mut best: Option<(f64, usize)> = None;
    for (k, s) in ss.iter().enumerate() {
        let (fa, fb) = (dot(s.n, a) - s.d, dot(s.n, b) - s.d);
        if fa >= 0.0 && fb < 0.0 {
            let t = fa / (fa - fb);
            if best.is_none_or(|(bt, _)| t < bt) {
                best = Some((t, k));
            }
        }
    }
    best.map(|(t, k)| (k, add(a, scale(sub(b, a), t))))
}

/// Fly a droplet from `p` with velocity `v`; returns the surface, impact point and velocity.
fn fly(o: &Options, ss: &[Surface], p0: V3, v0: V3, diameter: f64) -> Option<(usize, V3, V3)> {
    if o.flight == Flight::Straight {
        return hit(ss, p0, add(p0, scale(unit(v0), 50.0))).map(|(k, q)| (k, q, unit(v0)));
    }
    // Quadratic drag on a sphere: a = −k |v| v, k = (3 ρ_air C_d) / (4 ρ_blood d).
    let k = 3.0 * 1.2 * 0.47 / (4.0 * 1060.0 * diameter);
    let (mut p, mut v) = (p0, v0);
    let dt = 1e-4;
    for _ in 0..200_000 {
        let s = dot(v, v).sqrt();
        let a = add([0.0, 0.0, -9.81], scale(v, -k * s));
        let v2 = add(v, scale(a, dt));
        let p2 = add(p, scale(add(v, v2), dt / 2.0));
        if let Some((kk, q)) = hit(ss, p, p2) {
            return Some((kk, q, unit(v2)));
        }
        (p, v) = (p2, v2);
    }
    None
}

pub fn truth(o: &Options) -> Truth {
    let mut rng = Rng::new(o.seed);
    let ss = surfaces(o.room);
    let mut stains = vec![];
    let mut discarded = 0;
    for i in 0..o.droplets {
        let dir = rng.direction();
        let d = rng.range(o.diameter[0], o.diameter[1]);
        let speed = rng.range(o.speed[0], o.speed[1]);
        let Some((k, centre, vel)) = fly(o, &ss, o.origin, scale(dir, speed), d) else {
            discarded += 1;
            continue;
        };
        let s = &ss[k];
        let sin_a = -dot(vel, s.n);
        let impact_deg = sin_a.clamp(-1.0, 1.0).asin().to_degrees();
        let width = o.spread * d;
        if width < o.min_width
            || impact_deg < o.angle_range[0]
            || impact_deg > o.angle_range[1]
            // Stains near the surface's edges (corners, the floor line) could run onto the
            // next surface; walls end at 2.5 m (the room has no ceiling here).
            || (0..2).any(|c| s.n[c] == 0.0 && (centre[c] < 0.05 || centre[c] > o.room[c] - 0.05))
            || s.n[2] == 0.0 && !(0.05..2.5).contains(&centre[2])
        {
            discarded += 1;
            continue;
        }
        let length = width / sin_a;
        let travel = unit(add(vel, scale(s.n, sin_a)));
        let across = cross(s.n, travel);
        // Measurement: each edge off by σ_edge; axes edge to edge (√2 σ).
        let edge = |axis: f64| o.edge_fixed + o.edge_rel * axis;
        let mw = width + rng.normal() * edge(width) * 2f64.sqrt();
        let ml = (length + rng.normal() * edge(length) * 2f64.sqrt()).max(mw);
        let sigma_theta = (edge(length) * 2f64.sqrt()).atan2((length - width).max(1e-9) / 2.0);
        let th = rng.normal() * sigma_theta.min(std::f64::consts::FRAC_PI_4);
        let axis = unit(add(scale(travel, th.cos()), scale(across, th.sin())));
        // The photo: the stain's surface frame, image x along the surface's first axis.
        let (pu, pv) = if s.name == "floor" {
            ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0])
        } else {
            (unit(cross([0.0, 0.0, 1.0], s.n)), [0.0, 0.0, 1.0])
        };
        let ppm = o.px_per_mm * 1000.0;
        let half = (length * 0.5 + 0.02) * ppm;
        let size = (2.0 * half).ceil() as u32;
        let to_px = |w: V3| {
            let r = sub(w, centre);
            [half + dot(r, pu) * ppm, half - dot(r, pv) * ppm]
        };
        let fiducials = [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0)]
            .map(|(a, b)| {
                let off = (half / ppm) * 0.8;
                let w = add(centre, add(scale(pu, a * off), scale(pv, b * off)));
                Fiducial {
                    world: w,
                    px: to_px(w),
                }
            })
            .to_vec();
        stains.push(Stain {
            surface: s.name.into(),
            centre,
            normal: s.n,
            velocity: vel,
            impact_deg,
            travel,
            width,
            length,
            measured_width: mw,
            measured_length: ml,
            measured_axis: axis,
            measured_impact_deg: (mw / ml).clamp(0.0, 1.0).asin().to_degrees(),
            upward: vel[2] > 0.0,
            photo: Photo {
                file: format!("stain-{i:03}.png"),
                size_px: [size, size],
                scale: ppm,
                u: pu,
                v: pv,
                fiducials,
            },
        });
    }
    Truth {
        options: o.clone(),
        stains,
        discarded,
    }
}

/// A stain photo: the true ellipse (dark red on a light surface) with a short tail in the
/// direction of travel, and a black cross at each fiducial. Grey, 8-bit RGB, row-major.
pub fn photo(o: &Options, s: &Stain) -> Vec<u8> {
    let [w, h] = s.photo.size_px;
    let ppm = s.photo.scale;
    let (pu, pv) = (s.photo.u, s.photo.v);
    let half = w as f64 / 2.0;
    // Image-space ellipse frame.
    let t = [dot(s.travel, pu), -dot(s.travel, pv)];
    let (a, b) = (s.length / 2.0 * ppm, s.width / 2.0 * ppm);
    let mut img = vec![0u8; (w * h * 3) as usize];
    let ss = 4; // supersampling per axis
    for y in 0..h {
        for x in 0..w {
            let mut inside = 0;
            for sy in 0..ss {
                for sx in 0..ss {
                    let px = x as f64 + (sx as f64 + 0.5) / ss as f64 - half;
                    let py = y as f64 + (sy as f64 + 0.5) / ss as f64 - half;
                    let (l, c) = (px * t[0] + py * t[1], -px * t[1] + py * t[0]);
                    let ellipse = (l / a).powi(2) + (c / b).powi(2) <= 1.0;
                    // Tail: a narrowing spine beyond the leading end.
                    let tail = l > a && l < a * 1.35 && c.abs() < b * 0.25 * (1.35 - l / a) / 0.35;
                    if ellipse || tail {
                        inside += 1;
                    }
                }
            }
            let f = inside as f64 / (ss * ss) as f64;
            let bg = [214.0, 208.0, 196.0];
            let ink = [110.0, 14.0, 18.0];
            let i = ((y * w + x) * 3) as usize;
            for c in 0..3 {
                img[i + c] = (bg[c] * (1.0 - f) + ink[c] * f).round() as u8;
            }
        }
    }
    for f in &s.photo.fiducials {
        let (cx, cy) = (f.px[0].round() as i64, f.px[1].round() as i64);
        for d in -8..=8i64 {
            for (x, y) in [(cx + d, cy), (cx, cy + d)] {
                if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h {
                    let i = ((y as u32 * w + x as u32) * 3) as usize;
                    img[i..i + 3].copy_from_slice(&[0, 0, 0]);
                }
            }
        }
    }
    let _ = o;
    img
}

/// The room's surfaces around the stains as points (5 mm grid, 0.5 m around each stain),
/// stains coloured where they are.
pub fn points(t: &Truth) -> Vec<Point> {
    let spacing = 0.005;
    let mut out = vec![];
    let mut seen = std::collections::HashSet::new();
    for s in &t.stains {
        let (pu, pv) = (s.photo.u, s.photo.v);
        let n = (0.5 / spacing) as i64;
        for i in -n..=n {
            for j in -n..=n {
                let p = add(
                    s.centre,
                    add(scale(pu, i as f64 * spacing), scale(pv, j as f64 * spacing)),
                );
                let key = p.map(|v| (v / spacing).round() as i64);
                if !seen.insert(key)
                    || p.iter()
                        .take(2)
                        .enumerate()
                        .any(|(c, v)| *v < 0.0 || *v > t.options.room[c])
                    || p[2] < 0.0
                {
                    continue;
                }
                let stained = t.stains.iter().any(|q| {
                    let r = sub(p, q.centre);
                    dot(r, q.normal).abs() < 1e-6
                        && (dot(r, q.travel) / (q.length / 2.0)).powi(2)
                            + (dot(r, cross(q.normal, q.travel)) / (q.width / 2.0)).powi(2)
                            <= 1.0
                });
                out.push(Point {
                    xyz: p,
                    intensity: if stained { 300 } else { 1500 },
                    rgb: if stained {
                        [110, 14, 18]
                    } else {
                        [214, 208, 196]
                    },
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_stains_point_back_at_the_origin() {
        let t = truth(&Options::default());
        assert!(t.stains.len() > 40, "{}", t.stains.len());
        for s in &t.stains {
            // The ray back from the stain along its true path passes through the origin.
            let r = sub(t.options.origin, s.centre);
            let off = sub(r, scale(s.velocity, dot(r, s.velocity)));
            assert!(dot(off, off).sqrt() < 1e-9);
            // Width / length gives the impact angle exactly for the true ellipse.
            assert!(((s.width / s.length).asin().to_degrees() - s.impact_deg).abs() < 1e-9);
            // The travel direction lies in the surface.
            assert!(dot(s.travel, s.normal).abs() < 1e-12);
        }
    }

    #[test]
    fn measured_angles_are_off_by_a_few_degrees_and_worse_near_ninety() {
        let t = truth(&Options {
            droplets: 4000,
            ..Options::default()
        });
        let rms = |lo: f64, hi: f64| {
            let e: Vec<f64> = t
                .stains
                .iter()
                .filter(|s| (lo..hi).contains(&s.impact_deg))
                .map(|s| s.measured_impact_deg - s.impact_deg)
                .collect();
            (e.iter().map(|x| x * x).sum::<f64>() / e.len() as f64).sqrt()
        };
        let (mid, steep) = (rms(20.0, 60.0), rms(70.0, 80.0));
        // Small stains (2–5 mm wide): a tenth of a millimetre on an edge is a few percent.
        assert!(mid > 1.0 && mid < 8.0, "20–60°: rms {mid}°");
        // asin(w/l) is ill-conditioned as w/l → 1: the same edge error costs more.
        assert!(steep > 1.5 * mid, "70–80°: rms {steep}° vs {mid}°");
    }

    #[test]
    fn gravity_bends_the_paths_down() {
        let o = Options {
            flight: Flight::Ballistic,
            ..Options::default()
        };
        let t = truth(&o);
        assert!(t.stains.len() > 30);
        // Straight rays back from the stains pass on average above the true origin.
        let mean_dz = t
            .stains
            .iter()
            .map(|s| {
                let r = sub(o.origin, s.centre);
                let along = dot(r, s.velocity);
                add(s.centre, scale(s.velocity, along))[2] - o.origin[2]
            })
            .sum::<f64>()
            / t.stains.len() as f64;
        assert!(mean_dz > 0.0, "{mean_dz}");
    }

    #[test]
    fn photos_put_the_fiducials_where_they_say() {
        let o = Options::default();
        let t = truth(&o);
        let s = &t.stains[0];
        let img = photo(&o, s);
        let [w, _] = s.photo.size_px;
        for f in &s.photo.fiducials {
            let i = ((f.px[1].round() as u32 * w + f.px[0].round() as u32) * 3) as usize;
            assert_eq!(&img[i..i + 3], &[0, 0, 0]);
        }
        // The stain centre is ink.
        let c = ((s.photo.size_px[1] / 2 * w + w / 2) * 3) as usize;
        assert!(img[c] < 150 && img[c + 1] < 60);
    }
}
