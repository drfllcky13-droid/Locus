//! Every reader against small fixtures generated here, so no binary test data lives in git.

use locus_core::hash::sha256_file;
use locus_core::{ImageKind, LinearUnit, Project};
use locus_io::{commit, extract_e57_images, inspect, preview, Error, Format};
use proptest::prelude::*;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

fn quiet() -> impl FnMut(locus_io::Progress) {
    |_| {}
}

fn write(dir: &Path, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, bytes).unwrap();
    p
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

// ---------- E57 ----------

/// Two scans with poses and one spherical panorama tied to the second scan.
fn e57_fixture(path: &Path, jpeg: &[u8]) {
    use e57::*;
    let mut w = E57Writer::from_file(path, "file-guid").unwrap();
    let xyz = |name| Record {
        name,
        data_type: RecordDataType::Double {
            min: None,
            max: None,
        },
    };
    let proto = vec![
        xyz(RecordName::CartesianX),
        xyz(RecordName::CartesianY),
        xyz(RecordName::CartesianZ),
        Record {
            name: RecordName::CartesianInvalidState,
            data_type: RecordDataType::Integer { min: 0, max: 2 },
        },
    ];
    let point = |x: f64, y: f64, z: f64, invalid: i64| {
        vec![
            RecordValue::Double(x),
            RecordValue::Double(y),
            RecordValue::Double(z),
            RecordValue::Integer(invalid),
        ]
    };

    let mut pc = w.add_pointcloud("scan-a", proto.clone()).unwrap();
    pc.set_name(Some("Station 1".into()));
    for i in 0..10 {
        pc.add_point(point(i as f64, -(i as f64), 0.5, 0)).unwrap();
    }
    pc.finalize().unwrap();

    let mut pc = w.add_pointcloud("scan-b", proto).unwrap();
    pc.set_name(Some("Station 2".into()));
    let h = std::f64::consts::FRAC_1_SQRT_2;
    pc.set_transform(Some(Transform {
        rotation: Quaternion {
            w: h,
            x: 0.0,
            y: 0.0,
            z: h,
        },
        translation: Translation {
            x: 5.0,
            y: 6.0,
            z: 7.0,
        },
    }));
    pc.add_point(point(1.0, 2.0, 3.0, 0)).unwrap();
    pc.add_point(point(-4.0, 0.0, 9.0, 0)).unwrap();
    pc.add_point(point(0.0, 0.0, 0.0, 2)).unwrap(); // no return
    pc.finalize().unwrap();

    let mut img = w.add_image("image-1").unwrap();
    img.set_name("Pano 2");
    img.set_pointcloud_guid("scan-b");
    img.add_spherical(
        ImageFormat::Jpeg,
        &mut Cursor::new(jpeg),
        SphericalImageProperties {
            width: 64,
            height: 32,
            pixel_width: std::f64::consts::TAU / 64.0,
            pixel_height: std::f64::consts::PI / 32.0,
        },
        None,
    )
    .unwrap();
    img.finalize().unwrap();
    w.finalize().unwrap();
}

#[test]
fn e57_scans_poses_invalid_points_and_panorama() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.e57");
    let jpeg = jpeg_fixture(64, 32, None);
    e57_fixture(&path, &jpeg);

    let c = inspect(&path, &mut quiet()).unwrap();
    assert_eq!(c.format, "E57");
    assert_eq!(c.declared_unit, Some(LinearUnit::Meter));
    assert_eq!(c.scans.len(), 2);
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);

    let a = &c.scans[0];
    assert_eq!(
        (a.name.as_str(), a.point_count, a.invalid_points),
        ("Station 1", 10, 0)
    );
    let b = a.bounds.unwrap();
    assert_eq!((b.min, b.max), ([0.0, -9.0, 0.5], [9.0, 0.0, 0.5]));
    assert_eq!(a.pose, locus_core::IDENTITY);

    let s = &c.scans[1];
    assert_eq!((s.point_count, s.invalid_points), (3, 1));
    let b = s.bounds.unwrap();
    assert_eq!(
        (b.min, b.max),
        ([-4.0, 0.0, 3.0], [1.0, 2.0, 9.0]),
        "bounds are scan-local, pose not applied"
    );
    // Quarter turn about Z: row-major [0 -1 0 5; 1 0 0 6; 0 0 1 7].
    let expect = [
        0.0, -1.0, 0.0, 5.0, 1.0, 0.0, 0.0, 6.0, 0.0, 0.0, 1.0, 7.0, 0.0, 0.0, 0.0, 1.0,
    ];
    assert!(
        s.pose.iter().zip(expect).all(|(a, b)| close(*a, b)),
        "{:?}",
        s.pose
    );

    assert_eq!(c.images.len(), 1);
    let img = &c.images[0];
    assert_eq!(
        (img.kind, img.width, img.height, img.scan),
        (ImageKind::Spherical, 64, 32, Some(1))
    );

    let out = dir.path().join("derived");
    let files = extract_e57_images(&path, &out).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(
        fs::read(&files[0]).unwrap(),
        jpeg,
        "panorama extracted byte-for-byte"
    );
}

