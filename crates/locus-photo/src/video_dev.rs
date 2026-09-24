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
        let mut w = crate::mp4::Writer::new(out, first.width, first.height, fps as f64)?;
        for f in frames {
            if (f.width, f.height) != (first.width, first.height) {
                return Err("frames differ in size".into());
            }
            w.push_bgrx(&f.bgrx)?;
        }
        w.finish().map(|_| ())
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
pub fn load_image(_: &Path, _: f64) -> Result<Frame, String> {
    Err("loading images for video needs Windows Imaging Component".into())
}

#[cfg(not(windows))]
pub fn write_mp4(_: &Path, _: &[Frame], _: u32) -> Result<(), String> {
    Err("writing MP4 needs Windows Media Foundation".into())
}
