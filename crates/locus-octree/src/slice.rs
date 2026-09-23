//! Top-down orthographic slice of the scene: every visible point between two heights,
//! binned into square cells in the project frame. Cells are aligned to whole multiples of the
//! resolution, so the raster sits exactly in project coordinates: pixel (col, row) covers
//! x ∈ [x0 + col·r, x0 + (col+1)·r), y ∈ (y1 − (row+1)·r, y1 − row·r].

use crate::cleanup::for_points_near;
use crate::scene::Scene;
use crate::Result;
use std::collections::HashMap;

/// Largest raster accepted (cells), so a fine resolution over a big scene fails clearly
/// instead of exhausting memory.
pub const MAX_CELLS: u64 = 4096 * 4096;

#[derive(Debug, Clone, PartialEq)]
pub struct Slice {
    /// Project x, y of the raster's top-left corner (west, north edges), metres.
    pub origin: [f64; 2],
    /// Metres per pixel.
    pub resolution: f64,
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA, top row first; empty cells are transparent.
    pub rgba: Vec<u8>,
    /// Points that fell in the slice.
    pub points: u64,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SliceError {
    #[error("the resolution must be positive")]
    Resolution,
    #[error("the lower height must be below the upper height")]
    Heights,
    #[error("no visible points between those heights")]
    Empty,
    #[error("{0} × {1} pixels is too many; choose a coarser resolution")]
    TooBig(u64, u64),
}

/// What one point contributes to its cell.
#[derive(Clone, Copy)]
pub enum Shade {
    Rgb([u8; 3]),
    /// Raw intensity; scaled to grey by the brightest cell in the slice.
    Intensity(u16),
    /// Neither: cells are drawn dark grey.
    None,
}

#[derive(Default, Clone, Copy)]
struct Acc {
    rgb: [u64; 3],
    n_rgb: u64,
    intensity: u64,
    n_int: u64,
    n: u64,
}

/// Bin points (project x, y, z) into a raster. Pure; `slice` feeds it from the scene.
pub fn rasterise(
    points: impl IntoIterator<Item = ([f64; 3], Shade)>,
    z: (f64, f64),
    resolution: f64,
) -> std::result::Result<Slice, SliceError> {
    let mut b = Binner::new(z, resolution)?;
    for (p, shade) in points {
        b.add(p, shade);
    }
    b.finish()
}

/// Accumulates points cell by cell, so a slice of a huge scene holds cells, not points.
struct Binner {
    z: (f64, f64),
    resolution: f64,
    cells: HashMap<(i64, i64), Acc>,
}

impl Binner {
    fn new(z: (f64, f64), resolution: f64) -> std::result::Result<Binner, SliceError> {
        if !(resolution.is_finite() && resolution > 0.0) {
            return Err(SliceError::Resolution);
        }
        if z.0.partial_cmp(&z.1) != Some(std::cmp::Ordering::Less) {
            return Err(SliceError::Heights);
        }
        Ok(Binner {
            z,
            resolution,
            cells: HashMap::new(),
        })
    }

    fn add(&mut self, p: [f64; 3], shade: Shade) {
        let (z, resolution) = (self.z, self.resolution);
        if p[2] < z.0 || p[2] > z.1 {
            return;
        }
        let key = (
            (p[0] / resolution).floor() as i64,
            (p[1] / resolution).floor() as i64,
        );
        let a = self.cells.entry(key).or_default();
        a.n += 1;
        match shade {
            Shade::Rgb(c) => {
                for (sum, v) in a.rgb.iter_mut().zip(c) {
                    *sum += v as u64;
                }
                a.n_rgb += 1;
            }
            Shade::Intensity(i) => {
                a.intensity += i as u64;
                a.n_int += 1;
            }
            Shade::None => {}
        }
    }

