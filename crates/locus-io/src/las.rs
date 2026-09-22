//! LAS and LAZ, streamed through the `las` crate.
//!
//! The linear unit is taken from the CRS stored in the file (GeoTIFF keys or WKT).
//! Geographic (degree) coordinates are refused: they are not lengths.

use crate::stats::{stem, ScanStats};
use crate::{parse_err, Progress, ProgressFn, Result, Stage};
use las::crs::GeoTiffData;
use locus_core::{Contents, LinearUnit, IDENTITY};
use std::path::Path;

const FMT: &str = "LAS";

/// GeoTIFF / EPSG unit-of-measure codes.
fn unit_from_code(code: u16) -> Option<LinearUnit> {
    match code {
        9001 => Some(LinearUnit::Meter),
        9002 => Some(LinearUnit::Foot),
        9003 => Some(LinearUnit::UsSurveyFoot),
        _ => None,
    }
}

/// Meters-per-unit factor as written in WKT. Files often truncate factors (US survey foot
/// as 0.304800609601219 or 0.30480061), so match to 1e-8 relative; the closest distinct
/// units, foot and US survey foot, differ by 2e-6.
fn unit_from_factor(f: f64) -> Option<LinearUnit> {
    [
        LinearUnit::Meter,
        LinearUnit::Foot,
        LinearUnit::UsSurveyFoot,
        LinearUnit::Centimeter,
        LinearUnit::Millimeter,
    ]
    .into_iter()
    .find(|u| (u.meters() - f).abs() <= 1e-8 * u.meters())
}

/// What the file says about its coordinate system.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct CrsInfo {
    pub crs: Option<String>,
    pub geographic: bool,
    pub horizontal: Option<Result<LinearUnit, String>>,
    pub vertical: Option<Result<LinearUnit, String>>,
}

