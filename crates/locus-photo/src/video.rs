//! Frames sampled from a video at a chosen interval, written as PNG files for COLMAP. Windows
//! only, through Media Foundation (part of the operating system: nothing to ship); on macOS and
//! Linux video import is refused with a message for now.

use std::path::{Path, PathBuf};

/// One frame written out: its file and its time from the video's first frame (s).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Frame {
    pub file: PathBuf,
    pub time: f64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Sampled {
    pub width: u32,
    pub height: u32,
    /// The video's duration (s) and the number of frames decoded.
    pub duration: f64,
    pub decoded: u64,
    /// The first frame's presentation timestamp (s): frame times count from it.
    pub first_timestamp: f64,
    pub frames: Vec<Frame>,
}

/// Write one frame every `interval` seconds of `video` into `out` (created) as
/// `frame_000001.png`, …, starting with the first frame. `cancel` stops it between frames.
pub fn sample_frames(
    video: &Path,
    interval: f64,
    out: &Path,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(f64, f64),
) -> Result<Sampled, String> {
    if !(interval > 0.0 && interval.is_finite()) {
        return Err("the frame interval must be more than 0 s".into());
    }
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    imp::sample(video, interval, out, cancel, progress)
}

/// A frame (BGRX, rows top to bottom) as an RGB PNG.
fn write_png(path: &Path, w: u32, h: u32, bgrx: &[u8], stride: usize) -> Result<(), String> {
    let f = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h as usize {
        let row = &bgrx[y * stride..y * stride + w as usize * 4];
        for px in row.as_chunks::<4>().0 {
            rgb.extend([px[2], px[1], px[0]]);
        }
    }
    wr.write_image_data(&rgb).map_err(|e| e.to_string())
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub fn sample(
        _: &Path,
        _: f64,
        _: &Path,
        _: &std::sync::atomic::AtomicBool,
        _: &mut dyn FnMut(f64, f64),
    ) -> Result<Sampled, String> {
        Err("Video import uses Windows Media Foundation and isn't available on this system yet. Extract frames with another tool and add them as photos.".into())
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::sync::atomic::Ordering;
    use windows::core::HSTRING;
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    const FIRST_VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    const MEDIA_SOURCE: u32 = MF_SOURCE_READER_MEDIASOURCE.0 as u32;

    pub fn sample(
        video: &Path,
        interval: f64,
        out: &Path,
        cancel: &std::sync::atomic::AtomicBool,
        progress: &mut dyn FnMut(f64, f64),
    ) -> Result<Sampled, String> {
        let e = |m: &str, err: windows::core::Error| format!("{m}: {err}");
        unsafe {
            // Already initialised on this thread (in another mode) is fine.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(|x| e("Media Foundation", x))?;
            let r = read(video, interval, out, cancel, progress);
            let _ = MFShutdown();
            r
        }
    }

    unsafe fn read(
        video: &Path,
        interval: f64,
        out: &Path,
        cancel: &std::sync::atomic::AtomicBool,
        progress: &mut dyn FnMut(f64, f64),
    ) -> Result<Sampled, String> {
        let e = |m: &str, err: windows::core::Error| format!("{m}: {err}");
        let mut attrs: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attrs, 1).map_err(|x| e("attributes", x))?;
        let attrs = attrs.ok_or("no attributes")?;
        attrs
            .SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)
            .map_err(|x| e("attributes", x))?;
        let reader = MFCreateSourceReaderFromURL(&HSTRING::from(video.as_os_str()), &attrs)
            .map_err(|x| e(&format!("could not open {}", video.display()), x))?;
        let mt = MFCreateMediaType().map_err(|x| e("media type", x))?;
        mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .map_err(|x| e("media type", x))?;
        mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
            .map_err(|x| e("media type", x))?;
        reader
            .SetCurrentMediaType(FIRST_VIDEO, None, &mt)
            .map_err(|x| e("this video can't be decoded to RGB", x))?;
        let cur = reader
            .GetCurrentMediaType(FIRST_VIDEO)
            .map_err(|x| e("media type", x))?;
        let size = cur
            .GetUINT64(&MF_MT_FRAME_SIZE)
            .map_err(|x| e("frame size", x))?;
        let (w, h) = ((size >> 32) as u32, (size & 0xFFFF_FFFF) as u32);
        let stride = cur
            .GetUINT32(&MF_MT_DEFAULT_STRIDE)
            .map(|s| s as i32)
            .unwrap_or((w * 4) as i32);
        let duration = reader
            .GetPresentationAttribute(MEDIA_SOURCE, &MF_PD_DURATION)
            .ok()
            .and_then(|v: PROPVARIANT| u64::try_from(&v).ok())
            .map_or(0.0, |d| d as f64 / 1e7);
        let (mut frames, mut decoded, mut next) = (vec![], 0u64, 0.0f64);
        let mut first = None::<f64>;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            let (mut flags, mut ts, mut sample) = (0u32, 0i64, None::<IMFSample>);
            reader
                .ReadSample(
                    FIRST_VIDEO,
                    0,
                    None,
                    Some(&mut flags),
                    Some(&mut ts),
                    Some(&mut sample),
                )
                .map_err(|x| e("reading a frame", x))?;
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
            let Some(sample) = sample else { continue };
            decoded += 1;
            let t0 = *first.get_or_insert(ts as f64 / 1e7);
            let t = ts as f64 / 1e7 - t0;
            if t + 1e-6 < next {
                continue;
            }
            let buf = sample
                .ConvertToContiguousBuffer()
                .map_err(|x| e("frame buffer", x))?;
            let (mut ptr, mut len) = (std::ptr::null_mut::<u8>(), 0u32);
            buf.Lock(&mut ptr, None, Some(&mut len))
                .map_err(|x| e("frame buffer", x))?;
            let data = std::slice::from_raw_parts(ptr, len as usize);
            // A negative stride means the rows are stored bottom to top.
            let abs = stride.unsigned_abs() as usize;
            let rows: Vec<u8> = if stride < 0 {
                (0..h as usize)
                    .rev()
                    .flat_map(|y| data[y * abs..(y + 1) * abs].iter().copied())
                    .collect()
            } else {
                data[..abs * h as usize].to_vec()
            };
            let _ = buf.Unlock();
            let file = out.join(format!("frame_{:06}.png", frames.len() + 1));
            write_png(&file, w, h, &rows, abs)?;
            frames.push(Frame { file, time: t });
            progress(t, duration);
            next = (t / interval).floor() * interval + interval;
        }
        if frames.is_empty() {
            return Err(format!("no frames could be read from {}", video.display()));
        }
        Ok(Sampled {
            width: w,
            height: h,
            duration,
            decoded,
            first_timestamp: first.unwrap_or(0.0),
            frames,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    /// A 4 s, 25 fps test video written with Media Foundation (a moving square on a gradient),
    /// sampled every 0.5 s.
    #[test]
    #[cfg(windows)]
    fn frames_are_sampled_at_the_interval() {
        let dir = std::env::temp_dir().join(format!("locus-video-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mp4 = dir.join("test.mp4");
        let frames: Vec<crate::video_dev::Frame> = (0..100)
            .map(|k| {
                let (w, h) = (320u32, 240u32);
                let mut bgrx = vec![0u8; (w * h * 4) as usize];
                for y in 0..h {
                    for x in 0..w {
                        let i = ((y * w + x) * 4) as usize;
                        let inside = x.abs_diff(40 + 2 * k) < 20 && y.abs_diff(120) < 20;
                        bgrx[i] = if inside { 255 } else { (x * 255 / w) as u8 };
                        bgrx[i + 1] = if inside { 255 } else { (y * 255 / h) as u8 };
                        bgrx[i + 2] = if inside { 255 } else { 64 };
                    }
                }
                crate::video_dev::Frame {
                    width: w,
                    height: h,
                    bgrx,
                }
            })
            .collect();
        crate::video_dev::write_mp4(&mp4, &frames, 25).unwrap();
        let s = sample_frames(
            &mp4,
            0.5,
            &dir.join("frames"),
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap();
        assert_eq!((s.width, s.height), (320, 240));
        assert!((s.duration - 4.0).abs() < 0.1, "{}", s.duration);
        assert_eq!(s.decoded, 100);
        assert_eq!(
            s.frames.len(),
            8,
            "{:?}",
            s.frames.iter().map(|f| f.time).collect::<Vec<_>>()
        );
        for (k, f) in s.frames.iter().enumerate() {
            assert!(
                (f.time - 0.5 * k as f64).abs() < 0.021,
                "{} at {}",
                k,
                f.time
            );
        }
        let img = std::fs::read(&s.frames[3].file).unwrap();
        assert_eq!(&img[1..4], b"PNG");
        // The pixels: frame 3 is at 1.5 s, video frame 37 or 38, so the white square is centred
        // near x = 40 + 2 × 37.5 = 115, y = 120; the background there is the gradient.
        let dec = png::Decoder::new(std::io::Cursor::new(img));
        let mut r = dec.read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut buf).unwrap();
        let px = |x: usize, y: usize| {
            let i = (y * info.width as usize + x) * 3;
            [buf[i], buf[i + 1], buf[i + 2]]
        };
        let sq = px(115, 120);
        assert!(sq.iter().all(|c| *c > 200), "square {sq:?}");
        // Above the square: red 64, green from y, blue from x (x = 115 → 91; y = 60 → 64).
        let bg = px(115, 60);
        assert!(
            (bg[0] as i32 - 64).abs() < 20
                && (bg[1] as i32 - 64).abs() < 20
                && (bg[2] as i32 - 91).abs() < 20,
            "background {bg:?}"
        );
        // Not upside down: the green channel grows downwards.
        assert!(px(20, 220)[1] > px(20, 20)[1] + 100);
        assert!(sample_frames(
            &mp4,
            0.0,
            &dir.join("x"),
            &AtomicBool::new(false),
            &mut |_, _| {}
        )
        .is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod dump {
    /// Sample a video to a folder, to look at the frames: LOCUS_VIDEO=file LOCUS_FRAMES=dir.
    #[test]
    #[ignore = "a tool: needs LOCUS_VIDEO and LOCUS_FRAMES"]
    fn dump_frames() {
        let (v, d) = (
            std::env::var("LOCUS_VIDEO").unwrap(),
            std::env::var("LOCUS_FRAMES").unwrap(),
        );
        let s = super::sample_frames(
            std::path::Path::new(&v),
            0.5,
            std::path::Path::new(&d),
            &std::sync::atomic::AtomicBool::new(false),
            &mut |_, _| {},
        )
        .unwrap();
        println!(
            "{}×{}, {} frames, first at {}",
            s.width,
            s.height,
            s.frames.len(),
            s.first_timestamp
        );
    }
}
