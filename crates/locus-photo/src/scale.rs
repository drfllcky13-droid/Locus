//! Scaling and georeferencing a reconstruction: a similarity transform (scale, rotation,
//! translation) from ground control points, or a scale alone from known distances.
//!
//! GCPs: Umeyama's closed form (1991), with each point's residual and, for check points held
//! out of the fit, their errors. Distances: the scale is the weighted mean of true / model
//! length, weights from each distance's stated tolerance; its 1σ is the larger of what the
//! tolerances give and the spread between the distances.

use crate::model::P3;
use nalgebra::{Matrix3, Vector3};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Similarity {
    pub scale: f64,
    /// Rotation, rows.
    pub rotation: [[f64; 3]; 3],
    pub translation: P3,
}

impl Similarity {
    pub fn apply(&self, p: P3) -> P3 {
        std::array::from_fn(|i| {
            self.scale * (0..3).map(|k| self.rotation[i][k] * p[k]).sum::<f64>()
                + self.translation[i]
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GcpFit {
    pub transform: Similarity,
    /// |T(model) − world| for each control point (m), and their RMS.
    pub residuals: Vec<f64>,
    pub rms: f64,
    /// Errors at the check points, held out of the fit (m).
    pub check_errors: Vec<f64>,
}

fn dist(a: P3, b: P3) -> f64 {
    (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f64>().sqrt()
}

/// The similarity taking `model` points onto `world` (at least 3, not collinear), and the
/// errors it leaves at `checks` (model, world) held out of the fit.
pub fn fit_gcps(model: &[P3], world: &[P3], checks: &[(P3, P3)]) -> Result<GcpFit, String> {
    let n = model.len();
    if n != world.len() || n < 3 {
        return Err("give at least 3 control points, each in the model and in the world".into());
    }
    let v = |p: &P3| Vector3::from(*p);
    let (mx, my) = (
        model.iter().map(v).sum::<Vector3<f64>>() / n as f64,
        world.iter().map(v).sum::<Vector3<f64>>() / n as f64,
    );
    let mut cov = Matrix3::zeros();
    let mut var = 0.0;
    for (a, b) in model.iter().zip(world) {
        let (da, db) = (v(a) - mx, v(b) - my);
        cov += db * da.transpose();
        var += da.norm_squared();
    }
    cov /= n as f64;
    var /= n as f64;
    let svd = cov.svd(true, true);
    let (u, vt) = (svd.u.unwrap(), svd.v_t.unwrap());
    let mut sv = svd.singular_values;
    // Collinear points leave the rotation about their line free.
    let mut sorted = sv.as_slice().to_vec();
    sorted.sort_by(|a, b| b.total_cmp(a));
    if sorted[1] <= sorted[0] * 1e-9 {
        return Err("the control points are collinear".into());
    }
    let mut s = Matrix3::identity();
    if (u.determinant() * vt.determinant()) < 0.0 {
        s[(2, 2)] = -1.0;
        sv[2] = -sv[2];
    }
    let r = u * s * vt;
    let scale = (sv[0] + sv[1] + sv[2]) / var;
    let t = my - scale * r * mx;
    let transform = Similarity {
        scale,
        rotation: std::array::from_fn(|i| std::array::from_fn(|k| r[(i, k)])),
        translation: t.into(),
    };
    let residuals: Vec<f64> = model
        .iter()
        .zip(world)
        .map(|(a, b)| dist(transform.apply(*a), *b))
        .collect();
    let rms = (residuals.iter().map(|x| x * x).sum::<f64>() / n as f64).sqrt();
    let check_errors = checks
        .iter()
        .map(|(a, b)| dist(transform.apply(*a), *b))
        .collect();
    Ok(GcpFit {
        transform,
        residuals,
        rms,
        check_errors,
    })
}

/// A known distance: its two ends in the model, and its true length with a 1σ (m).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KnownDistance {
    pub a: P3,
    pub b: P3,
    pub length: f64,
    pub sigma: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScaleFit {
    pub scale: f64,
    pub sigma: f64,
    /// Each distance's own scale (true / model) and how far it is from the fit (%).
    pub ratios: Vec<f64>,
    pub deviations_pct: Vec<f64>,
}

/// The scale from known distances (at least 1; 2 or more to check each other).
pub fn scale_from_distances(d: &[KnownDistance]) -> Result<ScaleFit, String> {
    if d.is_empty() {
        return Err("give at least one known distance".into());
    }
    let mut ratios = vec![];
    let mut weights = vec![];
    for k in d {
        let m = dist(k.a, k.b);
        if m <= 0.0 || k.length.is_nan() || k.length <= 0.0 || k.sigma.is_nan() || k.sigma <= 0.0 {
            return Err(
                "each known distance needs two distinct ends, a length and a tolerance".into(),
            );
        }
        ratios.push(k.length / m);
        // σ of the ratio from the tape: σ_L / m.
        weights.push((m / k.sigma).powi(2));
    }
    let w: f64 = weights.iter().sum();
    let scale = ratios.iter().zip(&weights).map(|(r, w)| r * w).sum::<f64>() / w;
    let stated = 1.0 / w.sqrt();
    let spread = if ratios.len() > 1 {
        let n = ratios.len() as f64;
        (ratios
            .iter()
            .zip(&weights)
            .map(|(r, wi)| wi * (r - scale).powi(2))
            .sum::<f64>()
            / w
            * n
            / (n - 1.0)
            / n)
            .sqrt()
    } else {
        0.0
    };
    Ok(ScaleFit {
        scale,
        sigma: stated.max(spread),
        deviations_pct: ratios.iter().map(|r| (r / scale - 1.0) * 100.0).collect(),
        ratios,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn truth() -> Similarity {
        let (c, s) = (0.6f64.cos(), 0.6f64.sin());
        Similarity {
            scale: 3.7,
            rotation: [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]],
            translation: [100.0, -20.0, 5.0],
        }
    }

    #[test]
    fn control_points_give_the_similarity() {
        let t = truth();
        let model = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.2],
            [0.0, 2.0, 0.0],
            [0.5, 0.5, 1.0],
        ];
        let world: Vec<P3> = model.iter().map(|p| t.apply(*p)).collect();
        let check = [([0.3, 0.8, 0.4], t.apply([0.3, 0.8, 0.4]))];
        let f = fit_gcps(&model, &world, &check).unwrap();
        assert!((f.transform.scale - 3.7).abs() < 1e-12);
        assert!(f.rms < 1e-9 && f.check_errors[0] < 1e-9);
        assert!(fit_gcps(&model[..2], &world[..2], &[]).is_err());
        let line = [[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
        let wl: Vec<P3> = line.iter().map(|p| t.apply(*p)).collect();
        assert!(fit_gcps(&line, &wl, &[]).is_err());
    }

    #[test]
    fn known_distances_give_the_scale() {
        // Model lengths 2 and 4, true 5.0 ± 0.01 and 10.1 ± 0.01: ratios 2.5 and 2.525,
        // weights (2/0.01)² and (4/0.01)² → 2.5 × 0.2 + 2.525 × 0.8 = 2.52.
        let d = [
            KnownDistance {
                a: [0.0; 3],
                b: [2.0, 0.0, 0.0],
                length: 5.0,
                sigma: 0.01,
            },
            KnownDistance {
                a: [0.0; 3],
                b: [0.0, 4.0, 0.0],
                length: 10.1,
                sigma: 0.01,
            },
        ];
        let f = scale_from_distances(&d).unwrap();
        assert!((f.scale - 2.52).abs() < 1e-12, "{}", f.scale);
        // The two disagree by 1 %, far more than their tolerances: the spread sets σ.
        assert!(f.sigma > 1.0 / ((2.0f64 / 0.01).powi(2) + (4.0f64 / 0.01).powi(2)).sqrt());
        let one = scale_from_distances(&d[..1]).unwrap();
        assert!((one.scale - 2.5).abs() < 1e-12 && (one.sigma - 0.005).abs() < 1e-12);
    }
}