// ---------- LAS / LAZ ----------

fn las_fixture(path: &Path) {
    let mut b = las::Builder::from((1, 4));
    b.point_format = las::point::Format::new(2).unwrap();
    let mut w = las::Writer::from_path(path, b.into_header().unwrap()).unwrap();
    for (x, y, z) in [(1.0, 2.0, 3.0), (-1.5, 0.25, 10.0), (100.125, -50.0, 0.0)] {
        w.write_point(las::Point {
            x,
            y,
            z,
            color: Some(las::Color::new(1, 2, 3)),
            ..Default::default()
        })
        .unwrap();
    }
    w.close().unwrap();
}

#[test]
fn las_and_laz_counts_bounds_and_missing_unit() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["cloud.las", "cloud.laz"] {
        let path = dir.path().join(name);
        las_fixture(&path);
        let c = inspect(&path, &mut quiet()).unwrap();
        let s = &c.scans[0];
        assert_eq!(s.point_count, 3, "{name}");
        let b = s.bounds.unwrap();
        assert_eq!(
            (b.min, b.max),
            ([-1.5, -50.0, 0.0], [100.125, 2.0, 10.0]),
            "{name}"
        );
        assert!(s.attributes.contains(&"color".to_string()));
        assert_eq!(c.declared_unit, None, "no CRS in file");
        assert!(c.needs_unit());
    }
}

// ---------- PLY ----------

#[test]
fn ply_ascii_mesh() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(
        dir.path(),
        "tri.ply",
        "ply\nformat ascii 1.0\ncomment made by hand\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n1 0 0\n0 2 -1\n3 0 1 2\n",
    );
    let c = inspect(&p, &mut quiet()).unwrap();
    assert!(c.scans.is_empty());
    let m = &c.meshes[0];
    assert_eq!((m.vertex_count, m.face_count), (3, 1));
    assert_eq!(m.bounds.unwrap().min, [0.0, 0.0, -1.0]);
}

fn binary_ply(big_endian: bool, truncate: usize) -> Vec<u8> {
    let enc = if big_endian {
        "binary_big_endian"
    } else {
        "binary_little_endian"
    };
    let mut v = format!(
        "ply\nformat {enc} 1.0\nelement vertex 2\nproperty double x\nproperty double y\nproperty double z\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nproperty list uchar int extra\nend_header\n"
    )
    .into_bytes();
    for (p, list) in [([1.5f64, -2.0, 3.25], 0u8), ([7.0, 8.0, -9.0], 2)] {
        for c in p {
            v.extend(if big_endian {
                c.to_be_bytes()
            } else {
                c.to_le_bytes()
            });
        }
        v.extend([10, 20, 30, list]);
        for i in 0..list as i32 {
            v.extend(if big_endian {
                i.to_be_bytes()
            } else {
                i.to_le_bytes()
            });
        }
    }
    v.truncate(v.len() - truncate);
    v
}

