//! Camera images with known ground truth: a room with control markers and standing people
//! of known heights, rendered through known cameras (pose, focal length, principal point,
//! radial and tangential lens distortion), plus the room as a point cloud. For camera
//! matching (pose and intrinsics from image-to-scan point pairs), subject height by
//! reverse projection, and witness perspective.
//!
//! Camera model (as OpenCV's): camera axes x right, y down, z forward;
//! `x_c = R (X − C)`; normalised `(a, b) = (x_c / z_c, y_c / z_c)`; with `r² = a² + b²`,
//! `a' = a (1 + k1 r² + k2 r⁴ + k3 r⁶) + 2 p1 a b + p2 (r² + 2a²)` and
//! `b' = b (1 + k1 r² + k2 r⁴ + k3 r⁶) + p1 (r² + 2b²) + 2 p2 a b`;
//! pixel `u = fx a' + cx`, `v = fy b' + cy` (pixel centres at integer + 0.5).
//!
//! People: stature is floor to top of head, standing, without footwear. Body proportions
//! are fractions of stature from Drillis & Contini (1966), as reproduced in Winter,
//! *Biomechanics and Motor Control of Human Movement* (4th ed., 2009), Fig. 4.1.

use crate::{add, cross, dot, scale, sub, Point, Rng};
use serde::{Deserialize, Serialize};

type V3 = [f64; 3];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Camera {
    pub name: String,
    /// Optical centre (m, project frame).
    pub position: V3,
    /// Heading of the optical axis, clockwise from +y (degrees); pitch up from horizontal;
    /// roll about the optical axis (clockwise as seen by the camera).
    pub heading_deg: f64,
    pub pitch_deg: f64,
    pub roll_deg: f64,
    pub size: [u32; 2],
    pub fx: f64,
    pub fy: f64,
    pub cx: f64,
    pub cy: f64,
    /// k1, k2, k3, p1, p2.
    pub distortion: [f64; 5],
}

impl Camera {
    /// Rows: the camera's x (right), y (down), z (forward) axes in the project frame.
    pub fn rotation(&self) -> [V3; 3] {
        let (h, p, r) = (
            self.heading_deg.to_radians(),
            self.pitch_deg.to_radians(),
            self.roll_deg.to_radians(),
        );
        let fwd = [h.sin() * p.cos(), h.cos() * p.cos(), p.sin()];
        let right0 = [h.cos(), -h.sin(), 0.0];
        let down0 = cross(fwd, right0);
        let right = add(scale(right0, r.cos()), scale(down0, r.sin()));
        let down = cross(fwd, right);
        [right, down, fwd]
    }

    /// Pixel of a project-frame point, or None behind the camera.
    pub fn project(&self, x: V3) -> Option<[f64; 2]> {
        let rot = self.rotation();
        let d = sub(x, self.position);
        let c = [dot(rot[0], d), dot(rot[1], d), dot(rot[2], d)];
        if c[2] <= 1e-6 || !self.within_lens((c[0] / c[2]).powi(2) + (c[1] / c[2]).powi(2)) {
            return None;
        }
        let [ad, bd] = self.distort(c[0] / c[2], c[1] / c[2]);
        Some([self.fx * ad + self.cx, self.fy * bd + self.cy])
    }

    /// Is the lens model valid out to radius² `r2`: does the radial distortion keep
    /// increasing with radius all the way there? Its slope, 1 + 3k1 x + 5k2 x² + 7k3 x³ in
    /// x = r², is 1 at the centre; checked exactly at `r2` and at its turning points before it.
    /// Past the first zero the polynomial folds back, and a point outside the field of view
    /// would land in the image.
    pub fn within_lens(&self, r2: f64) -> bool {
        let [k1, k2, k3, ..] = self.distortion;
        let slope = |x: f64| 1.0 + 3.0 * k1 * x + 5.0 * k2 * x * x + 7.0 * k3 * x * x * x;
        if slope(r2) <= 0.0 {
            return false;
        }
        // Turning points: 3k1 + 10k2 x + 21k3 x² = 0.
        let (a, b, c) = (21.0 * k3, 10.0 * k2, 3.0 * k1);
        let roots: Vec<f64> = if a.abs() < 1e-300 {
            if b.abs() < 1e-300 {
                vec![]
            } else {
                vec![-c / b]
            }
        } else {
            let d = b * b - 4.0 * a * c;
            if d < 0.0 {
                vec![]
            } else {
                vec![(-b - d.sqrt()) / (2.0 * a), (-b + d.sqrt()) / (2.0 * a)]
            }
        };
        roots
            .into_iter()
            .filter(|x| *x > 0.0 && *x < r2)
            .all(|x| slope(x) > 0.0)
    }

