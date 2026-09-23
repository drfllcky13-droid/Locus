//! Camera matching, subject height by reverse projection, and lines of sight. Method,
//! assumptions and limitations: docs/methods/camera-height.md.
//!
//! Camera model (OpenCV's, as the generator in locus-synth): camera axes x right, y down,
//! z forward; `x_c = R (X − C)`; normalised `(a, b) = (x_c / z_c, y_c / z_c)`; with
//! `r² = a² + b²`, `a' = a (1 + k1 r² + k2 r⁴ + k3 r⁶) + 2 p1 a b + p2 (r² + 2a²)` and
//! `b' = b (1 + k1 r² + k2 r⁴ + k3 r⁶) + p1 (r² + 2b²) + 2 p2 a b`; pixel
//! `u = f a' + cx`, `v = f b' + cy` (square pixels, no skew; pixel centres at integer + 0.5).
//!
//! The camera is solved from image-to-scan point pairs: a start from the direct linear
//! transform (normalised DLT, decomposed into K, R, C), refined by Levenberg–Marquardt on the
//! reprojection errors in σ units over the pose and the lens model chosen. Its covariance is
//! (JᵀJ)⁻¹, inflated by the Birge ratio when the residuals scatter more than stated.

use crate::measure::{cross, dot, norm, sub, Measured, P3};
use crate::trajectory::{PhotoRef, PointSource};
use nalgebra::{DMatrix, DVector, Matrix3, Vector3};
use serde::{Deserialize, Serialize};

pub const METHOD: &str = "camera/1";
pub const WITNESS_METHOD: &str = "witness/1";

pub type P2 = [f64; 2];

pub const ASSUMPTIONS: &[&str] = &[
    "The image is a single central projection (a pinhole camera) with the lens distortion of the chosen model, square pixels and no skew.",
    "The scene has not changed between the image and the scan at the points paired: each pair is the same physical point.",
    "A subject's feet point is on the floor plane, and the top of the head is vertically above it (standing upright).",
    "Frames other than the camera's photo come from the same fixed camera (not moved or zoomed between them).",
    "When the pairs lie on one plane, the pixels are square and the principal point is at the image centre (the planar start's assumptions).",
];

pub const LIMITATIONS: &[&str] = &[
    "The solve is only as good as the pairs: few pairs, pairs bunched in one part of the image, or pairs near one plane leave the focal length and distortion poorly determined; the stated uncertainties show this.",
    "Height by reverse projection measures to the top of what was marked, in that frame. Apparent height changes with the phase of the gait (a walking person is shortest at mid-stride and tallest at mid-stance, by a few centimetres), with footwear and headwear, and with posture (a slouch, a head tilt, a lean). The result is the height of the image feature, not the subject's stature. Measure the subject in several frames and report the range across them.",
    "Rolling-shutter, motion blur, compression and interlacing in video frames are not modelled.",
    "A line of sight is tested against the scan only: anything not in the scan (people, vehicles, lighting, smoke) is not an obstruction here.",
];

pub const WITNESS_ASSUMPTIONS: &[&str] = &[
    "The eye position is where the examiner states it: a point on the floor and an eye height above it.",
    "A line of sight is blocked where scan points lie within the stated radius of it, away from its ends.",
];

pub const WITNESS_LIMITATIONS: &[&str] = &[
    "Only what is in the scan can block a line of sight. People, vehicles, lighting and visibility at the time are not modelled.",
    "Gaps in the scan (occlusions, glass, dark surfaces) can make a blocked line look clear.",
    "The view shows geometry, not what a person would notice or remember.",
];

#[derive(Debug, Clone, PartialEq)]
pub enum CameraError {
    TooFewPairs(usize),
    OnePlane,
    Degenerate(&'static str),
    Height(&'static str),
}

impl std::fmt::Display for CameraError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CameraError::TooFewPairs(n) => write!(
                f,
                "{n} point pairs; at least 6 are needed (more than the lens model has unknowns)"
            ),
            CameraError::OnePlane => write!(
                f,
                "the scan points lie on or near one plane; pair points on at least two surfaces (a wall and the floor)"
            ),
            CameraError::Degenerate(m) => write!(f, "{m}"),
            CameraError::Height(m) => write!(f, "{m}"),
        }
    }
}

// ---------------------------------------------------------------------------------------
// The camera
// ---------------------------------------------------------------------------------------

/// Which lens parameters are solved for; the rest stay at their defaults (principal point
/// at the image centre, no distortion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LensModel {
    /// Focal length only.
    Pinhole,
    /// Focal length and k1.
    Radial1,
    /// Focal length, k1 and k2.
    Radial2,
    /// Focal length, principal point, k1, k2, k3, p1, p2.
    Full,
    /// Chosen by leave-one-out cross-validation among the four.
    Auto,
}

