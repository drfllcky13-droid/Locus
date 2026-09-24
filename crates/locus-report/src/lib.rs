//! Typst templates and PDF generation.
//!
//! Reports are Typst documents compiled in-process by a minimal [`World`]: one template, one
//! JSON data file, no file system access, no packages, no clock, and only the fonts bundled
//! here (the Go fonts, BSD-3-Clause). The data is fully formatted before it reaches the
//! template, so the numbers in a report are exactly the strings computed in Rust, and
//! [`Rendered::text`] lets tests check them against the laid-out document.

pub mod analysis;
pub mod animation;
pub mod bloodstain;
pub mod camera;
pub mod case;
pub mod crash;
pub mod diagram;
pub mod dxf;
pub mod photo;
pub mod raster;
pub mod registration;
pub mod trajectory;

use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::layout::{Frame, FrameItem};
use typst::syntax::{FileId, Source};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};
use typst_layout::PagedDocument;

const FONTS: [&[u8]; 4] = [
    include_bytes!("../fonts/Go-Regular.ttf"),
    include_bytes!("../fonts/Go-Bold.ttf"),
    include_bytes!("../fonts/Go-Italic.ttf"),
    include_bytes!("../fonts/Go-Mono.ttf"),
];

/// The fonts reports may use: Go, Go Mono.
pub fn fonts() -> Vec<Font> {
    FONTS
        .iter()
        .flat_map(|data| Font::iter(Bytes::new(data.to_vec())))
        .collect()
}

struct MemWorld {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    main: Source,
    data: Bytes,
    /// Further read-only files the template may use (symbol images), by path.
    files: Vec<(String, Bytes)>,
}

impl World for MemWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }
    fn main(&self) -> FileId {
        self.main.id()
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() {
            Ok(self.main.clone())
        } else {
            Err(FileError::AccessDenied)
        }
    }
    /// Only `data.json` and the files given to [`render_with`] exist.
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        let path = id.get();
        if !matches!(path.root(), typst::syntax::VirtualRoot::Project) {
            return Err(FileError::AccessDenied);
        }
        let name = path.vpath().get_without_slash();
        if name == "data.json" {
            return Ok(self.data.clone());
        }
        self.files
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, b)| b.clone())
            .ok_or(FileError::AccessDenied)
    }
    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index).cloned()
    }
    /// Reports must be reproducible: no clock inside the document; dates come in the data.
    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        None
    }
}

pub struct Rendered {
    pub pdf: Vec<u8>,
    /// The text as laid out, in reading order within each frame, pages separated by form
    /// feeds. For checking what the report says.
    pub text: String,
}

/// Compile `template` with `data` (the bytes of `data.json`) to PDF.
pub fn render(template: &str, data: Vec<u8>) -> Result<Rendered, String> {
    render_with(template, data, vec![])
}

/// As [`render`], with further read-only files the template can load by name.
pub fn render_with(
    template: &str,
    data: Vec<u8>,
    files: Vec<(String, Vec<u8>)>,
) -> Result<Rendered, String> {
    let doc = compile(template, data, files)?;
    let mut text = String::new();
    for page in doc.pages() {
        let mut last = None;
        collect(&page.frame, &mut text, &mut last);
        text.push('\u{c}');
    }
    let pdf = typst_pdf::pdf(&doc, &typst_pdf::PdfOptions::default())
        .map_err(|e| format!("PDF export failed: {e:?}"))?;
    Ok(Rendered { pdf, text })
}

fn compile(
    template: &str,
    data: Vec<u8>,
    files: Vec<(String, Vec<u8>)>,
) -> Result<PagedDocument, String> {
    let fonts = fonts();
    let world = MemWorld {
        library: LazyHash::new(Library::default()),
        book: LazyHash::new(FontBook::from_fonts(&fonts)),
        fonts,
        main: Source::detached(template),
        data: Bytes::new(data),
        files: files.into_iter().map(|(n, b)| (n, Bytes::new(b))).collect(),
    };
    let warned = typst::compile::<PagedDocument>(&world);
    // A missing font or glyph is only a warning in Typst and the text silently vanishes; in
    // an evidence report that must be an error.
    if !warned.warnings.is_empty() {
        return Err(format!("report template warnings: {:?}", warned.warnings));
    }
    warned
        .output
        .map_err(|e| format!("report template errors: {e:?}"))
}

/// Page images of a report (tests and visual checks only).
#[cfg(test)]
pub(crate) fn pages_png(
    template: &str,
    data: Vec<u8>,
    files: Vec<(String, Vec<u8>)>,
) -> Vec<Vec<u8>> {
    let doc = compile(template, data, files).unwrap();
    doc.pages()
        .iter()
        .map(|p| {
            let opts = typst_render::RenderOptions {
                pixel_per_pt: 1.5.into(),
                ..Default::default()
            };
            typst_render::render(p, &opts).encode_png().unwrap()
        })
        .collect()
}

/// Text runs in order. Runs continuing on the same line (a run split by shaping, like "µ" and
/// "m") are joined; anything else is separated by a space. `last` is the end of the previous
/// run within the current group: (y, x) in points.
fn collect(frame: &Frame, out: &mut String, last: &mut Option<(f64, f64)>) {
    for (pos, item) in frame.items() {
        match item {
            FrameItem::Group(g) => {
                *last = None;
                collect(&g.frame, out, &mut None);
                out.push(' ');
            }
            FrameItem::Text(t) => {
                let (y, x) = (pos.y.to_pt(), pos.x.to_pt());
                let joined =
                    last.is_some_and(|(ly, lx)| (ly - y).abs() < 0.01 && (lx - x).abs() < 0.5);
                if !joined && !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push_str(&t.text);
                *last = Some((y, x + t.width().to_pt()));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_symbols_with_the_bundled_fonts() {
        let r = render(
            "#set text(font: \"Go\")\n0.22 mm, 0.0008°, χ², 5 µm ± 2 #text(font: \"Go Mono\")[abc]",
            b"{}".to_vec(),
        )
        .unwrap();
        assert!(r.pdf.starts_with(b"%PDF"));
        for s in ["0.22", "0.0008°", "χ²", "µm", "±", "abc"] {
            assert!(r.text.contains(s), "{s:?} missing from {:?}", r.text);
        }
    }

    #[test]
    fn a_font_that_is_not_bundled_is_an_error_not_missing_text() {
        assert!(render("#set text(font: \"Nonexistent\")\nx", b"{}".to_vec()).is_err());
    }

    #[test]
    fn only_data_json_is_readable() {
        assert!(render(
            "#set text(font: \"Go\")\n#json(\"data.json\").a",
            br#"{"a": 1}"#.to_vec()
        )
        .is_ok());
        assert!(render(
            "#set text(font: \"Go\")\n#read(\"other.txt\")",
            b"{}".to_vec()
        )
        .is_err());
    }
}
