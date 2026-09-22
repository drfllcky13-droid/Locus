//! E57 (ASTM E2807): multiple scans with poses, and embedded images.
//!
//! Points are read in scan-local coordinates; each scan's pose is reported separately
//! and never baked into the points. Spherical scans are converted to Cartesian.
//!
//! Records are decoded here from the raw reader rather than the `e57` crate's simple
//! reader, which fails on some short final data packets (see DECISIONS.md, 2026-09-22).

use crate::stats::{rescale, Point, ScanStats, Visitor};
use crate::{parse_err, Progress, ProgressFn, Result, Stage};
use e57::{
    E57Reader, ImageFormat, PointCloud, Projection, RecordDataType, RecordName, RecordValue,
    Transform,
};
use locus_core::{Contents, ImageInfo, ImageKind, LinearUnit, IDENTITY};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

const FMT: &str = "E57";

/// Row-major 4x4 from an E57 pose (unit quaternion + translation, in meters).
pub(crate) fn pose_matrix(t: Option<&Transform>) -> [f64; 16] {
    let Some(t) = t else { return IDENTITY };
    let q = &t.rotation;
    let n = (q.w * q.w + q.x * q.x + q.y * q.y + q.z * q.z).sqrt();
    let (w, x, y, z) = (q.w / n, q.x / n, q.y / n, q.z / n);
    let tr = &t.translation;
    [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y - w * z),
        2.0 * (x * z + w * y),
        tr.x,
        2.0 * (x * y + w * z),
        1.0 - 2.0 * (x * x + z * z),
        2.0 * (y * z - w * x),
        tr.y,
        2.0 * (x * z - w * y),
        2.0 * (y * z + w * x),
        1.0 - 2.0 * (x * x + y * y),
        tr.z,
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

/// Where each coordinate lives in a scan's raw records, and how to scale it.
struct Layout {
    cartesian: Option<[(usize, RecordDataType); 3]>,
    spherical: Option<[(usize, RecordDataType); 3]>,
    cartesian_state: Option<usize>,
    spherical_state: Option<usize>,
    /// Record position and value range for red, green, blue.
    color: Option<[(usize, RecordDataType, f64, f64); 3]>,
    intensity: Option<(usize, RecordDataType, f64, f64)>,
}

/// Value range a channel spans: from the record's declared type when it is an integer,
/// otherwise 0 to 1.
fn range(t: &RecordDataType) -> (f64, f64) {
    match t {
        RecordDataType::Integer { min, max } => (*min as f64, *max as f64),
        RecordDataType::ScaledInteger {
            min,
            max,
            scale,
            offset,
        } => (*min as f64 * scale + offset, *max as f64 * scale + offset),
        RecordDataType::Single {
            min: Some(lo),
            max: Some(hi),
        } => (*lo as f64, *hi as f64),
        RecordDataType::Double {
            min: Some(lo),
            max: Some(hi),
        } => (*lo, *hi),
        _ => (0.0, 1.0),
    }
}

impl Layout {
    fn new(pc: &PointCloud) -> Result<Self> {
        let find = |name: RecordName| {
            pc.prototype
                .iter()
                .position(|r| r.name == name)
                .map(|i| (i, pc.prototype[i].data_type.clone()))
        };
        let triple = |a, b, c| match (find(a), find(b), find(c)) {
            (Some(a), Some(b), Some(c)) => Some([a, b, c]),
            _ => None,
        };
        let layout = Layout {
            cartesian: triple(
                RecordName::CartesianX,
                RecordName::CartesianY,
                RecordName::CartesianZ,
            ),
            spherical: triple(
                RecordName::SphericalRange,
                RecordName::SphericalAzimuth,
                RecordName::SphericalElevation,
            ),
            cartesian_state: find(RecordName::CartesianInvalidState).map(|f| f.0),
            spherical_state: find(RecordName::SphericalInvalidState).map(|f| f.0),
            color: match (
                find(RecordName::ColorRed),
                find(RecordName::ColorGreen),
                find(RecordName::ColorBlue),
            ) {
                (Some(r), Some(g), Some(b)) => Some([r, g, b].map(|(i, t)| {
                    let (lo, hi) = range(&t);
                    (i, t, lo, hi)
                })),
                _ => None,
            },
            intensity: find(RecordName::Intensity).map(|(i, t)| {
                let (lo, hi) = range(&t);
                (i, t, lo, hi)
            }),
        };
        if layout.cartesian.is_none() && layout.spherical.is_none() {
            return Err(parse_err(
                FMT,
                "scan has neither Cartesian nor spherical coordinates",
            ));
        }
        Ok(layout)
    }

    /// Scan-local position in meters, or None when the record has no valid position
    /// (invalid state 1 = direction only, 2 = no return).
    fn position(&self, raw: &[RecordValue]) -> Option<[f64; 3]> {
        let valid = |state: Option<usize>| state.is_none_or(|i| value(&raw[i], None) == 0.0);
        let get =
            |f: &[(usize, RecordDataType); 3]| f.clone().map(|(i, t)| value(&raw[i], Some(&t)));
        if let Some(f) = &self.cartesian {
            return valid(self.cartesian_state).then(|| get(f));
        }
        let f = self.spherical.as_ref()?;
        valid(self.spherical_state).then(|| {
            let [r, az, el] = get(f);
            [
                r * el.cos() * az.cos(),
                r * el.cos() * az.sin(),
                r * el.sin(),
            ]
        })
    }
}

impl Layout {
    fn attributes(&self, raw: &[RecordValue]) -> (Option<[u8; 3]>, Option<u16>) {
        let rgb = self.color.as_ref().map(|c| {
            c.clone()
                .map(|(i, t, lo, hi)| rescale(value(&raw[i], Some(&t)), lo, hi, 255.0) as u8)
        });
        let intensity = self
            .intensity
            .as_ref()
            .map(|(i, t, lo, hi)| rescale(value(&raw[*i], Some(t)), *lo, *hi, 65535.0) as u16);
        (rgb, intensity)
    }
}

/// Numeric value of a raw record, applying the scaled-integer scale and offset.
fn value(v: &RecordValue, t: Option<&RecordDataType>) -> f64 {
    match (v, t) {
        (RecordValue::Double(d), _) => *d,
        (RecordValue::Single(f), _) => *f as f64,
        (
            RecordValue::ScaledInteger(i),
            Some(RecordDataType::ScaledInteger { scale, offset, .. }),
        ) => *i as f64 * scale + offset,
        (RecordValue::ScaledInteger(i) | RecordValue::Integer(i), _) => *i as f64,
    }
}

pub(crate) fn inspect(path: &Path, progress: ProgressFn, mut visit: Visitor) -> Result<Contents> {
    let mut reader = E57Reader::from_file(path).map_err(|e| parse_err(FMT, e))?;
    let mut c = Contents::new(FMT);
    c.declared_unit = Some(LinearUnit::Meter); // E57 coordinates are meters by definition
    c.crs = reader
        .coordinate_metadata()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let clouds = reader.pointclouds();
    for (i, pc) in clouds.iter().enumerate() {
        if visit.as_ref().is_some_and(|(scan, _)| *scan != i) {
            continue; // a visitor only needs its own scan
        }
        let layout = Layout::new(pc)?;
        let points = reader.pointcloud_raw(pc).map_err(|e| parse_err(FMT, e))?;
        let mut stats = ScanStats::default();
        for raw in points {
            let raw = raw.map_err(|e| parse_err(FMT, e))?;
            let position = layout.position(&raw);
            if let (Some(p), Some((_, f))) = (position, visit.as_mut()) {
                let (rgb, intensity) = layout.attributes(&raw);
                f(&Point {
                    p,
                    rgb,
                    intensity,
                    index: stats.count,
                });
            }
            match position {
                Some(p) => stats.point(p),
                None => stats.invalid(),
            }
            if stats.count % (1 << 20) == 0 {
                progress(Progress {
                    stage: Stage::Points,
                    done: stats.count,
                    total: pc.records,
                });
            }
        }
        let name = pc.name.clone().unwrap_or_else(|| format!("Scan {}", i + 1));
        if stats.count != pc.records {
            c.warnings.push(format!(
                "{name}: header says {} points, file contains {}",
                pc.records, stats.count
            ));
        }
        let mut attributes = vec![];
        if pc.has_intensity() {
            attributes.push("intensity".into());
        }
        if pc.has_color() {
            attributes.push("color".into());
        }
        if pc.has_spherical() {
            attributes.push("spherical".into());
        }
        if pc.has_row_column() {
            attributes.push("row_column".into());
        }
        if pc.has_timestamp() {
            attributes.push("timestamp".into());
        }
        c.scans
            .push(stats.into_scan(name, pose_matrix(pc.transform.as_ref()), attributes));
    }

    for (i, img) in reader.images().iter().enumerate() {
        let name = img
            .name
            .clone()
            .unwrap_or_else(|| format!("Image {}", i + 1));
        let (kind, width, height) = match (&img.projection, &img.visual_reference) {
            (Some(Projection::Pinhole(p)), _) => {
                (ImageKind::Pinhole, p.properties.width, p.properties.height)
            }
            (Some(Projection::Spherical(p)), _) => (
                ImageKind::Spherical,
                p.properties.width,
                p.properties.height,
            ),
            (Some(Projection::Cylindrical(p)), _) => (
                ImageKind::Cylindrical,
                p.properties.width,
                p.properties.height,
            ),
            (None, Some(v)) => (
                ImageKind::VisualReference,
                v.properties.width,
                v.properties.height,
            ),
            (None, None) => {
                c.warnings
                    .push(format!("{name}: image entry has no image data"));
                continue;
            }
        };
        let scan = img
            .pointcloud_guid
            .as_deref()
            .and_then(|g| clouds.iter().position(|pc| pc.guid.as_deref() == Some(g)));
        c.images.push(ImageInfo {
            name,
            kind,
            width,
            height,
            scan,
            pose: img.transform.as_ref().map(|t| pose_matrix(Some(t))),
            exif: vec![],
        });
    }
    Ok(c)
}

/// Write each embedded image to `out_dir` as `NNN.jpg` / `NNN.png` (derived data).
/// Returns the written paths, in the same order as `Contents::images`.
pub fn extract_images(e57_path: &Path, out_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut reader = E57Reader::from_file(e57_path).map_err(|e| parse_err(FMT, e))?;
    std::fs::create_dir_all(out_dir)?;
    let mut written = vec![];
    for (i, img) in reader.images().iter().enumerate() {
        let blob = match (&img.projection, &img.visual_reference) {
            (Some(Projection::Pinhole(p)), _) => &p.blob,
            (Some(Projection::Spherical(p)), _) => &p.blob,
            (Some(Projection::Cylindrical(p)), _) => &p.blob,
            (None, Some(v)) => &v.blob,
            (None, None) => continue,
        };
        let ext = match blob.format {
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Png => "png",
        };
        let out = out_dir.join(format!("{i:03}.{ext}"));
        let mut file = OpenOptions::new().write(true).create_new(true).open(&out)?;
        reader
            .blob(&blob.data, &mut file)
            .map_err(|e| parse_err(FMT, e))?;
        written.push(out);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use e57::{Quaternion, Translation};

    fn apply(m: &[f64; 16], p: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|r| {
            m[r * 4] * p[0] + m[r * 4 + 1] * p[1] + m[r * 4 + 2] * p[2] + m[r * 4 + 3]
        })
    }

    #[test]
    fn quarter_turn_about_z_then_translate() {
        let h = std::f64::consts::FRAC_1_SQRT_2;
        let t = Transform {
            rotation: Quaternion {
                w: h,
                x: 0.0,
                y: 0.0,
                z: h,
            },
            translation: Translation {
                x: 10.0,
                y: 20.0,
                z: 30.0,
            },
        };
        let p = apply(&pose_matrix(Some(&t)), [1.0, 0.0, 0.0]);
        // x axis rotates onto y, then shifts.
        for (a, b) in p.iter().zip([10.0, 21.0, 30.0]) {
            assert!((a - b).abs() < 1e-12, "{p:?}");
        }
    }

    #[test]
    fn spherical_and_scaled_values() {
        let d = |v| RecordValue::Double(v);
        let dt = || RecordDataType::Double {
            min: None,
            max: None,
        };
        let layout = Layout {
            cartesian: None,
            spherical: Some([(0, dt()), (1, dt()), (2, dt())]),
            cartesian_state: None,
            spherical_state: Some(3),
            color: None,
            intensity: None,
        };
        // Range 2 m, azimuth 90 degrees, elevation 30 degrees.
        let p = layout
            .position(&[
                d(2.0),
                d(std::f64::consts::FRAC_PI_2),
                d(std::f64::consts::FRAC_PI_6),
                RecordValue::Integer(0),
            ])
            .unwrap();
        let expect = [0.0, 3f64.sqrt(), 1.0];
        assert!(
            p.iter().zip(expect).all(|(a, b)| (a - b).abs() < 1e-12),
            "{p:?}"
        );
        assert_eq!(
            layout.position(&[d(2.0), d(0.0), d(0.0), RecordValue::Integer(2)]),
            None
        );

        let scaled = RecordDataType::ScaledInteger {
            min: -1000,
            max: 1000,
            scale: 0.001,
            offset: 100.0,
        };
        assert_eq!(
            value(&RecordValue::ScaledInteger(-250), Some(&scaled)),
            99.75
        );
    }

    #[test]
    fn unnormalized_quaternion_is_normalized() {
        let t = Transform {
            rotation: Quaternion {
                w: 2.0,
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            translation: Translation {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        };
        assert_eq!(pose_matrix(Some(&t)), IDENTITY);
    }
}