impl LensModel {
    /// Which of the lens parameters [f, cx, cy, k1, k2, k3, p1, p2] are free.
    fn free(self) -> [bool; 8] {
        match self {
            LensModel::Pinhole => [true, false, false, false, false, false, false, false],
            LensModel::Radial1 => [true, false, false, true, false, false, false, false],
            LensModel::Radial2 => [true, false, false, true, true, false, false, false],
            LensModel::Full | LensModel::Auto => [true; 8],
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            LensModel::Pinhole => "focal length only",
            LensModel::Radial1 => "focal length and k1",
            LensModel::Radial2 => "focal length, k1 and k2",
            LensModel::Full => "the full model",
            LensModel::Auto => "chosen by leave-one-out",
        }
    }
    pub fn describe(self) -> &'static str {
        match self {
            LensModel::Pinhole => {
                "focal length only (principal point at the image centre, no distortion)"
            }
            LensModel::Radial1 => {
                "focal length and radial distortion k1 (principal point at the image centre)"
            }
            LensModel::Radial2 => {
                "focal length and radial distortion k1, k2 (principal point at the image centre)"
            }
            LensModel::Full => {
                "focal length, principal point, radial distortion k1–k3 and tangential p1, p2"
            }
            LensModel::Auto => "chosen by leave-one-out cross-validation",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    /// Optical centre (m, project frame).
    pub position: P3,
    /// Rows: the camera's x (right), y (down) and z (forward) axes in the project frame.
    pub rotation: [P3; 3],
    /// Image size (px).
    pub size: [u32; 2],
    /// Focal length (px) and principal point (px).
    pub f: f64,
    pub cx: f64,
    pub cy: f64,
    /// k1, k2, k3, p1, p2.
    pub distortion: [f64; 5],
}

impl Camera {
    fn distort(&self, a: f64, b: f64) -> P2 {
        let [k1, k2, k3, p1, p2] = self.distortion;
        let r2 = a * a + b * b;
        let radial = 1.0 + k1 * r2 + k2 * r2 * r2 + k3 * r2 * r2 * r2;
        [
            a * radial + 2.0 * p1 * a * b + p2 * (r2 + 2.0 * a * a),
            b * radial + p1 * (r2 + 2.0 * b * b) + 2.0 * p2 * a * b,
        ]
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

    /// A point in the camera's frame.
    pub fn to_camera(&self, x: P3) -> P3 {
        let d = sub(x, self.position);
        [
            dot(self.rotation[0], d),
            dot(self.rotation[1], d),
            dot(self.rotation[2], d),
        ]
    }

    /// Pixel of a project-frame point, or None behind the camera.
    pub fn project(&self, x: P3) -> Option<P2> {
        let c = self.to_camera(x);
        if c[2] <= 1e-9 || !self.within_lens((c[0] / c[2]).powi(2) + (c[1] / c[2]).powi(2)) {
            return None;
        }
        let [a, b] = self.distort(c[0] / c[2], c[1] / c[2]);
        Some([self.f * a + self.cx, self.f * b + self.cy])
    }

    /// Undistorted normalised coordinates of a pixel (Newton's method on the lens model);
    /// None where it doesn't converge.
    pub fn undistort(&self, px: P2) -> Option<P2> {
        let t = [(px[0] - self.cx) / self.f, (px[1] - self.cy) / self.f];
        let (mut a, mut b) = (t[0], t[1]);
        for _ in 0..60 {
            let d = self.distort(a, b);
            let r = [d[0] - t[0], d[1] - t[1]];
            if r[0].abs() + r[1].abs() < 1e-14 {
                return Some([a, b]);
            }
            let h = 1e-7;
            let (da, db) = (self.distort(a + h, b), self.distort(a, b + h));
            let j = [
                [(da[0] - d[0]) / h, (db[0] - d[0]) / h],
                [(da[1] - d[1]) / h, (db[1] - d[1]) / h],
            ];
            let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
            if det.abs() < 1e-12 {
                return None;
            }
            a -= (j[1][1] * r[0] - j[0][1] * r[1]) / det;
            b -= (-j[1][0] * r[0] + j[0][0] * r[1]) / det;
        }
        let d = self.distort(a, b);
        ((d[0] - t[0]).abs() + (d[1] - t[1]).abs() < 1e-9).then_some([a, b])
    }

    /// Unit direction in the project frame of the ray through a pixel.
    pub fn ray(&self, px: P2) -> Option<P3> {
        let [a, b] = self.undistort(px)?;
        let r = &self.rotation;
        let d = [0, 1, 2].map(|k| r[0][k] * a + r[1][k] * b + r[2][k]);
        let n = norm(d);
        Some(d.map(|v| v / n))
    }

    /// Heading of the optical axis clockwise from +y, pitch up from horizontal, and roll
    /// about the axis (clockwise as the camera sees it), in degrees: the generator's angles.
    pub fn angles(&self) -> [f64; 3] {
        let [right, _, fwd] = self.rotation;
        let pitch = fwd[2].clamp(-1.0, 1.0).asin();
        let heading = fwd[0].atan2(fwd[1]);
        let right0 = [heading.cos(), -heading.sin(), 0.0];
        let down0 = cross(fwd, right0);
        let roll = dot(right, down0).atan2(dot(right, right0));
        [
            heading.to_degrees().rem_euclid(360.0),
            pitch.to_degrees(),
            roll.to_degrees(),
        ]
    }

    /// Horizontal and vertical field of view (degrees), without distortion.
    pub fn fov(&self) -> [f64; 2] {
        [0, 1].map(|k| (2.0 * (self.size[k] as f64 / 2.0 / self.f).atan()).to_degrees())
    }

    /// The 14 parameters [ω (3), C (3), f, cx, cy, k1, k2, k3, p1, p2] about this camera
    /// (ω = 0: a small turn of the camera).
    fn params(&self) -> [f64; 14] {
        let [k1, k2, k3, p1, p2] = self.distortion;
        let c = self.position;
        [
            0.0, 0.0, 0.0, c[0], c[1], c[2], self.f, self.cx, self.cy, k1, k2, k3, p1, p2,
        ]
    }

    /// This camera with its parameters replaced by `p` (ω turning its rotation).
    fn with(&self, p: &[f64; 14]) -> Camera {
        let w = Vector3::new(p[0], p[1], p[2]);
        let turn = nalgebra::Rotation3::new(w);
        let r0 = Matrix3::from_rows(
            &self
                .rotation
                .map(|r| Vector3::new(r[0], r[1], r[2]).transpose()),
        );
        // x_c = R (X − C); a small turn of the camera: R' = R · turnᵀ.
        let r = r0 * turn.matrix().transpose();
        Camera {
            position: [p[3], p[4], p[5]],
            rotation: [0, 1, 2].map(|i| [r[(i, 0)], r[(i, 1)], r[(i, 2)]]),
            size: self.size,
            f: p[6],
            cx: p[7],
            cy: p[8],
            distortion: [p[9], p[10], p[11], p[12], p[13]],
        }
    }
}

// ---------------------------------------------------------------------------------------
// Solving
// ---------------------------------------------------------------------------------------

/// A pixel in the image and the same point in the scan (m, project frame).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraPair {
    pub px: P2,
    pub world: P3,
}

/// One lens model's leave-one-out score: each pair left out in turn, the camera solved from
/// the rest, and the left-out pair's reprojection error (px).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelScore {
    pub model: LensModel,
    /// RMS held-out error (px); none when the model has too many unknowns for the pairs, or
    /// can't be solved on a plane.
    pub held_out_rms: Option<f64>,
    /// RMS error over all pairs when solved from all of them (px).
    pub fit_rms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    pub scores: Vec<ModelScore>,
    pub reason: String,
    /// The models the bootstrap was pooled over (the chosen one and those the pairs can't
    /// tell from it).
    #[serde(default)]
    pub pooled: Vec<LensModel>,
}

/// The plane the pairs lie on, when they do (the planar start was used).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Planar {
    pub point: P3,
    pub normal: P3,
    /// RMS and largest distance of the pairs' scan points from it (m), and their extent in
    /// it (largest distance from their centre, m).
    pub rms: f64,
    pub max_off: f64,
    pub extent: f64,
    /// The start's focal length came from the homography, or was assumed (a photo taken
    /// nearly square on to the plane gives the homography no focal length).
    pub f_assumed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Solve {
    /// The lens model used, and how it was chosen (leave-one-out, when automatic).
    pub model: LensModel,
    #[serde(default)]
    pub selection: Option<Selection>,
    #[serde(default)]
    pub planar: Option<Planar>,
    pub camera: Camera,
    /// 1σ of the position (m), of heading, pitch and roll (degrees), of the focal length and
    /// principal point (px), and of k1, k2, k3, p1, p2 (0 where fixed), from the bootstrap.
    pub position_sigma: P3,
    pub angles_sigma: P3,
    pub f_sigma: f64,
    pub principal_sigma: P2,
    pub distortion_sigma: [f64; 5],
    /// The parameters solved for (indices into [ω, C, f, cx, cy, k1, k2, k3, p1, p2]).
    pub free: Vec<usize>,
    /// Each pair's reprojection residual (px, x and y) and its size in σ.
    pub residuals: Vec<P2>,
    pub residual_sigmas: Vec<f64>,
    pub rms_px: f64,
    pub chi2: f64,
    pub dof: usize,
    /// √(χ²/dof) when above 1: the factor the bootstrap's pixel noise was inflated by.
    pub birge: f64,
    /// Bootstrap re-solves made, and those that failed.
    #[serde(default)]
    pub bootstrap: usize,
    #[serde(default)]
    pub bootstrap_failed: usize,
    pub warnings: Vec<String>,
    /// The bootstrap's cameras, for the heights' Monte Carlo (not stored).
    #[serde(skip)]
    pub draws: Vec<Camera>,
}

/// The parameters' finite-difference steps.
const STEPS: [f64; 14] = [
    1e-7, 1e-7, 1e-7, 1e-6, 1e-6, 1e-6, 1e-3, 1e-3, 1e-3, 1e-7, 1e-7, 1e-7, 1e-8, 1e-8,
];

fn free_of(m: LensModel, planar: bool) -> Vec<usize> {
    let lens = m.free();
    (0..14)
        .filter(|&i| i < 6 || lens[i - 6])
        // On a plane the principal point stays at the image centre (the planar start's
        // assumption).
        .filter(|&i| !(planar && (i == 7 || i == 8)))
        .collect()
}

/// The pairs and their uncertainties, for Levenberg–Marquardt.
struct Problem<'a> {
    pairs: &'a [CameraPair],
    pick: f64,
    point: f64,
}

