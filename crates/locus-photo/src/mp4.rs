//! Writing an H.264 MP4 frame by frame, and reading one back to count its frames and
//! duration: Windows Media Foundation (part of the operating system: nothing to ship). On
//! macOS and Linux both are refused with a message for now.

use std::path::Path;

/// Bit rate (bits/s): high, as the frames are exhibits.
pub const BITRATE: u32 = 40_000_000;

/// What a written file holds, read back from it.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Probe {
    pub width: u32,
    pub height: u32,
    pub frames: u64,
    /// The container's duration (s).
    pub duration: f64,
}

/// A frame rate as a ratio (Media Foundation's form): 30 → 30/1, 29.97 → 29970/1000.
pub fn rate(fps: f64) -> (u32, u32) {
    let den = if (fps - fps.round()).abs() < 1e-9 {
        1
    } else {
        1000
    };
    ((fps * den as f64).round() as u32, den)
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    const NO: &str =
        "Rendering video uses Windows Media Foundation and isn't available on this system yet.";
    pub struct Writer;
    impl Writer {
        pub fn new(_: &Path, _: u32, _: u32, _: f64) -> Result<Writer, String> {
            Err(NO.into())
        }
        pub fn push_bgrx(&mut self, _: &[u8]) -> Result<(), String> {
            Err(NO.into())
        }
        pub fn finish(self) -> Result<u64, String> {
            Err(NO.into())
        }
    }
    pub fn probe(_: &Path) -> Result<Probe, String> {
        Err(NO.into())
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::HSTRING;
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    fn e(m: &str) -> impl Fn(windows::core::Error) -> String + '_ {
        move |x| format!("{m}: {x}")
    }

    /// An open MP4 being written. Create, push and finish it on one thread.
    pub struct Writer {
        // Taken (released) before Media Foundation is shut down.
        writer: Option<IMFSinkWriter>,
        stream: u32,
        width: u32,
        height: u32,
        rate: (u32, u32),
        count: u64,
    }

    impl Writer {
        /// Width and height must be even (H.264).
        pub fn new(out: &Path, width: u32, height: u32, fps: f64) -> Result<Writer, String> {
            if !width.is_multiple_of(2) || !height.is_multiple_of(2) || width == 0 || height == 0 {
                return Err("the width and height must be even and more than 0".into());
            }
            if !(fps > 0.0 && fps <= 240.0) {
                return Err("the frame rate must be over 0 and at most 240".into());
            }
            let rate = rate(fps);
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(e("Media Foundation"))?;
                let r = (|| -> Result<(IMFSinkWriter, u32), String> {
                    let writer =
                        MFCreateSinkWriterFromURL(&HSTRING::from(out.as_os_str()), None, None)
                            .map_err(e("sink writer"))?;
                    let size = ((width as u64) << 32) | height as u64;
                    let fr = ((rate.0 as u64) << 32) | rate.1 as u64;
                    let set = |t: &IMFMediaType, sub: &windows::core::GUID| -> Result<(), String> {
                        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                            .map_err(e("type"))?;
                        t.SetGUID(&MF_MT_SUBTYPE, sub).map_err(e("type"))?;
                        t.SetUINT64(&MF_MT_FRAME_SIZE, size).map_err(e("type"))?;
                        t.SetUINT64(&MF_MT_FRAME_RATE, fr).map_err(e("type"))?;
                        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)
                            .map_err(e("type"))?;
                        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
                            .map_err(e("type"))?;
                        Ok(())
                    };
                    let outt = MFCreateMediaType().map_err(e("type"))?;
                    set(&outt, &MFVideoFormat_H264)?;
                    outt.SetUINT32(&MF_MT_AVG_BITRATE, BITRATE)
                        .map_err(e("type"))?;
                    let stream = writer.AddStream(&outt).map_err(e("H.264 stream"))?;
                    let int = MFCreateMediaType().map_err(e("type"))?;
                    set(&int, &MFVideoFormat_RGB32)?;
                    // Positive stride: rows top to bottom.
                    int.SetUINT32(&MF_MT_DEFAULT_STRIDE, width * 4)
                        .map_err(e("type"))?;
                    writer
                        .SetInputMediaType(stream, &int, None)
                        .map_err(e("input type"))?;
                    writer.BeginWriting().map_err(e("begin"))?;
                    Ok((writer, stream))
                })();
                match r {
                    Ok((writer, stream)) => Ok(Writer {
                        writer: Some(writer),
                        stream,
                        width,
                        height,
                        rate,
                        count: 0,
                    }),
                    Err(x) => {
                        let _ = MFShutdown();
                        Err(x)
                    }
                }
            }
        }

        /// 100 ns units at frame `k`.
        fn time(&self, k: u64) -> i64 {
            (k as i128 * 10_000_000 * self.rate.1 as i128 / self.rate.0 as i128) as i64
        }

        /// One frame: BGRX, rows top to bottom.
        pub fn push_bgrx(&mut self, bgrx: &[u8]) -> Result<(), String> {
            let len = self.width * self.height * 4;
            if bgrx.len() != len as usize {
                return Err(format!("a frame must be {} bytes, not {}", len, bgrx.len()));
            }
            let (t0, t1) = (self.time(self.count), self.time(self.count + 1));
            unsafe {
                let buf = MFCreateMemoryBuffer(len).map_err(e("buffer"))?;
                let mut ptr = std::ptr::null_mut::<u8>();
                buf.Lock(&mut ptr, None, None).map_err(e("buffer"))?;
                std::ptr::copy_nonoverlapping(bgrx.as_ptr(), ptr, len as usize);
                buf.Unlock().map_err(e("buffer"))?;
                buf.SetCurrentLength(len).map_err(e("buffer"))?;
                let s = MFCreateSample().map_err(e("sample"))?;
                s.AddBuffer(&buf).map_err(e("sample"))?;
                s.SetSampleTime(t0).map_err(e("sample"))?;
                s.SetSampleDuration(t1 - t0).map_err(e("sample"))?;
                self.writer
                    .as_ref()
                    .ok_or("closed")?
                    .WriteSample(self.stream, &s)
                    .map_err(e("write"))?;
            }
            self.count += 1;
            Ok(())
        }

        /// Close the file; the number of frames written.
        pub fn finish(self) -> Result<u64, String> {
            let r = match &self.writer {
                Some(w) => unsafe { w.Finalize() }.map_err(e("finalize")),
                None => Err("closed".into()),
            };
            let n = self.count;
            drop(self);
            r.map(|_| n)
        }
    }

    impl Drop for Writer {
        fn drop(&mut self) {
            self.writer.take();
            unsafe {
                let _ = MFShutdown();
            }
        }
    }

    /// Count a file's video frames (without decoding them) and read its size and duration.
    pub fn probe(path: &Path) -> Result<Probe, String> {
        const FIRST_VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        const MEDIA_SOURCE: u32 = MF_SOURCE_READER_MEDIASOURCE.0 as u32;
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(e("Media Foundation"))?;
            let r = (|| -> Result<Probe, String> {
                let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), None)
                    .map_err(e("could not open the video"))?;
                let cur = reader
                    .GetCurrentMediaType(FIRST_VIDEO)
                    .map_err(e("media type"))?;
                let size = cur.GetUINT64(&MF_MT_FRAME_SIZE).map_err(e("frame size"))?;
                let duration = reader
                    .GetPresentationAttribute(MEDIA_SOURCE, &MF_PD_DURATION)
                    .ok()
                    .and_then(|v: PROPVARIANT| u64::try_from(&v).ok())
                    .map_or(0.0, |d| d as f64 / 1e7);
                let mut frames = 0u64;
                loop {
                    let (mut flags, mut sample) = (0u32, None::<IMFSample>);
                    reader
                        .ReadSample(
                            FIRST_VIDEO,
                            0,
                            None,
                            Some(&mut flags),
                            None,
                            Some(&mut sample),
                        )
                        .map_err(e("reading a frame"))?;
                    if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                        break;
                    }
                    if sample.is_some() {
                        frames += 1;
                    }
                }
                Ok(Probe {
                    width: (size >> 32) as u32,
                    height: (size & 0xFFFF_FFFF) as u32,
                    frames,
                    duration,
                })
            })();
            let _ = MFShutdown();
            r
        }
    }
}

