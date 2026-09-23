//! COLMAP's camera models (src/colmap/sensor/models.h): projecting a point in the camera frame
//! to pixels, and a pixel back to a ray (the distortion inverted by Newton's method); and
//! triangulating a point from rays in several images. Used to place ground control points and
//! scale-bar ends clicked in photos, and to check reconstructions against ground truth.

use crate::model::{Camera, Image, P3};
use nalgebra::{Matrix2, Matrix3, Vector2, Vector3};

/// The distortion of normalised coordinates (u, v) = (x/z, y/z): the (du, dv) added to them.
fn distortion(model: &str, p: &[f64], u: f64, v: f64) -> Result<(f64, f64), String> {
    let (u2, v2, uv) = (u * u, v * v, u * v);
    let r2 = u2 + v2;
    Ok(match model {
        "SIMPLE_PINHOLE" | "PINHOLE" => (0.0, 0.0),
        "SIMPLE_RADIAL" => (u * p[3] * r2, v * p[3] * r2),
        "RADIAL" => {
            let k = p[3] * r2 + p[4] * r2 * r2;
            (u * k, v * k)
        }
        "OPENCV" => {
            let (k1, k2, p1, p2) = (p[4], p[5], p[6], p[7]);
            let radial = k1 * r2 + k2 * r2 * r2;
            (
                u * radial + 2.0 * p1 * uv + p2 * (r2 + 2.0 * u2),
                v * radial + 2.0 * p2 * uv + p1 * (r2 + 2.0 * v2),
            )
        }
        "OPENCV_FISHEYE" | "THIN_PRISM_FISHEYE" => {
            let r = r2.sqrt();
            let (ud, vd) = if r > f64::EPSILON {
                let th = r.atan();
                let t2 = th * th;
                let (k1, k2, k3, k4) = if model == "OPENCV_FISHEYE" {
                    (p[4], p[5], p[6], p[7])
                } else {
                    (p[4], p[5], p[8], p[9])
                };
                let thd = th * (1.0 + t2 * (k1 + t2 * (k2 + t2 * (k3 + t2 * k4))));
                (u * thd / r, v * thd / r)
            } else {
                (u, v)
            };
            if model == "OPENCV_FISHEYE" {
                (ud - u, vd - v)
            } else {
                let (p1, p2, sx1, sy1) = (p[6], p[7], p[10], p[11]);
                let rd2 = ud * ud + vd * vd;
                (
                    ud + 2.0 * p1 * ud * vd + p2 * (rd2 + 2.0 * ud * ud) + sx1 * rd2 - u,
                    vd + 2.0 * p2 * ud * vd + p1 * (rd2 + 2.0 * vd * vd) + sy1 * rd2 - v,
                )
            }
        }
        m => return Err(format!("camera model {m} isn't supported")),
    })
}

/// Focal lengths and principal point.
fn intrinsics(c: &Camera) -> Result<(f64, f64, f64, f64), String> {
    let p = &c.params;
    let need = match c.model.as_str() {
        "SIMPLE_PINHOLE" => 3,
        "PINHOLE" => 4,
        "SIMPLE_RADIAL" => 4,
        "RADIAL" => 5,
        "OPENCV" | "OPENCV_FISHEYE" => 8,
        "THIN_PRISM_FISHEYE" => 12,
        m => return Err(format!("camera model {m} isn't supported")),
    };
    if p.len() < need {
        return Err(format!("camera {} has too few parameters", c.id));
    }
    Ok(match c.model.as_str() {
        "SIMPLE_PINHOLE" | "SIMPLE_RADIAL" | "RADIAL" => (p[0], p[0], p[1], p[2]),
        _ => (p[0], p[1], p[2], p[3]),
    })
}

/// A point in the camera frame to pixels (None behind the camera).
pub fn project(c: &Camera, x: P3) -> Result<Option<[f64; 2]>, String> {
    if x[2] <= 0.0 {
        return Ok(None);
    }
    let (fx, fy, cx, cy) = intrinsics(c)?;
    let (u, v) = (x[0] / x[2], x[1] / x[2]);
    let (du, dv) = distortion(&c.model, &c.params, u, v)?;
    Ok(Some([fx * (u + du) + cx, fy * (v + dv) + cy]))
}

/// A pixel to its normalised undistorted coordinates (u, v): the ray (u, v, 1) in the camera
/// frame. Newton's method on the distortion, with a numerical Jacobian.
pub fn unproject(c: &Camera, px: [f64; 2]) -> Result<[f64; 2], String> {
    let (fx, fy, cx, cy) = intrinsics(c)?;
    let target = Vector2::new((px[0] - cx) / fx, (px[1] - cy) / fy);
    let f = |w: Vector2<f64>| -> Result<Vector2<f64>, String> {
        let (du, dv) = distortion(&c.model, &c.params, w[0], w[1])?;
        Ok(Vector2::new(w[0] + du, w[1] + dv))
    };
    let mut w = target;
    for _ in 0..100 {
        let r = f(w)? - target;
        if r.norm() < 1e-12 {
            break;
        }
        let h = 1e-7;
        let (fu, fv) = (
            (f(w + Vector2::new(h, 0.0))? - f(w)?) / h,
            (f(w + Vector2::new(0.0, h))? - f(w)?) / h,
        );
        let j = Matrix2::from_columns(&[fu, fv]);
        let Some(inv) = j.try_inverse() else { break };
        w -= inv * r;
    }
    Ok([w[0], w[1]])
}

