//! Writing a scene as a multi-scan E57, as scanner software exports it.

use crate::{scan_points, Options, Point, Truth};
use e57::{
    E57Writer, Quaternion, Record, RecordDataType, RecordName, RecordValue, Transform, Translation,
};
use std::path::Path;

/// Write every scan of the scene to `out`, each in its own frame with its stored pose.
/// Coordinates are scaled integers with 0.1 mm resolution. Returns total records written.
pub fn write_e57(
    out: &Path,
    opts: &Options,
    truth: &Truth,
    progress: &mut dyn FnMut(usize, u64),
) -> e57::Result<u64> {
    let mut w = E57Writer::from_file(out, "locus-synthetic-room")?;
    let coord = |name| Record {
        name,
        data_type: RecordDataType::ScaledInteger {
            min: -700_000,
            max: 700_000,
            scale: 0.0001,
            offset: 0.0,
        },
    };
    let int = |name, max| Record {
        name,
        data_type: RecordDataType::Integer { min: 0, max },
    };
    let proto = vec![
        coord(RecordName::CartesianX),
        coord(RecordName::CartesianY),
        coord(RecordName::CartesianZ),
        int(RecordName::CartesianInvalidState, 2),
        int(RecordName::Intensity, 2047),
        int(RecordName::ColorRed, 255),
        int(RecordName::ColorGreen, 255),
        int(RecordName::ColorBlue, 255),
    ];
    let mut total = 0;
    for (s, scan) in truth.scans.iter().enumerate() {
        let mut pc = w.add_pointcloud(&format!("station-{s}"), proto.clone())?;
        pc.set_name(Some(scan.name.clone()));
        pc.set_transform(scan.stored_pose.map(|p| Transform {
            rotation: Quaternion {
                w: p.rotation[0],
                x: p.rotation[1],
                y: p.rotation[2],
                z: p.rotation[3],
            },
            translation: Translation {
                x: p.translation[0],
                y: p.translation[1],
                z: p.translation[2],
            },
        }));
        let (mut i, mut err) = (0u64, None);
        scan_points(opts, truth, s, &mut |p| {
            if err.is_some() {
                return;
            }
            let q = |v: f64| RecordValue::ScaledInteger((v / 0.0001).round() as i64);
            let values = match p {
                Some(p) => vec![
                    q(p.xyz[0]),
                    q(p.xyz[1]),
                    q(p.xyz[2]),
                    RecordValue::Integer(0),
                    RecordValue::Integer(p.intensity.into()),
                    RecordValue::Integer(p.rgb[0].into()),
                    RecordValue::Integer(p.rgb[1].into()),
                    RecordValue::Integer(p.rgb[2].into()),
                ],
                None => {
                    let mut v = vec![q(0.0), q(0.0), q(0.0), RecordValue::Integer(2)];
                    v.extend([0, 0, 0, 0].map(RecordValue::Integer));
                    v
                }
            };
            if let Err(e) = pc.add_point(values) {
                err = Some(e);
            }
            if i % 1_000_000 == 0 {
                progress(s, i);
            }
            i += 1;
        });
        if let Some(e) = err {
            return Err(e);
        }
        pc.finalize()?;
        total += opts.points_per_scan;
    }
    w.finalize()?;
    Ok(total)
}

/// Write one scan of points already in the project frame (identity pose), as a generator's
/// scene for the app to import.
pub fn write_points(out: &Path, name: &str, points: &[Point]) -> e57::Result<()> {
    let mut w = E57Writer::from_file(out, "locus-synthetic")?;
    let coord = |name| Record {
        name,
        data_type: RecordDataType::ScaledInteger {
            min: -700_000,
            max: 700_000,
            scale: 0.0001,
            offset: 0.0,
        },
    };
    let int = |name, max| Record {
        name,
        data_type: RecordDataType::Integer { min: 0, max },
    };
    let proto = vec![
        coord(RecordName::CartesianX),
        coord(RecordName::CartesianY),
        coord(RecordName::CartesianZ),
        int(RecordName::Intensity, 2047),
        int(RecordName::ColorRed, 255),
        int(RecordName::ColorGreen, 255),
        int(RecordName::ColorBlue, 255),
    ];
    let mut pc = w.add_pointcloud(name, proto)?;
    pc.set_name(Some(name.to_string()));
    let q = |v: f64| RecordValue::ScaledInteger((v / 0.0001).round() as i64);
    for p in points {
        pc.add_point(vec![
            q(p.xyz[0]),
            q(p.xyz[1]),
            q(p.xyz[2]),
            RecordValue::Integer(p.intensity.into()),
            RecordValue::Integer(p.rgb[0].into()),
            RecordValue::Integer(p.rgb[1].into()),
            RecordValue::Integer(p.rgb[2].into()),
        ])?;
    }
    pc.finalize()?;
    w.finalize()
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn e57_round_trip_matches_the_scene_within_quantisation() {
        let dir = std::env::temp_dir().join(format!("locus-synth-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("scene.e57");
        let opts = Options {
            scans: 2,
            points_per_scan: 5_000,
            seed: 3,
            ..Options::default()
        };
        let t = truth(&opts);
        write_e57(&path, &opts, &t, &mut |_, _| {}).unwrap();

        let contents = locus_io::inspect(&path, &mut |_| {}).unwrap();
        assert_eq!(contents.scans.len(), 2);
        for s in 0..2 {
            let info = &contents.scans[s];
            let m = t.scans[s].pose.matrix();
            assert!(info.pose.iter().zip(m).all(|(a, b)| (a - b).abs() < 1e-12));
            let mut expected = vec![];
            scan_points(&opts, &t, s, &mut |p| expected.push(p));
            let mut n = 0;
            locus_io::for_each_point(&path, &contents, s, &mut |_| {}, &mut |p| {
                let e = expected[p.index as usize].expect("a valid record was a return");
                assert!(p
                    .p
                    .iter()
                    .zip(e.xyz)
                    .all(|(a, b)| (a - b).abs() <= 0.5e-4 + 1e-9));
                n += 1;
            })
            .unwrap();
            assert_eq!(n, expected.iter().flatten().count());
        }
        std::fs::remove_dir_all(dir).ok();
    }
}
