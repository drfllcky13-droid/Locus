//! The surface under a picked point, for snapping models to a point cloud: a plane fitted to
//! the pick's neighbours, the pick moved onto it, and how flat the surface really is.

use crate::measure::{fit_plane, MeasureError, P3};
use serde::Serialize;

/// Fewest neighbours a surface is fitted from.
pub const MIN_POINTS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Surface {
    /// The picked point moved along the normal onto the fitted plane (m, project frame).
    pub point: P3,
    /// Unit normal, turned toward `toward` (the camera), so it points out of the surface
    /// the examiner is looking at.
    pub normal: P3,
    /// RMS and largest distance of the neighbours from the plane (m): the fit's residual.
    pub rms: f64,
    pub max_abs: f64,
    /// Neighbours the plane was fitted from.
    pub points: usize,
}

/// Fit the surface around `pick` from `neighbours` (the cloud's points near it).
pub fn surface_at(pick: P3, neighbours: &[P3], toward: P3) -> Result<Surface, MeasureError> {
    if neighbours.len() < MIN_POINTS {
        return Err(MeasureError::NeedPoints(MIN_POINTS));
    }
    let plane = fit_plane(neighbours)?;
    let n = plane.normal;
    let d = (0..3)
        .map(|k| (pick[k] - plane.point[k]) * n[k])
        .sum::<f64>();
    let point = std::array::from_fn(|k| pick[k] - d * n[k]);
    let facing = (0..3).map(|k| (toward[k] - point[k]) * n[k]).sum::<f64>();
    Ok(Surface {
        point,
        normal: if facing < 0.0 { n.map(|v| -v) } else { n },
        rms: plane.rms,
        max_abs: plane.max_abs,
        points: neighbours.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic noise in [-a, a].
    fn noise(i: usize, a: f64) -> f64 {
        let x = ((i as f64 * 12.9898).sin() * 43758.5453).rem_euclid(1.0);
        (x * 2.0 - 1.0) * a
    }

    /// Points on the plane through `c` with unit normal `n`, over a 0.2 m disc, with noise
    /// of up to 1 mm along the normal.
    fn disc(c: P3, n: P3) -> Vec<P3> {
        let t = if n[2].abs() < 0.9 {
            [0.0, 0.0, 1.0]
        } else {
            [1.0, 0.0, 0.0]
        };
        let u = unit(cross(n, t));
        let v = cross(n, u);
        (0..400)
            .map(|i| {
                let (r, a) = (
                    0.2 * ((i % 20) as f64 + 0.5) / 20.0,
                    (i / 20) as f64 * 0.314,
                );
                let e = noise(i, 0.001);
                std::array::from_fn(|k| c[k] + r * (a.cos() * u[k] + a.sin() * v[k]) + e * n[k])
            })
            .collect()
    }

    fn cross(a: P3, b: P3) -> P3 {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }
    fn unit(a: P3) -> P3 {
        let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
        a.map(|x| x / l)
    }
    fn angle(a: P3, b: P3) -> f64 {
        (a[0] * b[0] + a[1] * b[1] + a[2] * b[2])
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees()
    }

    #[test]
    fn snaps_onto_a_floor_within_a_millimetre() {
        let floor = disc([512_000.0, 4_310_000.0, 181.25], [0.0, 0.0, 1.0]);
        // A pick 4 mm above the floor (a point of noise, or a speck of dust).
        let s = surface_at(
            [512_000.03, 4_310_000.02, 181.254],
            &floor,
            [512_003.0, 4_310_000.0, 183.0],
        )
        .unwrap();
        assert!((s.point[2] - 181.25).abs() < 1e-3, "{}", s.point[2]);
        assert!(angle(s.normal, [0.0, 0.0, 1.0]) < 0.5);
        // Noise is at most 1 mm; the fitted plane is not exactly the true one, so allow a little more.
        assert!(
            s.rms < 1e-3 && s.max_abs < 1.5e-3,
            "{} {}",
            s.rms,
            s.max_abs
        );
        assert_eq!(s.points, 400);
    }

    #[test]
    fn snaps_onto_a_tilted_plane_and_a_wall_facing_the_camera() {
        let n = unit([0.3, -0.2, 0.9]);
        let c = [10.0, 20.0, 1.0];
        let s = surface_at([10.01, 20.0, 1.0], &disc(c, n), [15.0, 15.0, 10.0]).unwrap();
        let off = (0..3).map(|k| (s.point[k] - c[k]) * n[k]).sum::<f64>();
        assert!(off.abs() < 1e-3, "{off}");
        assert!(angle(s.normal, n) < 0.5);
        // A wall seen from -y: its normal points back at the camera, not away.
        let wall = disc([0.0, 5.0, 1.5], [0.0, 1.0, 0.0]);
        let s = surface_at([0.0, 5.0, 1.5], &wall, [0.0, 0.0, 1.6]).unwrap();
        assert!(angle(s.normal, [0.0, -1.0, 0.0]) < 0.5);
    }

    #[test]
    fn needs_enough_points_on_a_surface() {
        let few = vec![[0.0; 3]; MIN_POINTS - 1];
        assert!(surface_at([0.0; 3], &few, [0.0, 0.0, 1.0]).is_err());
        let line: Vec<P3> = (0..20).map(|i| [i as f64, 0.0, 0.0]).collect();
        assert!(surface_at([0.0; 3], &line, [0.0, 0.0, 1.0]).is_err());
    }
}
