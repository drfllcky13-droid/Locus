//! `locus-validate import`: create or open a project and import files through exactly the
//! path the app uses (preview → commit → octree per scan), reporting times and peak memory.
//! For performance runs on large scenes without driving the UI.

use locus_core::{LinearUnit, Project};
use locus_octree::scene::build_scan;
use std::path::Path;
use std::time::Instant;

/// Peak working set and peak committed memory of this process, bytes.
#[cfg(windows)]
pub fn peak_memory() -> (u64, u64) {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: a zeroed PROCESS_MEMORY_COUNTERS is valid, and cb holds its true size.
    let mut c: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: `c` is writable and correctly sized for the call.
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    (c.PeakWorkingSetSize as u64, c.PeakPagefileUsage as u64)
}

#[cfg(not(windows))]
pub fn peak_memory() -> (u64, u64) {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let hwm = status
        .lines()
        .find(|l| l.starts_with("VmHWM:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
        * 1024;
    (hwm, hwm)
}

pub fn run(
    project_dir: &Path,
    examiner: &str,
    files: &[String],
    unit: Option<LinearUnit>,
) -> Result<(), String> {
    let err = |e: &dyn std::fmt::Display| e.to_string();
    let mut project = if project_dir.join("project.sqlite").is_file() {
        Project::open(project_dir, examiner).map_err(|e| err(&e))?
    } else {
        let name = project_dir
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        Project::create(project_dir, &name, examiner).map_err(|e| err(&e))?
    };
    for file in files {
        let path = Path::new(file);
        let t = Instant::now();
        let pv = locus_io::preview(path, &mut |_| {}).map_err(|e| err(&e))?;
        let t_preview = t.elapsed();
        let rec = locus_io::commit(&mut project, &pv, unit, &mut |_| {}).map_err(|e| err(&e))?;
        let t_commit = t.elapsed() - t_preview;
        eprintln!(
            "{file}: {} points in {} scans, SHA-256 {}; preview {:.1?}, copy {:.1?}",
            rec.contents.point_count(),
            rec.contents.scans.len(),
            rec.sha256,
            t_preview,
            t_commit
        );
        let t = Instant::now();
        for s in 0..rec.contents.scans.len() {
            let ts = Instant::now();
            let meta = build_scan(project.root(), &rec, s, &mut |_| {}).map_err(|e| err(&e))?;
            project
                .record_octree(rec.id, s, "built", meta.points, "")
                .map_err(|e| err(&e))?;
            eprintln!(
                "  scan {}: {} points, {} nodes, {:.1?}",
                s + 1,
                meta.points,
                meta.nodes,
                ts.elapsed()
            );
        }
        let (ws, commit) = peak_memory();
        eprintln!(
            "  octrees built in {:.1?}; peak working set {} MB, peak committed {} MB",
            t.elapsed(),
            ws / 1_000_000,
            commit / 1_000_000
        );
    }
    Ok(())
}