impl Problem<'_> {
    /// Each pair's 1σ in pixels: the pick σ and the scan point's σ projected at its depth.
    fn sigmas(&self, c: &Camera) -> Vec<f64> {
        self.pairs
            .iter()
            .map(|p| {
                let z = c.to_camera(p.world)[2].max(1e-6);
                (self.pick.powi(2) + (c.f * self.point / z).powi(2)).sqrt()
            })
            .collect()
    }

    fn residuals(&self, c: &Camera, sig: &[f64]) -> DVector<f64> {
        let mut r = DVector::zeros(2 * self.pairs.len());
        for (i, p) in self.pairs.iter().enumerate() {
            match c.project(p.world) {
                Some(q) => {
                    r[2 * i] = (q[0] - p.px[0]) / sig[i];
                    r[2 * i + 1] = (q[1] - p.px[1]) / sig[i];
                }
                None => {
                    r[2 * i] = 1e6;
                    r[2 * i + 1] = 1e6;
                }
            }
        }
        r
    }

    fn jacobian(&self, c: &Camera, sig: &[f64], r0: &DVector<f64>, free: &[usize]) -> DMatrix<f64> {
        let base = c.params();
        let mut j = DMatrix::zeros(r0.len(), free.len());
        for (col, &i) in free.iter().enumerate() {
            let mut p = base;
            p[i] += STEPS[i];
            let r1 = self.residuals(&c.with(&p), sig);
            j.set_column(col, &((r1 - r0) / STEPS[i]));
        }
        j
    }

    /// Levenberg–Marquardt over `free` from `cam`, twice (the weights set from the estimate
    /// each time).
    fn refine(&self, cam: Camera, free: &[usize]) -> Camera {
        self.refine_with(cam, free, 2, 200)
    }

    /// Levenberg–Marquardt: `passes` times (the weights set from the estimate each time), at
    /// most `iterations` steps each, stopping when a step improves χ² by less than 1 part in
    /// 10⁹.
    fn refine_with(
        &self,
        mut cam: Camera,
        free: &[usize],
        passes: usize,
        iterations: usize,
    ) -> Camera {
        for _ in 0..passes {
            let sig = self.sigmas(&cam);
            let mut r = self.residuals(&cam, &sig);
            let mut cost = r.norm_squared();
            let mut lambda = 1e-3;
            for _ in 0..iterations {
                let j = self.jacobian(&cam, &sig, &r, free);
                let jtj = j.transpose() * &j;
                let g = -(j.transpose() * &r);
                let mut stepped = false;
                for _ in 0..12 {
                    let mut a = jtj.clone();
                    for k in 0..a.nrows() {
                        a[(k, k)] += lambda * jtj[(k, k)].max(1e-12);
                    }
                    let Some(dx) = a.cholesky().map(|c| c.solve(&g)) else {
                        lambda *= 10.0;
                        continue;
                    };
                    let mut p = cam.params();
                    for (k, &i) in free.iter().enumerate() {
                        p[i] += dx[k];
                    }
                    let c2 = cam.with(&p);
                    let r2 = self.residuals(&c2, &sig);
                    let cost2 = r2.norm_squared();
                    if cost2 < cost {
                        let small = dx.norm() < 1e-12 || (cost - cost2) < 1e-9 * cost;
                        (cam, r, cost) = (c2, r2, cost2);
                        lambda = (lambda / 3.0).max(1e-12);
                        stepped = !small;
                        break;
                    }
                    lambda *= 10.0;
                }
                if !stepped {
                    break;
                }
            }
        }
        cam
    }
}

/// Normalising transform for points: centred, mean distance √dim.
fn normaliser(pts: &[Vec<f64>]) -> (Vec<f64>, f64) {
    let dim = pts[0].len();
    let n = pts.len() as f64;
    let c: Vec<f64> = (0..dim)
        .map(|k| pts.iter().map(|p| p[k]).sum::<f64>() / n)
        .collect();
    let mean = pts
        .iter()
        .map(|p| (0..dim).map(|k| (p[k] - c[k]).powi(2)).sum::<f64>().sqrt())
        .sum::<f64>()
        / n;
    (c, (dim as f64).sqrt() / mean.max(1e-300))
}

/// The unit vector of `a`'s smallest singular value (AᵀA's eigenvector).
fn null_vector(a: &DMatrix<f64>) -> DVector<f64> {
    let e = (a.transpose() * a).symmetric_eigen();
    let k = e
        .eigenvalues
        .iter()
        .enumerate()
        .min_by(|x, y| x.1.total_cmp(y.1))
        .map(|(i, _)| i)
        .unwrap();
    e.eigenvectors.column(k).into_owned()
}

/// The DLT start: P from the pairs (normalised), decomposed into K, R and C.
fn dlt(pairs: &[CameraPair], size: [u32; 2]) -> Result<Camera, CameraError> {
    let px: Vec<Vec<f64>> = pairs.iter().map(|p| p.px.to_vec()).collect();
    let wx: Vec<Vec<f64>> = pairs.iter().map(|p| p.world.to_vec()).collect();
    let (cp, sp) = normaliser(&px);
    let (cw, sw) = normaliser(&wx);
    let n = pairs.len();
    let mut a = DMatrix::<f64>::zeros(2 * n, 12);
    for (i, p) in pairs.iter().enumerate() {
        let x = [0, 1, 2].map(|k| (p.world[k] - cw[k]) * sw);
        let u = (p.px[0] - cp[0]) * sp;
        let v = (p.px[1] - cp[1]) * sp;
        let xh = [x[0], x[1], x[2], 1.0];
        for k in 0..4 {
            a[(2 * i, k)] = xh[k];
            a[(2 * i, 8 + k)] = -u * xh[k];
            a[(2 * i + 1, 4 + k)] = xh[k];
            a[(2 * i + 1, 8 + k)] = -v * xh[k];
        }
    }
    let pn = DMatrix::from_row_slice(3, 4, null_vector(&a).as_slice());
    // Undo the normalisations: P = T_px⁻¹ Pn T_w.
    let tpx_inv = DMatrix::from_row_slice(
        3,
        3,
        &[1.0 / sp, 0.0, cp[0], 0.0, 1.0 / sp, cp[1], 0.0, 0.0, 1.0],
    );
    let mut tw = DMatrix::<f64>::identity(4, 4) * sw;
    tw[(3, 3)] = 1.0;
    for k in 0..3 {
        tw[(k, 3)] = -cw[k] * sw;
    }
    let mut p = tpx_inv * pn * tw;
    // Sign: the points in front of the camera (positive depth).
    let depth: f64 = pairs
        .iter()
        .map(|q| (0..3).map(|k| p[(2, k)] * q.world[k]).sum::<f64>() + p[(2, 3)])
        .sum();
    if depth < 0.0 {
        p = -p;
    }
    let m = Matrix3::from_fn(|i, j| p[(i, j)]);
    let p4 = Vector3::new(p[(0, 3)], p[(1, 3)], p[(2, 3)]);
    let bad = || CameraError::Degenerate("the pairs don't determine a camera");
    let minv = m.try_inverse().ok_or_else(bad)?;
    let c = -(minv * p4);
    // RQ by Cholesky: M Mᵀ = K Kᵀ with K upper triangular (through the exchange matrix J).
    let j = Matrix3::new(0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0);
    let l = (j * m * m.transpose() * j).cholesky().ok_or_else(bad)?.l();
    let kmat = j * l * j;
    let r = kmat.try_inverse().ok_or_else(bad)? * m;
    if r.determinant() < 0.0 {
        return Err(CameraError::Degenerate(
            "the pairs give a mirror-image camera; check that each pixel is paired with the right scan point",
        ));
    }
    let kmat = kmat / kmat[(2, 2)];
    Ok(Camera {
        position: [c[0], c[1], c[2]],
        rotation: [0, 1, 2].map(|i| [r[(i, 0)], r[(i, 1)], r[(i, 2)]]),
        size,
        f: (kmat[(0, 0)] + kmat[(1, 1)]) / 2.0,
        cx: size[0] as f64 / 2.0,
        cy: size[1] as f64 / 2.0,
        distortion: [0.0; 5],
    })
}

/// The scan points' plane: centre, axes (the last is the normal, right-handed), and the
/// smallest over the largest eigenvalue of their scatter (0 for points on one plane).
fn plane_of(pairs: &[CameraPair]) -> (Vector3<f64>, [Vector3<f64>; 3], f64) {
    let n = pairs.len() as f64;
    let c = Vector3::from_fn(|k, _| pairs.iter().map(|p| p.world[k]).sum::<f64>() / n);
    let mut s = Matrix3::zeros();
    for p in pairs {
        let d = Vector3::new(p.world[0], p.world[1], p.world[2]) - c;
        s += d * d.transpose();
    }
    let e = s.symmetric_eigen();
    let mut order = [0, 1, 2];
    order.sort_by(|&a, &b| e.eigenvalues[b].total_cmp(&e.eigenvalues[a]));
    let e1 = e.eigenvectors.column(order[0]).into_owned();
    let e2 = e.eigenvectors.column(order[1]).into_owned();
    let e3 = e1.cross(&e2);
    let (hi, lo) = (e.eigenvalues[order[0]], e.eigenvalues[order[2]]);
    (c, [e1, e2, e3], if hi > 0.0 { lo / hi } else { 0.0 })
}

/// Pairs this flat (smallest over largest scatter eigenvalue) are solved from a planar start.
const PLANAR: f64 = 1e-3;

