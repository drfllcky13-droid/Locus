//! Writing derived point clouds (photogrammetry output) as E57, to be imported as evidence like
//! any other file: one scan, identity pose, double-precision coordinates in metres, colour.

use ::e57::{E57Writer, Record, RecordDataType, RecordName, RecordValue};
use std::path::Path;

/// Write `xyz` (m) with colours `rgb` (same length, or empty for none) as one scan named `name`.
pub fn write_e57(
    out: &Path,
    guid: &str,
    name: &str,
    xyz: &[[f64; 3]],
    rgb: &[[u8; 3]],
) -> ::e57::Result<()> {
    let mut w = E57Writer::from_file(out, guid)?;
    let coord = |name| Record {
        name,
        data_type: RecordDataType::F64,
    };
    let int = |name| Record {
        name,
        data_type: RecordDataType::Integer { min: 0, max: 255 },
    };
    let colour = rgb.len() == xyz.len() && !rgb.is_empty();
    let mut proto = vec![
        coord(RecordName::CartesianX),
        coord(RecordName::CartesianY),
        coord(RecordName::CartesianZ),
    ];
    if colour {
        proto.extend([
            int(RecordName::ColorRed),
            int(RecordName::ColorGreen),
            int(RecordName::ColorBlue),
        ]);
    }
    let mut pc = w.add_pointcloud(name, proto)?;
    pc.set_name(Some(name.to_string()));
    for (k, p) in xyz.iter().enumerate() {
        let mut v = vec![
            RecordValue::Double(p[0]),
            RecordValue::Double(p[1]),
            RecordValue::Double(p[2]),
        ];
        if colour {
            v.extend(rgb[k].map(|c| RecordValue::Integer(c.into())));
        }
        pc.add_point(v)?;
    }
    pc.finalize()?;
    w.finalize()
}

/// A point to write: project-frame metres, and colour and intensity when the source has them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutPoint {
    pub xyz: [f64; 3],
    pub rgb: Option<[u8; 3]>,
    pub intensity: Option<u16>,
}

/// Points handed to a writer one at a time; the writer's `feed` calls it for every point.
pub type Sink<'a> = dyn FnMut(OutPoint) -> Result<(), String> + 'a;

/// Stream points into an E57 file as one scan named `name` with an identity pose, coordinates
/// in double precision. `feed` pushes every point; returns how many were written.
pub fn stream_e57(
    out: &Path,
    guid: &str,
    name: &str,
    colour: bool,
    intensity: bool,
    feed: &mut dyn FnMut(&mut Sink) -> Result<(), String>,
) -> Result<u64, String> {
    let e = |x: ::e57::Error| x.to_string();
    let mut w = E57Writer::from_file(out, guid).map_err(e)?;
    let rec = |name, data_type| Record { name, data_type };
    let mut proto = vec![
        rec(RecordName::CartesianX, RecordDataType::F64),
        rec(RecordName::CartesianY, RecordDataType::F64),
        rec(RecordName::CartesianZ, RecordDataType::F64),
    ];
    let byte = RecordDataType::Integer { min: 0, max: 255 };
    if colour {
        proto.extend([
            rec(RecordName::ColorRed, byte.clone()),
            rec(RecordName::ColorGreen, byte.clone()),
            rec(RecordName::ColorBlue, byte),
        ]);
    }
    if intensity {
        proto.push(rec(
            RecordName::Intensity,
            RecordDataType::Integer { min: 0, max: 65535 },
        ));
    }
    let mut pc = w.add_pointcloud(name, proto).map_err(e)?;
    pc.set_name(Some(name.to_string()));
    let mut n = 0u64;
    feed(&mut |p: OutPoint| {
        let mut v = p.xyz.map(RecordValue::Double).to_vec();
        if colour {
            v.extend(
                p.rgb
                    .unwrap_or([0; 3])
                    .map(|c| RecordValue::Integer(c.into())),
            );
        }
        if intensity {
            v.push(RecordValue::Integer(p.intensity.unwrap_or(0).into()));
        }
        pc.add_point(v).map_err(e)?;
        n += 1;
        Ok(())
    })?;
    pc.finalize().map_err(e)?;
    w.finalize().map_err(e)?;
    Ok(n)
}

