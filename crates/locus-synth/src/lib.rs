//! Synthetic multi-scan scenes with known ground truth.
//!
//! A warehouse-sized room (floor, ceiling, walls, pillars, crates) with sphere and
//! checkerboard targets, scanned from several stations the way a terrestrial laser scanner
//! would: each station casts rays in random directions, keeps the first hit, adds range
//! noise, and stores points in its own frame. Every station has a full 6-DOF pose (any
//! heading plus a small levelling error), and the true poses, target geometry and any
//! injected faults are returned as a [`Truth`], so registration can be scored against them.
//!
//! Scans are generated independently from `(seed, scan)`, so one scan can be regenerated
//! without the others. Everything is deterministic for a given [`Options`].

use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;
use std::path::Path;

mod e57_out;
pub use e57_out::write_e57;

/// Sphere target radius (a 145 mm reference sphere).
pub const SPHERE_RADIUS: f64 = 0.0725;
/// Checkerboard target edge; 2 × 2 squares, so each square is half this.
pub const BOARD_SIZE: f64 = 0.3;

struct Rng(u64);

impl Rng {
    /// Seeded from any u64, mixed (splitmix64) so nearby seeds give unrelated streams.
    fn new(seed: u64) -> Self {
        let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        Rng((z ^ (z >> 31)) | 1)
    }

    fn unit(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + self.unit() * (hi - lo)
    }

    /// Standard normal (Box–Muller).
    fn normal(&mut self) -> f64 {
        let (u, v) = (self.unit().max(1e-300), self.unit());
        (-2.0 * u.ln()).sqrt() * (TAU * v).cos()
    }

    fn direction(&mut self) -> [f64; 3] {
        let z = self.unit() * 2.0 - 1.0;
        let a = self.unit() * TAU;
        let r = (1.0 - z * z).sqrt();
        [r * a.cos(), r * a.sin(), z]
    }
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Rigid scan-to-world transform: `world = R · local + translation`, with R the unit
/// quaternion `rotation` = [w, x, y, z].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    pub rotation: [f64; 4],
    pub translation: [f64; 3],
}

impl Pose {
    pub const IDENTITY: Pose = Pose {
        rotation: [1.0, 0.0, 0.0, 0.0],
        translation: [0.0; 3],
    };

    /// Heading about z, then pitch about y and roll about x (R = Rz · Ry · Rx), in radians.
    pub fn from_yaw_pitch_roll(yaw: f64, pitch: f64, roll: f64, translation: [f64; 3]) -> Pose {
        let q = |a: f64, axis: usize| {
            let mut q = [(a / 2.0).cos(), 0.0, 0.0, 0.0];
            q[axis + 1] = (a / 2.0).sin();
            q
        };
        Pose {
            rotation: qmul(qmul(q(yaw, 2), q(pitch, 1)), q(roll, 0)),
            translation,
        }
    }

    pub fn rotate(&self, v: [f64; 3]) -> [f64; 3] {
        let w = self.rotation[0];
        let u = [self.rotation[1], self.rotation[2], self.rotation[3]];
        let t = scale(cross(u, v), 2.0);
        add(add(v, scale(t, w)), cross(u, t))
    }

    pub fn apply(&self, local: [f64; 3]) -> [f64; 3] {
        add(self.rotate(local), self.translation)
    }

    pub fn inverse(&self) -> Pose {
        let [w, x, y, z] = self.rotation;
        let inv = Pose {
            rotation: [w, -x, -y, -z],
            translation: [0.0; 3],
        };
        Pose {
            translation: scale(inv.rotate(self.translation), -1.0),
            ..inv
        }
    }

    /// `self ∘ other`: apply `other`, then `self`.
    pub fn compose(&self, other: &Pose) -> Pose {
        Pose {
            rotation: qmul(self.rotation, other.rotation),
            translation: self.apply(other.translation),
        }
    }

    /// Row-major 4 × 4 matrix, the layout `locus_core::ScanInfo::pose` uses.
    pub fn matrix(&self) -> [f64; 16] {
        let c = |v| self.rotate(v);
        let (x, y, z) = (c([1.0, 0.0, 0.0]), c([0.0, 1.0, 0.0]), c([0.0, 0.0, 1.0]));
        let t = self.translation;
        [
            x[0], y[0], z[0], t[0], x[1], y[1], z[1], t[1], x[2], y[2], z[2], t[2], 0.0, 0.0, 0.0,
            1.0,
        ]
    }

