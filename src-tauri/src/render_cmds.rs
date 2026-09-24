//! Rendering an animation view to MP4. The view draws each frame (with its overlays) and sends
//! it here as RGBA; a writer thread encodes it (Media Foundation H.264). When the last frame is
//! in, the file is read back and its frame count and duration checked against the timeline, then
//! hashed and logged as a "render" analysis record (audit-logged). Refused on macOS and Linux.

use crate::commands::{blocking, err, CmdResult};
use locus_analysis::animation::{
    permanent_labels, render_frames, view_hfov, Overlays, RenderRecord, ViewKind,
};
use locus_photo::mp4;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::Mutex;
use std::thread::JoinHandle;
use tauri::AppHandle;

#[derive(Debug, Clone, Deserialize)]
pub struct RenderRequest {
    pub scene_id: i64,
    pub view: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub from: f64,
    pub to: f64,
    pub overlays: Overlays,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RenderPlan {
    pub frames: u64,
    /// Labels to draw on every frame whatever the overlays.
    pub labels: Vec<String>,
}

struct Job {
    tx: Option<SyncSender<Vec<u8>>>,
    writer: Option<JoinHandle<Result<u64, String>>>,
    record: RenderRecord,
    path: PathBuf,
    sent: u64,
}

static JOB: Mutex<Option<Job>> = Mutex::new(None);

/// Stop a job and remove its partial file.
fn abandon(mut j: Job) {
    drop(j.tx.take());
    if let Some(h) = j.writer.take() {
        let _ = h.join();
    }
    let _ = std::fs::remove_file(&j.path);
}

/// Check the request against the saved scene and start the writer.
#[tauri::command]
pub async fn render_start(app: AppHandle, request: RenderRequest) -> CmdResult<RenderPlan> {
    crate::license_cmds::require(locus_core::license::Feature::Animation)?;
    if !cfg!(windows) {
        return Err(
            "Rendering video uses Windows Media Foundation and isn't available on this system yet."
                .into(),
        );
    }
    let q = request;
    if !(q.width >= 16 && q.height >= 16 && q.width.is_multiple_of(2) && q.height.is_multiple_of(2))
    {
        return Err("The width and height must be even, and at least 16.".into());
    }
    if !(q.fps > 0.0 && q.fps <= 120.0) {
        return Err("The frame rate must be over 0 and at most 120.".into());
    }
    let (rev, a, audit_head) = blocking(app, move |s| {
        let guard = s.project.lock().unwrap();
        let p = guard.as_ref().ok_or("Open or create a project first.")?;
        let (rev, a) = crate::scene3d_cmds::scene_animation(p, q.scene_id)?;
        let head = p
            .state_head()
            .map_err(err)?
            .map(|e| e.hash)
            .unwrap_or_default();
        Ok((rev, a, head))
    })
    .await?;
    let v = a
        .views
        .iter()
        .find(|v| v.id == q.view)
        .ok_or("That view isn't in the saved animation.")?;
    if !(q.from >= a.from - 1e-9 && q.to <= a.to + 1e-9 && q.to > q.from) {
        return Err(format!(
            "The render must lie within the timeline ({:.2} to {:.2} s).",
            a.from, a.to
        ));
    }
    let frames = render_frames(q.from, q.to, q.fps);
    let labels = permanent_labels(v);
    let path = PathBuf::from(&q.path);
    let mut guard = JOB.lock().unwrap();
    if let Some(old) = guard.take() {
        abandon(old);
    }
    let (tx, rx) = sync_channel::<Vec<u8>>(4);
    let (w, h, fps, out) = (q.width, q.height, q.fps, path.clone());
    // Media Foundation objects stay on the thread that made them.
    let writer = std::thread::spawn(move || -> Result<u64, String> {
        let mut wr = mp4::Writer::new(&out, w, h, fps)?;
        for rgba in rx {
            wr.push_bgrx(&mp4::rgba_to_bgrx(&rgba))?;
        }
        wr.finish()
    });
    let kind = match v.kind {
        ViewKind::Driver { .. } => "driver",
        ViewKind::Witness { .. } => "witness",
        ViewKind::Orbit { .. } => "orbit",
        ViewKind::Follow { .. } => "follow",
        ViewKind::FlyThrough { .. } => "fly-through",
        ViewKind::Mirror { .. } => "mirror",
        ViewKind::Panorama { .. } => "360°",
    };
    *guard = Some(Job {
        tx: Some(tx),
        writer: Some(writer),
        record: RenderRecord {
            scene_id: q.scene_id,
            scene_revision: rev.number,
            audit_head,
            view: format!("{} ({kind}, {:.1}° horizontal)", v.name, view_hfov(v)),
            width: q.width,
            height: q.height,
            fps: q.fps,
            from: q.from,
            to: q.to,
            frames,
            overlays: q.overlays,
            file: q.path.clone(),
            sha256: String::new(),
            view_id: v.id.clone(),
            hfov_deg: view_hfov(v),
            labels: labels.clone(),
            read_back_frames: 0,
            read_back_duration: 0.0,
            encoder: format!(
                "Windows Media Foundation H.264, {} Mbit/s",
                mp4::BITRATE / 1_000_000
            ),
            summary: String::new(),
        },
        path,
        sent: 0,
    });
    Ok(RenderPlan { frames, labels })
}

/// One frame, RGBA rows top to bottom, as the request's raw body.
#[tauri::command]
pub async fn render_frame(request: tauri::ipc::Request<'_>) -> CmdResult<u64> {
    let tauri::ipc::InvokeBody::Raw(rgba) = request.body() else {
        return Err("A frame is sent as raw bytes.".into());
    };
    let (tx, want) = {
        let mut guard = JOB.lock().unwrap();
        let j = guard.as_mut().ok_or("No render is running.")?;
        let want = (j.record.width * j.record.height * 4) as usize;
        if j.sent >= j.record.frames {
            return Err("All the frames are already in.".into());
        }
        j.sent += 1;
        (j.tx.clone().ok_or("No render is running.")?, want)
    };
    if rgba.len() != want {
        return Err(format!("A frame must be {want} bytes, not {}.", rgba.len()));
    }
    let data = rgba.clone();
    // The channel is bounded: this waits while the encoder catches up.
    tauri::async_runtime::spawn_blocking(move || tx.send(data))
        .await
        .map_err(err)?
        .map_err(|_| "The video writer stopped.".to_string())?;
    Ok(JOB.lock().unwrap().as_ref().map_or(0, |j| j.sent))
}

/// Close the file, read it back, check it, hash it and log it.
#[tauri::command]
pub async fn render_finish(app: AppHandle, name: String) -> CmdResult<locus_core::AnalysisRecord> {
    let mut j = JOB.lock().unwrap().take().ok_or("No render is running.")?;
    drop(j.tx.take());
    let written = j
        .writer
        .take()
        .ok_or("No render is running.")?
        .join()
        .map_err(|_| "The video writer failed.".to_string())?;
    let fail = |m: String| {
        let _ = std::fs::remove_file(&j.path);
        Err(format!(
            "{m} The file was removed and nothing was recorded."
        ))
    };
    let written = match written {
        Ok(n) => n,
        Err(e) => return fail(format!("Writing the video failed: {e}.")),
    };
    let r = &mut j.record;
    if written != r.frames {
        return fail(format!("{written} of {} frames were rendered.", r.frames));
    }
    let probe = match mp4::probe(&j.path) {
        Ok(p) => p,
        Err(e) => return fail(format!("The video can't be read back: {e}.")),
    };
    let duration = r.frames as f64 / r.fps;
    if probe.frames != r.frames
        || (probe.width, probe.height) != (r.width, r.height)
        || (probe.duration - duration).abs() > 0.5 / r.fps
    {
        return fail(format!(
            "Read back, the video has {} frames of {}×{} over {:.3} s; the timeline needs {} frames of {}×{} over {:.3} s.",
            probe.frames, probe.width, probe.height, probe.duration, r.frames, r.width, r.height, duration
        ));
    }
    let (sha, _) = locus_core::hash::sha256_file(&j.path, &mut |_| {}).map_err(err)?;
    r.sha256 = sha;
    r.read_back_frames = probe.frames;
    r.read_back_duration = probe.duration;
    r.summary = format!(
        "{}: {} frames at {} fps, {:.2} to {:.2} s, {}×{}; read back: {} frames, {:.3} s.",
        r.view, r.frames, r.fps, r.from, r.to, r.width, r.height, probe.frames, probe.duration
    );
    let record = serde_json::to_value(&*r).map_err(err)?;
    blocking(app, move |s| {
        let mut guard = s.project.lock().unwrap();
        let p = guard.as_mut().ok_or("Open or create a project first.")?;
        p.add_analysis("render", "animation-render/1", &name, &record, None)
            .map_err(err)
    })
    .await
}

/// Stop a render and remove its partial file.
#[tauri::command]
pub async fn render_cancel() -> CmdResult<()> {
    if let Some(j) = JOB.lock().unwrap().take() {
        abandon(j);
    }
    Ok(())
}
