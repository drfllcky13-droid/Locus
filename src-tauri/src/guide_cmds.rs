//! The guided workflows' support: what the open project already has (so each guide step can
//! tick itself), and the sample case's evidence, generated on this computer from the
//! validation generators (so nothing large ships with the app).

use crate::commands::{blocking, err, CmdResult};
use serde::Serialize;
use std::path::PathBuf;
use tauri::AppHandle;

#[derive(Serialize, Default)]
pub struct GuideState {
    pub scans: usize,
    pub photos: usize,
    pub videos: usize,
    pub verified: bool,
    pub registrations: usize,
    pub cleanups: usize,
    pub measurements: usize,
    pub diagrams: usize,
    pub diagram_prints: usize,
    pub scenes: usize,
    pub animations: usize,
    /// Analyses by tool (not withdrawn).
    pub analyses: std::collections::BTreeMap<String, usize>,
    pub reports_printed: usize,
    pub case_reports: usize,
    pub packages: usize,
}

#[tauri::command]
pub async fn guide_state(app: AppHandle) -> CmdResult<GuideState> {
    blocking(app, move |s| {
        let guard = s.project.lock().unwrap();
        let Some(p) = guard.as_ref() else {
            return Ok(GuideState::default());
        };
        let mut g = GuideState::default();
        for e in p.evidence().map_err(err)? {
            g.scans += e.contents.scans.len();
            g.photos += e.contents.images.len();
            g.videos += (e.contents.format == "Video") as usize;
        }
        g.registrations = p.registrations().map_err(err)?.len();
        g.cleanups = p.cleanups().map_err(err)?.len();
        g.measurements = p.measurements().map_err(err)?.len();
        g.diagrams = p.diagrams().map_err(err)?.len();
        let scenes = p.scenes().map_err(err)?;
        g.scenes = scenes.len();
        g.animations = scenes
            .iter()
            .filter(|r| r.document.get("animation").is_some_and(|a| !a.is_null()))
            .count();
        for a in p.analyses().map_err(err)? {
            if a.withdrawn.is_none() {
                *g.analyses.entry(a.tool).or_default() += 1;
            }
        }
        for e in p.audit_log().map_err(err)? {
            match e.action.as_str() {
                "evidence.verified" | "project.opened" => g.verified = true,
                "analysis.reported" | "report.exported" => g.reports_printed += 1,
                "diagram.exported" => g.diagram_prints += 1,
                "export.written" => {
                    let what = serde_json::from_str::<serde_json::Value>(&e.details)
                        .ok()
                        .and_then(|d| d["what"].as_str().map(String::from))
                        .unwrap_or_default();
                    match what.as_str() {
                        "case report" => g.case_reports += 1,
                        "case package" => g.packages += 1,
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        Ok(g)
    })
    .await
}

/// Write the sample indoor case's evidence in a new folder in `parent`: a synthetic room's scan
/// with bloodstains on its walls, and each stain's close-up photo with a scale. Returns the
/// folder. Everything is synthetic: made for practice, not from any real case.
#[tauri::command]
pub async fn sample_create(app: AppHandle, parent: String) -> CmdResult<String> {
    blocking(app, move |_| {
        use locus_synth::bloodstain as b;
        let dir = PathBuf::from(&parent).join("Locus sample case (synthetic)");
        if dir.exists() {
            return Err(format!("{} already exists.", dir.display()));
        }
        let photos = dir.join("stain photos");
        std::fs::create_dir_all(&photos).map_err(err)?;
        let o = b::Options {
            seed: 7,
            droplets: 40,
            ..b::Options::default()
        };
        let truth = b::truth(&o);
        for s in &truth.stains {
            let rgb = b::photo(&o, s);
            let f = std::fs::File::create(photos.join(&s.photo.file)).map_err(err)?;
            let mut enc = png::Encoder::new(
                std::io::BufWriter::new(f),
                s.photo.size_px[0],
                s.photo.size_px[1],
            );
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().map_err(err)?;
            w.write_image_data(&rgb).map_err(err)?;
        }
        locus_synth::write_points(&dir.join("room scan.e57"), "sample room", &b::points(&truth))
            .map_err(err)?;
        std::fs::write(
            dir.join("README.txt"),
            format!(
                "Locus sample case (synthetic)\r\n\r\n\
                 Made on this computer for practice with the in-app guide \"Indoor crime scene\". \
                 Nothing here comes from a real case.\r\n\r\n\
                 room scan.e57   a scanned room (metres) with bloodstains on its walls and floor\r\n\
                 stain photos/   a close-up photo of each of the {} stains, with a scale\r\n\r\n\
                 Follow the guide: make a new project, import the scan (unit: metre) and the photos, \
                 measure, draw a diagram, find the area of origin, print the reports and make a case \
                 package. The blood was cast from about 1.1 m above the floor.\r\n",
                truth.stains.len()
            ),
        )
        .map_err(err)?;
        Ok(dir.to_string_lossy().into_owned())
    })
    .await
}