/// A ray in the world from a pixel of an image: the camera centre and a unit direction.
pub fn ray(c: &Camera, im: &Image, px: [f64; 2]) -> Result<(P3, P3), String> {
    let [u, v] = unproject(c, px)?;
    let r = im.rotation();
    // Direction in the world: Rᵀ (u, v, 1).
    let d = Vector3::new(
        r[0][0] * u + r[1][0] * v + r[2][0],
        r[0][1] * u + r[1][1] * v + r[2][1],
        r[0][2] * u + r[1][2] * v + r[2][2],
    )
    .normalize();
    Ok((im.centre(), d.into()))
}

/// The point closest to all the rays (least squares), and the largest angle between any two
/// of them (°): small angles triangulate poorly.
pub fn triangulate(rays: &[(P3, P3)]) -> Option<(P3, f64)> {
    if rays.len() < 2 {
        return None;
    }
    let (mut a, mut b) = (Matrix3::zeros(), Vector3::zeros());
    for (c, d) in rays {
        let d = Vector3::from(*d);
        let m = Matrix3::identity() - d * d.transpose();
        a += m;
        b += m * Vector3::from(*c);
    }
    let x = a.try_inverse()? * b;
    let mut angle = 0.0f64;
    for i in 0..rays.len() {
        for j in i + 1..rays.len() {
            let c = Vector3::from(rays[i].1).dot(&Vector3::from(rays[j].1));
            angle = angle.max(c.clamp(-1.0, 1.0).acos().to_degrees());
        }
    }
    Some((x.into(), angle))
}

/// A world point to pixels in an image.
pub fn project_world(c: &Camera, im: &Image, x: P3) -> Result<Option<[f64; 2]>, String> {
    let r = im.rotation();
    let xc: P3 = std::array::from_fn(|i| (0..3).map(|k| r[i][k] * x[k]).sum::<f64>() + im.t[i]);
    project(c, xc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(model: &str, params: &[f64]) -> Camera {
        Camera {
            id: 1,
            model: model.into(),
            width: 6048,
            height: 4032,
            params: params.to_vec(),
        }
    }

    #[test]
    fn every_model_round_trips() {
        let cams = [
            cam("PINHOLE", &[3000.0, 3010.0, 3024.0, 2016.0]),
            cam("SIMPLE_RADIAL", &[3000.0, 3024.0, 2016.0, -0.08]),
            cam("RADIAL", &[3000.0, 3024.0, 2016.0, -0.08, 0.02]),
            cam(
                "OPENCV",
                &[3000.0, 3010.0, 3024.0, 2016.0, -0.1, 0.03, 0.001, -0.0005],
            ),
            cam(
                "OPENCV_FISHEYE",
                &[3430.0, 3429.0, 3033.0, 2004.0, 0.2, 0.15, -0.03, 0.3],
            ),
            // ETH3D's pipes camera.
            cam(
                "THIN_PRISM_FISHEYE",
                &[
                    3430.27,
                    3429.23,
                    3032.95,
                    2003.59,
                    0.218983,
                    0.155576,
                    -0.000274629,
                    -0.000387159,
                    -0.0364669,
                    0.302935,
                    0.00133613,
                    0.00101227,
                ],
            ),
        ];
        for c in &cams {
            for px in [
                [3024.0, 2016.0],
                [100.0, 150.0],
                [5900.0, 3900.0],
                [1234.5, 3001.25],
            ] {
                let [u, v] = unproject(c, px).unwrap();
                let back = project(c, [u * 2.0, v * 2.0, 2.0]).unwrap().unwrap();
                assert!(
                    (back[0] - px[0]).abs() < 1e-6 && (back[1] - px[1]).abs() < 1e-6,
                    "{}: {px:?} → {back:?}",
                    c.model
                );
            }
        }
        assert!(project(&cams[0], [0.0, 0.0, -1.0]).unwrap().is_none());
        assert!(unproject(&cam("FOV", &[1.0; 5]), [0.0, 0.0]).is_err());
    }

    #[test]
    fn rays_meet_at_the_point() {
        let c = cam(
            "OPENCV",
            &[3000.0, 3010.0, 3024.0, 2016.0, -0.1, 0.03, 0.001, -0.0005],
        );
        let x = [0.4, -0.3, 5.0];
        // Two images: identity at the origin, and one 1 m to the right turned slightly.
        let (s, co) = (0.1f64.sin(), 0.1f64.cos());
        let ims = [
            Image {
                id: 1,
                camera: 1,
                name: "a".into(),
                q: [1.0, 0.0, 0.0, 0.0],
                t: [0.0; 3],
                observations: 0,
                keypoints: vec![],
            },
            Image {
                id: 2,
                camera: 1,
                name: "b".into(),
                q: [(0.05f64).cos(), 0.0, -(0.05f64).sin(), 0.0],
                t: [-co, 0.0, -s],
                observations: 0,
                keypoints: vec![],
            },
        ];
        let rays: Vec<_> = ims
            .iter()
            .map(|im| ray(&c, im, project_world(&c, im, x).unwrap().unwrap()).unwrap())
            .collect();
        let (p, angle) = triangulate(&rays).unwrap();
        assert!((0..3).all(|k| (p[k] - x[k]).abs() < 1e-6), "{p:?}");
        assert!(angle > 5.0 && angle < 20.0, "{angle}");
    }
}
