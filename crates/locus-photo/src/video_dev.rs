//! Development and test utilities, not used by the app: an H.264 MP4 writer (Media Foundation's
//! sink writer) and an image loader that decodes and scales with Windows Imaging Component, so
//! tests can make videos without ffmpeg. Windows only.

use std::path::Path;

/// A frame: width, height, and BGRX pixels, rows top to bottom.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgrx: Vec<u8>,
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::{Interface, HSTRING};
    use windows::Win32::Foundation::GENERIC_READ;
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };

    fn e(m: &str) -> impl Fn(windows::core::Error) -> String + '_ {
        move |x| format!("{m}: {x}")
    }

    /// Write `frames` (all one size, even width and height) as an H.264 MP4 at `fps`.
    pub fn write_mp4(out: &Path, frames: &[Frame], fps: u32) -> Result<(), String> {
        let Some(first) = frames.first() else {
            return Err("no frames".into());
        };
        let (w, h) = (first.width, first.height);
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(e("Media Foundation"))?;
            let r = (|| -> Result<(), String> {
                let writer = MFCreateSinkWriterFromURL(&HSTRING::from(out.as_os_str()), None, None)
                    .map_err(e("sink writer"))?;
                let size = ((w as u64) << 32) | h as u64;
                let rate = ((fps as u64) << 32) | 1;
                let set = |t: &IMFMediaType, sub: &windows::core::GUID| -> Result<(), String> {
                    t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                        .map_err(e("type"))?;
                    t.SetGUID(&MF_MT_SUBTYPE, sub).map_err(e("type"))?;
                    t.SetUINT64(&MF_MT_FRAME_SIZE, size).map_err(e("type"))?;
                    t.SetUINT64(&MF_MT_FRAME_RATE, rate).map_err(e("type"))?;
                    t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)
                        .map_err(e("type"))?;
                    t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
                        .map_err(e("type"))?;
                    Ok(())
                };
                let outt = MFCreateMediaType().map_err(e("type"))?;
                set(&outt, &MFVideoFormat_H264)?;
                // A high bit rate: the frames are for measurement.
                outt.SetUINT32(&MF_MT_AVG_BITRATE, 40_000_000)
                    .map_err(e("type"))?;
                let stream = writer.AddStream(&outt).map_err(e("H.264 stream"))?;
                let int = MFCreateMediaType().map_err(e("type"))?;
                set(&int, &MFVideoFormat_RGB32)?;
                // Positive stride: rows top to bottom.
                int.SetUINT32(&MF_MT_DEFAULT_STRIDE, w * 4)
                    .map_err(e("type"))?;
                writer
                    .SetInputMediaType(stream, &int, None)
                    .map_err(e("input type"))?;
                writer.BeginWriting().map_err(e("begin"))?;
                let dur = 10_000_000i64 / fps as i64;
                for (k, f) in frames.iter().enumerate() {
                    if (f.width, f.height) != (w, h) {
                        return Err("frames differ in size".into());
                    }
                    let len = w * h * 4;
                    let buf = MFCreateMemoryBuffer(len).map_err(e("buffer"))?;
                    let mut ptr = std::ptr::null_mut::<u8>();
                    buf.Lock(&mut ptr, None, None).map_err(e("buffer"))?;
                    std::ptr::copy_nonoverlapping(f.bgrx.as_ptr(), ptr, len as usize);
                    buf.Unlock().map_err(e("buffer"))?;
                    buf.SetCurrentLength(len).map_err(e("buffer"))?;
                    let s = MFCreateSample().map_err(e("sample"))?;
                    s.AddBuffer(&buf).map_err(e("sample"))?;
                    s.SetSampleTime(k as i64 * dur).map_err(e("sample"))?;
                    s.SetSampleDuration(dur).map_err(e("sample"))?;
                    writer.WriteSample(stream, &s).map_err(e("write"))?;
                }
                writer.Finalize().map_err(e("finalize"))
            })();
            let _ = MFShutdown();
            r
        }
    }

    /// Decode an image file and scale it to `scale` of its size (rounded down to even
    /// dimensions, as H.264 needs), as BGRX.
    pub fn load_image(path: &Path, scale: f64) -> Result<Frame, String> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let f: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                    .map_err(e("WIC"))?;
            let dec = f
                .CreateDecoderFromFilename(
                    &HSTRING::from(path.as_os_str()),
                    None,
                    GENERIC_READ,
                    WICDecodeMetadataCacheOnDemand,
                )
                .map_err(e(&format!("could not decode {}", path.display())))?;
            let frame = dec.GetFrame(0).map_err(e("frame"))?;
            let (mut w0, mut h0) = (0u32, 0u32);
            frame.GetSize(&mut w0, &mut h0).map_err(e("size"))?;
            let (w, h) = (
                ((w0 as f64 * scale) as u32) & !1,
                ((h0 as f64 * scale) as u32) & !1,
            );
            let scaler = f.CreateBitmapScaler().map_err(e("scaler"))?;
            scaler
                .Initialize(&frame, w, h, WICBitmapInterpolationModeFant)
                .map_err(e("scaler"))?;
            let conv = f.CreateFormatConverter().map_err(e("converter"))?;
            conv.Initialize(
                &scaler.cast::<IWICBitmapSource>().map_err(e("cast"))?,
                &GUID_WICPixelFormat32bppBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .map_err(e("converter"))?;
            let mut bgrx = vec![0u8; (w * h * 4) as usize];
            conv.CopyPixels(std::ptr::null(), w * 4, &mut bgrx)
                .map_err(e("pixels"))?;
            Ok(Frame {
                width: w,
                height: h,
                bgrx,
            })
        }
    }
}

#[cfg(windows)]
pub use imp::{load_image, write_mp4};

#[cfg(not(windows))]
pub fn write_mp4(_: &Path, _: &[Frame], _: u32) -> Result<(), String> {
    Err("writing MP4 needs Windows Media Foundation".into())
}