    fn finish(self) -> std::result::Result<Slice, SliceError> {
        let (cells, resolution) = (self.cells, self.resolution);
        if cells.is_empty() {
            return Err(SliceError::Empty);
        }
        let (mut ix0, mut ix1, mut iy0, mut iy1) = (i64::MAX, i64::MIN, i64::MAX, i64::MIN);
        for &(ix, iy) in cells.keys() {
            (ix0, ix1, iy0, iy1) = (ix0.min(ix), ix1.max(ix), iy0.min(iy), iy1.max(iy));
        }
        let (w, h) = ((ix1 - ix0 + 1) as u64, (iy1 - iy0 + 1) as u64);
        if w * h > MAX_CELLS {
            return Err(SliceError::TooBig(w, h));
        }
        let max_int = cells
            .values()
            .filter(|a| a.n_int > 0)
            .map(|a| a.intensity / a.n_int)
            .max()
            .unwrap_or(0)
            .max(1);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        let mut points = 0;
        for (&(ix, iy), a) in &cells {
            points += a.n;
            let (col, row) = ((ix - ix0) as u64, (iy1 - iy) as u64);
            let px = &mut rgba[((row * w + col) * 4) as usize..][..4];
            let c = if a.n_rgb > 0 {
                a.rgb.map(|s| (s / a.n_rgb) as u8)
            } else if let Some(mean) = a.intensity.checked_div(a.n_int) {
                [(mean * 255 / max_int) as u8; 3]
            } else {
                [64; 3]
            };
            px.copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
        Ok(Slice {
            origin: [ix0 as f64 * resolution, (iy1 + 1) as f64 * resolution],
            resolution,
            width: w as u32,
            height: h as u32,
            rgba,
            points,
        })
    }
}

/// Slice every visible point of the scene (registered poses applied).
pub fn slice(
    scene: &Scene,
    z: (f64, f64),
    resolution: f64,
) -> Result<std::result::Result<Slice, SliceError>> {
    let mut b = match Binner::new(z, resolution) {
        Ok(b) => b,
        Err(e) => return Ok(Err(e)),
    };
    for c in scene.scans.values() {
        let meta = &c.tree.meta;
        let (lo, hi) = c.project_bounds();
        for_points_near(
            c,
            [lo[0], lo[1], z.0],
            [hi[0], hi[1], z.1],
            &mut |n, k, p| {
                let shade = if meta.has_color {
                    Shade::Rgb(n.rgb[k])
                } else if meta.has_intensity {
                    Shade::Intensity(n.intensity[k])
                } else {
                    Shade::None
                };
                b.add(p, shade);
            },
        )?;
    }
    Ok(b.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_sit_on_the_project_grid() {
        // Two points 1 m apart at 0.1 m resolution; the one above the slice is ignored.
        let s = rasterise(
            [
                ([2.05, 3.05, 1.0], Shade::Rgb([200, 0, 0])),
                ([3.05, 3.05, 1.0], Shade::Rgb([0, 0, 200])),
                ([2.55, 3.55, 5.0], Shade::Rgb([0, 255, 0])),
            ],
            (0.0, 2.0),
            0.1,
        )
        .unwrap();
        assert_eq!((s.width, s.height, s.points), (11, 1, 2));
        assert!((s.origin[0] - 2.0).abs() < 1e-12 && (s.origin[1] - 3.1).abs() < 1e-12);
        assert_eq!(&s.rgba[..4], &[200, 0, 0, 255]);
        assert_eq!(&s.rgba[40..44], &[0, 0, 200, 255]);
        assert_eq!(s.rgba[4 * 5 + 3], 0, "empty cells are transparent");
    }

    #[test]
    fn north_is_the_top_row() {
        let s = rasterise(
            [
                ([0.5, 0.5, 0.0], Shade::Intensity(100)),
                ([0.5, 1.5, 0.0], Shade::Intensity(200)),
            ],
            (-1.0, 1.0),
            1.0,
        )
        .unwrap();
        assert_eq!((s.width, s.height), (1, 2));
        assert_eq!(s.origin, [0.0, 2.0]);
        assert_eq!(&s.rgba[..4], &[255, 255, 255, 255]); // the northern, brighter point
        assert_eq!(s.rgba[4], 127);
    }

    #[test]
    fn bad_requests_are_refused() {
        let p = [([0.0, 0.0, 0.0], Shade::None)];
        assert_eq!(rasterise(p, (0.0, 1.0), 0.0), Err(SliceError::Resolution));
        assert_eq!(rasterise(p, (1.0, 1.0), 0.1), Err(SliceError::Heights));
        assert_eq!(rasterise(p, (2.0, 3.0), 0.1), Err(SliceError::Empty));
        let far = [
            ([0.0, 0.0, 0.0], Shade::None),
            ([1000.0, 1000.0, 0.0], Shade::None),
        ];
        assert!(matches!(
            rasterise(far, (-1.0, 1.0), 0.01),
            Err(SliceError::TooBig(..))
        ));
    }
}