#[test]
fn ply_binary_both_endians_and_truncation() {
    let dir = tempfile::tempdir().unwrap();
    for be in [false, true] {
        let p = write(dir.path(), "c.ply", binary_ply(be, 0));
        let c = inspect(&p, &mut quiet()).unwrap();
        let s = &c.scans[0];
        assert_eq!(s.point_count, 2);
        let b = s.bounds.unwrap();
        assert_eq!(
            (b.min, b.max),
            ([1.5, -2.0, -9.0], [7.0, 8.0, 3.25]),
            "big endian: {be}"
        );
        assert_eq!(s.attributes, ["color"]);
    }
    let p = write(dir.path(), "cut.ply", binary_ply(false, 3));
    assert!(matches!(
        inspect(&p, &mut quiet()),
        Err(Error::Parse { .. })
    ));
}

// ---------- PTS / XYZ ----------

#[test]
fn pts_blocks_and_count_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(
        dir.path(),
        "two.pts",
        "2\n0 0 0 100 1 2 3\n1 1 1 200 1 2 3\n3\n5 5 5 1 1 1 1\n",
    );
    let c = inspect(&p, &mut quiet()).unwrap();
    assert_eq!(c.scans.len(), 2);
    assert_eq!(c.scans[0].point_count, 2);
    assert_eq!(c.scans[1].point_count, 1);
    assert_eq!(c.scans[0].attributes, ["intensity", "color"]);
    assert_eq!(
        c.warnings,
        ["Block 2: header says 3 points, file contains 1"]
    );
}

#[test]
fn xyz_malformed_line_is_an_error_not_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(dir.path(), "bad.xyz", "0 0 0\n1 1\n2 2 2\n");
    match inspect(&p, &mut quiet()) {
        Err(Error::Parse { message, .. }) => assert!(message.starts_with("line 2:"), "{message}"),
        other => panic!("expected parse error, got {other:?}"),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// Whatever points go in come back as the same count and exact bounds.
    #[test]
    fn xyz_round_trip(points in prop::collection::vec(prop::array::uniform3(-1e6f64..1e6), 1..200),
                      sep in prop::sample::select(vec![" ", ",", "\t", " , "])) {
        let dir = tempfile::tempdir().unwrap();
        let text: String = points.iter().map(|p| format!("{:?}{sep}{:?}{sep}{:?}\n", p[0], p[1], p[2])).collect();
        let path = write(dir.path(), "p.xyz", text);
        let c = inspect(&path, &mut quiet()).unwrap();
        prop_assert_eq!(c.scans[0].point_count, points.len() as u64);
        let b = c.scans[0].bounds.unwrap();
        for i in 0..3 {
            prop_assert_eq!(b.min[i], points.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min));
            prop_assert_eq!(b.max[i], points.iter().map(|p| p[i]).fold(f64::NEG_INFINITY, f64::max));
        }
    }
}

// ---------- OBJ / glTF ----------

#[test]
fn obj_mesh_with_missing_material_library() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(
        dir.path(),
        "box.obj",
        "mtllib box.mtl\no Box\nv 0 0 0\nv 2 0 0\nv 2 3 0\nv 0 3 4\nusemtl red\nf 1 2 3 4\nf 1 2 3\n",
    );
    let c = inspect(&p, &mut quiet()).unwrap();
    let m = &c.meshes[0];
    assert_eq!(
        (m.name.as_str(), m.vertex_count, m.face_count),
        ("Box", 4, 2)
    );
    assert_eq!(m.bounds.unwrap().max, [2.0, 3.0, 4.0]);
    assert_eq!(c.warnings.len(), 1, "{:?}", c.warnings);
    assert!(c.needs_unit());
}

#[test]
fn gltf_counts_bounds_units_and_external_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let mut bin = vec![];
    for v in [0f32, 0., 0., 1., 0., 0., 0., 2., 3.] {
        bin.extend(v.to_le_bytes());
    }
    write(dir.path(), "tri.bin", &bin);
    let p = write(
        dir.path(),
        "tri.gltf",
        r#"{"asset":{"version":"2.0"},
            "buffers":[{"uri":"tri.bin","byteLength":36}],
            "bufferViews":[{"buffer":0,"byteLength":36}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,2,3]}],
            "meshes":[{"name":"Tri","primitives":[{"attributes":{"POSITION":0}}]}]}"#,
    );
    let c = inspect(&p, &mut quiet()).unwrap();
    assert_eq!(c.declared_unit, Some(LinearUnit::Meter));
    assert!(c.y_up);
    let m = &c.meshes[0];
    assert_eq!(
        (m.name.as_str(), m.vertex_count, m.face_count),
        ("Tri", 3, 1)
    );
    assert_eq!(m.bounds.unwrap().max, [1.0, 2.0, 3.0]);
    assert!(c.warnings[0].contains("tri.bin"));
}

