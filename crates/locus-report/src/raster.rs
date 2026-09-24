//! Pages as images at a chosen resolution: PNG (with its pHYs resolution) and baseline TIFF
//! (uncompressed RGB, with its X and Y resolution in dots per inch), so a printed image keeps
//! its scale.

/// An image: width, height and RGBA pixels, rows top to bottom.
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The highest resolution offered (dots per inch): A3 at 600 dpi is 7016 × 9921 pixels.
pub const MAX_DPI: f64 = 600.0;

/// Render every page of a compiled document at `dpi`, on white.
pub(crate) fn pages(doc: &typst_layout::PagedDocument, dpi: f64) -> Vec<Raster> {
    doc.pages()
        .iter()
        .map(|p| {
            let opts = typst_render::RenderOptions {
                pixel_per_pt: (dpi / 72.0).into(),
                ..Default::default()
            };
            let px = typst_render::render(p, &opts);
            // Premultiplied RGBA over white (a page's own fill is usually opaque already).
            let mut rgba = px.data().to_vec();
            for c in rgba.as_chunks_mut::<4>().0 {
                let a = c[3] as u16;
                for v in &mut c[..3] {
                    *v = (*v as u16 + (255 - a)).min(255) as u8;
                }
                c[3] = 255;
            }
            Raster {
                width: px.width(),
                height: px.height(),
                rgba,
            }
        })
        .collect()
}

fn rgb(r: &Raster) -> Vec<u8> {
    r.rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect()
}

pub fn png(r: &Raster, dpi: f64) -> Result<Vec<u8>, String> {
    let mut out = vec![];
    let mut enc = png::Encoder::new(&mut out, r.width, r.height);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let per_m = (dpi / 0.0254).round() as u32;
    enc.set_pixel_dims(Some(png::PixelDimensions {
        xppu: per_m,
        yppu: per_m,
        unit: png::Unit::Meter,
    }));
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(&rgb(r)).map_err(|e| e.to_string())?;
    drop(w);
    Ok(out)
}

/// Baseline TIFF: little-endian, one strip, 8-bit RGB, uncompressed.
pub fn tiff(r: &Raster, dpi: f64) -> Vec<u8> {
    let pixels = rgb(r);
    // Header, then the directory, then its out-of-line values, then the pixels.
    let tags: u16 = 12;
    let ifd = 8u32;
    let bps_at = ifd + 2 + 12 * tags as u32 + 4; // 3 × u16
    let xres_at = bps_at + 6; // rational
    let yres_at = xres_at + 8;
    let data_at = yres_at + 8;
    let mut b = Vec::with_capacity(data_at as usize + pixels.len());
    b.extend(b"II");
    b.extend(42u16.to_le_bytes());
    b.extend(ifd.to_le_bytes());
    b.extend(tags.to_le_bytes());
    let mut tag = |id: u16, typ: u16, count: u32, value: u32| {
        b.extend(id.to_le_bytes());
        b.extend(typ.to_le_bytes());
        b.extend(count.to_le_bytes());
        b.extend(value.to_le_bytes());
    };
    const SHORT: u16 = 3;
    const LONG: u16 = 4;
    const RATIONAL: u16 = 5;
    tag(256, LONG, 1, r.width); // ImageWidth
    tag(257, LONG, 1, r.height); // ImageLength
    tag(258, SHORT, 3, bps_at); // BitsPerSample 8, 8, 8
    tag(259, SHORT, 1, 1); // Compression: none
    tag(262, SHORT, 1, 2); // PhotometricInterpretation: RGB
    tag(273, LONG, 1, data_at); // StripOffsets
    tag(277, SHORT, 1, 3); // SamplesPerPixel
    tag(278, LONG, 1, r.height); // RowsPerStrip
    tag(279, LONG, 1, pixels.len() as u32); // StripByteCounts
    tag(282, RATIONAL, 1, xres_at); // XResolution
    tag(283, RATIONAL, 1, yres_at); // YResolution
    tag(296, SHORT, 1, 2); // ResolutionUnit: inch
    b.extend(0u32.to_le_bytes()); // no next directory
    for _ in 0..3 {
        b.extend(8u16.to_le_bytes());
    }
    // The resolution as a rational, to 1/1000 dpi.
    let num = (dpi * 1000.0).round() as u32;
    for _ in 0..2 {
        b.extend(num.to_le_bytes());
        b.extend(1000u32.to_le_bytes());
    }
    debug_assert_eq!(b.len() as u32, data_at);
    b.extend(pixels);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Raster {
        // 3 × 2: red, green, blue / white, black, grey.
        let px: [[u8; 4]; 6] = [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 255, 255],
            [0, 0, 0, 255],
            [128, 128, 128, 255],
        ];
        Raster {
            width: 3,
            height: 2,
            rgba: px.concat(),
        }
    }

    #[test]
    fn png_keeps_its_pixels_and_resolution() {
        let bytes = png(&sample(), 300.0).unwrap();
        let dec = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut r = dec.read_info().unwrap();
        let dims = r.info().pixel_dims.unwrap();
        // 300 dpi is 11811 pixels per metre.
        assert_eq!((dims.xppu, dims.yppu), (11811, 11811));
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        r.next_frame(&mut buf).unwrap();
        assert_eq!(&buf[..9], &[255, 0, 0, 0, 255, 0, 0, 0, 255]);
    }

    #[test]
    fn tiff_is_baseline_rgb_with_its_resolution() {
        let t = tiff(&sample(), 300.0);
        assert_eq!(&t[..4], b"II*\0");
        let u16at = |i: usize| u16::from_le_bytes([t[i], t[i + 1]]);
        let u32at = |i: usize| u32::from_le_bytes([t[i], t[i + 1], t[i + 2], t[i + 3]]);
        let n = u16at(8) as usize;
        let find = |id: u16| {
            (0..n)
                .map(|k| 10 + 12 * k)
                .find(|&o| u16at(o) == id)
                .unwrap()
        };
        assert_eq!(u32at(find(256) + 8), 3);
        assert_eq!(u32at(find(257) + 8), 2);
        let xres = u32at(find(282) + 8) as usize;
        assert_eq!(u32at(xres) as f64 / u32at(xres + 4) as f64, 300.0);
        assert_eq!(u16at(find(296) + 8), 2);
        let data = u32at(find(273) + 8) as usize;
        assert_eq!(&t[data..data + 6], &[255, 0, 0, 0, 255, 0]);
        assert_eq!(t.len() - data, 18);
    }
}
