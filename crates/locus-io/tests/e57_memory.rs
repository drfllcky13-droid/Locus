//! Phase 1 acceptance: a 100M-point E57 imports without exceeding 4 GB of RAM.
//!
//! The fixture is generated into `target/fixtures/` on first run (about 1 GB, never
//! committed) and reused afterwards. The peak covers the whole test process, generation
//! included, so it overstates what import alone uses. Run with:
//!
//!     cargo test -p locus-io --release --test e57_memory -- --ignored --nocapture
//!
//! (or with the other heavy tests: `cargo nextest run --workspace --release --run-ignored only`).

use locus_core::{LinearUnit, Project};
use locus_io::{commit, preview};
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

const SCANS: u64 = 4;
const POINTS_PER_SCAN: u64 = 25_000_000;
const LIMIT: u64 = 4 << 30;
/// Coordinates are scaled integers at 0.1 mm over +/-50 m, like a typical scanner export.
const SCALE: f64 = 0.0001;
const MAX_INT: i64 = 500_000;

fn fixture_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures");
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Deterministic xorshift, so the fixture is identical on every machine.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn fixture() -> PathBuf {
    use e57::*;
    let path = fixture_dir().join("synthetic-100m.e57");
    let done = path.with_extension("e57.complete");
    if done.exists() {
        return path;
    }
    let _ = fs::remove_file(&path);
    let t = Instant::now();
    let mut w = E57Writer::from_file(&path, "locus-synthetic-100m").unwrap();
    let coord = |name| Record {
        name,
        data_type: RecordDataType::ScaledInteger {
            min: -MAX_INT,
            max: MAX_INT,
            scale: SCALE,
            offset: 0.0,
        },
    };
    let proto = vec![
        coord(RecordName::CartesianX),
        coord(RecordName::CartesianY),
        coord(RecordName::CartesianZ),
        Record {
            name: RecordName::Intensity,
            data_type: RecordDataType::Integer { min: 0, max: 2047 },
        },
    ];
    let mut rng = 0x9E37_79B9_7F4A_7C15u64;
    for s in 0..SCANS {
        let mut pc = w
            .add_pointcloud(&format!("scan-{s}"), proto.clone())
            .unwrap();
        pc.set_name(Some(format!("Station {}", s + 1)));
        // The first two points pin the bounds exactly: -50 m and +50 m on every axis.
        for corner in [-MAX_INT, MAX_INT] {
            pc.add_point(
                vec![RecordValue::ScaledInteger(corner); 3]
                    .into_iter()
                    .chain([RecordValue::Integer(0)])
                    .collect(),
            )
            .unwrap();
        }
        for _ in 2..POINTS_PER_SCAN {
            let mut c = || (next(&mut rng) % (2 * MAX_INT as u64 + 1)) as i64 - MAX_INT;
            let (x, y, z) = (c(), c(), c());
            let i = (next(&mut rng) % 2048) as i64;
            pc.add_point(vec![
                RecordValue::ScaledInteger(x),
                RecordValue::ScaledInteger(y),
                RecordValue::ScaledInteger(z),
                RecordValue::Integer(i),
            ])
            .unwrap();
        }
        pc.finalize().unwrap();
    }
    w.finalize().unwrap();
    fs::write(&done, "").unwrap();
    println!(
        "generated {} ({:.2} GB) in {:.0?}",
        path.display(),
        fs::metadata(&path).unwrap().len() as f64 / 1e9,
        t.elapsed()
    );
    path
}

/// (peak working set, peak committed private memory) of this process, in bytes.
#[cfg(windows)]
fn peak_memory() -> (u64, u64) {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut c: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: `c` is a correctly sized, writable PROCESS_MEMORY_COUNTERS.
    let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    assert!(ok != 0, "GetProcessMemoryInfo failed");
    (c.PeakWorkingSetSize as u64, c.PeakPagefileUsage as u64)
}

#[cfg(target_os = "linux")]
fn peak_memory() -> (u64, u64) {
    let status = fs::read_to_string("/proc/self/status").unwrap();
    let kb = |key: &str| {
        status
            .lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap()
            * 1024
    };
    // Linux has no cheap peak-commit figure; resident high-water mark covers both.
    (kb("VmHWM:"), kb("VmHWM:"))
}

#[test]
#[ignore = "heavy: generates a 1 GB fixture (see .config/nextest.toml)"]
fn heavy_import_100m_point_e57_under_4gb() {
    // Calibrate the instrument: touch 64 MiB so a working meter must report at least that.
    const PROBE: usize = 64 << 20;
    let probe = vec![1u8; PROBE];
    assert_eq!(
        std::hint::black_box(&probe)
            .iter()
            .map(|&b| b as usize)
            .sum::<usize>(),
        PROBE
    );
    drop(probe);
    let (ws, commit_peak) = peak_memory();
    assert!(
        ws >= PROBE as u64 && commit_peak >= PROBE as u64,
        "memory meter not working: {ws} / {commit_peak}"
    );

    let src = fixture();
    let work = tempfile::tempdir_in(fixture_dir()).unwrap();
    let mut project =
        Project::create(&work.path().join("case.locus"), "Memory test", "CI").unwrap();

    let t = Instant::now();
    let pv = preview(&src, &mut |_| {}).unwrap();
    let rec = commit(&mut project, &pv, Some(LinearUnit::Meter), &mut |_| {}).unwrap();
    let elapsed = t.elapsed();

    assert_eq!(rec.contents.point_count(), SCANS * POINTS_PER_SCAN);
    for scan in &rec.contents.scans {
        assert_eq!(scan.invalid_points, 0);
        let b = scan.bounds.unwrap();
        assert_eq!((b.min, b.max), ([-50.0; 3], [50.0; 3]), "{}", scan.name);
    }

    let (working_set, committed) = peak_memory();
    println!(
        "imported {} points ({:.2} GB) in {elapsed:.0?}; peak working set {:.0} MB, peak committed {:.0} MB",
        rec.contents.point_count(),
        pv.size as f64 / 1e9,
        working_set as f64 / 1e6,
        committed as f64 / 1e6
    );
    assert!(working_set < LIMIT, "peak working set {working_set} bytes");
    assert!(committed < LIMIT, "peak committed memory {committed} bytes");
}