/// The planar start: the homography from the plane to the image, with square pixels and the
/// principal point at the image centre, decomposed into the focal length and the pose
/// (Zhang 2000, with the focal length the only intrinsic unknown).
fn planar_start(pairs: &[CameraPair], size: [u32; 2]) -> Result<(Camera, Planar), CameraError> {
    let (c, [e1, e2, e3], _) = plane_of(pairs);
    let centre = [size[0] as f64 / 2.0, size[1] as f64 / 2.0];
    let uv: Vec<Vec<f64>> = pairs
        .iter()
        .map(|p| {
            let d = Vector3::new(p.world[0], p.world[1], p.world[2]) - c;
            vec![d.dot(&e1), d.dot(&e2)]
        })
        .collect();
    let q: Vec<Vec<f64>> = pairs
        .iter()
        .map(|p| vec![p.px[0] - centre[0], p.px[1] - centre[1]])
        .collect();
    let (cu, su) = normaliser(&uv);
    let (cq, sq) = normaliser(&q);
    let n = pairs.len();
    let mut a = DMatrix::<f64>::zeros(2 * n, 9);
    for i in 0..n {
        let (x, y) = ((uv[i][0] - cu[0]) * su, (uv[i][1] - cu[1]) * su);
        let (u, v) = ((q[i][0] - cq[0]) * sq, (q[i][1] - cq[1]) * sq);
        for (k, val) in [x, y, 1.0].into_iter().enumerate() {
            a[(2 * i, k)] = val;
            a[(2 * i, 6 + k)] = -u * val;
            a[(2 * i + 1, 3 + k)] = val;
            a[(2 * i + 1, 6 + k)] = -v * val;
        }
    }
    let hn = Matrix3::from_row_slice(null_vector(&a).as_slice());
    let t_uv = Matrix3::new(su, 0.0, -cu[0] * su, 0.0, su, -cu[1] * su, 0.0, 0.0, 1.0);
    let t_q_inv = Matrix3::new(1.0 / sq, 0.0, cq[0], 0.0, 1.0 / sq, cq[1], 0.0, 0.0, 1.0);
    let h = t_q_inv * hn * t_uv;
    let (h1, h2, h3) = (h.column(0), h.column(1), h.column(2));
    // The focal length from r1 ⟂ r2 and |r1| = |r2|, where r = K⁻¹ h with K = diag(f, f, 1).
    let mut fs = vec![];
    let orth = -(h1[0] * h2[0] + h1[1] * h2[1]) / (h1[2] * h2[2]);
    if orth.is_finite() && orth > 0.0 {
        fs.push(orth.sqrt());
    }
    let eq = (h1[0].powi(2) + h1[1].powi(2) - h2[0].powi(2) - h2[1].powi(2))
        / (h2[2].powi(2) - h1[2].powi(2));
    if eq.is_finite() && eq > 0.0 {
        fs.push(eq.sqrt());
    }
    let span = size[0].max(size[1]) as f64;
    let good: Vec<f64> = fs
        .into_iter()
        .filter(|f| *f > 0.1 * span && *f < 20.0 * span)
        .collect();
    let f_assumed = good.is_empty();
    let f = if f_assumed {
        span // about a 53° field of view across the longer side
    } else {
        good.iter().sum::<f64>() / good.len() as f64
    };
    let kinv = |v: nalgebra::VectorView3<f64>| Vector3::new(v[0] / f, v[1] / f, v[2]);
    let (k1, k2, k3) = (kinv(h1), kinv(h2), kinv(h3));
    let mut lambda = 1.0 / k1.norm();
    if (k3 * lambda)[2] < 0.0 {
        lambda = -lambda; // the plane in front of the camera
    }
    let (r1, r2, t) = (k1 * lambda, k2 * lambda, k3 * lambda);
    let r = Matrix3::from_columns(&[r1, r2, r1.cross(&r2)]);
    let svd = r.svd(true, true);
    let r = svd.u.unwrap() * svd.v_t.unwrap();
    // x_c = R (u e1 + v e2 + w e3) + t, so R_world = R Eᵀ and C = c − R_worldᵀ t.
    let e = Matrix3::from_columns(&[e1, e2, e3]);
    let rw = r * e.transpose();
    let cam_c = c - rw.transpose() * t;
    let offs: Vec<f64> = pairs
        .iter()
        .map(|p| (Vector3::new(p.world[0], p.world[1], p.world[2]) - c).dot(&e3))
        .collect();
    let extent = uv.iter().map(|p| p[0].hypot(p[1])).fold(0.0, f64::max);
    Ok((
        Camera {
            position: [cam_c[0], cam_c[1], cam_c[2]],
            rotation: [0, 1, 2].map(|i| [rw[(i, 0)], rw[(i, 1)], rw[(i, 2)]]),
            size,
            f,
            cx: centre[0],
            cy: centre[1],
            distortion: [0.0; 5],
        },
        Planar {
            point: [c[0], c[1], c[2]],
            normal: [e3[0], e3[1], e3[2]],
            rms: (offs.iter().map(|v| v * v).sum::<f64>() / n as f64).sqrt(),
            max_off: offs.iter().fold(0.0, |m, v| m.max(v.abs())),
            extent,
            f_assumed,
        },
    ))
}

/// The camera for one lens model: the start (the DLT on the pairs nearest the image centre,
/// where distortion is least, or the planar start), then Levenberg–Marquardt in stages (the
/// pose and focal length, then k1, k2 and the rest as the model has them). A strong
/// wide-angle distortion otherwise leaves the start, which has none, in the wrong basin.
fn fit(
    problem: &Problem,
    size: [u32; 2],
    model: LensModel,
) -> Result<(Camera, Option<Planar>), CameraError> {
    let pairs = problem.pairs;
    let (_, _, flat) = plane_of(pairs);
    let (mut cam, planar) = if flat < PLANAR {
        if model == LensModel::Full {
            return Err(CameraError::Degenerate(
                "the full lens model needs pairs off one plane (a plane can't fix the principal point); choose a simpler lens model, or add pairs on another surface",
            ));
        }
        let (c, p) = planar_start(pairs, size)?;
        (c, Some(p))
    } else {
        let centre = [size[0] as f64 / 2.0, size[1] as f64 / 2.0];
        let mut near: Vec<CameraPair> = pairs.to_vec();
        near.sort_by(|a, b| {
            let r = |p: &CameraPair| (p.px[0] - centre[0]).hypot(p.px[1] - centre[1]);
            r(a).total_cmp(&r(b))
        });
        near.truncate((pairs.len() / 2).max(8).min(pairs.len()));
        let start: &[CameraPair] = if plane_of(&near).2 > PLANAR {
            &near
        } else {
            pairs
        };
        (dlt(start, size)?, None)
    };
    let lens = model.free();
    for stage in [
        LensModel::Pinhole,
        LensModel::Radial1,
        LensModel::Radial2,
        LensModel::Full,
    ]
    .into_iter()
    .filter(|m| m.free().iter().zip(lens).all(|(a, b)| !a || b))
    {
        cam = problem.refine(cam, &free_of(stage, planar.is_some()));
    }
    Ok((cam, planar))
}

/// Candidate lens models, simplest first.
const MODELS: [LensModel; 4] = [
    LensModel::Pinhole,
    LensModel::Radial1,
    LensModel::Radial2,
    LensModel::Full,
];

/// Lens models whose held-out error is within this factor of the best are taken as ones the
/// pairs can't tell apart: the bootstrap is pooled over them, so the uncertainty includes the
/// choice of model.
pub const PLAUSIBLE: f64 = 1.25;

/// Choose the lens model by leave-one-out: each pair left out in turn and predicted by the
/// camera solved from the others (started from the solve on all pairs), the model with the
/// smallest held-out reprojection error chosen, a more complex one only when strictly
/// smaller. Returns the choice, the scores, and each usable model's camera on all pairs.
#[allow(clippy::type_complexity)]
fn select(
    problem: &Problem,
    size: [u32; 2],
) -> Result<
    (
        LensModel,
        Selection,
        Vec<(LensModel, Camera, Option<Planar>)>,
    ),
    CameraError,
