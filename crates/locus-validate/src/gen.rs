//! Ground-truth generators for the analysis tools. Each writes, into `--out DIR`:
//! `truth.json` (the options used and everything true and measured), `scene.e57` (the
//! surfaces as a point cloud, to import into a project) and any images.
//!
//! Every command takes `--seed S` and `--options FILE.json` (a full options object, as
//! found in a truth file's `options`, to reproduce or vary a scenario).

use crate::arg;
use serde::{de::DeserializeOwned, Serialize};
use std::path::{Path, PathBuf};

fn options<T: DeserializeOwned + Default>(args: &[String]) -> Result<T, String> {
    match args.iter().position(|a| a == "--options") {
        Some(i) => {
            let path = args.get(i + 1).ok_or("--options needs a file")?;
            let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
            serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))
        }
        None => Ok(T::default()),
    }
}

fn out_dir(args: &[String]) -> Result<PathBuf, String> {
    let out: String = arg(args, "--out", None)?;
    let dir = PathBuf::from(out);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

fn write_json<T: Serialize>(path: &Path, v: &T) -> Result<(), String> {
    let text = serde_json::to_vec_pretty(v).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn write_png(path: &Path, size: [u32; 2], rgb: &[u8]) -> Result<(), String> {
    let f = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), size[0], size[1]);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(rgb).map_err(|e| e.to_string())
}

fn write_scene(dir: &Path, name: &str, points: &[locus_synth::Point]) -> Result<(), String> {
    locus_synth::write_points(&dir.join("scene.e57"), name, points).map_err(|e| e.to_string())
}

pub fn trajectory(args: &[String]) -> Result<(), String> {
    use locus_synth::trajectory as t;
    let dir = out_dir(args)?;
    let mut o: t::Options = options(args)?;
    o.seed = arg(args, "--seed", Some(o.seed))?;
    let truth = t::truth(&o);
    write_json(&dir.join("truth.json"), &truth)?;
    let pts = t::points(&truth);
    write_scene(&dir, "trajectory panels", &pts)?;
    eprintln!(
        "trajectory: {} panels, {} points, rod play {:.1}° (max {:.1}°), in {}",
        truth.panels.len(),
        pts.len(),
        truth.rod.play_deg,
        truth.rod.max_play_deg,
        dir.display()
    );
    Ok(())
}

pub fn bloodstain(args: &[String]) -> Result<(), String> {
    use locus_synth::bloodstain as b;
    let dir = out_dir(args)?;
    let mut o: b::Options = options(args)?;
    o.seed = arg(args, "--seed", Some(o.seed))?;
    if let Some(f) = args.iter().position(|a| a == "--flight") {
        o.flight = match args.get(f + 1).map(String::as_str) {
            Some("straight") => b::Flight::Straight,
            Some("ballistic") => b::Flight::Ballistic,
            _ => return Err("--flight is straight or ballistic".into()),
        };
    }
    let truth = b::truth(&o);
    write_json(&dir.join("truth.json"), &truth)?;
    let photos = dir.join("photos");
    std::fs::create_dir_all(&photos).map_err(|e| e.to_string())?;
    for s in &truth.stains {
        write_png(
            &photos.join(&s.photo.file),
            s.photo.size_px,
            &b::photo(&o, s),
        )?;
    }
    let pts = b::points(&truth);
    write_scene(&dir, "bloodstain surfaces", &pts)?;
    eprintln!(
        "bloodstain: {} stains ({} upward, {} droplets discarded), {} points, in {}",
        truth.stains.len(),
        truth.stains.iter().filter(|s| s.upward).count(),
        truth.discarded,
        pts.len(),
        dir.display()
    );
    Ok(())
}

/// Volumetric crush: `reference.e57` (an undamaged vehicle's front corner) and `damaged.e57`
/// (the same with a dent of known volume), each where its options place it.
pub fn crush(args: &[String]) -> Result<(), String> {
    use locus_synth::crush as c;
    let dir = out_dir(args)?;
    let mut o: c::Options = options(args)?;
    o.seed = arg(args, "--seed", Some(o.seed))?;
    let truth = c::truth(&o);
    write_json(&dir.join("truth.json"), &truth)?;
    let points = |pts: Vec<[f64; 3]>, pose: locus_synth::Pose| -> Vec<locus_synth::Point> {
        pts.into_iter()
            .map(|p| locus_synth::Point {
                xyz: pose.apply(p),
                intensity: 1200,
                rgb: [170, 175, 185],
            })
            .collect()
    };
    let r = points(
        c::vehicle(None, o.noise, 2 * o.seed),
        c::pose(o.reference_at, o.reference_heading_deg),
    );
    let d = points(
        c::vehicle(Some((o.radius, o.depth)), o.noise, 2 * o.seed + 1),
        c::pose(o.damaged_at, o.damaged_heading_deg),
    );
    for (file, name, pts) in [
        ("reference.e57", "reference vehicle", &r),
        ("damaged.e57", "damaged vehicle", &d),
    ] {
        locus_synth::write_points(&dir.join(file), name, pts).map_err(|e| e.to_string())?;
    }
    eprintln!(
        "crush: dent {:.3} m × {:.3} m, volume {:.3} L; {} + {} points, in {}",
        o.radius,
        o.depth,
        truth.volume * 1000.0,
        r.len(),
        d.len(),
        dir.display()
    );
    Ok(())
}

pub fn camera(args: &[String]) -> Result<(), String> {
    use locus_synth::camera as c;
    let dir = out_dir(args)?;
    let mut o: c::Options = options(args)?;
    o.seed = arg(args, "--seed", Some(o.seed))?;
    let truth = c::truth(&o);
    write_json(&dir.join("truth.json"), &truth)?;
    for (cam, view) in o.cameras.iter().zip(&truth.views) {
        write_png(&dir.join(&view.file), cam.size, &c::render(&o, &truth, cam))?;
    }
    let pts = c::points(&o, &truth);
    write_scene(&dir, "camera room", &pts)?;
    eprintln!(
        "camera: {} views ({} markers seen), {} people, {} points, in {}",
        truth.views.len(),
        truth
            .views
            .iter()
            .map(|v| v
                .markers
                .iter()
                .filter(|m| m.px.is_some())
                .count()
                .to_string())
            .collect::<Vec<_>>()
            .join(", "),
        o.people.len(),
        pts.len(),
        dir.display()
    );
    Ok(())
}
