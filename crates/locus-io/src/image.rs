//! JPEG and PNG photos: dimensions and every EXIF field, verbatim.

use crate::stats::stem;
use crate::{parse_err, Result};
use locus_core::{Contents, ExifField, ImageInfo, ImageKind};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

pub(crate) fn inspect(path: &Path) -> Result<Contents> {
    let size = imagesize::size(path).map_err(|e| parse_err("image", e))?;
    let mut c = Contents::new("");
    let exif = match exif::Reader::new().read_from_container(&mut BufReader::new(File::open(path)?))
    {
        Ok(exif) => exif
            .fields()
            .map(|f| ExifField {
                ifd: f.ifd_num.to_string(),
                tag: f.tag.to_string(),
                value: f.display_value().with_unit(&exif).to_string(),
            })
            .collect(),
        Err(exif::Error::NotFound(_)) => vec![],
        Err(e) => {
            c.warnings.push(format!("EXIF data could not be read: {e}"));
            vec![]
        }
    };
    c.images.push(ImageInfo {
        name: stem(path),
        kind: ImageKind::Photo,
        width: size.width as u32,
        height: size.height as u32,
        scan: None,
        pose: None,
        exif,
    });
    Ok(c)
}