// ---------- JPEG / PNG ----------

/// Minimal JPEG: SOI, optional APP1 EXIF, SOF0 with the dimensions, EOI.
fn jpeg_fixture(width: u16, height: u16, exif: Option<Vec<u8>>) -> Vec<u8> {
    let mut v = vec![0xFF, 0xD8];
    if let Some(tiff) = exif {
        v.extend([0xFF, 0xE1]);
        v.extend(((tiff.len() + 8) as u16).to_be_bytes());
        v.extend(b"Exif\0\0");
        v.extend(tiff);
    }
    v.extend([0xFF, 0xC0, 0x00, 0x11, 8]);
    v.extend(height.to_be_bytes());
    v.extend(width.to_be_bytes());
    v.extend([3, 1, 0x11, 0, 2, 0x11, 1, 3, 0x11, 1, 0xFF, 0xD9]);
    v
}

#[test]
fn jpeg_dimensions_and_exif() {
    use exif::{experimental::Writer, Field, In, Tag, Value};
    let make = Field {
        tag: Tag::Make,
        ifd_num: In::PRIMARY,
        value: Value::Ascii(vec![b"Lotus Test Camera".to_vec()]),
    };
    let taken = Field {
        tag: Tag::DateTimeOriginal,
        ifd_num: In::PRIMARY,
        value: Value::Ascii(vec![b"2026:09:22 10:30:00".to_vec()]),
    };
    let mut w = Writer::new();
    w.push_field(&make);
    w.push_field(&taken);
    let mut tiff = Cursor::new(vec![]);
    w.write(&mut tiff, false).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let p = write(
        dir.path(),
        "photo.jpg",
        jpeg_fixture(640, 480, Some(tiff.into_inner())),
    );
    let c = inspect(&p, &mut quiet()).unwrap();
    let img = &c.images[0];
    assert_eq!(
        (img.kind, img.width, img.height),
        (ImageKind::Photo, 640, 480)
    );
    let get = |t: &str| {
        img.exif
            .iter()
            .find(|f| f.tag == t)
            .map(|f| f.value.clone())
    };
    assert_eq!(get("Make").as_deref(), Some("\"Lotus Test Camera\""));
    assert_eq!(
        get("DateTimeOriginal").as_deref(),
        Some("2026-09-22 10:30:00")
    );
    assert!(!c.needs_unit(), "photos need no unit");
}

#[test]
fn png_dimensions_without_exif() {
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    png.extend(300u32.to_be_bytes());
    png.extend(200u32.to_be_bytes());
    png.extend([8, 2, 0, 0, 0, 0, 0, 0, 0]);
    let dir = tempfile::tempdir().unwrap();
    let p = write(dir.path(), "shot.png", png);
    let c = inspect(&p, &mut quiet()).unwrap();
    assert_eq!((c.images[0].width, c.images[0].height), (300, 200));
    assert!(c.images[0].exif.is_empty());
}

// ---------- detection and the import round trip ----------

#[test]
fn renamed_or_unknown_files_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let fake = write(dir.path(), "notreally.e57", "0 0 0\n");
    assert!(matches!(Format::detect(&fake), Err(Error::Parse { .. })));
    let other = write(dir.path(), "scan.fls", "whatever");
    assert!(matches!(Format::detect(&other), Err(Error::Unsupported(_))));
}