/// Remove every `KEYWORD[...]` block (with nested brackets) for the given keywords.
fn strip_blocks(wkt: &str, keywords: &[&str]) -> String {
    let mut out = String::with_capacity(wkt.len());
    let mut i = 0;
    while i < wkt.len() {
        let at_word_start = wkt[..i]
            .chars()
            .last()
            .is_none_or(|c| !c.is_ascii_alphabetic());
        let hit = at_word_start
            && keywords
                .iter()
                .any(|k| wkt[i..].starts_with(k) && wkt[i + k.len()..].starts_with('['));
        if hit {
            let mut depth = 0;
            for (j, c) in wkt[i..].char_indices() {
                match c {
                    '[' => depth += 1,
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            i += j + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if depth != 0 {
                break; // unbalanced: drop the rest
            }
            continue;
        }
        let c = wkt[i..].chars().next().unwrap();
        out.push(c);
        i += c.len_utf8();
    }
    out
}

#[test]
fn strip_blocks_nested_and_ordered() {
    let s = strip_blocks("A[P[x,U[1]],E[y],K[z],P[w]]", &["E", "P"]);
    assert_eq!(s, "A[,,K[z],]");
    assert_eq!(strip_blocks("XP[1],P[2]", &["P"]), "XP[1],");
}

/// Axis length units in a WKT1 or WKT2 string, in order of appearance. Units belonging to
/// the ellipsoid, projection parameters and the base geographic CRS are ignored, as are
/// angular and scale units. Unrecognized factors come back as `Err(description)`.
pub(crate) fn wkt_length_units(wkt: &str) -> Vec<Result<LinearUnit, String>> {
    let wkt = strip_blocks(
        wkt,
        &[
            "ELLIPSOID",
            "SPHEROID",
            "PARAMETER",
            "BASEGEOGCRS",
            "BASEGEODCRS",
            "GEOGCS",
            "DATUM",
        ],
    );
    let mut out = vec![];
    let mut rest = wkt.as_str();
    while let Some(i) = rest.find("UNIT[") {
        let keyword_start = rest[..i]
            .rfind(|c: char| !c.is_ascii_alphabetic())
            .map_or(0, |j| j + 1);
        let keyword = &rest[keyword_start..i];
        let body = &rest[i + 5..];
        rest = body;
        if matches!(keyword, "ANGLE" | "SCALE" | "TIME" | "PARAMETRIC") {
            continue;
        }
        let Some(name) = body.split('"').nth(1) else {
            continue;
        };
        let lname = name.to_ascii_lowercase();
        if ["degree", "radian", "grad", "arc-second", "unity", "ppm"]
            .iter()
            .any(|a| lname.contains(a))
        {
            continue;
        }
        let after_name = body.splitn(3, '"').nth(2).unwrap_or_default();
        let factor = after_name
            .trim_start_matches([',', ' '])
            .split([',', ']'])
            .next()
            .and_then(|s| s.trim().parse::<f64>().ok());
        out.push(match factor.and_then(unit_from_factor) {
            Some(u) => Ok(u),
            None => Err(format!("unit \"{name}\" ({factor:?} m)")),
        });
    }
    out
}

pub(crate) fn crs_info(header: &las::Header) -> Result<CrsInfo> {
    let mut info = CrsInfo::default();
    if let Some(bytes) = header.get_wkt_crs_bytes() {
        let wkt = String::from_utf8_lossy(bytes)
            .trim_end_matches('\0')
            .trim()
            .to_string();
        let upper = wkt.to_ascii_uppercase();
        let projected = upper.contains("PROJCS[") || upper.contains("PROJCRS[");
        info.geographic = !projected
            && (upper.contains("GEOGCS[")
                || upper.contains("GEOGCRS[")
                || upper.contains("GEODCRS["));
        let units = wkt_length_units(&wkt);
        info.horizontal = units.first().cloned();
        if upper.contains("COMPD_CS[") || upper.contains("COMPOUNDCRS[") {
            // Horizontal CRS comes first, vertical last.
            info.vertical = units.last().cloned().filter(|_| units.len() > 1);
        } else if let Some(other) = units.iter().find(|u| Some(*u) != units.first()) {
            // Per-axis units that disagree are reported as a horizontal/vertical mismatch.
            info.vertical = Some(other.clone());
        }
        info.crs = Some(wkt);
    } else if let Some(gt) = header
        .get_geotiff_crs()
        .map_err(|e| parse_err(FMT, format!("unreadable GeoTIFF CRS keys: {e}")))?
    {
        let mut parts = vec![];
        for e in &gt.entries {
            let GeoTiffData::U16(v) = e.data else {
                continue;
            };
            let unit = || unit_from_code(v).ok_or_else(|| format!("unit code {v}"));
            match e.id {
                1024 => info.geographic = v == 2,
                3072 if (1024..=32766).contains(&v) => parts.push(format!("EPSG:{v}")),
                4096 if (1024..=32766).contains(&v) => parts.push(format!("EPSG:{v} (vertical)")),
                3076 => info.horizontal = Some(unit()),
                4099 => info.vertical = Some(unit()),
                _ => {}
            }
        }
        info.crs = (!parts.is_empty()).then(|| parts.join(" + "));
    }
    Ok(info)
}

/// Turn CRS facts into a declared unit and warnings, or refuse the file.
pub(crate) fn resolve_unit(info: &CrsInfo, c: &mut Contents) -> Result<()> {
    if info.geographic {
        return Err(parse_err(
            FMT,
            "coordinates are geographic (latitude/longitude in degrees). Reproject to a projected coordinate system before importing.",
        ));
    }
    match (&info.horizontal, &info.vertical) {
        (Some(Ok(h)), Some(Ok(v))) if h != v => {
            return Err(parse_err(
                FMT,
                format!("horizontal unit is {h:?} but vertical unit is {v:?}; mixed units are not supported"),
            ))
        }
        (Some(Ok(h)), Some(Ok(_))) => c.declared_unit = Some(*h),
        (Some(Ok(h)), None) => {
            c.declared_unit = Some(*h);
            c.warnings.push(format!(
                "the file states a horizontal unit ({h:?}) but no vertical unit; heights are assumed to be in the same unit"
            ));
        }
        (Some(Err(e)), _) | (_, Some(Err(e))) => c.warnings.push(format!(
            "the file's coordinate system uses an unrecognized {e}; choose the unit manually"
        )),
        (None, _) if info.crs.is_some() => c.warnings.push(
            "the file names a coordinate system but not its unit; choose the unit manually".into(),
        ),
        (None, _) => {}
    }
    Ok(())
}

pub(crate) fn inspect(path: &Path, progress: ProgressFn) -> Result<Contents> {
    let mut reader = las::Reader::from_path(path).map_err(|e| parse_err(FMT, e))?;
    let header = reader.header().clone();
    let mut c = Contents::new(FMT);
    let info = crs_info(&header)?;
    resolve_unit(&info, &mut c)?;
    c.crs = info.crs;

    let total = header.number_of_points();
    let mut stats = ScanStats::default();
    let mut pd = reader.read_points(0).map_err(|e| parse_err(FMT, e))?;
    loop {
        let n = reader
            .fill_points(1 << 20, &mut pd)
            .map_err(|e| parse_err(FMT, e))?;
        if n == 0 {
            break;
        }
        for ((x, y), z) in pd.x().zip(pd.y()).zip(pd.z()) {
            stats.point([x, y, z]);
        }
        progress(Progress {
            stage: Stage::Points,
            done: stats.count,
            total,
        });
    }
    if stats.count != total {
        c.warnings.push(format!(
            "header says {total} points, file contains {}",
            stats.count
        ));
    }
    let format = header.point_format();
    let mut attributes = vec!["intensity".to_string(), "classification".into()];
    if format.has_color {
        attributes.push("color".into());
    }
    if format.has_gps_time {
        attributes.push("gps_time".into());
    }
    c.scans
        .push(stats.into_scan(stem(path), IDENTITY, attributes));
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use LinearUnit::*;

    #[test]
    fn wkt1_projected_us_feet() {
        // Pennsylvania North, US survey feet, with a US-feet vertical datum.
        let wkt = r#"COMPD_CS["NAD83 / PA N (ftUS) + NAVD88 (ftUS)",PROJCS["NAD83 / Pennsylvania North (ftUS)",GEOGCS["NAD83",DATUM["North_American_Datum_1983",SPHEROID["GRS 1980",6378137,298.257222101]],PRIMEM["Greenwich",0],UNIT["degree",0.0174532925199433]],PROJECTION["Lambert_Conformal_Conic_2SP"],UNIT["US survey foot",0.304800609601219]],VERT_CS["NAVD88 height (ftUS)",VERT_DATUM["North American Vertical Datum 1988",2005],UNIT["US survey foot",0.304800609601219]]]"#;
        assert_eq!(
            wkt_length_units(wkt),
            vec![Ok(UsSurveyFoot), Ok(UsSurveyFoot)]
        );
        let intl = wkt.replace("US survey foot\",0.304800609601219", "foot\",0.3048");
        assert_eq!(wkt_length_units(&intl), vec![Ok(Foot), Ok(Foot)]);
        let odd = wkt.replace("0.304800609601219", "0.3");
        assert!(wkt_length_units(&odd).iter().all(|u| u.is_err()));
    }

    #[test]
    fn wkt2_meters_skips_angle_units() {
        let wkt = r#"PROJCRS["WGS 84 / UTM zone 18N",BASEGEOGCRS["WGS 84",DATUM["World Geodetic System 1984",ELLIPSOID["WGS 84",6378137,298.257223563,LENGTHUNIT["metre",1]]],ANGLEUNIT["degree",0.0174532925199433]],CONVERSION["UTM zone 18N",METHOD["Transverse Mercator"],PARAMETER["Scale factor at natural origin",0.9996,SCALEUNIT["unity",1]]],CS[Cartesian,2],LENGTHUNIT["metre",1]]"#;
        // Only the axis unit counts: the ellipsoid's metre is not an axis unit.
        assert_eq!(wkt_length_units(wkt), vec![Ok(Meter)]);
    }

    #[test]
    fn wkt2_feet_axes_with_metre_ellipsoid_and_parameters() {
        let wkt = r#"PROJCRS["NAD83 / Pennsylvania North (ftUS)",BASEGEOGCRS["NAD83",DATUM["North American Datum 1983",ELLIPSOID["GRS 1980",6378137,298.257222101,LENGTHUNIT["metre",1]]],ANGLEUNIT["degree",0.0174532925199433]],CONVERSION["SPCS83 PA N",METHOD["Lambert Conic Conformal (2SP)"],PARAMETER["False easting",1968500,LENGTHUNIT["US survey foot",0.304800609601219]],PARAMETER["False northing",0,LENGTHUNIT["metre",1]]],CS[Cartesian,2],AXIS["easting (X)",east,LENGTHUNIT["US survey foot",0.304800609601219]],AXIS["northing (Y)",north,LENGTHUNIT["US survey foot",0.304800609601219]]]"#;
        assert_eq!(
            wkt_length_units(wkt),
            vec![Ok(UsSurveyFoot), Ok(UsSurveyFoot)]
        );
    }

    fn resolved(info: CrsInfo) -> Result<Contents> {
        let mut c = Contents::new(FMT);
        resolve_unit(&info, &mut c).map(|_| c)
    }

    #[test]
    fn unit_resolution() {
        let both = |h, v| CrsInfo {
            crs: Some("x".into()),
            horizontal: Some(Ok(h)),
            vertical: Some(Ok(v)),
            ..Default::default()
        };
        assert_eq!(
            resolved(both(Foot, Foot)).unwrap().declared_unit,
            Some(Foot)
        );
        assert!(resolved(both(Meter, Foot)).is_err(), "mixed units refused");

        let geographic = CrsInfo {
            geographic: true,
            ..Default::default()
        };
        assert!(resolved(geographic).is_err(), "degrees refused");

        let named_only = CrsInfo {
            crs: Some("EPSG:2271".into()),
            ..Default::default()
        };
        let c = resolved(named_only).unwrap();
        assert_eq!(c.declared_unit, None);
        assert_eq!(c.warnings.len(), 1);

        let unknown = CrsInfo {
            horizontal: Some(Err("unit code 9999".into())),
            ..Default::default()
        };
        assert_eq!(resolved(unknown).unwrap().declared_unit, None);
    }
}
