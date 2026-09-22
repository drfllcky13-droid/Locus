//! Phase 2 rendering spike (SPEC section 5): can the webview stream and draw a large point
//! cloud fast enough? Built only with `--features spike`; never shipped. See
//! docs/spike-rendering.md for how to run it and what it found.
//!
//! Chunk layout: `POINTS` float32 xyz (relative to a local origin), then `POINTS` RGB8.

use std::fs::{self, File};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use tauri::http::{header, Response};
use tauri::ipc;

pub const CHUNKS: u64 = 50;
pub const POINTS: u64 = 1_000_000;
const CHUNK_BYTES: u64 = POINTS * 15;
/// Chunks are 10 m x 10 m tiles laid out on this many columns.
const COLUMNS: u64 = 10;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/fixtures")
}

fn fixture() -> PathBuf {
    fixture_dir().join("spike-chunks.bin")
}

/// Terrain-like synthetic scene: each chunk is one tile of a gently rolling surface with
/// some vertical "walls", so the points overlap on screen like a real scan.
pub fn ensure_fixture() -> std::io::Result<()> {
    let path = fixture();
    if fs::metadata(&path).is_ok_and(|m| m.len() == CHUNKS * CHUNK_BYTES) {
        return Ok(());
    }
    fs::create_dir_all(fixture_dir())?;
    let mut out = BufWriter::with_capacity(1 << 22, File::create(&path)?);
    let mut rng = 0x2545_F491_4F6C_DD1Du64;
    let mut unit = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 11) as f32 / (1u64 << 53) as f32
    };
    for c in 0..CHUNKS {
        let (ox, oy) = ((c % COLUMNS) as f32 * 10.0, (c / COLUMNS) as f32 * 10.0);
        let mut colors = Vec::with_capacity(POINTS as usize * 3);
        for i in 0..POINTS {
            let (x, y) = (ox + unit() * 10.0, oy + unit() * 10.0);
            let ground = (x * 0.3).sin() * 0.8 + (y * 0.21).cos() * 0.6;
            // One point in eight lands on a wall along the tile's west edge.
            let (px, pz) = if i % 8 == 0 {
                (ox, ground + unit() * 3.0)
            } else {
                (x, ground)
            };
            for v in [px, y, pz] {
                out.write_all(&v.to_le_bytes())?;
            }
            let shade = (100.0 + pz * 40.0).clamp(0.0, 255.0) as u8;
            colors.extend([shade, (shade / 2).saturating_add(60), 200 - shade / 2]);
        }
        out.write_all(&colors)?;
    }
    out.flush()
}

fn read_chunk(i: u64) -> std::io::Result<Vec<u8>> {
    if i >= CHUNKS {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no such chunk",
        ));
    }
    let mut f = File::open(fixture())?;
    f.seek(SeekFrom::Start(i * CHUNK_BYTES))?;
    let mut buf = vec![0u8; CHUNK_BYTES as usize];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

/// `spike://localhost/<i>` (http://spike.localhost/<i> on Windows).
pub fn protocol(request: tauri::http::Request<Vec<u8>>) -> Response<Vec<u8>> {
    let chunk = request
        .uri()
        .path()
        .rsplit('/')
        .next()
        .and_then(|s| s.parse::<u64>().ok());
    match chunk.map(read_chunk) {
        Some(Ok(bytes)) => Response::builder()
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(bytes)
            .unwrap(),
        _ => Response::builder().status(404).body(vec![]).unwrap(),
    }
}

/// The same bytes through the IPC channel, for comparison.
#[tauri::command]
pub fn spike_chunk(i: u64) -> Result<ipc::Response, String> {
    read_chunk(i)
        .map(ipc::Response::new)
        .map_err(|e| e.to_string())
}

/// Run options from `LOCUS_SPIKE_QUERY`, e.g. `power=low-power&label=igpu`.
#[tauri::command]
pub fn spike_params() -> String {
    std::env::var("LOCUS_SPIKE_QUERY").unwrap_or_default()
}

/// Write the measurements, then quit.
#[tauri::command]
pub fn spike_report(app: tauri::AppHandle, label: String, json: String) -> Result<(), String> {
    let path = fixture_dir().join(format!("spike-results-{label}.json"));
    fs::write(&path, json).map_err(|e| e.to_string())?;
    println!("spike results written to {}", path.display());
    app.exit(0);
    Ok(())
}