    /// Distorted normalised coordinates of undistorted ones (the lens model).
    fn distort(&self, a: f64, b: f64) -> [f64; 2] {
        let [k1, k2, k3, p1, p2] = self.distortion;
        let r2 = a * a + b * b;
        let radial = 1.0 + k1 * r2 + k2 * r2 * r2 + k3 * r2 * r2 * r2;
        [
            a * radial + 2.0 * p1 * a * b + p2 * (r2 + 2.0 * a * a),
            b * radial + p1 * (r2 + 2.0 * b * b) + 2.0 * p2 * a * b,
        ]
    }

    /// Undistorted normalised coordinates `(x_c / z_c, y_c / z_c)` of a pixel: the inverse
    /// of the distortion, by Newton's method. NaN where it doesn't converge (outside the
    /// range the lens model is valid for).
    pub fn undistort(&self, px: [f64; 2]) -> [f64; 2] {
        let t = [(px[0] - self.cx) / self.fx, (px[1] - self.cy) / self.fy];
        let (mut a, mut b) = (t[0], t[1]);
        for _ in 0..50 {
            let f = self.distort(a, b);
            let r = [f[0] - t[0], f[1] - t[1]];
            if r[0].abs() + r[1].abs() < 1e-14 {
                return [a, b];
            }
            let h = 1e-7;
            let fa = self.distort(a + h, b);
            let fb = self.distort(a, b + h);
            let j = [
                [(fa[0] - f[0]) / h, (fb[0] - f[0]) / h],
                [(fa[1] - f[1]) / h, (fb[1] - f[1]) / h],
            ];
            let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
            if det.abs() < 1e-12 {
                break;
            }
            a -= (j[1][1] * r[0] - j[0][1] * r[1]) / det;
            b -= (-j[1][0] * r[0] + j[0][0] * r[1]) / det;
        }
        let f = self.distort(a, b);
        if (f[0] - t[0]).abs() + (f[1] - t[1]).abs() < 1e-9 {
            [a, b]
        } else {
            [f64::NAN; 2]
        }
    }
}