    /// Rotation angle (radians) and translation length (m) between two poses.
    pub fn difference(&self, other: &Pose) -> (f64, f64) {
        let d = self.inverse().compose(other);
        let w = d.rotation[0].abs().min(1.0);
        (2.0 * w.acos(), dot(d.translation, d.translation).sqrt())
    }
}

fn qmul(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[0] * b[0] - a[1] * b[1] - a[2] * b[2] - a[3] * b[3],
        a[0] * b[1] + a[1] * b[0] + a[2] * b[3] - a[3] * b[2],
        a[0] * b[2] - a[1] * b[3] + a[2] * b[0] + a[3] * b[1],
        a[0] * b[3] + a[1] * b[2] - a[2] * b[1] + a[3] * b[0],
    ]
}

/// What the E57 records as each scan's pose.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum StoredPose {
    /// The true pose (a registered export).
    True,
    /// No pose (an unregistered export: every scan in its own frame).
    None,
    /// The true pose disturbed by a rotation of `degrees` about a random axis and a shift of
    /// `metres` in a random direction (a rough field registration).
    Perturbed { metres: f64, degrees: f64 },
}

/// Fault injection: sphere `sphere` is displaced by `offset` (m) for scans `from_scan` onward,
/// as if bumped between set-ups. Links that rely on it become inconsistent.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MovedSphere {
    pub sphere: usize,
    pub from_scan: usize,
    pub offset: [f64; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub scans: usize,
    pub points_per_scan: u64,
    pub seed: u64,
    /// Sphere and checkerboard targets in the room.
    pub targets: bool,
    /// Largest levelling error, roll and pitch each drawn uniformly within ± this (degrees).
    pub tilt_deg: f64,
    /// Range noise, 1σ (m).
    pub range_noise_m: f64,
    /// Fraction of returns that are outliers: a random range short of the true surface, as
    /// mixed pixels and airborne dust give.
    pub outliers: f64,
    pub stored_pose: StoredPose,
    pub moved_sphere: Option<MovedSphere>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            scans: 4,
            points_per_scan: 1_000_000,
            seed: 1,
            targets: true,
            tilt_deg: 0.5,
            range_noise_m: 0.001,
            outliers: 0.0,
            stored_pose: StoredPose::True,
            moved_sphere: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScanTruth {
    pub name: String,
    /// The true scan-to-world pose.
    pub pose: Pose,
    /// The pose written to the file, if any.
    pub stored_pose: Option<Pose>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SphereTruth {
    /// Centre in world coordinates (before any [`MovedSphere`] offset).
    pub centre: [f64; 3],
    pub radius: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardTruth {
    /// The point where the four squares meet, on the board's front face, in world coordinates.
    pub centre: [f64; 3],
    /// Unit normal pointing into the room.
    pub normal: [f64; 3],
    pub size: f64,
}

/// Everything registration should recover, plus how the scene was made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Truth {
    pub generator: String,
    pub seed: u64,
    pub points_per_scan: u64,
    pub range_noise_m: f64,
    pub outliers: f64,
    pub scans: Vec<ScanTruth>,
    pub spheres: Vec<SphereTruth>,
    pub boards: Vec<BoardTruth>,
    pub moved_sphere: Option<MovedSphere>,
}

impl Truth {
    /// Where sphere `k` is while scan `scan` is taken.
    pub fn sphere_centre(&self, k: usize, scan: usize) -> [f64; 3] {
        let c = self.spheres[k].centre;
        match self.moved_sphere {
            Some(m) if m.sphere == k && scan >= m.from_scan => add(c, m.offset),
            _ => c,
        }
    }

    /// The true pose of `scan` relative to scan 0, the frame registration reports in.
    pub fn relative_pose(&self, scan: usize) -> Pose {
        self.scans[0].pose.inverse().compose(&self.scans[scan].pose)
    }
}

/// One return, in the scan's own frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub xyz: [f64; 3],
    /// 0–2047, as written to the E57.
    pub intensity: u16,
    pub rgb: [u8; 3],
}

/// Axis-aligned box. A box with a `checker` is a checkerboard target.
struct Solid {
    lo: [f64; 3],
    hi: [f64; 3],
    rgb: [u8; 3],
    checker: Option<[f64; 3]>,
}

struct Sphere {
    centre: [f64; 3],
    radius: f64,
}

/// What a scan sees: the room, with targets where they were for that scan.
struct World {
    solids: Vec<Solid>,
    spheres: Vec<Sphere>,
}

struct Hit {
    t: f64,
    rgb: [u8; 3],
    reflectance: f64,
    /// |cos| of the angle between the beam and the surface normal.
    incidence: f64,
}

const ROOM: [f64; 3] = [40.0, 30.0, 5.0];

fn solid(lo: [f64; 3], hi: [f64; 3], rgb: [u8; 3]) -> Solid {
    Solid {
        lo,
        hi,
        rgb,
        checker: None,
    }
}

/// Sphere centres, on stands spread through the room at different heights.
fn sphere_layout() -> Vec<SphereTruth> {
    [
        [5.0, 5.0, 1.2],
        [20.0, 4.0, 1.6],
        [35.0, 6.0, 1.0],
        [6.0, 24.5, 1.5],
        [21.0, 26.0, 1.8],
        [34.0, 23.0, 1.3],
    ]
    .into_iter()
    .map(|centre| SphereTruth {
        centre,
        radius: SPHERE_RADIUS,
    })
    .collect()
}

/// Checkerboards on the four walls, facing into the room.
fn board_layout() -> Vec<BoardTruth> {
    let (x, y) = (ROOM[0], ROOM[1]);
    let b = |centre, normal| BoardTruth {
        centre,
        normal,
        size: BOARD_SIZE,
    };
    // Boards are 3 mm thick, so the front face stands 3 mm off the wall.
    let t = 0.003;
    vec![
        b([t, 8.0, 1.8], [1.0, 0.0, 0.0]),
        b([t, 22.0, 2.2], [1.0, 0.0, 0.0]),
        b([x - t, 10.0, 2.0], [-1.0, 0.0, 0.0]),
        b([x - t, 20.0, 1.6], [-1.0, 0.0, 0.0]),
        b([12.0, t, 1.7], [0.0, 1.0, 0.0]),
        b([30.0, t, 2.3], [0.0, 1.0, 0.0]),
        b([10.0, y - t, 2.1], [0.0, -1.0, 0.0]),
        b([28.0, y - t, 1.9], [0.0, -1.0, 0.0]),
    ]
}

/// Footprints (x, y, half-width) kept clear of crates so targets stay visible.
fn clearings(truth: &Truth) -> Vec<[f64; 3]> {
    truth
        .spheres
        .iter()
        .map(|s| [s.centre[0], s.centre[1], 0.6])
        .collect()
}

fn world(truth: &Truth, scan: usize, targets: bool) -> World {
    let (x, y, z) = (ROOM[0], ROOM[1], ROOM[2]);
    let mut solids = vec![
        solid([-0.3, -0.3, -0.3], [x + 0.3, y + 0.3, 0.0], [120, 115, 105]), // floor
        solid(
            [-0.3, -0.3, z],
            [x + 0.3, y + 0.3, z + 0.3],
            [200, 200, 205],
        ), // ceiling
        solid([-0.3, -0.3, 0.0], [0.0, y + 0.3, z], [180, 170, 150]),        // walls
        solid([x, -0.3, 0.0], [x + 0.3, y + 0.3, z], [180, 170, 150]),
        solid([0.0, -0.3, 0.0], [x, 0.0, z], [170, 175, 160]),
        solid([0.0, y, 0.0], [x, y + 0.3, z], [170, 175, 160]),
    ];
    for i in 0..4 {
        for j in 0..3 {
            let (px, py) = (8.0 + i as f64 * 8.0, 7.5 + j as f64 * 7.5);
            solids.push(solid(
                [px - 0.25, py - 0.25, 0.0],
                [px + 0.25, py + 0.25, z],
                [140, 140, 150],
            ));
        }
    }
    let clear = if targets { clearings(truth) } else { vec![] };
    let mut r = Rng(0x51_7cc1_b727_220a);
    for _ in 0..30 {
        let (cx, cy) = (2.0 + r.unit() * 35.0, 2.0 + r.unit() * 25.0);
        let (w, d, h) = (
            0.4 + r.unit() * 1.5,
            0.4 + r.unit() * 1.5,
            0.3 + r.unit() * 1.8,
        );
        let c = [
            90 + (r.unit() * 120.0) as u8,
            60 + (r.unit() * 100.0) as u8,
            40 + (r.unit() * 80.0) as u8,
        ];
        let overlaps = clear
            .iter()
            .any(|&[fx, fy, h]| cx < fx + h && cx + w > fx - h && cy < fy + h && cy + d > fy - h);
        if !overlaps {
            solids.push(solid([cx, cy, 0.0], [cx + w, cy + d, h], c));
        }
    }
    let mut spheres = vec![];
    if targets {
        for k in 0..truth.spheres.len() {
            let c = truth.sphere_centre(k, scan);
            let r = truth.spheres[k].radius;
            // Stand: a 3 cm pole from the floor to the bottom of the sphere.
            solids.push(solid(
                [c[0] - 0.015, c[1] - 0.015, 0.0],
                [c[0] + 0.015, c[1] + 0.015, c[2] - r],
                [60, 60, 60],
            ));
            spheres.push(Sphere {
                centre: c,
                radius: r,
            });
        }
        for b in &truth.boards {
            let axis = (0..3).find(|&a| b.normal[a] != 0.0).expect("axis-aligned");
            let (mut lo, mut hi) = (
                sub(b.centre, [b.size / 2.0; 3]),
                add(b.centre, [b.size / 2.0; 3]),
            );
            // Front face at the centre, 3 mm of board behind it.
            lo[axis] = b.centre[axis].min(b.centre[axis] - b.normal[axis] * 0.003);
            hi[axis] = b.centre[axis].max(b.centre[axis] - b.normal[axis] * 0.003);
            solids.push(Solid {
                lo,
                hi,
                rgb: [0, 0, 0],
                checker: Some(b.centre),
            });
        }
    }
    World { solids, spheres }
}

/// Nearest hit of a ray from `o` along unit `d`.
fn cast(o: [f64; 3], d: [f64; 3], w: &World) -> Option<Hit> {
    let mut best: Option<Hit> = None;
    for s in &w.solids {
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
        if miss || t0 <= 1e-6 || best.as_ref().is_some_and(|b| t0 >= b.t) {
            continue;
        }
        let (rgb, reflectance) = match s.checker {
            // 2 × 2 squares: white where the two in-plane offsets have the same sign.
            Some(c) => {
                let p = add(o, scale(d, t0));
                let (u, v) = match axis {
                    0 => (p[1] - c[1], p[2] - c[2]),
                    1 => (p[0] - c[0], p[2] - c[2]),
                    _ => (p[0] - c[0], p[1] - c[1]),
                };
                if (u >= 0.0) == (v >= 0.0) {
                    ([235, 235, 235], 0.9)
                } else {
                    ([20, 20, 20], 0.08)
                }
            }
            None => (s.rgb, 1.0),
        };
        best = Some(Hit {
            t: t0,
            rgb,
            reflectance,
            incidence: d[axis].abs(),
        });
    }
    for s in &w.spheres {
        let oc = sub(o, s.centre);
        let b = dot(d, oc);
        let disc = b * b - (dot(oc, oc) - s.radius * s.radius);
        if disc < 0.0 {
            continue;
        }
        let t = -b - disc.sqrt();
        if t <= 1e-6 || best.as_ref().is_some_and(|h| t >= h.t) {
            continue;
        }
        let n = scale(add(oc, scale(d, t)), 1.0 / s.radius);
        best = Some(Hit {
            t,
            rgb: [230, 230, 230],
            reflectance: 0.95,
            incidence: dot(d, n).abs(),
        });
    }
    best
}

/// True poses, stored poses and targets. Cheap: no points are generated.
pub fn truth(opts: &Options) -> Truth {
    let mut t = Truth {
        generator: format!("locus-synth {}", env!("CARGO_PKG_VERSION")),
        seed: opts.seed,
        points_per_scan: opts.points_per_scan,
        range_noise_m: opts.range_noise_m,
        outliers: opts.outliers,
        scans: vec![],
        spheres: if opts.targets {
            sphere_layout()
        } else {
            vec![]
        },
        boards: if opts.targets { board_layout() } else { vec![] },
        moved_sphere: opts.moved_sphere,
    };
    let room = world(&t, 0, opts.targets);
    let mut rng = Rng::new(opts.seed);
    let cols = (opts.scans as f64).sqrt().ceil() as usize;
    let rows = opts.scans.div_ceil(cols.max(1));
    for s in 0..opts.scans {
        // Stations on a jittered grid across the room, kept 0.5 m clear of every object.
        let (cw, ch) = (36.0 / cols as f64, 26.0 / rows as f64);
        let base = [2.0 + (s % cols) as f64 * cw, 2.0 + (s / cols) as f64 * ch];
        let mut station = [0.0; 3];
        for _ in 0..1000 {
            station = [
                base[0] + rng.unit() * cw,
                base[1] + rng.unit() * ch,
                rng.range(1.2, 1.8),
            ];
            let clear = room.solids[6..].iter().all(|b| {
                station[0] < b.lo[0] - 0.5
                    || station[0] > b.hi[0] + 0.5
                    || station[1] < b.lo[1] - 0.5
                    || station[1] > b.hi[1] + 0.5
            }) && room
                .spheres
                .iter()
                .all(|sp| (station[0] - sp.centre[0]).hypot(station[1] - sp.centre[1]) > 0.6);
            if clear {
                break;
            }
        }
        let tilt = opts.tilt_deg.to_radians();
        let pose = Pose::from_yaw_pitch_roll(
            rng.unit() * TAU,
            rng.range(-tilt, tilt),
            rng.range(-tilt, tilt),
            station,
        );
        let stored_pose = match opts.stored_pose {
            StoredPose::True => Some(pose),
            StoredPose::None => None,
            StoredPose::Perturbed { metres, degrees } => {
                let axis = rng.direction();
                let half = degrees.to_radians() / 2.0;
                let dq = Pose {
                    rotation: [
                        half.cos(),
                        axis[0] * half.sin(),
                        axis[1] * half.sin(),
                        axis[2] * half.sin(),
                    ],
                    translation: [0.0; 3],
                };
                Some(Pose {
                    rotation: qmul(dq.rotation, pose.rotation),
                    translation: add(pose.translation, scale(rng.direction(), metres)),
                })
            }
        };
        t.scans.push(ScanTruth {
            name: format!("Station {}", s + 1),
            pose,
            stored_pose,
        });
    }
    t
}

/// Stream scan `scan`'s returns in order; `None` is a beam with no return (recorded as an
/// invalid point, as scanners do).
pub fn scan_points(opts: &Options, truth: &Truth, scan: usize, f: &mut dyn FnMut(Option<Point>)) {
    let w = world(truth, scan, opts.targets);
    let pose = truth.scans[scan].pose;
    let inv = pose.inverse();
    let station = pose.translation;
    let mut rng = Rng::new(opts.seed ^ (scan as u64 + 1).wrapping_mul(0xd1b5_4a32_d192_ed03));
    for _ in 0..opts.points_per_scan {
        // Uniform direction in the scanner frame, skipping its own footprint below it.
        let local_dir = loop {
            let d = rng.direction();
            if d[2] > -0.95 {
                break d;
            }
        };
        let d = pose.rotate(local_dir);
        // About 0.3% of beams get no return (glass, dark surfaces, out of range).
        let no_return = rng.unit() < 0.003;
        let outlier = rng.unit() < opts.outliers;
        let noise = rng.normal();
        let short = rng.range(0.2, 0.95);
        let point = match cast(station, d, &w) {
            Some(h) if !no_return => {
                let range = if outlier {
                    h.t * short
                } else {
                    h.t + noise * opts.range_noise_m
                };
                let intensity = (h.reflectance * (h.incidence * 0.8 + 0.2) / (1.0 + h.t * 0.03)
                    * 2047.0) as u16;
                let shade = |v: u8| (v as f64 * (0.6 + 0.4 * h.incidence)) as u8;
                Some(Point {
                    xyz: inv.rotate(scale(d, range)),
                    intensity: intensity.min(2047),
                    rgb: h.rgb.map(shade),
                })
            }
            _ => None,
        };
        f(point);
    }
}

/// The ground-truth sidecar path for a scene file: `<file>.truth.json`.
pub fn truth_path(scene: &Path) -> std::path::PathBuf {
    let mut s = scene.as_os_str().to_owned();
    s.push(".truth.json");
    s.into()
}

pub fn write_truth(truth: &Truth, path: &Path) -> std::io::Result<()> {
    std::fs::write(path, serde_json::to_vec_pretty(truth)?)
}

pub fn read_truth(path: &Path) -> std::io::Result<Truth> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(points: u64) -> Options {
        Options {
            scans: 3,
            points_per_scan: points,
            seed: 7,
            range_noise_m: 0.0,
            tilt_deg: 2.0,
            ..Options::default()
        }
    }

    #[test]
    fn rays_hit_the_nearest_face() {
        let t = truth(&Options {
            targets: false,
            ..opts(0)
        });
        let w = world(&t, 0, false);
        // Straight down from 1.5 m: the floor at z = 0.
        let h = cast([20.0, 3.0, 1.5], [0.0, 0.0, -1.0], &w).unwrap();
        assert!((h.t - 1.5).abs() < 1e-12);
        assert_eq!(h.incidence, 1.0);
        // Along +x from x = 1 near the south wall: the east wall at x = 40.
        let h = cast([1.0, 0.5, 4.9], [1.0, 0.0, 0.0], &w).unwrap();
        assert!((h.t - 39.0).abs() < 1e-12);
    }

    #[test]
    fn rays_hit_targets_where_the_truth_says() {
        let t = truth(&opts(0));
        let w = world(&t, 0, true);
        // Toward a sphere centre from 3 m away: the surface at 3 m − r.
        let c = t.spheres[1].centre;
        let o = add(c, [0.0, 3.0, 0.0]);
        let h = cast(o, [0.0, -1.0, 0.0], &w).unwrap();
        assert!((h.t - (3.0 - SPHERE_RADIUS)).abs() < 1e-12);
        // Just off each board centre: white in the (+, +) square, black in (+, −).
        for b in &t.boards {
            let axis = (0..3).find(|&a| b.normal[a] != 0.0).unwrap();
            let (u, v) = match axis {
                0 => (1, 2),
                1 => (0, 2),
                _ => unreachable!(),
            };
            let aim = |du: f64, dv: f64| {
                let mut p = b.centre;
                p[u] += du;
                p[v] += dv;
                let o = add(p, scale(b.normal, 2.0));
                cast(o, scale(b.normal, -1.0), &w).unwrap()
            };
            let white = aim(0.05, 0.05);
            assert!(
                (white.t - 2.0).abs() < 1e-12,
                "board front face at its centre"
            );
            assert_eq!(white.rgb, [235, 235, 235]);
            assert_eq!(aim(0.05, -0.05).rgb, [20, 20, 20]);
        }
    }

    #[test]
    fn every_point_lies_on_the_scene_through_its_true_pose() {
        let o = opts(20_000);
        let t = truth(&o);
        for s in 0..o.scans {
            let w = world(&t, s, true);
            let pose = t.scans[s].pose;
            let mut n = 0;
            scan_points(&o, &t, s, &mut |p| {
                let Some(p) = p else { return };
                let world = pose.apply(p.xyz);
                let ray = sub(world, pose.translation);
                let range = dot(ray, ray).sqrt();
                let hit = cast(pose.translation, scale(ray, 1.0 / range), &w).unwrap();
                assert!(
                    (hit.t - range).abs() < 1e-9,
                    "scan {s}: {} vs {range}",
                    hit.t
                );
                n += 1;
            });
            assert!(n > 19_000);
        }
    }

    #[test]
    fn poses_are_levelled_within_the_tilt_and_stations_are_clear() {
        let o = Options {
            scans: 16,
            ..opts(0)
        };
        let t = truth(&o);
        for s in &t.scans {
            let up = s.pose.rotate([0.0, 0.0, 1.0]);
            // Roll and pitch each ≤ 2°, so the combined tilt is ≤ 2√2°.
            assert!(up[2].acos().to_degrees() <= 2.0 * 2f64.sqrt() + 1e-9);
            assert!((1.2..=1.8).contains(&s.pose.translation[2]));
            let p = s.pose.translation;
            assert!(p[0] > 0.5 && p[0] < 39.5 && p[1] > 0.5 && p[1] < 29.5);
        }
    }

    #[test]
    fn pose_algebra() {
        let a = Pose::from_yaw_pitch_roll(1.0, 0.1, -0.2, [3.0, -4.0, 1.5]);
        let b = Pose::from_yaw_pitch_roll(-2.5, -0.05, 0.3, [10.0, 2.0, 1.2]);
        let p = [0.3, -1.7, 2.2];
        let back = a.inverse().apply(a.apply(p));
        assert!(sub(back, p).iter().all(|v| v.abs() < 1e-12));
        let ab = a.compose(&b).apply(p);
        let expect = a.apply(b.apply(p));
        assert!(sub(ab, expect).iter().all(|v| v.abs() < 1e-12));
        let m = a.matrix();
        let mp = [
            m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[3],
            m[4] * p[0] + m[5] * p[1] + m[6] * p[2] + m[7],
            m[8] * p[0] + m[9] * p[1] + m[10] * p[2] + m[11],
        ];
        assert!(sub(mp, a.apply(p)).iter().all(|v| v.abs() < 1e-12));
        let (angle, dist) = a.difference(&a);
        assert!(angle.abs() < 1e-7 && dist < 1e-12);
    }

    #[test]
    fn perturbed_stored_pose_is_off_by_exactly_the_requested_amount() {
        let o = Options {
            stored_pose: StoredPose::Perturbed {
                metres: 0.25,
                degrees: 3.0,
            },
            ..opts(0)
        };
        for s in truth(&o).scans {
            // Rotation about the world origin, so compare rotation and translation separately.
            let stored = s.stored_pose.unwrap();
            let (angle, _) = s.pose.difference(&stored);
            let shift = sub(stored.translation, s.pose.translation);
            assert!((angle.to_degrees() - 3.0).abs() < 1e-9);
            assert!((dot(shift, shift).sqrt() - 0.25).abs() < 1e-12);
        }
        assert!(truth(&Options {
            stored_pose: StoredPose::None,
            ..opts(0)
        })
        .scans
        .iter()
        .all(|s| s.stored_pose.is_none()));
    }

    #[test]
    fn a_moved_sphere_is_where_the_truth_says_for_each_scan() {
        let offset = [0.0, 0.04, 0.0];
        let o = Options {
            moved_sphere: Some(MovedSphere {
                sphere: 2,
                from_scan: 1,
                offset,
            }),
            ..opts(0)
        };
        let t = truth(&o);
        assert_eq!(t.sphere_centre(2, 0), t.spheres[2].centre);
        assert_eq!(t.sphere_centre(2, 2), add(t.spheres[2].centre, offset));
        assert_eq!(t.sphere_centre(3, 2), t.spheres[3].centre);
        for s in 0..3 {
            let c = t.sphere_centre(2, s);
            let o = add(c, [3.0, 0.0, 0.0]);
            let h = cast(o, [-1.0, 0.0, 0.0], &world(&t, s, true)).unwrap();
            assert!((h.t - (3.0 - SPHERE_RADIUS)).abs() < 1e-12, "scan {s}");
        }
    }

    #[test]
    fn deterministic_per_scan_and_seed() {
        let o = opts(2_000);
        let t = truth(&o);
        let collect = |o: &Options, s| {
            let mut v = vec![];
            scan_points(o, &truth(o), s, &mut |p| v.push(p));
            v
        };
        assert_eq!(collect(&o, 1), collect(&o, 1));
        assert_ne!(collect(&o, 1), collect(&o, 2));
        assert_ne!(
            collect(&o, 1),
            collect(
                &Options {
                    seed: 8,
                    ..o.clone()
                },
                1
            )
        );
        assert_eq!(t, truth(&o));
    }

    #[test]
    fn truth_round_trips_through_json() {
        let t = truth(&Options {
            moved_sphere: Some(MovedSphere {
                sphere: 0,
                from_scan: 2,
                offset: [0.01, 0.0, 0.0],
            }),
            ..opts(0)
        });
        let back: Truth = serde_json::from_slice(&serde_json::to_vec(&t).unwrap()).unwrap();
        assert_eq!(back, t);
    }
}
