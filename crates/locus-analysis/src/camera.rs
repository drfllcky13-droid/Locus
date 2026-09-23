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
];

pub const LIMITATIONS: &[&str] = &[
    "The solve is only as good as the pairs: few pairs, pairs bunched in one part of the image, or pairs near one plane leave the focal length and distortion poorly determined; the stated uncertainties show this.",
    "Height by reverse projection measures to the top of what was clicked: hair, headwear and footwear, posture (a stride, a slouch, a head tilt) and the frame's timing within the gait all change it by centimetres. The result is the height of the image feature, not the subject's stature.",
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
}

impl LensModel {
    /// Which of the lens parameters [f, cx, cy, k1, k2, k3, p1, p2] are free.
    fn free(self) -> [bool; 8] {
        match self {
            LensModel::Pinhole => [true, false, false, false, false, false, false, false],
            LensModel::Radial1 => [true, false, false, true, false, false, false, false],
            LensModel::Radial2 => [true, false, false, true, true, false, false, false],
            LensModel::Full => [true; 8],
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

    /// The largest undistorted radius the lens model is valid for: where the radial
    /// distortion stops increasing with radius (searched to r = 10). Beyond it the
    /// polynomial folds back, and a point outside the field of view would land in the image.
    pub fn max_radius(&self) -> f64 {
        let [k1, k2, k3, ..] = self.distortion;
        let slope = |r2: f64| 1.0 + 3.0 * k1 * r2 + 5.0 * k2 * r2 * r2 + 7.0 * k3 * r2 * r2 * r2;
        (1..=20_000)
            .map(|i| i as f64 * 0.005)
            .find(|r2| slope(*r2) <= 0.0)
            .map_or(f64::INFINITY, f64::sqrt)
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
        if c[2] <= 1e-9 || (c[0] / c[2]).hypot(c[1] / c[2]) >= self.max_radius() {
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Solve {
    pub model: LensModel,
    pub camera: Camera,
    /// 1σ of the position (m), of heading, pitch and roll (degrees), of the focal length and
    /// principal point (px), and of k1, k2, k3, p1, p2 (0 where fixed).
    pub position_sigma: P3,
    pub angles_sigma: P3,
    pub f_sigma: f64,
    pub principal_sigma: P2,
    pub distortion_sigma: [f64; 5],
    /// The free parameters' covariance, in the order of `free` (row-major), for Monte Carlo.
    pub free: Vec<usize>,
    pub covariance: Vec<f64>,
    /// Each pair's reprojection residual (px, x and y) and its size in σ.
    pub residuals: Vec<P2>,
    pub residual_sigmas: Vec<f64>,
    pub rms_px: f64,
    pub chi2: f64,
    pub dof: usize,
    /// √(χ²/dof) when above 1: the factor the covariance was inflated by.
    pub birge: f64,
    pub warnings: Vec<String>,
}

impl Solve {
    /// The camera with its free parameters drawn from their covariance (`z` standard normals,
    /// one per free parameter).
    fn draw(&self, z: &[f64], chol: &DMatrix<f64>) -> Camera {
        let mut p = self.camera.params();
        let dz = chol * DVector::from_column_slice(z);
        for (k, &i) in self.free.iter().enumerate() {
            p[i] += dz[k];
        }
        self.camera.with(&p)
    }

    fn cholesky(&self) -> DMatrix<f64> {
        let n = self.free.len();
        let c = DMatrix::from_row_slice(n, n, &self.covariance);
        match c.clone().cholesky() {
            Some(l) => l.l(),
            None => {
                // Not positive definite (numerically): clamp its eigenvalues at zero.
                let e = c.symmetric_eigen();
                let d = DMatrix::from_diagonal(&e.eigenvalues.map(|v| v.max(0.0).sqrt()));
                e.eigenvectors * d
            }
        }
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
    // The right singular vector of the smallest singular value (via AᵀA's eigenvectors).
    let ata = a.transpose() * &a;
    let e = ata.symmetric_eigen();
    let k = e
        .eigenvalues
        .iter()
        .enumerate()
        .min_by(|x, y| x.1.total_cmp(y.1))
        .map(|(i, _)| i)
        .unwrap();
    let h = e.eigenvectors.column(k);
    let pn = DMatrix::from_row_slice(3, 4, h.as_slice());
    // Undo the normalisations: P = T_px⁻¹ Pn T_w.
    let tpx_inv = DMatrix::from_row_slice(
        3,
        3,
        &[1.0 / sp, 0.0, cp[0], 0.0, 1.0 / sp, cp[1], 0.0, 0.0, 1.0],
    );
    let tw = DMatrix::from_row_slice(
        4,
        4,
        &[
            sw,
            0.0,
            0.0,
            -cw[0] * sw,
            0.0,
            sw,
            0.0,
            -cw[1] * sw,
            0.0,
            0.0,
            sw,
            -cw[2] * sw,
            0.0,
            0.0,
            0.0,
            1.0,
        ],
    );
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
    let minv = m.try_inverse().ok_or(CameraError::Degenerate(
        "the pairs don't determine a camera",
    ))?;
    let c = -(minv * p4);
    // RQ by Cholesky: M Mᵀ = K Kᵀ with K upper triangular (through the exchange matrix J).
    let j = Matrix3::new(0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0);
    let mm = j * m * m.transpose() * j;
    let l = mm
        .cholesky()
        .ok_or(CameraError::Degenerate(
            "the pairs don't determine a camera",
        ))?
        .l();
    let kmat = j * l * j;
    let r = kmat.try_inverse().ok_or(CameraError::Degenerate(
        "the pairs don't determine a camera",
    ))? * m;
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
        cx: kmat[(0, 2)],
        cy: kmat[(1, 2)],
        distortion: [0.0; 5],
    })
}

/// How flat the scan points are: the smallest over the largest eigenvalue of their scatter.
fn flatness(pairs: &[CameraPair]) -> f64 {
    let n = pairs.len() as f64;
    let c = [0, 1, 2].map(|k| pairs.iter().map(|p| p.world[k]).sum::<f64>() / n);
    let mut s = Matrix3::zeros();
    for p in pairs {
        let d = Vector3::new(p.world[0] - c[0], p.world[1] - c[1], p.world[2] - c[2]);
        s += d * d.transpose();
    }
    let e = s.symmetric_eigenvalues();
    let (lo, hi) = (e.min(), e.max());
    if hi > 0.0 {
        lo / hi
    } else {
        0.0
    }
}

/// Solve a camera from image-to-scan pairs. `pick_sigma_px` is each pixel's 1σ and
/// `point_sigma` each scan point's (m, projected into the image at its depth).
pub fn solve(
    pairs: &[CameraPair],
    size: [u32; 2],
    model: LensModel,
    pick_sigma_px: f64,
    point_sigma: f64,
) -> Result<Solve, CameraError> {
    let lens = model.free();
    let n_free = 6 + lens.iter().filter(|f| **f).count();
    if pairs.len() < 6 || 2 * pairs.len() <= n_free {
        return Err(CameraError::TooFewPairs(pairs.len()));
    }
    if flatness(pairs) < 1e-4 {
        return Err(CameraError::OnePlane);
    }
    // The start: the DLT on the pairs nearest the image centre (at least 8, or half of them),
    // where lens distortion is least, unless those lie on one plane.
    let centre = [size[0] as f64 / 2.0, size[1] as f64 / 2.0];
    let mut near: Vec<CameraPair> = pairs.to_vec();
    near.sort_by(|a, b| {
        let r = |p: &CameraPair| (p.px[0] - centre[0]).hypot(p.px[1] - centre[1]);
        r(a).total_cmp(&r(b))
    });
    near.truncate((pairs.len() / 2).max(8).min(pairs.len()));
    let start: &[CameraPair] = if flatness(&near) > 1e-3 { &near } else { pairs };
    let mut cam = dlt(start, size)?;
    // The principal point starts at the image centre (the DLT's is poor with distortion).
    cam.cx = centre[0];
    cam.cy = centre[1];
    let free_of = |m: LensModel| -> Vec<usize> {
        let lens = m.free();
        (0..14).filter(|&i| i < 6 || lens[i - 6]).collect()
    };
    let free = free_of(model);
    let sigma_of = |c: &Camera, w: P3| {
        let z = c.to_camera(w)[2].max(1e-6);
        (pick_sigma_px.powi(2) + (c.f * point_sigma / z).powi(2)).sqrt()
    };
    let residuals = |c: &Camera, sig: &[f64]| -> DVector<f64> {
        let mut r = DVector::zeros(2 * pairs.len());
        for (i, p) in pairs.iter().enumerate() {
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
    };
    let steps = [
        1e-7, 1e-7, 1e-7, 1e-6, 1e-6, 1e-6, 1e-3, 1e-3, 1e-3, 1e-7, 1e-7, 1e-7, 1e-8, 1e-8,
    ];
    let jacobian = |c: &Camera, sig: &[f64], r0: &DVector<f64>, free: &[usize]| -> DMatrix<f64> {
        let base = c.params();
        let mut j = DMatrix::zeros(r0.len(), free.len());
        for (col, &i) in free.iter().enumerate() {
            let mut p = base;
            p[i] += steps[i];
            let r1 = residuals(&c.with(&p), sig);
            j.set_column(col, &((r1 - r0) / steps[i]));
        }
        j
    };
    // In stages, each started from the last: the pose and focal length, then k1, k2 and the
    // rest as the model has them. A strong wide-angle distortion otherwise leaves the DLT's
    // start (which has none) in the wrong basin. At each stage, two passes: the weights (each
    // scan point's σ at its depth) are set from the estimate.
    let stages: Vec<LensModel> = [
        LensModel::Pinhole,
        LensModel::Radial1,
        LensModel::Radial2,
        LensModel::Full,
    ]
    .into_iter()
    .filter(|m| m.free().iter().zip(lens).all(|(a, b)| !a || b))
    .collect();
    for stage in stages.iter().flat_map(|m| [free_of(*m), free_of(*m)]) {
        let sig: Vec<f64> = pairs.iter().map(|p| sigma_of(&cam, p.world)).collect();
        let mut r = residuals(&cam, &sig);
        let mut cost = r.norm_squared();
        let mut lambda = 1e-3;
        for _ in 0..200 {
            let j = jacobian(&cam, &sig, &r, &stage);
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
                for (k, &i) in stage.iter().enumerate() {
                    p[i] += dx[k];
                }
                let c2 = cam.with(&p);
                let r2 = residuals(&c2, &sig);
                let cost2 = r2.norm_squared();
                if cost2 < cost {
                    let small = dx.norm() < 1e-12 || (cost - cost2) < 1e-12 * cost;
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
    let sig: Vec<f64> = pairs.iter().map(|p| sigma_of(&cam, p.world)).collect();
    let r = residuals(&cam, &sig);
    let j = jacobian(&cam, &sig, &r, &free);
    let chi2 = r.norm_squared();
    let dof = 2 * pairs.len() - free.len();
    let birge = (chi2 / dof as f64).sqrt().max(1.0);
    let cov = (j.transpose() * &j)
        .try_inverse()
        .ok_or(CameraError::Degenerate(
        "the pairs don't pin the camera down (spread them over more of the image and the scene)",
    ))? * (birge * birge);
    let var = |i: usize| {
        free.iter()
            .position(|&k| k == i)
            .map_or(0.0, |k| cov[(k, k)].max(0.0).sqrt())
    };
    // Heading, pitch and roll: first-order through the rotation increment.
    let angles_sigma = {
        let base = cam.params();
        let a0 = cam.angles();
        let mut ja = DMatrix::zeros(3, free.len());
        for (col, &i) in free.iter().enumerate().filter(|(_, &i)| i < 3) {
            let mut p = base;
            p[i] += 1e-7;
            let a1 = cam.with(&p).angles();
            for k in 0..3 {
                let mut d = a1[k] - a0[k];
                if d > 180.0 {
                    d -= 360.0;
                } else if d < -180.0 {
                    d += 360.0;
                }
                ja[(k, col)] = d / 1e-7;
            }
        }
        let ca = &ja * &cov * ja.transpose();
        [0, 1, 2].map(|k| ca[(k, k)].max(0.0).sqrt())
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
    let mut warnings = vec![];
    if birge > 2.0 {
        warnings.push(format!(
            "The pairs fit worse than their stated uncertainty (χ² = {chi2:.0} on {dof} degrees of freedom): a pair may be wrong, or the lens model too simple. The uncertainties were inflated by {birge:.1}."
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
    if var(6) > 0.05 * cam.f {
        warnings.push(format!(
            "The focal length is poorly determined ({:.0} ± {:.0} px): add pairs at different depths and toward the image's edges.",
            cam.f,
            var(6)
        ));
    }
    if pairs.len() < free.len() {
        warnings.push(format!(
            "Only {} pairs for the {} unknowns of this lens model: the solve has little redundancy to show a wrong pair, and the lens parameters can absorb errors. Add pairs or choose a simpler lens model.",
            pairs.len(),
            free.len()
        ));
    }
    Ok(Solve {
        model,
        position_sigma: [var(3), var(4), var(5)],
        angles_sigma,
        f_sigma: var(6),
        principal_sigma: [var(7), var(8)],
        distortion_sigma: [var(9), var(10), var(11), var(12), var(13)],
        free: free.clone(),
        covariance: cov.transpose().as_slice().to_vec(),
        residuals: res,
        residual_sigmas: res_sig,
        rms_px: (sq / pairs.len() as f64).sqrt(),
        chi2,
        dof,
        birge,
        warnings,
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

/// A subject's height with its uncertainty: Monte Carlo over the camera's covariance and
/// each image point's 1σ (`pick_sigma_px`; a head point from a matched model is drawn the
/// same way).
pub fn height(
    solve: &Solve,
    input: &HeightInput,
    floor_z: f64,
    pick_sigma_px: f64,
    draws: usize,
    seed: u64,
) -> Result<Height, CameraError> {
    let Some((h, feet, head, miss)) = reverse(&solve.camera, floor_z, input.feet_px, input.head_px)
    else {
        return Err(CameraError::Height(
            "the feet point's ray doesn't reach the floor in front of the camera; check the feet and head points",
        ));
    };
    let chol = solve.cholesky();
    let mut rng = Rng(seed.max(1).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut hs = Vec::with_capacity(draws);
    let mut failed = 0;
    let mut z = vec![0.0; solve.free.len()];
    for _ in 0..draws {
        z.iter_mut().for_each(|v| *v = rng.gauss());
        let cam = solve.draw(&z, &chol);
        let fp = [
            input.feet_px[0] + pick_sigma_px * rng.gauss(),
            input.feet_px[1] + pick_sigma_px * rng.gauss(),
        ];
        let hp = [
            input.head_px[0] + pick_sigma_px * rng.gauss(),
            input.head_px[1] + pick_sigma_px * rng.gauss(),
        ];
        match reverse(&cam, floor_z, fp, hp) {
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
    /// Monte Carlo draws for heights, and the seed.
    pub draws: usize,
    pub seed: u64,
}

impl Default for Parameters {
    fn default() -> Self {
        Parameters {
            model: LensModel::Radial2,
            pick_sigma_px: 1.0,
            point_sigma: 0.002,
            floor_z: 0.0,
            draws: 2000,
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
    pub summary: String,
    pub assumptions: Vec<String>,
    pub limitations: Vec<String>,
}

pub fn run(
    photo: Option<PhotoRef>,
    pairs: Vec<PairInput>,
    size: [u32; 2],
    parameters: Parameters,
    subjects: &[HeightInput],
) -> Result<Run, CameraError> {
    let p = &parameters;
    let cp: Vec<CameraPair> = pairs
        .iter()
        .map(|q| CameraPair {
            px: q.px,
            world: q.world,
        })
        .collect();
    let solve = solve(&cp, size, p.model, p.pick_sigma_px, p.point_sigma)?;
    let heights = subjects
        .iter()
        .enumerate()
        .map(|(k, s)| {
            height(
                &solve,
                s,
                p.floor_z,
                p.pick_sigma_px,
                p.draws,
                p.seed + k as u64,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
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
        summary += &format!(
            "; {} {:.3} ± {:.3} m",
            h.input.label, h.height.value, h.height.sigma
        );
    }
    Ok(Run {
        method: METHOD.into(),
        photo,
        pairs,
        parameters,
        solve,
        heights,
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
            )
            .unwrap();
            for k in 0..3 {
                z2.push(((s.camera.position[k] - cam.position[k]) / s.position_sigma[k]).powi(2));
            }
        }
        let mean = z2.iter().sum::<f64>() / z2.len() as f64;
        assert!((0.6..1.6).contains(&mean), "mean z² {mean}");
    }

    #[test]
    fn pairs_on_one_plane_or_too_few_are_refused() {
        let cam = cctv();
        let floor: Vec<CameraPair> = pairs(&cam, 60, 0.0, 1)
            .into_iter()
            .filter(|p| p.world[2] == 0.0)
            .collect();
        assert_eq!(
            solve(&floor, cam.size, LensModel::Pinhole, 0.5, 0.0).unwrap_err(),
            CameraError::OnePlane
        );
        let few = pairs(&cam, 5, 0.0, 1);
        assert!(matches!(
            solve(&few, cam.size, LensModel::Pinhole, 0.5, 0.0),
            Err(CameraError::TooFewPairs(5))
        ));
    }

    #[test]
    fn a_standing_subject_measures_exactly_and_the_interval_covers() {
        let cam = cctv();
        let s = solve(
            &pairs(&cam, 30, 0.0, 1),
            cam.size,
            LensModel::Full,
            0.5,
            0.0,
        )
        .unwrap();
        let feet = [3.2, 3.4, 0.0];
        let input = HeightInput {
            label: "A".into(),
            feet_px: cam.project(feet).unwrap(),
            head_px: cam.project([3.2, 3.4, 1.63]).unwrap(),
            matched_model: None,
        };
        let h = height(&s, &input, 0.0, 0.5, 500, 1).unwrap();
        assert!((h.height.value - 1.63).abs() < 1e-6, "{:?}", h.height);
        assert!(h.miss < 1e-6 && norm(sub(h.feet, feet)) < 1e-6);
        assert!(h.height.sigma > 0.0 && h.interval95[0] < 1.63 && h.interval95[1] > 1.63);
        // A feet point above the camera's horizon is refused.
        let bad = HeightInput {
            feet_px: cam.project([3.2, 3.4, 5.0]).unwrap(),
            ..input
        };
        assert!(height(&s, &bad, 0.0, 0.5, 10, 1).is_err());
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