pub use imp::{probe, Writer};

/// RGBA (rows top to bottom, as a canvas gives them) to BGRX.
pub fn rgba_to_bgrx(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.as_chunks::<4>().0 {
        out.extend([px[2], px[1], px[0], 255]);
    }
    out
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn a_written_file_reads_back_with_its_frames_and_duration() {
        let dir = std::env::temp_dir().join(format!("locus-mp4-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("t.mp4");
        let (w, h, fps, n) = (320u32, 240u32, 30.0, 61u64);
        let mut wr = Writer::new(&out, w, h, fps).unwrap();
        for k in 0..n {
            // A bar moving across, so every frame differs.
            let mut rgba = vec![40u8; (w * h * 4) as usize];
            for y in 0..h {
                let x = (k as u32 * 4) % w;
                let i = ((y * w + x) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
            wr.push_bgrx(&rgba_to_bgrx(&rgba)).unwrap();
        }
        assert_eq!(wr.finish().unwrap(), n);
        let p = probe(&out).unwrap();
        assert_eq!((p.width, p.height, p.frames), (w, h, n), "{p:?}");
        assert!((p.duration - n as f64 / fps).abs() < 1.0 / fps, "{p:?}");
        assert_eq!(rate(29.97), (29970, 1000));
        std::fs::remove_dir_all(&dir).ok();
    }
}