> {
    let pairs = problem.pairs;
    let planar = plane_of(pairs).2 < PLANAR;
    let mut scores = vec![];
    let mut fits = vec![];
    for m in MODELS {
        let unknowns = 6 + m.free().iter().filter(|f| **f).count();
        let usable = !(planar && m == LensModel::Full) && 2 * (pairs.len() - 1) > unknowns;
        let full = if usable {
            fit(problem, size, m).ok()
        } else {
            None
        };
        let held_out = full.as_ref().and_then(|(cam, pl)| {
            let free = free_of(m, pl.is_some());
            let (mut sq, mut ok) = (0.0, 0usize);
            for i in 0..pairs.len() {
                let rest: Vec<CameraPair> = pairs
                    .iter()
                    .enumerate()
                    .filter(|(k, _)| *k != i)
                    .map(|(_, p)| *p)
                    .collect();
                let sub = Problem {
                    pairs: &rest,
                    ..*problem
                };
                if let Some(q) = sub.refine(cam.clone(), &free).project(pairs[i].world) {
                    sq += (q[0] - pairs[i].px[0]).powi(2) + (q[1] - pairs[i].px[1]).powi(2);
                    ok += 1;
                }
            }
            // Every pair must be predictable (in front of the camera, inside the lens).
            (ok == pairs.len()).then(|| (sq / ok as f64).sqrt())
        });
        scores.push(ModelScore {
            model: m,
            held_out_rms: held_out,
            fit_rms: full.as_ref().map(|(c, _)| rms(c, pairs)),
        });
        if let (Some((c, pl)), Some(_)) = (full, held_out) {
            fits.push((m, c, pl));
        }
    }
    let mut best: Option<(LensModel, f64)> = None;
    for sc in &scores {
        if let Some(e) = sc.held_out_rms {
            if best.is_none_or(|(_, b)| e < b) {
                best = Some((sc.model, e));
            }
        }
    }
    let Some((model, err)) = best else {
        return Err(CameraError::Degenerate(
            "no lens model could be solved from the pairs with each one left out; add pairs, spread over the photo",
        ));
    };
    let pooled: Vec<LensModel> = scores
        .iter()
        .filter(|sc| sc.held_out_rms.is_some_and(|e| e <= PLAUSIBLE * err))
        .map(|sc| sc.model)
        .collect();
    let others: Vec<String> = scores
        .iter()
        .filter(|sc| sc.model != model)
        .map(|sc| match sc.held_out_rms {
            Some(e) => format!("{} {e:.2} px", sc.model.short()),
            None => format!("{} not possible", sc.model.short()),
        })
        .collect();
    let mut reason = format!(
        "chosen by leave-one-out: {} had the smallest held-out reprojection error, {err:.2} px (each pair left out in turn and predicted from the others; {})",
        model.short(),
        others.join(", ")
    );
    if pooled.len() > 1 {
        reason += &format!(
            ". The pairs can't tell it from {} (held-out error within {:.0} % of the best), so the uncertainty is from a bootstrap pooled over them",
            pooled
                .iter()
                .filter(|m| **m != model)
                .map(|m| m.short())
                .collect::<Vec<_>>()
                .join(" and "),
            (PLAUSIBLE - 1.0) * 100.0
        );
    }
    fits.retain(|(m, ..)| pooled.contains(m));
    Ok((
        model,
        Selection {
            scores,
            reason,
            pooled,
        },
        fits,
    ))
}

fn rms(c: &Camera, pairs: &[CameraPair]) -> f64 {
    let sq: f64 = pairs
        .iter()
        .map(|p| {
            c.project(p.world).map_or(1e12, |q| {
                (q[0] - p.px[0]).powi(2) + (q[1] - p.px[1]).powi(2)
            })
        })
        .sum();
    (sq / pairs.len() as f64).sqrt()
}