/// Stature fractions (Drillis & Contini 1966, via Winter 2009, Fig. 4.1).
pub mod anthropometry {
    /// Heights above the floor as fractions of stature.
    pub const ANKLE: f64 = 0.039;
    pub const KNEE: f64 = 0.285;
    pub const HIP: f64 = 0.530;
    pub const SHOULDER: f64 = 0.818;
    pub const CHIN: f64 = 0.870;
    /// Lengths as fractions of stature.
    pub const UPPER_ARM: f64 = 0.186;
    pub const FOREARM: f64 = 0.146;
    pub const HAND: f64 = 0.108;
    pub const SHOULDER_WIDTH: f64 = 0.259;
    pub const HIP_WIDTH: f64 = 0.191;
    pub const FOOT_LENGTH: f64 = 0.152;
    pub const FOOT_WIDTH: f64 = 0.055;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Person {
    /// Point on the floor midway between the feet.
    pub feet: V3,
    /// Stature: floor to top of head, standing, without footwear (m).
    pub height: f64,
    /// Facing, clockwise from +y (degrees).
    pub heading_deg: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Options {
    pub seed: u64,
    /// Room: floor at z = 0 over [0, room[0]] × [0, room[1]], walls to room[2].
    pub room: V3,
    pub cameras: Vec<Camera>,
    pub people: Vec<Person>,
    /// Control markers (m); each is a 0.1 m square on a surface, its centre the control point.
    pub markers: usize,
    /// 1σ of a picked marker centre in an image (pixels).
    pub pick_sigma_px: f64,
    /// Point-cloud spacing (m).
    pub spacing: f64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            seed: 1,
            room: [8.0, 6.0, 3.0],
            cameras: vec![
                // A CCTV camera high in a corner: wide lens, strong barrel distortion.
                Camera {
                    name: "cctv".into(),
                    position: [0.3, 0.3, 2.7],
                    heading_deg: 52.0,
                    pitch_deg: -24.0,
                    roll_deg: 1.5,
                    size: [1280, 720],
                    fx: 620.0,
                    fy: 620.0,
                    cx: 646.0,
                    cy: 355.0,
                    distortion: [-0.28, 0.09, -0.012, 0.0004, -0.0003],
                },
                // A handheld photo at eye level from across the room: normal lens, little
                // distortion.
                Camera {
                    name: "photo".into(),
                    position: [7.7, 5.7, 1.62],
                    heading_deg: -122.0,
                    pitch_deg: -9.0,
                    roll_deg: -0.8,
                    size: [1600, 1200],
                    fx: 1150.0,
                    fy: 1150.0,
                    cx: 804.0,
                    cy: 596.0,
                    distortion: [-0.05, 0.02, 0.0, 0.0, 0.0],
                },
            ],
            people: vec![
                Person {
                    feet: [3.2, 3.4, 0.0],
                    height: 1.63,
                    heading_deg: 200.0,
                },
                Person {
                    feet: [4.9, 2.6, 0.0],
                    height: 1.84,
                    heading_deg: 250.0,
                },
                Person {
                    feet: [2.4, 4.6, 0.0],
                    height: 1.72,
                    heading_deg: 160.0,
                },
            ],
            markers: 30,
            pick_sigma_px: 0.5,
            spacing: 0.01,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Marker {
    pub centre: V3,
    pub normal: V3,
    pub rgb: [u8; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    /// Index into the markers or people.
    pub index: usize,
    /// True pixel, and as picked (with noise); None if outside the image or hidden.
    pub px: Option<[f64; 2]>,
    pub picked: Option<[f64; 2]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonView {
    pub index: usize,
    /// Pixels of the top of the head and of the point between the feet.
    pub head: Option<[f64; 2]>,
    pub feet: Option<[f64; 2]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct View {
    pub camera: String,
    pub file: String,
    pub markers: Vec<Observation>,
    pub people: Vec<PersonView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Truth {
    pub options: Options,
    pub markers: Vec<Marker>,
    pub views: Vec<View>,
}

fn unit(a: V3) -> V3 {
    scale(a, 1.0 / dot(a, a).sqrt())
}

/// Coloured triangles making up the scene.
struct Tri {
    v: [V3; 3],
    rgb: [u8; 3],
}

fn quad(out: &mut Vec<Tri>, o: V3, a: V3, b: V3, n: usize, rgb: impl Fn(usize, usize) -> [u8; 3]) {
    // Tessellate so lens distortion (applied per vertex) bends straight edges correctly.
    for i in 0..n {
        for j in 0..n {
            let p = |s: usize, t: usize| {
                add(
                    o,
                    add(scale(a, s as f64 / n as f64), scale(b, t as f64 / n as f64)),
                )
            };
            let c = rgb(i, j);
            out.push(Tri {
                v: [p(i, j), p(i + 1, j), p(i + 1, j + 1)],
                rgb: c,
            });
            out.push(Tri {
                v: [p(i, j), p(i + 1, j + 1), p(i, j + 1)],
                rgb: c,
            });
        }
    }
}

fn cylinder(out: &mut Vec<Tri>, a: V3, b: V3, r: f64, rgb: [u8; 3]) {
    let axis = unit(sub(b, a));
    let t = if axis[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let u = unit(cross(axis, t));
    let w = cross(axis, u);
    let n = 16;
    let ring = |c: V3, k: usize| {
        let ang = std::f64::consts::TAU * k as f64 / n as f64;
        add(c, add(scale(u, r * ang.cos()), scale(w, r * ang.sin())))
    };
    for k in 0..n {
        let (a0, a1, b0, b1) = (ring(a, k), ring(a, k + 1), ring(b, k), ring(b, k + 1));
        out.push(Tri {
            v: [a0, a1, b1],
            rgb,
        });
        out.push(Tri {
            v: [a0, b1, b0],
            rgb,
        });
        out.push(Tri {
            v: [a, a1, a0],
            rgb,
        });
        out.push(Tri {
            v: [b, b0, b1],
            rgb,
        });
    }
}

fn sphere(out: &mut Vec<Tri>, c: V3, r: f64, rgb: [u8; 3]) {
    let n = 12;
    let p = |i: usize, k: usize| {
        let (th, ph) = (
            std::f64::consts::PI * i as f64 / n as f64,
            std::f64::consts::TAU * k as f64 / (2 * n) as f64,
        );
        [
            c[0] + r * th.sin() * ph.cos(),
            c[1] + r * th.sin() * ph.sin(),
            c[2] + r * th.cos(),
        ]
    };
    for i in 0..n {
        for k in 0..2 * n {
            out.push(Tri {
                v: [p(i, k), p(i + 1, k), p(i + 1, k + 1)],
                rgb,
            });
            out.push(Tri {
                v: [p(i, k), p(i + 1, k + 1), p(i, k + 1)],
                rgb,
            });
        }
    }
}

/// A standing figure: legs, torso, arms and head as cylinders and a sphere, proportioned
/// from `anthropometry`, with the top of the head exactly at `height`.
fn person(out: &mut Vec<Tri>, p: &Person) {
    use anthropometry::*;
    let h = p.height;
    let hd = p.heading_deg.to_radians();
    let side = [hd.cos(), -hd.sin(), 0.0];
    let at = |s: f64, z: f64| add(p.feet, add(scale(side, s), [0.0, 0.0, z]));
    let (cloth, skin) = ([54, 70, 96], [196, 154, 124]);
    let head_r = (1.0 - CHIN) * h / 2.0;
    for s in [-1.0, 1.0] {
        let x = s * HIP_WIDTH * h / 4.0;
        cylinder(out, at(x, ANKLE * h), at(x, HIP * h), 0.06 * h / 2.0, cloth);
        // Foot: a short cylinder along the facing direction, sole on the floor.
        let fwd = [hd.sin(), hd.cos(), 0.0];
        let heel = at(x, FOOT_WIDTH * h / 2.0);
        cylinder(
            out,
            heel,
            add(heel, scale(fwd, FOOT_LENGTH * h * 0.8)),
            FOOT_WIDTH * h / 2.0,
            [30, 30, 30],
        );
        cylinder(
            out,
            at(x, 0.0 + FOOT_WIDTH * h / 2.0),
            at(x, ANKLE * h + 0.01),
            0.05 * h / 2.0,
            cloth,
        );
        let sh = at(s * SHOULDER_WIDTH * h / 2.0 * 0.85, SHOULDER * h);
        let elbow = add(sh, [0.0, 0.0, -UPPER_ARM * h]);
        let wrist = add(elbow, [0.0, 0.0, -FOREARM * h]);
        cylinder(out, sh, elbow, 0.022 * h, cloth);
        cylinder(out, elbow, wrist, 0.018 * h, skin);
    }
    cylinder(
        out,
        at(0.0, HIP * h - 0.03 * h),
        at(0.0, SHOULDER * h),
        0.075 * h,
        cloth,
    );
    cylinder(
        out,
        at(0.0, SHOULDER * h),
        at(0.0, CHIN * h),
        0.03 * h,
        skin,
    );
    sphere(out, at(0.0, h - head_r), head_r, skin);
}

fn marker_colour(k: usize) -> [u8; 3] {
    const C: [[u8; 3]; 6] = [
        [230, 40, 40],
        [40, 180, 60],
        [40, 90, 230],
        [240, 200, 30],
        [200, 50, 200],
        [30, 200, 210],
    ];
    C[k % C.len()]
}

fn markers(o: &Options) -> Vec<Marker> {
    let mut rng = Rng::new(o.seed ^ 0x3a7);
    let [x, y, z] = o.room;
    (0..o.markers)
        .map(|k| {
            // Spread over the floor and the four walls, clear of corners.
            let (centre, normal) = match k % 5 {
                0 => (
                    [rng.range(0.5, x - 0.5), rng.range(0.5, y - 0.5), 0.0],
                    [0.0, 0.0, 1.0],
                ),
                1 => (
                    [0.0, rng.range(0.5, y - 0.5), rng.range(0.3, z - 0.3)],
                    [1.0, 0.0, 0.0],
                ),
                2 => (
                    [rng.range(0.5, x - 0.5), 0.0, rng.range(0.3, z - 0.3)],
                    [0.0, 1.0, 0.0],
                ),
                3 => (
                    [x, rng.range(0.5, y - 0.5), rng.range(0.3, z - 0.3)],
                    [-1.0, 0.0, 0.0],
                ),
                _ => (
                    [rng.range(0.5, x - 0.5), y, rng.range(0.3, z - 0.3)],
                    [0.0, -1.0, 0.0],
                ),
            };
            Marker {
                centre,
                normal,
                rgb: marker_colour(k),
            }
        })
        .collect()
}

fn marker_axes(m: &Marker) -> (V3, V3) {
    let t = if m.normal[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let u = unit(cross(m.normal, t));
    (u, cross(m.normal, u))
}

fn scene(o: &Options, ms: &[Marker]) -> Vec<Tri> {
    let [x, y, z] = o.room;
    let mut t = vec![];
    // Floor: a 0.5 m checker, so the image has texture for matching by eye.
    let cells = ((x.max(y)) / 0.5).ceil() as usize;
    quad(
        &mut t,
        [0.0, 0.0, 0.0],
        [cells as f64 * 0.5, 0.0, 0.0],
        [0.0, cells as f64 * 0.5, 0.0],
        cells,
        |i, j| {
            if (i + j) % 2 == 0 {
                [150, 146, 138]
            } else {
                [120, 116, 110]
            }
        },
    );
    let n = 16;
    quad(
        &mut t,
        [0.0, 0.0, 0.0],
        [0.0, y, 0.0],
        [0.0, 0.0, z],
        n,
        |_, _| [200, 196, 186],
    );
    quad(
        &mut t,
        [0.0, 0.0, 0.0],
        [x, 0.0, 0.0],
        [0.0, 0.0, z],
        n,
        |_, _| [190, 186, 176],
    );
    quad(
        &mut t,
        [x, 0.0, 0.0],
        [0.0, y, 0.0],
        [0.0, 0.0, z],
        n,
        |_, _| [205, 200, 190],
    );
    quad(
        &mut t,
        [0.0, y, 0.0],
        [x, 0.0, 0.0],
        [0.0, 0.0, z],
        n,
        |_, _| [196, 192, 182],
    );
    // A table and a cabinet, for occlusion and depth cues.
    for (lo, hi) in [
        ([5.6, 4.0, 0.0], [6.8, 4.8, 0.75]),
        ([0.02, 2.2, 0.0], [0.5, 3.0, 1.8]),
    ] {
        let d = sub(hi, lo);
        let c = [110, 84, 60];
        quad(
            &mut t,
            [lo[0], lo[1], hi[2]],
            [d[0], 0.0, 0.0],
            [0.0, d[1], 0.0],
            2,
            |_, _| c,
        );
        quad(&mut t, lo, [d[0], 0.0, 0.0], [0.0, 0.0, d[2]], 2, |_, _| c);
        quad(
            &mut t,
            [lo[0], hi[1], 0.0],
            [d[0], 0.0, 0.0],
            [0.0, 0.0, d[2]],
            2,
            |_, _| c,
        );
        quad(&mut t, lo, [0.0, d[1], 0.0], [0.0, 0.0, d[2]], 2, |_, _| c);
        quad(
            &mut t,
            [hi[0], lo[1], 0.0],
            [0.0, d[1], 0.0],
            [0.0, 0.0, d[2]],
            2,
            |_, _| c,
        );
    }
    // Markers: a coloured 0.1 m square with a white 0.03 m centre, just off the surface.
    for m in ms {
        let (u, v) = marker_axes(m);
        let lift = scale(m.normal, 0.001);
        let corner = |s: f64| add(add(m.centre, lift), add(scale(u, -s), scale(v, -s)));
        quad(
            &mut t,
            corner(0.05),
            scale(u, 0.1),
            scale(v, 0.1),
            1,
            |_, _| m.rgb,
        );
        let lift2 = scale(m.normal, 0.002);
        let c2 = add(
            add(m.centre, lift2),
            add(scale(u, -0.015), scale(v, -0.015)),
        );
        quad(&mut t, c2, scale(u, 0.03), scale(v, 0.03), 1, |_, _| {
            [250, 250, 250]
        });
    }
    for p in &o.people {
        person(&mut t, p);
    }
    t
}

/// Supersampling per axis.
const SS: u32 = 2;

/// The scene at `SS`× resolution: shaded colours, and each sample's unshaded source colour
/// (which identifies what was hit, e.g. a marker's white centre).
fn raster(o: &Options, truth: &Truth, cam: &Camera) -> (Vec<[u8; 3]>, Vec<[u8; 3]>) {
    // Render a distortion-free (pinhole) image, where triangles stay straight, over the
    // area the lens sees, then look each output sample up through the inverse distortion.
    let tris = scene(o, &truth.markers);
    let [w, h] = cam.size;
    let (sw, sh) = (w * SS, h * SS);
    let sample_ab = |x: u32, y: u32| {
        cam.undistort([(x as f64 + 0.5) / SS as f64, (y as f64 + 0.5) / SS as f64])
    };
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for x in (0..sw).step_by(8).chain([sw - 1]) {
        for y in [0, sh - 1] {
            let ab = sample_ab(x, y);
            for k in 0..2 {
                if ab[k].is_finite() {
                    lo[k] = lo[k].min(ab[k]);
                    hi[k] = hi[k].max(ab[k]);
                }
            }
        }
    }
    for y in (0..sh).step_by(8).chain([sh - 1]) {
        for x in [0, sw - 1] {
            let ab = sample_ab(x, y);
            for k in 0..2 {
                if ab[k].is_finite() {
                    lo[k] = lo[k].min(ab[k]);
                    hi[k] = hi[k].max(ab[k]);
                }
            }
        }
    }
    // Pinhole grid at the output's sample density at the image centre.
    let k = cam.fx * SS as f64;
    let (pw, ph) = (
        ((hi[0] - lo[0]) * k).ceil() as usize + 2,
        ((hi[1] - lo[1]) * k * cam.fy / cam.fx).ceil() as usize + 2,
    );
    assert!(
        pw * ph < 200_000_000,
        "lens model out of range for this image"
    );
    let ky = cam.fy * SS as f64;
    let to_grid = |ab: [f64; 2]| [(ab[0] - lo[0]) * k, (ab[1] - lo[1]) * ky];
    let mut depth = vec![f64::INFINITY; pw * ph];
    let mut pcol = vec![[30u8, 31, 34]; pw * ph];
    let mut psrc = vec![[0u8; 3]; pw * ph];
    let rot = cam.rotation();
    let near = 0.05;
    for t in &tris {
        let cc = t.v.map(|v| {
            let d = sub(v, cam.position);
            [dot(rot[0], d), dot(rot[1], d), dot(rot[2], d)]
        });
        // Clip against the near plane (Sutherland–Hodgman), then fan into triangles.
        let mut poly: Vec<V3> = vec![];
        for i in 0..3 {
            let (a, b) = (cc[i], cc[(i + 1) % 3]);
            if a[2] >= near {
                poly.push(a);
            }
            if (a[2] >= near) != (b[2] >= near) {
                let t = (near - a[2]) / (b[2] - a[2]);
                poly.push(add(a, scale(sub(b, a), t)));
            }
        }
        if poly.len() < 3 {
            continue;
        }
        let n = unit(cross(sub(t.v[1], t.v[0]), sub(t.v[2], t.v[0])));
        let view = unit(sub(cam.position, t.v[0]));
        let shade = 0.55 + 0.45 * dot(n, view).abs();
        let rgb = t.rgb.map(|c| (c as f64 * shade).min(255.0) as u8);
        for j in 1..poly.len() - 1 {
            let tri = [poly[0], poly[j], poly[j + 1]];
            let [a, b, c] = tri.map(|q| to_grid([q[0] / q[2], q[1] / q[2]]));
            let z = tri.map(|q| q[2]);
            let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if area.abs() < 1e-12 {
                continue;
            }
            let x0 = a[0].min(b[0]).min(c[0]).floor().max(0.0) as i64;
            let x1 = a[0].max(b[0]).max(c[0]).ceil().min(pw as f64 - 1.0) as i64;
            let y0 = a[1].min(b[1]).min(c[1]).floor().max(0.0) as i64;
            let y1 = a[1].max(b[1]).max(c[1]).ceil().min(ph as f64 - 1.0) as i64;
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let p = [x as f64 + 0.5, y as f64 + 0.5];
                    let w0 = ((b[0] - p[0]) * (c[1] - p[1]) - (b[1] - p[1]) * (c[0] - p[0])) / area;
                    let w1 = ((c[0] - p[0]) * (a[1] - p[1]) - (c[1] - p[1]) * (a[0] - p[0])) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    // Perspective-correct: 1/z is linear across a pinhole image.
                    let d = 1.0 / (w0 / z[0] + w1 / z[1] + w2 / z[2]);
                    let i = y as usize * pw + x as usize;
                    if d < depth[i] {
                        depth[i] = d;
                        pcol[i] = rgb;
                        psrc[i] = t.rgb;
                    }
                }
            }
        }
    }
    let mut col = vec![[30u8, 31, 34]; (sw * sh) as usize];
    let mut src = vec![[0u8; 3]; (sw * sh) as usize];
    for y in 0..sh {
        for x in 0..sw {
            let g = to_grid(sample_ab(x, y));
            let (gx, gy) = (g[0] as usize, g[1] as usize);
            if g[0].is_finite() && g[1].is_finite() && gx < pw && gy < ph {
                let i = (y * sw + x) as usize;
                col[i] = pcol[gy * pw + gx];
                src[i] = psrc[gy * pw + gx];
            }
        }
    }
    (col, src)
}

/// Render the scene through a camera: z-buffer, flat colours shaded by the angle to the
/// camera, 2 × 2 supersampling. RGB, row-major.
pub fn render(o: &Options, truth: &Truth, cam: &Camera) -> Vec<u8> {
    let [w, h] = cam.size;
    let ss = SS;
    let sw = w * ss;
    let (col, _) = raster(o, truth, cam);
    let mut img = vec![0u8; (w * h * 3) as usize];
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0u32; 3];
            for sy in 0..ss {
                for sx in 0..ss {
                    let c = col[((y * ss + sy) * sw + x * ss + sx) as usize];
                    for k in 0..3 {
                        acc[k] += c[k] as u32;
                    }
                }
            }
            let i = ((y * w + x) * 3) as usize;
            for k in 0..3 {
                img[i + k] = (acc[k] / (ss * ss)) as u8;
            }
        }
    }
    img
}

/// Is `x` visible from the camera (nothing of the scene nearer along the ray)?
fn visible(tris: &[Tri], cam: &Camera, x: V3) -> bool {
    let (o, d) = (cam.position, sub(x, cam.position));
    let dist = dot(d, d).sqrt();
    let dir = scale(d, 1.0 / dist);
    !tris.iter().any(|t| {
        // Möller–Trumbore.
        let (e1, e2) = (sub(t.v[1], t.v[0]), sub(t.v[2], t.v[0]));
        let p = cross(dir, e2);
        let det = dot(e1, p);
        if det.abs() < 1e-12 {
            return false;
        }
        let s = sub(o, t.v[0]);
        let u = dot(s, p) / det;
        if !(0.0..=1.0).contains(&u) {
            return false;
        }
        let q = cross(s, e1);
        let v = dot(dir, q) / det;
        if v < 0.0 || u + v > 1.0 {
            return false;
        }
        let tt = dot(e2, q) / det;
        tt > 1e-6 && tt < dist - 0.005
    })
}

pub fn truth(o: &Options) -> Truth {
    let ms = markers(o);
    let tris = scene(o, &ms);
    let mut rng = Rng::new(o.seed);
    let views = o
        .cameras
        .iter()
        .map(|cam| {
            let inside = |p: [f64; 2]| {
                p[0] >= 0.0 && p[1] >= 0.0 && p[0] < cam.size[0] as f64 && p[1] < cam.size[1] as f64
            };
            let markers = ms
                .iter()
                .enumerate()
                .map(|(k, m)| {
                    // Observable: in the image, not hidden, and seen within 75° of face-on
                    // (a marker seen edge-on can't be picked).
                    let facing = dot(m.normal, unit(sub(cam.position, m.centre)));
                    let px = cam.project(m.centre).filter(|p| {
                        inside(*p)
                            && facing > 75f64.to_radians().cos()
                            && visible(&tris, cam, add(m.centre, scale(m.normal, 0.003)))
                    });
                    let picked = px.map(|p| {
                        [
                            p[0] + rng.normal() * o.pick_sigma_px,
                            p[1] + rng.normal() * o.pick_sigma_px,
                        ]
                    });
                    Observation {
                        index: k,
                        px,
                        picked,
                    }
                })
                .collect();
            let people = o
                .people
                .iter()
                .enumerate()
                .map(|(k, p)| PersonView {
                    index: k,
                    head: cam
                        .project(add(p.feet, [0.0, 0.0, p.height]))
                        .filter(|q| inside(*q)),
                    feet: cam.project(p.feet).filter(|q| inside(*q)),
                })
                .collect();
            View {
                camera: cam.name.clone(),
                file: format!("{}.png", cam.name),
                markers,
                people,
            }
        })
        .collect();
    Truth {
        options: o.clone(),
        markers: ms,
        views,
    }
}

/// The room as scanned points (floor, walls, furniture and markers, no people: the scan
/// is taken after the event), on a grid of `spacing`.
pub fn points(o: &Options, t: &Truth) -> Vec<Point> {
    let mut out = vec![];
    let tris = scene(
        &Options {
            people: vec![],
            ..o.clone()
        },
        &t.markers,
    );
    let mut rng = Rng::new(o.seed ^ 0x9c);
    for tr in &tris {
        let (e1, e2) = (sub(tr.v[1], tr.v[0]), sub(tr.v[2], tr.v[0]));
        let area = dot(cross(e1, e2), cross(e1, e2)).sqrt() / 2.0;
        let n = (area / (o.spacing * o.spacing)).round() as usize;
        for _ in 0..n {
            let (mut a, mut b) = (rng.unit(), rng.unit());
            if a + b > 1.0 {
                (a, b) = (1.0 - a, 1.0 - b);
            }
            out.push(Point {
                xyz: add(tr.v[0], add(scale(e1, a), scale(e2, b))),
                intensity: 1500,
                rgb: tr.rgb,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_optical_axis_projects_to_the_principal_point() {
        let o = Options::default();
        for c in &o.cameras {
            let fwd = c.rotation()[2];
            let p = c.project(add(c.position, scale(fwd, 5.0))).unwrap();
            assert!((p[0] - c.cx).abs() < 1e-9 && (p[1] - c.cy).abs() < 1e-9);
            // Axes are orthonormal and right-handed.
            let r = c.rotation();
            assert!((dot(cross(r[0], r[1]), r[2]) - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn up_is_up_in_the_image() {
        let c = &Options::default().cameras[1];
        let fwd = c.rotation()[2];
        let at = add(c.position, scale(fwd, 4.0));
        let lo = c.project(at).unwrap();
        let hi = c.project(add(at, [0.0, 0.0, 0.5])).unwrap();
        assert!(hi[1] < lo[1]);
    }

    #[test]
    fn markers_and_people_are_seen_and_rendered_where_projected() {
        let o = Options::default();
        let t = truth(&o);
        for (view, cam) in t.views.iter().zip(&o.cameras) {
            let seen = view.markers.iter().filter(|m| m.px.is_some()).count();
            assert!(seen >= 6, "{}: only {seen} markers visible", view.camera);
            let (_, src) = raster(&o, &t, cam);
            // What the renderer drew under each visible marker's projected centre is that
            // marker's white centre square.
            for (m, k) in view
                .markers
                .iter()
                .filter_map(|m| m.px.map(|p| (p, m.index)))
            {
                let (x, y) = ((m[0] * SS as f64) as u32, (m[1] * SS as f64) as u32);
                let c = src[(y * cam.size[0] * SS + x) as usize];
                assert_eq!(
                    c,
                    [250, 250, 250],
                    "{} {:?} {:?}",
                    view.camera,
                    m,
                    t.markers[k]
                );
            }
            assert!(view
                .people
                .iter()
                .all(|p| p.head.is_some() && p.feet.is_some()));
        }
    }
}