#[test]
fn preview_then_commit_leaves_source_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("scene.e57");
    e57_fixture(&src, &jpeg_fixture(64, 32, None));
    let before = sha256_file(&src, &mut |_| {}).unwrap().0;
    let mtime = fs::metadata(&src).unwrap().modified().unwrap();

    let mut project = Project::create(&dir.path().join("case.locus"), "Case", "A").unwrap();
    let pv = preview(&src, &mut quiet()).unwrap();
    assert_eq!(pv.sha256, before);
    let rec = commit(&mut project, &pv, None, &mut quiet()).unwrap();
    assert_eq!(rec.unit, Some(LinearUnit::Meter), "E57 declares meters");
    assert_eq!(rec.contents.point_count(), 13);

    assert_eq!(sha256_file(&src, &mut |_| {}).unwrap().0, before);
    assert_eq!(fs::metadata(&src).unwrap().modified().unwrap(), mtime);
    let stored = project.root().join(&rec.stored_path);
    assert_eq!(sha256_file(&stored, &mut |_| {}).unwrap().0, before);

    // A file changed after preview is refused at commit.
    let xyz = write(dir.path(), "pts.xyz", "0 0 0\n");
    let pv = preview(&xyz, &mut quiet()).unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(&xyz)
        .unwrap()
        .write_all(b"1 1 1\n")
        .unwrap();
    assert!(matches!(
        commit(&mut project, &pv, Some(LinearUnit::Meter), &mut quiet()),
        Err(Error::Project(locus_core::Error::SourceChanged { .. }))
    ));
}

// ---------- point streaming ----------

fn points_of(path: &Path, scan: usize) -> Vec<locus_io::Point> {
    let contents = inspect(path, &mut quiet()).unwrap();
    let mut out = vec![];
    locus_io::for_each_point(path, &contents, scan, &mut quiet(), &mut |p| out.push(*p)).unwrap();
    out
}

#[test]
fn e57_points_keep_source_record_numbers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.e57");
    e57_fixture(&path, &jpeg_fixture(8, 8, None));
    let a = points_of(&path, 0);
    assert_eq!(a.len(), 10);
    assert_eq!(a[3].p, [3.0, -3.0, 0.5]);
    // Scan 2's third record has no return: it is skipped, but numbering still counts it.
    let b = points_of(&path, 1);
    assert_eq!(b.iter().map(|p| p.index).collect::<Vec<_>>(), [0, 1]);
    assert_eq!(b[1].p, [-4.0, 0.0, 9.0], "scan-local, pose not applied");
}

#[test]
fn las_points_with_eight_bit_colour_in_sixteen_bit_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cloud.las");
    las_fixture(&path);
    let c = inspect(&path, &mut quiet()).unwrap();
    assert!(c.scans[0].attributes.contains(&"color_8bit".to_string()));
    let pts = points_of(&path, 0);
    assert_eq!(pts.len(), 3);
    assert_eq!(pts[1].p, [-1.5, 0.25, 10.0]);
    assert_eq!(
        pts[1].rgb,
        Some([1, 2, 3]),
        "8-bit values are not shifted away"
    );
}

#[test]
fn pts_block_points_carry_intensity_and_colour() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(
        dir.path(),
        "two.pts",
        "1\n0 0 0 -2048 1 2 3\n2\n1 1 1 2047 250 0 9\n2 2 2 0 0 0 0\n",
    );
    let second = points_of(&p, 1);
    assert_eq!(second.len(), 2);
    assert_eq!(second[0].intensity, Some(65535));
    assert_eq!(second[0].rgb, Some([250, 0, 9]));
    assert_eq!(points_of(&p, 0)[0].intensity, Some(0));
}

#[test]
fn ply_points_scale_float_and_byte_colours() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(
        dir.path(),
        "c.ply",
        "ply\nformat ascii 1.0\nelement vertex 2\nproperty float x\nproperty float y\nproperty float z\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nproperty float intensity\nend_header\n1 2 3 255 128 0 1.0\n4 5 6 0 0 0 0.5\n",
    );
    let pts = points_of(&p, 0);
    assert_eq!(pts[0].rgb, Some([255, 128, 0]));
    assert_eq!(pts[0].intensity, Some(65535));
    assert_eq!(pts[1].intensity, Some(32768));
    assert_eq!(pts[1].index, 1);
}