/// Solve a camera from image-to-scan pairs. `pick_sigma_px` is each pixel's 1σ and
/// `point_sigma` each scan point's (m, projected into the image at its depth). The lens
/// model is chosen by leave-one-out when `model` is `Auto`. The uncertainty is a parametric
/// bootstrap: `draws` re-solves from the solved camera's projections of the scan points plus
/// pixel noise of each pair's σ (inflated by the Birge ratio), seeded by `seed`.
pub fn solve(
    pairs: &[CameraPair],
    size: [u32; 2],
    model: LensModel,
    pick_sigma_px: f64,
    point_sigma: f64,
    draws: usize,
    seed: u64,
) -> Result<Solve, CameraError> {
    if pairs.len() < 6 {
        return Err(CameraError::TooFewPairs(pairs.len()));
    }
    let problem = Problem {
        pairs,
        pick: pick_sigma_px,
        point: point_sigma,
    };
    let (model, selection, fits) = if model == LensModel::Auto {
        let (m, s, f) = select(&problem, size)?;
        (m, Some(s), f)
    } else {
        let (c, p) = fit(&problem, size, model)?;
        (model, None, vec![(model, c, p)])
    };
    let (cam, planar) = fits
        .iter()
        .find(|(m, ..)| *m == model)
        .map(|(_, c, p)| (c.clone(), p.clone()))
        .expect("the chosen model's camera");
    let free = free_of(model, planar.is_some());
    if 2 * pairs.len() <= free.len() {
        return Err(CameraError::TooFewPairs(pairs.len()));
    }
    let sig = problem.sigmas(&cam);
    let r = problem.residuals(&cam, &sig);
    let chi2 = r.norm_squared();
    let dof = 2 * pairs.len() - free.len();
    let birge = (chi2 / dof as f64).sqrt().max(1.0);
    // Parametric bootstrap: synthetic pixels from each plausible model's camera plus noise
    // (its pairs' σ, inflated by its Birge ratio), re-solved from that camera; the draws
    // shared equally between the models.
    let mut rng = Rng(seed.max(1).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut cams = Vec::with_capacity(draws);
    let mut failed = 0;
    for (k, (m, c0, pl)) in fits.iter().enumerate() {
        let free_m = free_of(*m, pl.is_some());
        let sig_m = problem.sigmas(c0);
        let chi2_m = problem.residuals(c0, &sig_m).norm_squared();
        let dof_m = (2 * pairs.len()).saturating_sub(free_m.len()).max(1);
        let birge_m = (chi2_m / dof_m as f64).sqrt().max(1.0);
        let ideal: Vec<P2> = pairs
            .iter()
            .map(|p| c0.project(p.world).unwrap_or(p.px))
            .collect();
        let share = draws / fits.len() + usize::from(k < draws % fits.len());
        for _ in 0..share {
            let sim: Vec<CameraPair> = pairs
                .iter()
                .zip(&ideal)
                .zip(&sig_m)
                .map(|((p, q), s)| CameraPair {
                    px: [
                        q[0] + s * birge_m * rng.gauss(),
                        q[1] + s * birge_m * rng.gauss(),
                    ],
                    world: p.world,
                })
                .collect();
            let sub = Problem {
                pairs: &sim,
                ..problem
            };
            // From the solution, with its weights: a few steps suffice.
            let c = sub.refine_with(c0.clone(), &free_m, 1, 30);
            if rms(&c, &sim) < 10.0 * (rms(c0, pairs) + pick_sigma_px) {
                cams.push(c);
            } else {
                failed += 1;
            }
        }
    }
    let sd = |f: &dyn Fn(&Camera) -> f64| {
        let v: Vec<f64> = cams.iter().map(f).collect();
        let n = v.len().max(2) as f64;
        let m = v.iter().sum::<f64>() / v.len().max(1) as f64;
        (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
    };
    let a0 = cam.angles();
    let angle_sd = |k: usize| {
        sd(&|c: &Camera| {
            let d = c.angles()[k] - a0[k];
            (d + 180.0).rem_euclid(360.0) - 180.0
        })
    };
    let mut res = vec![];
    let mut res_sig = vec![];
    let mut sq = 0.0;
    for (i, p) in pairs.iter().enumerate() {
        let q = cam.project(p.world).unwrap_or([f64::NAN; 2]);
        let d = [q[0] - p.px[0], q[1] - p.px[1]];
        sq += d[0] * d[0] + d[1] * d[1];
        res.push(d);
        res_sig.push(d[0].hypot(d[1]) / sig[i]);
    }
    let f_sigma = sd(&|c: &Camera| c.f);
    let mut warnings = vec![];
    if birge > 2.0 {
        warnings.push(format!(
            "The pairs fit worse than their stated uncertainty (χ² = {chi2:.0} on {dof} degrees of freedom): a pair may be wrong, or the lens model too simple. The bootstrap's noise was inflated by {birge:.1}."
        ));
    }
    let bad: Vec<String> = res_sig
        .iter()
        .enumerate()
        .filter(|(_, s)| **s > 3.0 * birge.max(1.0))
        .map(|(i, _)| (i + 1).to_string())
        .collect();
    if !bad.is_empty() {
        warnings.push(format!(
            "Pair(s) {} are more than 3σ from the solved camera: check that each pixel and scan point are the same place.",
            bad.join(", ")
        ));
    }
    if f_sigma > 0.05 * cam.f {
        warnings.push(format!(
            "The focal length is poorly determined ({:.0} ± {:.0} px): add pairs at different depths and toward the image's edges.",
            cam.f, f_sigma
        ));
    }
    if pairs.len() < free.len() {
        warnings.push(format!(
            "Only {} pairs for the {} unknowns of this lens model: the solve has little redundancy to show a wrong pair.",
            pairs.len(),
            free.len()
        ));
    }
    if let Some(p) = &planar {
        warnings.push(format!(
            "The pairs lie on one plane (RMS {:.0} mm off it): the solve started from the plane's homography, assuming square pixels and the principal point at the image centre{}.",
            p.rms * 1000.0,
            if p.f_assumed {
                "; the photo is nearly square on to the plane, so the start's focal length was assumed"
            } else {
                ""
            }
        ));
    }
    if failed * 20 > draws {
        warnings.push(format!(
            "{failed} of {draws} bootstrap re-solves failed: the camera is poorly determined."
        ));
    }
    Ok(Solve {
        model,
        selection,
        planar,
        position_sigma: [0, 1, 2].map(|k| sd(&|c: &Camera| c.position[k])),
        angles_sigma: [0, 1, 2].map(angle_sd),
        f_sigma,
        principal_sigma: [sd(&|c: &Camera| c.cx), sd(&|c: &Camera| c.cy)],
        distortion_sigma: [0, 1, 2, 3, 4].map(|k| sd(&|c: &Camera| c.distortion[k])),
        free,
        residuals: res,
        residual_sigmas: res_sig,
        rms_px: (sq / pairs.len() as f64).sqrt(),
        chi2,
        dof,
        birge,
        bootstrap: cams.len(),
        bootstrap_failed: failed,
        warnings,
        draws: cams,
        camera: cam,
    })
}

// ---------------------------------------------------------------------------------------
// Subject height by reverse projection
// ---------------------------------------------------------------------------------------

/// Deterministic normal draws (xorshift64*, Box–Muller), so a run repeats exactly.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11
    }
    fn gauss(&mut self) -> f64 {
        let u = (self.next() as f64 + 0.5) / (1u64 << 53) as f64;
        let v = self.next() as f64 / (1u64 << 53) as f64;
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

/// A subject: the image point on the floor midway between the feet, and the top of the head.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeightInput {
    pub label: String,
    pub feet_px: P2,
    pub head_px: P2,
    /// The head point was set by matching a person model of this stature to the frame
    /// (its projected top of head), not clicked.
    #[serde(default)]
    pub matched_model: Option<f64>,
    /// The frame the points were marked on, when not the photo the camera was solved from
    /// (another frame from the same fixed camera): its evidence id.
    #[serde(default)]
    pub frame: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Height {
    pub input: HeightInput,
    /// Height of the head point above the floor (m), 1σ from the Monte Carlo, and its 95 %
    /// interval (2.5th and 97.5th percentiles).
    pub height: Measured,
    pub interval95: P2,
    /// Where the feet ray meets the floor, and the head point above it (m).
    pub feet: P3,
    pub head: P3,
    /// How far the head ray passes from the vertical through the feet point (m): a lean, a
    /// stride, or a wrong feet point.
    pub miss: f64,
    pub draws: usize,
    pub failed: usize,
    /// The frame, when not the camera's photo (hash-checked like it).
    #[serde(default)]
    pub frame: Option<PhotoRef>,
}

/// One subject over the frames it was measured in: the range of the heights, their mean and
/// spread (apparent height changes with the gait and posture from frame to frame).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcrossFrames {
    pub label: String,
    pub frames: usize,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    /// Standard deviation of the frames' heights (0 for one frame).
    pub spread: f64,
    /// The lowest of the frames' 2.5th and the highest of their 97.5th percentiles.
    pub interval95: P2,
}

/// Height of the head point above the floor for one camera: the feet ray meets the floor,
/// and the head ray passes closest to the vertical there.
fn reverse(cam: &Camera, floor_z: f64, feet_px: P2, head_px: P2) -> Option<(f64, P3, P3, f64)> {
    let c = cam.position;
    let df = cam.ray(feet_px)?;
    if df[2] >= -1e-9 {
        return None; // the feet ray never comes down to the floor
    }
    let t = (floor_z - c[2]) / df[2];
    if t <= 0.0 {
        return None;
    }
    let feet = [c[0] + t * df[0], c[1] + t * df[1], floor_z];
    let dh = cam.ray(head_px)?;
    // Closest points between the vertical feet + s z and the ray c + u dh.
    let w0 = sub(feet, c);
    let b = dh[2];
    let (d, e) = (w0[2], dot(dh, w0));
    let den = 1.0 - b * b;
    if den < 1e-9 {
        return None; // the head ray is vertical
    }
    let s = (b * e - d) / den;
    let u = (e - b * d) / den;
    if u <= 0.0 {
        return None;
    }
    let head = [feet[0], feet[1], floor_z + s];
    let on_ray = [c[0] + u * dh[0], c[1] + u * dh[1], c[2] + u * dh[2]];
    Some((s, feet, head, norm(sub(head, on_ray))))
}

/// A subject's height with its uncertainty: Monte Carlo over the camera's bootstrap re-solves
/// and each image point's 1σ (`pick_sigma_px`; a head point from a matched model is drawn the
/// same way).
pub fn height(
    solve: &Solve,
    input: &HeightInput,
    floor_z: f64,
    pick_sigma_px: f64,
    seed: u64,
) -> Result<Height, CameraError> {
    let Some((h, feet, head, miss)) = reverse(&solve.camera, floor_z, input.feet_px, input.head_px)
    else {
        return Err(CameraError::Height(
            "the feet point's ray doesn't reach the floor in front of the camera; check the feet and head points",
        ));
    };
    let mut rng = Rng(seed.max(1).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut hs = Vec::with_capacity(solve.draws.len());
    let mut failed = 0;
    for cam in &solve.draws {
        let fp = [
            input.feet_px[0] + pick_sigma_px * rng.gauss(),
            input.feet_px[1] + pick_sigma_px * rng.gauss(),
        ];
        let hp = [
            input.head_px[0] + pick_sigma_px * rng.gauss(),
            input.head_px[1] + pick_sigma_px * rng.gauss(),
        ];
        match reverse(cam, floor_z, fp, hp) {
            Some((v, ..)) => hs.push(v),
            None => failed += 1,
        }
    }
    let n = hs.len().max(1) as f64;
    let mean = hs.iter().sum::<f64>() / n;
    let sd = (hs.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0)).sqrt();
    hs.sort_by(f64::total_cmp);
    let q = |p: f64| hs[((p * (hs.len() as f64 - 1.0)).round() as usize).min(hs.len() - 1)];
    Ok(Height {
        input: input.clone(),
        height: Measured {
            value: h,
            sigma: sd,
        },
        interval95: if hs.is_empty() {
            [h, h]
        } else {
            [q(0.025), q(0.975)]
        },
        feet,
        head,
        miss,
        draws: hs.len(),
        failed,
        frame: None,
    })
}

// ---------------------------------------------------------------------------------------
// Line of sight
// ---------------------------------------------------------------------------------------

/// A line is blocked when at least this many scan points lie within its radius.
pub const MIN_BLOCKING: usize = 3;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sight {
    pub label: String,
    pub from: P3,
    pub to: P3,
    #[serde(default)]
    pub target_source: Option<PointSource>,
    /// Points within `radius` of the line count, except within `end_clearance` of its ends
    /// (the surfaces the ends are on).
    pub radius: f64,
    pub end_clearance: f64,
    pub length: f64,
    pub clear: bool,
    pub blocking: usize,
    /// The blocking point nearest the eye, and its distance from the eye (m).
    pub first: Option<P3>,
    pub first_distance: Option<f64>,
}

/// Test a line of sight against scan points (those near the line are enough).
pub fn line_of_sight(
    label: &str,
    from: P3,
    to: P3,
    points: &[P3],
    radius: f64,
    end_clearance: f64,
) -> Sight {
    let d = sub(to, from);
    let len = norm(d);
    let u = d.map(|v| v / len.max(1e-12));
    let mut hits: Vec<(f64, P3)> = points
        .iter()
        .filter_map(|p| {
            let w = sub(*p, from);
            let t = dot(w, u);
            if t < end_clearance || t > len - end_clearance {
                return None;
            }
            let off = norm(sub(w, u.map(|v| v * t)));
            (off <= radius).then_some((t, *p))
        })
        .collect();
    hits.sort_by(|a, b| a.0.total_cmp(&b.0));
    let clear = hits.len() < MIN_BLOCKING;
    Sight {
        label: label.into(),
        from,
        to,
        target_source: None,
        radius,
        end_clearance,
        length: len,
        clear,
        blocking: hits.len(),
        first: (!clear).then(|| hits[0].1),
        first_distance: (!clear).then(|| hits[0].0),
    }
}

// ---------------------------------------------------------------------------------------
// Runs
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairInput {
    pub px: P2,
    pub world: P3,
    #[serde(default)]
    pub source: Option<PointSource>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameters {
    pub model: LensModel,
    /// 1σ of a clicked image point (px).
    pub pick_sigma_px: f64,
    /// 1σ of a scan point (m).
    pub point_sigma: f64,
    /// The floor's elevation (m), for heights.
    pub floor_z: f64,
    /// Bootstrap re-solves of the camera (each also a Monte Carlo draw for the heights), and
    /// the seed.
    pub draws: usize,
    pub seed: u64,
}

impl Default for Parameters {
    fn default() -> Self {
        Parameters {
            model: LensModel::Auto,
            pick_sigma_px: 1.0,
            point_sigma: 0.002,
            floor_z: 0.0,
            draws: 1000,
            seed: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub method: String,
    pub photo: Option<PhotoRef>,
    pub pairs: Vec<PairInput>,
    pub parameters: Parameters,
    pub solve: Solve,
    pub heights: Vec<Height>,
    /// Each subject measured in more than one frame, over its frames.
    #[serde(default)]
    pub across_frames: Vec<AcrossFrames>,
    pub summary: String,
    pub assumptions: Vec<String>,
    pub limitations: Vec<String>,
}

/// A camera match with its subjects' heights. `frames` gives each subject's frame (by its
/// evidence id) when it is not the camera's photo.
pub fn run(
    photo: Option<PhotoRef>,
    pairs: Vec<PairInput>,
    size: [u32; 2],
    parameters: Parameters,
    subjects: &[HeightInput],
    frames: &dyn Fn(i64) -> Option<PhotoRef>,
) -> Result<Run, CameraError> {
    let p = &parameters;
    let cp: Vec<CameraPair> = pairs
        .iter()
        .map(|q| CameraPair {
            px: q.px,
            world: q.world,
        })
        .collect();
    let mut solve = solve(
        &cp,
        size,
        p.model,
        p.pick_sigma_px,
        p.point_sigma,
        p.draws,
        p.seed,
    )?;
    let mut heights = vec![];
    for (k, s) in subjects.iter().enumerate() {
        let mut h = height(&solve, s, p.floor_z, p.pick_sigma_px, p.seed + 1 + k as u64)?;
        if let Some(id) = s.frame {
            h.frame = Some(frames(id).ok_or(CameraError::Height(
                "a subject's frame is not an image in the evidence",
            ))?);
        }
        heights.push(h);
    }
    // Heights far off the control points' plane rest on extrapolating a planar solve.
    if let Some(pl) = solve.planar.clone() {
        for h in &heights {
            let off = |x: P3| dot(sub(x, pl.point), pl.normal).abs();
            let far = off(h.head).max(off(h.feet));
            if far > 0.25 * pl.extent {
                solve.warnings.push(format!(
                    "{}: its points lie up to {far:.2} m off the plane of the control points (which spread {:.2} m across it). The height rests on extrapolating a solve from one plane; its interval comes from the bootstrap, but pairs on a second surface would make it more reliable.",
                    h.input.label, pl.extent
                ));
            }
        }
    }
    let mut across_frames = vec![];
    let mut labels: Vec<&str> = heights.iter().map(|h| h.input.label.as_str()).collect();
    labels.dedup();
    labels.sort();
    labels.dedup();
    for l in labels {
        let hs: Vec<&Height> = heights.iter().filter(|h| h.input.label == l).collect();
        if hs.len() < 2 {
            continue;
        }
        let v: Vec<f64> = hs.iter().map(|h| h.height.value).collect();
        let n = v.len() as f64;
        let mean = v.iter().sum::<f64>() / n;
        across_frames.push(AcrossFrames {
            label: l.into(),
            frames: hs.len(),
            min: v.iter().cloned().fold(f64::INFINITY, f64::min),
            max: v.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            mean,
            spread: (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt(),
            interval95: [
                hs.iter()
                    .map(|h| h.interval95[0])
                    .fold(f64::INFINITY, f64::min),
                hs.iter()
                    .map(|h| h.interval95[1])
                    .fold(f64::NEG_INFINITY, f64::max),
            ],
        });
    }
    let c = &solve.camera;
    let [hd, pt, _] = c.angles();
    let mut summary = format!(
        "Camera at ({:.3}, {:.3}, {:.3}) m, heading {:.1}°, pitch {:.1}°, focal length {:.0} px; {} pairs, {:.2} px RMS",
        c.position[0],
        c.position[1],
        c.position[2],
        hd,
        pt,
        c.f,
        pairs.len(),
        solve.rms_px
    );
    for h in &heights {
        if across_frames.iter().any(|a| a.label == h.input.label) {
            continue;
        }
        summary += &format!(
            "; {} {:.3} ± {:.3} m",
            h.input.label, h.height.value, h.height.sigma
        );
    }
    for a in &across_frames {
        summary += &format!(
            "; {} {:.3}–{:.3} m over {} frames",
            a.label, a.min, a.max, a.frames
        );
    }
    Ok(Run {
        method: METHOD.into(),
        photo,
        pairs,
        parameters,
        solve,
        heights,
        across_frames,
        summary,
        assumptions: ASSUMPTIONS.iter().map(|s| s.to_string()).collect(),
        limitations: LIMITATIONS.iter().map(|s| s.to_string()).collect(),
    })
}

/// A witness's view: an eye at a stated height above a floor point, looking toward a point,
/// with lines of sight to targets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WitnessRun {
    pub method: String,
    pub floor_point: P3,
    #[serde(default)]
    pub floor_source: Option<PointSource>,
    pub eye_height: f64,
    pub eye: P3,
    /// Where the view looks (m), and its horizontal field of view (degrees).
    pub look_at: P3,
    pub fov_deg: f64,
    pub sights: Vec<Sight>,
    pub summary: String,
    pub assumptions: Vec<String>,
    pub limitations: Vec<String>,
}

pub fn witness(
    floor_point: P3,
    eye_height: f64,
    look_at: P3,
    fov_deg: f64,
    sights: Vec<Sight>,
) -> WitnessRun {
    let eye = [floor_point[0], floor_point[1], floor_point[2] + eye_height];
    let clear = sights.iter().filter(|s| s.clear).count();
    WitnessRun {
        method: WITNESS_METHOD.into(),
        floor_point,
        floor_source: None,
        eye_height,
        eye,
        look_at,
        fov_deg,
        summary: format!(
            "Eye at ({:.3}, {:.3}, {:.3}) m ({:.2} m above the floor point); {} of {} lines of sight clear",
            eye[0],
            eye[1],
            eye[2],
            eye_height,
            clear,
            sights.len()
        ),
        sights,
        assumptions: WITNESS_ASSUMPTIONS.iter().map(|s| s.to_string()).collect(),
        limitations: WITNESS_LIMITATIONS.iter().map(|s| s.to_string()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A camera like the generator's CCTV: high in a corner, wide lens, strong distortion.
    fn cctv() -> Camera {
        let (h, p, r) = (
            52f64.to_radians(),
            (-24f64).to_radians(),
            1.5f64.to_radians(),
        );
        let fwd = [h.sin() * p.cos(), h.cos() * p.cos(), p.sin()];
        let right0 = [h.cos(), -h.sin(), 0.0];
        let down0 = cross(fwd, right0);
        let right = [0, 1, 2].map(|k| right0[k] * r.cos() + down0[k] * r.sin());
        let down = cross(fwd, right);
        Camera {
            position: [0.3, 0.3, 2.7],
            rotation: [right, down, fwd],
            size: [1280, 720],
            f: 620.0,
            cx: 646.0,
            cy: 355.0,
            distortion: [-0.28, 0.09, -0.012, 0.0004, -0.0003],
        }
    }

    /// Points on the floor and two walls of an 8 × 6 × 3 m room, seen by `cam`.
    fn pairs(cam: &Camera, n: usize, noise: f64, seed: u64) -> Vec<CameraPair> {
        let mut rng = Rng(seed | 1);
        let mut out = vec![];
        let mut k = 0u64;
        while out.len() < n {
            k += 1;
            let a = ((k * 7919) % 97) as f64 / 97.0;
            let b = ((k * 104_729) % 89) as f64 / 89.0;
            let w = match k % 3 {
                0 => [0.5 + 7.0 * a, 0.5 + 5.0 * b, 0.0],
                1 => [8.0, 0.5 + 5.0 * a, 0.2 + 2.6 * b],
                _ => [0.5 + 7.0 * a, 6.0, 0.2 + 2.6 * b],
            };
            if let Some(q) = cam.project(w) {
                if q[0] > 0.0 && q[1] > 0.0 && q[0] < 1280.0 && q[1] < 720.0 {
                    out.push(CameraPair {
                        px: [q[0] + noise * rng.gauss(), q[1] + noise * rng.gauss()],
                        world: w,
                    });
                }
            }
        }
        out
    }

    #[test]
    fn exact_pairs_give_the_exact_camera() {
        let cam = cctv();
        let s = solve(
            &pairs(&cam, 30, 0.0, 1),
            cam.size,
            LensModel::Full,
            0.5,
            0.0,
            10,
            1,
        )
        .unwrap();
        let c = &s.camera;
        assert!(
            norm(sub(c.position, cam.position)) < 1e-6,
            "{:?}",
            c.position
        );
        assert!((c.f - cam.f).abs() < 1e-3 && (c.cx - cam.cx).abs() < 1e-3);
        for (a, b) in c.distortion.iter().zip(cam.distortion) {
            assert!((a - b).abs() < 1e-5, "{:?}", c.distortion);
        }
        let [h, p, r] = c.angles();
        assert!((h - 52.0).abs() < 1e-6 && (p + 24.0).abs() < 1e-6 && (r - 1.5).abs() < 1e-6);
        assert!(s.rms_px < 1e-6);
    }

    #[test]
    fn noisy_pairs_give_an_honest_uncertainty() {
        // Over seeds, the position error in units of its σ behaves like a unit normal.
        let cam = cctv();
        let mut z2 = vec![];
        for seed in 1..=40 {
            let s = solve(
                &pairs(&cam, 30, 0.5, seed),
                cam.size,
                LensModel::Full,
                0.5,
                0.0,
                200,
                seed,
            )
            .unwrap();
            for k in 0..3 {
                z2.push(((s.camera.position[k] - cam.position[k]) / s.position_sigma[k]).powi(2));
            }
        }
        let mean = z2.iter().sum::<f64>() / z2.len() as f64;
        assert!((0.6..1.6).contains(&mean), "mean z² {mean}");
    }

    /// A camera that satisfies the planar start's assumptions: square pixels, principal point
    /// at the centre, k1 only.
    fn plain() -> Camera {
        Camera {
            cx: 640.0,
            cy: 360.0,
            distortion: [-0.1, 0.0, 0.0, 0.0, 0.0],
            ..cctv()
        }
    }

    #[test]
    fn pairs_on_one_plane_start_from_the_homography() {
        let cam = plain();
        let floor: Vec<CameraPair> = pairs(&cam, 90, 0.0, 1)
            .into_iter()
            .filter(|p| p.world[2] == 0.0)
            .collect();
        assert!(floor.len() >= 10);
        let s = solve(&floor, cam.size, LensModel::Radial1, 0.5, 0.0, 50, 1).unwrap();
        assert!(
            norm(sub(s.camera.position, cam.position)) < 1e-6,
            "{:?}",
            s.camera.position
        );
        assert!((s.camera.f - cam.f).abs() < 1e-3);
        let pl = s.planar.as_ref().unwrap();
        assert!(pl.rms < 1e-9 && (pl.normal[2].abs() - 1.0).abs() < 1e-9);
        assert!(s
            .warnings
            .iter()
            .any(|w| w.contains("principal point at the image centre")));
        // The full model can't be solved from one plane; too few pairs are refused.
        assert!(solve(&floor, cam.size, LensModel::Full, 0.5, 0.0, 10, 1).is_err());
        let few = pairs(&cam, 5, 0.0, 1);
        assert!(matches!(
            solve(&few, cam.size, LensModel::Pinhole, 0.5, 0.0, 10, 1),
            Err(CameraError::TooFewPairs(5))
        ));
    }

    #[test]
    fn leave_one_out_picks_the_lens_the_pairs_support() {
        // A strong wide-angle distortion with an off-centre principal point needs more than
        // a focal length; a lens with a little k1 doesn't need the full model.
        let s = solve(
            &pairs(&cctv(), 30, 0.5, 3),
            [1280, 720],
            LensModel::Auto,
            0.5,
            0.0,
            20,
            1,
        )
        .unwrap();
        let sel = s.selection.as_ref().unwrap();
        assert!(
            matches!(s.model, LensModel::Radial2 | LensModel::Full),
            "{sel:?}"
        );
        assert_eq!(sel.scores.len(), 4);
        assert!(sel.reason.contains("leave-one-out"));
        let s = solve(
            &pairs(&plain(), 30, 0.5, 3),
            [1280, 720],
            LensModel::Auto,
            0.5,
            0.0,
            20,
            1,
        )
        .unwrap();
        assert!(s.model != LensModel::Full, "{:?}", s.selection);
        assert!(s.model != LensModel::Pinhole, "{:?}", s.selection);
    }

    #[test]
    fn a_standing_subject_measures_exactly_and_the_interval_covers() {
        let cam = cctv();
        let s = solve(
            &pairs(&cam, 30, 0.2, 1),
            cam.size,
            LensModel::Full,
            0.5,
            0.0,
            300,
            1,
        )
        .unwrap();
        let feet = [3.2, 3.4, 0.0];
        let input = HeightInput {
            label: "A".into(),
            feet_px: cam.project(feet).unwrap(),
            head_px: cam.project([3.2, 3.4, 1.63]).unwrap(),
            matched_model: None,
            frame: None,
        };
        let h = height(&s, &input, 0.0, 0.5, 1).unwrap();
        assert!((h.height.value - 1.63).abs() < 0.01, "{:?}", h.height);
        assert!(h.miss < 0.01 && norm(sub(h.feet, feet)) < 0.02);
        assert!(h.height.sigma > 0.0 && h.interval95[0] < 1.63 && h.interval95[1] > 1.63);
        // A feet point above the camera's horizon is refused.
        let bad = HeightInput {
            feet_px: cam.project([3.2, 3.4, 5.0]).unwrap(),
            ..input
        };
        assert!(height(&s, &bad, 0.0, 0.5, 1).is_err());
    }

    #[test]
    fn a_subject_in_several_frames_is_reported_as_a_range() {
        let cam = cctv();
        let pairs: Vec<PairInput> = pairs(&cam, 30, 0.3, 2)
            .into_iter()
            .map(|p| PairInput {
                px: p.px,
                world: p.world,
                source: None,
            })
            .collect();
        // The same person at two places (two frames of the same fixed camera), 2 cm apart in
        // apparent height (a stride).
        let at = |x: f64, h: f64, frame: Option<i64>| HeightInput {
            label: "A".into(),
            feet_px: cam.project([x, 3.4, 0.0]).unwrap(),
            head_px: cam.project([x, 3.4, h]).unwrap(),
            matched_model: None,
            frame,
        };
        let photo = |id: i64| {
            Some(PhotoRef {
                evidence_id: id,
                name: format!("frame {id}"),
                file: format!("evidence/{id}/f.png"),
                sha256: "ab".repeat(32),
            })
        };
        let r = run(
            None,
            pairs,
            cam.size,
            Parameters {
                model: LensModel::Full,
                draws: 200,
                ..Parameters::default()
            },
            &[at(3.2, 1.70, None), at(4.0, 1.72, Some(9))],
            &photo,
        )
        .unwrap();
        assert_eq!(r.across_frames.len(), 1);
        let a = &r.across_frames[0];
        assert_eq!(a.frames, 2);
        assert!(
            (a.min - 1.70).abs() < 0.01 && (a.max - 1.72).abs() < 0.01,
            "{a:?}"
        );
        assert_eq!(r.heights[1].frame.as_ref().unwrap().evidence_id, 9);
        assert!(r.summary.contains("over 2 frames"));
    }

    #[test]
    fn a_line_of_sight_is_blocked_by_points_between_not_at_its_ends() {
        let wall: Vec<P3> = (0..100)
            .flat_map(|i| (0..30).map(move |j| [2.0, i as f64 * 0.02, j as f64 * 0.1]))
            .collect();
        let s = line_of_sight("t", [0.0, 1.0, 1.5], [4.0, 1.0, 1.5], &wall, 0.03, 0.1);
        assert!(!s.clear && (s.first_distance.unwrap() - 2.0).abs() < 0.01);
        // Over the wall's top (2.9 m high here), and ending on the wall itself.
        let s = line_of_sight("t", [0.0, 1.0, 3.3], [4.0, 1.0, 3.3], &wall, 0.03, 0.1);
        assert!(s.clear);
        let s = line_of_sight("t", [0.0, 1.0, 1.5], [2.0, 1.0, 1.5], &wall, 0.03, 0.1);
        assert!(s.clear, "{s:?}");
    }
}
