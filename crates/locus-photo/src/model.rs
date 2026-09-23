//! COLMAP's text model (`cameras.txt`, `images.txt`, `points3D.txt`), as written by
//! `colmap model_converter --output_type TXT`. See COLMAP's documentation, "Output format".

use std::collections::BTreeMap;

pub type P3 = [f64; 3];

#[derive(Debug, Clone, PartialEq)]
pub struct Camera {
    pub id: u32,
    /// COLMAP's camera model name (SIMPLE_RADIAL, OPENCV, …) and its parameters as written.
    pub model: String,
    pub width: u32,
    pub height: u32,
    pub params: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub id: u32,
    pub camera: u32,
    pub name: String,
    /// World-to-camera rotation (unit quaternion w, x, y, z) and translation, as COLMAP stores
    /// them: x_cam = R x_world + t.
    pub q: [f64; 4],
    pub t: P3,
    /// Observations with a 3D point.
    pub observations: usize,
}

impl Image {
    /// The rotation matrix R (world to camera), rows.
    pub fn rotation(&self) -> [[f64; 3]; 3] {
        let n = self.q.iter().map(|v| v * v).sum::<f64>().sqrt();
        let [w, x, y, z] = self.q.map(|v| v / n);
        [
            [
                1.0 - 2.0 * (y * y + z * z),
                2.0 * (x * y - w * z),
                2.0 * (x * z + w * y),
            ],
            [
                2.0 * (x * y + w * z),
                1.0 - 2.0 * (x * x + z * z),
                2.0 * (y * z - w * x),
            ],
            [
                2.0 * (x * z - w * y),
                2.0 * (y * z + w * x),
                1.0 - 2.0 * (x * x + y * y),
            ],
        ]
    }
    /// The camera's centre in the world, C = −Rᵀ t.
    pub fn centre(&self) -> P3 {
        let r = self.rotation();
        std::array::from_fn(|k| -(0..3).map(|i| r[i][k] * self.t[i]).sum::<f64>())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    pub id: u64,
    pub xyz: P3,
    pub rgb: [u8; 3],
    /// Mean reprojection error (px) and the number of images that see it.
    pub error: f64,
    pub track: usize,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Model {
    pub cameras: BTreeMap<u32, Camera>,
    pub images: BTreeMap<u32, Image>,
    pub points: Vec<Point>,
}

impl Model {
    /// Mean reprojection error over the points (px), weighted by track length as COLMAP does.
    pub fn mean_error(&self) -> f64 {
        let (s, n) = self.points.iter().fold((0.0, 0usize), |(s, n), p| {
            (s + p.error * p.track as f64, n + p.track)
        });
        if n == 0 {
            f64::NAN
        } else {
            s / n as f64
        }
    }
}

fn rows(text: &str) -> impl Iterator<Item = (usize, Vec<&str>)> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .map(|(n, l)| (n + 1, l.split_whitespace().collect()))
}

fn num<T: std::str::FromStr>(file: &str, line: usize, v: Option<&&str>) -> Result<T, String> {
    v.and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("{file} line {line}: expected a number"))
}

/// Read the three files' text.
pub fn read(cameras: &str, images: &str, points: &str) -> Result<Model, String> {
    let mut m = Model::default();
    for (n, f) in rows(cameras) {
        let c = Camera {
            id: num("cameras.txt", n, f.first())?,
            model: f
                .get(1)
                .ok_or(format!("cameras.txt line {n}: no model"))?
                .to_string(),
            width: num("cameras.txt", n, f.get(2))?,
            height: num("cameras.txt", n, f.get(3))?,
            params: f[4..]
                .iter()
                .map(|s| {
                    s.parse()
                        .map_err(|_| format!("cameras.txt line {n}: bad parameter"))
                })
                .collect::<Result<_, _>>()?,
        };
        m.cameras.insert(c.id, c);
    }
    // images.txt: a pose line, then a line of 2D points (x y point3D_id, −1 when none).
    let mut lines = images
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim_start().starts_with('#'));
    while let Some((n, l)) = lines.next() {
        if l.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 10 {
            return Err(format!("images.txt line {}: expected 10 fields", n + 1));
        }
        let g = |k: usize| num::<f64>("images.txt", n + 1, f.get(k));
        let obs = lines.next().map_or(0, |(_, l)| {
            l.split_whitespace()
                .collect::<Vec<_>>()
                .chunks(3)
                .filter(|c| c.len() == 3 && c[2] != "-1")
                .count()
        });
        let im = Image {
            id: num("images.txt", n + 1, f.first())?,
            q: [g(1)?, g(2)?, g(3)?, g(4)?],
            t: [g(5)?, g(6)?, g(7)?],
            camera: num("images.txt", n + 1, f.get(8))?,
            name: f[9..].join(" "),
            observations: obs,
        };
        if !m.cameras.contains_key(&im.camera) {
            return Err(format!(
                "images.txt line {}: no camera {}",
                n + 1,
                im.camera
            ));
        }
        m.images.insert(im.id, im);
    }
    for (n, f) in rows(points) {
        if f.len() < 8 {
            return Err(format!("points3D.txt line {n}: expected at least 8 fields"));
        }
        let g = |k: usize| num::<f64>("points3D.txt", n, f.get(k));
        m.points.push(Point {
            id: num("points3D.txt", n, f.first())?,
            xyz: [g(1)?, g(2)?, g(3)?],
            rgb: [
                num("points3D.txt", n, f.get(4))?,
                num("points3D.txt", n, f.get(5))?,
                num("points3D.txt", n, f.get(6))?,
            ],
            error: g(7)?,
            track: (f.len() - 8) / 2,
        });
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAMERAS: &str = "# Camera list\n1 SIMPLE_RADIAL 4000 3000 3200 2000 1500 -0.02\n";
    // Image 1 at the origin looking down +z; image 2 turned 90° about y (q = cos 45°, 0,
    // sin 45°, 0) with t = (0, 0, 2), so its centre is −Rᵀt = (2, 0, 0).
    const IMAGES: &str = "# Image list\n\
        1 1 0 0 0 0 0 0 1 IMG_0001.JPG\n\
        100 200 7 300 400 -1\n\
        2 0.7071067811865476 0 0.7071067811865476 0 0 0 2 1 IMG 0002.JPG\n\
        110 210 7\n";
    const POINTS: &str = "# 3D points\n7 1.0 2.0 3.0 200 100 50 0.8 1 0 2 0\n";

    #[test]
    fn a_text_model_is_read() {
        let m = read(CAMERAS, IMAGES, POINTS).unwrap();
        assert_eq!(m.cameras[&1].params, vec![3200.0, 2000.0, 1500.0, -0.02]);
        assert_eq!(m.images.len(), 2);
        assert_eq!(m.images[&2].name, "IMG 0002.JPG");
        assert_eq!(m.images[&1].observations, 1);
        assert_eq!(m.images[&1].centre(), [0.0, 0.0, 0.0]);
        let c = m.images[&2].centre();
        assert!(
            (c[0] - 2.0).abs() < 1e-12 && c[1].abs() < 1e-12 && c[2].abs() < 1e-12,
            "{c:?}"
        );
        assert_eq!(m.points[0].track, 2);
        assert!((m.mean_error() - 0.8).abs() < 1e-12);
        assert!(read(CAMERAS, "1 1 0 0 0 0 0 0 9 X.JPG\n\n", "").is_err());
    }
}
