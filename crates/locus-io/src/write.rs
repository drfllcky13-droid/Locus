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
}
