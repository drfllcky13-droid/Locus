//! A photo's GPS position from its EXIF GPS IFD, and the RTK tags DJI drones write in XMP.
//! Stdlib only: JPEG APP1 segments, a TIFF header and IFDs (Exif 2.32, CIPA DC-008), and the
//! XMP packet read as text.

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GpsTags {
    /// WGS84 latitude and longitude (°, north and east positive) and altitude (m; above sea
    /// level as the EXIF altitude reference says).
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude: Option<f64>,
    /// GPSDOP, and GPSHPositioningError (m).
    pub dop: Option<f64>,
    pub horizontal_error: Option<f64>,
    /// DJI XMP: RTK fix flag (50 = fixed) and the reported 1σ of latitude, longitude and
    /// height (m); the absolute altitude (m).
    pub rtk_flag: Option<i64>,
    pub rtk_std: Option<[f64; 3]>,
    pub absolute_altitude: Option<f64>,
}

struct Tiff<'a> {
    d: &'a [u8],
    le: bool,
}

impl Tiff<'_> {
    fn u16(&self, o: usize) -> Option<u16> {
        let b = self.d.get(o..o + 2)?;
        Some(if self.le {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    }
    fn u32(&self, o: usize) -> Option<u32> {
        let b: [u8; 4] = self.d.get(o..o + 4)?.try_into().ok()?;
        Some(if self.le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    }
    /// The entries of the IFD at `o`: (tag, type, count, value offset field position).
    fn ifd(&self, o: usize) -> Vec<(u16, u16, u32, usize)> {
        let Some(n) = self.u16(o) else { return vec![] };
        (0..n as usize)
            .filter_map(|k| {
                let e = o + 2 + 12 * k;
                Some((self.u16(e)?, self.u16(e + 2)?, self.u32(e + 4)?, e + 8))
            })
            .collect()
    }
    /// Unsigned rationals (type 5) of an entry.
    fn rationals(&self, count: u32, at: usize) -> Option<Vec<f64>> {
        let o = self.u32(at)? as usize;
        (0..count as usize)
            .map(|k| {
                let (n, d) = (self.u32(o + 8 * k)?, self.u32(o + 8 * k + 4)?);
                (d != 0).then(|| n as f64 / d as f64)
            })
            .collect()
    }
    /// An ASCII or BYTE value stored in the entry itself (count ≤ 4).
    fn inline_byte(&self, at: usize) -> Option<u8> {
        self.d.get(at).copied()
    }
}

/// The EXIF GPS tags from a TIFF block (the APP1 payload after "Exif\0\0").
fn gps_from_tiff(d: &[u8], g: &mut GpsTags) -> Option<()> {
    let le = match d.get(0..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let t = Tiff { d, le };
    let ifd0 = t.u32(4)? as usize;
    let gps = t.ifd(ifd0).into_iter().find(|e| e.0 == 0x8825)?;
    let gps_at = t.u32(gps.3)? as usize;
    let (mut lat_ref, mut lon_ref, mut alt_ref) = (b'N', b'E', 0u8);
    for (tag, ty, count, at) in t.ifd(gps_at) {
        let dms = |v: Vec<f64>| {
            v.first().copied().unwrap_or(0.0)
                + v.get(1).copied().unwrap_or(0.0) / 60.0
                + v.get(2).copied().unwrap_or(0.0) / 3600.0
        };
        match (tag, ty) {
            (1, 2) => lat_ref = t.inline_byte(at)?,
            (3, 2) => lon_ref = t.inline_byte(at)?,
            (5, 1) => alt_ref = t.inline_byte(at)?,
            (2, 5) => g.latitude = t.rationals(count, at).map(dms),
            (4, 5) => g.longitude = t.rationals(count, at).map(dms),
            (6, 5) => g.altitude = t.rationals(1, at).map(|v| v[0]),
            (11, 5) => g.dop = t.rationals(1, at).map(|v| v[0]),
            (31, 5) => g.horizontal_error = t.rationals(1, at).map(|v| v[0]),
            _ => {}
        }
    }
    if lat_ref == b'S' {
        g.latitude = g.latitude.map(|v| -v);
    }
    if lon_ref == b'W' {
        g.longitude = g.longitude.map(|v| -v);
    }
    if alt_ref == 1 {
        g.altitude = g.altitude.map(|v| -v);
    }
    Some(())
}

/// An XMP attribute's value, `name="value"` (any namespace prefix given in `name`).
fn xmp_attr(x: &str, name: &str) -> Option<f64> {
    let i = x.find(&format!("{name}=\""))? + name.len() + 2;
    let j = x[i..].find('"')? + i;
    x[i..j].trim().trim_start_matches('+').parse().ok()
}

/// GPS and RTK tags from a JPEG's bytes. Missing tags are `None`; a file with no EXIF or XMP
/// gives all `None`.
pub fn read_jpeg(bytes: &[u8]) -> GpsTags {
    let mut g = GpsTags::default();
    if bytes.get(0..2) != Some(&[0xFF, 0xD8]) {
        return g;
    }
    let mut o = 2;
    while o + 4 <= bytes.len() && bytes[o] == 0xFF {
        let marker = bytes[o + 1];
        if marker == 0xDA || marker == 0xD9 {
            break; // image data: no more metadata
        }
        let len = u16::from_be_bytes([bytes[o + 2], bytes[o + 3]]) as usize;
        let Some(seg) = bytes.get(o + 4..o + 2 + len) else {
            break;
        };
        if marker == 0xE1 {
            if let Some(tiff) = seg.strip_prefix(b"Exif\0\0") {
                gps_from_tiff(tiff, &mut g);
            } else if let Some(x) = seg.strip_prefix(b"http://ns.adobe.com/xap/1.0/\0") {
                let x = String::from_utf8_lossy(x);
                g.rtk_flag = xmp_attr(&x, "drone-dji:RtkFlag").map(|v| v as i64);
                if let (Some(a), Some(b), Some(c)) = (
                    xmp_attr(&x, "drone-dji:RtkStdLat"),
                    xmp_attr(&x, "drone-dji:RtkStdLon"),
                    xmp_attr(&x, "drone-dji:RtkStdHgt"),
                ) {
                    g.rtk_std = Some([a, b, c]);
                }
                g.absolute_altitude = xmp_attr(&x, "drone-dji:AbsoluteAltitude");
            }
        }
        o += 2 + len;
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal little-endian EXIF block: IFD0 with a GPS pointer, and a GPS IFD with
    /// 41° 24' 12.2" S, 2° 10' 26.5" E, 123.4 m, DOP 1.5, horizontal error 0.02 m.
    fn exif() -> Vec<u8> {
        let mut t = b"II*\0".to_vec();
        // IFD0 at 8, with one entry: the GPS pointer, to 26.
        t.extend(8u32.to_le_bytes());
        t.extend(1u16.to_le_bytes());
        t.extend(0x8825u16.to_le_bytes());
        t.extend(4u16.to_le_bytes());
        t.extend(1u32.to_le_bytes());
        t.extend(26u32.to_le_bytes());
        t.extend(0u32.to_le_bytes());
        assert_eq!(t.len(), 26);
        let n = 8u16;
        let data = 26 + 2 + 12 * n as usize + 4; // rationals after the GPS IFD
        let mut rats: Vec<u8> = vec![];
        let entry = |t: &mut Vec<u8>, tag: u16, ty: u16, count: u32, value: [u8; 4]| {
            t.extend(tag.to_le_bytes());
            t.extend(ty.to_le_bytes());
            t.extend(count.to_le_bytes());
            t.extend(value);
        };
        let rat = |rats: &mut Vec<u8>, v: &[(u32, u32)]| -> [u8; 4] {
            let at = (data + rats.len()) as u32;
            for (a, b) in v {
                rats.extend(a.to_le_bytes());
                rats.extend(b.to_le_bytes());
            }
            at.to_le_bytes()
        };
        t.extend(n.to_le_bytes());
        entry(&mut t, 1, 2, 2, *b"S\0\0\0");
        let v = rat(&mut rats, &[(41, 1), (24, 1), (122, 10)]);
        entry(&mut t, 2, 5, 3, v);
        entry(&mut t, 3, 2, 2, *b"E\0\0\0");
        let v = rat(&mut rats, &[(2, 1), (10, 1), (265, 10)]);
        entry(&mut t, 4, 5, 3, v);
        entry(&mut t, 5, 1, 1, [0, 0, 0, 0]);
        let v = rat(&mut rats, &[(1234, 10)]);
        entry(&mut t, 6, 5, 1, v);
        let v = rat(&mut rats, &[(15, 10)]);
        entry(&mut t, 11, 5, 1, v);
        let v = rat(&mut rats, &[(2, 100)]);
        entry(&mut t, 31, 5, 1, v);
        t.extend(0u32.to_le_bytes());
        t.extend(rats);
        t
    }

    fn jpeg(segments: &[Vec<u8>]) -> Vec<u8> {
        let mut j = vec![0xFF, 0xD8];
        for s in segments {
            j.extend([0xFF, 0xE1]);
            j.extend(((s.len() + 2) as u16).to_be_bytes());
            j.extend(s);
        }
        j.extend([0xFF, 0xDA, 0, 2, 0xFF, 0xD9]);
        j
    }

    #[test]
    fn gps_and_rtk_tags_are_read() {
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(exif());
        let mut xmp = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
        xmp.extend(
            br#"<x:xmpmeta><rdf:Description drone-dji:AbsoluteAltitude="+131.52" drone-dji:RtkFlag="50" drone-dji:RtkStdLon="0.01234" drone-dji:RtkStdLat="0.01100" drone-dji:RtkStdHgt="0.02500"/></x:xmpmeta>"#,
        );
        let g = read_jpeg(&jpeg(&[app1, xmp]));
        let lat = -(41.0 + 24.0 / 60.0 + 12.2 / 3600.0);
        assert!((g.latitude.unwrap() - lat).abs() < 1e-12, "{g:?}");
        assert!((g.longitude.unwrap() - (2.0 + 10.0 / 60.0 + 26.5 / 3600.0)).abs() < 1e-12);
        assert_eq!(g.altitude, Some(123.4));
        assert_eq!(g.dop, Some(1.5));
        assert_eq!(g.horizontal_error, Some(0.02));
        assert_eq!(g.rtk_flag, Some(50));
        assert_eq!(g.rtk_std, Some([0.011, 0.01234, 0.025]));
        assert_eq!(g.absolute_altitude, Some(131.52));
        // No metadata, or not a JPEG: nothing, and no panic.
        assert_eq!(read_jpeg(&jpeg(&[])), GpsTags::default());
        assert_eq!(read_jpeg(b"not a jpeg"), GpsTags::default());
        assert_eq!(
            read_jpeg(&[0xFF, 0xD8, 0xFF, 0xE1, 0x40]),
            GpsTags::default()
        );
    }
}