/// Stream points into a LAS 1.2 file (LAZ-compressed if `out` ends in .laz), point format 2
/// (colour) with 0.1 mm coordinate resolution about `offset`. No coordinate reference system
/// is written: the coordinates are the project frame's. Returns how many were written.
pub fn stream_las(
    out: &Path,
    offset: [f64; 3],
    feed: &mut dyn FnMut(&mut Sink) -> Result<(), String>,
) -> Result<u64, String> {
    let e = |x: las::Error| x.to_string();
    let mut b = las::Builder::from((1, 2));
    b.point_format = las::point::Format::new(2).map_err(e)?;
    let t = |o: f64| las::Transform {
        scale: 0.0001,
        offset: o.floor(),
    };
    b.transforms = las::Vector {
        x: t(offset[0]),
        y: t(offset[1]),
        z: t(offset[2]),
    };
    b.generating_software = "Locus".into();
    b.system_identifier = "EXTRACTION".into();
    let header = b.into_header().map_err(e)?;
    let mut w = las::Writer::from_path(out, header).map_err(e)?;
    let mut n = 0u64;
    feed(&mut |p: OutPoint| {
        let c = p.rgb.unwrap_or([0; 3]).map(|v| v as u16 * 257);
        w.write_point(las::Point {
            x: p.xyz[0],
            y: p.xyz[1],
            z: p.xyz[2],
            intensity: p.intensity.unwrap_or(0),
            color: Some(las::Color {
                red: c[0],
                green: c[1],
                blue: c[2],
            }),
            ..Default::default()
        })
        .map_err(e)?;
        n += 1;
        Ok(())
    })?;
    w.close().map_err(e)?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_cloud_reads_back() {
        let dir = std::env::temp_dir().join(format!("locus-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("out.e57");
        let xyz = [[1.25, -2.5, 3.0], [120.0, 5000.0, -0.001]];
        write_e57(
            &f,
            "guid-1",
            "photogrammetry",
            &xyz,
            &[[1, 2, 3], [250, 128, 0]],
        )
        .unwrap();
        let contents = crate::inspect(&f, &mut |_| {}).unwrap();
        assert_eq!(contents.scans.len(), 1);
        let mut got = vec![];
        crate::for_each_point(&f, &contents, 0, &mut |_| {}, &mut |p| got.push(*p)).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].p, xyz[1]);
        assert_eq!(got[1].rgb, Some([250, 128, 0]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn streamed_clouds_read_back_in_both_formats() {
        let dir = std::env::temp_dir().join(format!("locus-stream-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pts = [
            OutPoint {
                xyz: [1234.5678, -20.0001, 3.25],
                rgb: Some([10, 200, 30]),
                intensity: Some(1000),
            },
            OutPoint {
                xyz: [1240.0, -25.5, 0.0],
                rgb: Some([255, 0, 128]),
                intensity: Some(65535),
            },
        ];
        let mut feed = |sink: &mut Sink| pts.iter().try_for_each(|p| sink(*p));
        let e = dir.join("a.e57");
        assert_eq!(
            stream_e57(&e, "g", "export", true, true, &mut feed).unwrap(),
            2
        );
        let l = dir.join("a.las");
        assert_eq!(stream_las(&l, [1234.0, -26.0, 0.0], &mut feed).unwrap(), 2);
        for f in [&e, &l] {
            let c = crate::inspect(f, &mut |_| {}).unwrap();
            let mut got = vec![];
            crate::for_each_point(f, &c, 0, &mut |_| {}, &mut |p| got.push(*p)).unwrap();
            assert_eq!(got.len(), 2, "{f:?}");
            for (g, p) in got.iter().zip(&pts) {
                // LAS stores 0.1 mm steps; E57 is exact.
                for k in 0..3 {
                    assert!((g.p[k] - p.xyz[k]).abs() <= 0.00005 + 1e-9, "{f:?} {g:?}");
                }
                assert_eq!(g.rgb, p.rgb, "{f:?}");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
