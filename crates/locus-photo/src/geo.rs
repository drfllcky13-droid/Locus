//! WGS84 geodetic coordinates to a local east-north-up frame (NIMA TR8350.2 constants), for
//! georeferencing a reconstruction by the photos' GPS positions.

use crate::model::P3;

const A: f64 = 6_378_137.0;
const F: f64 = 1.0 / 298.257_223_563;

/// Latitude, longitude (°) and ellipsoidal height (m) to Earth-centred, Earth-fixed (m).
pub fn ecef(lat: f64, lon: f64, h: f64) -> P3 {
    let e2 = F * (2.0 - F);
    let (sl, cl) = lat.to_radians().sin_cos();
    let (so, co) = lon.to_radians().sin_cos();
    let n = A / (1.0 - e2 * sl * sl).sqrt();
    [
        (n + h) * cl * co,
        (n + h) * cl * so,
        (n * (1.0 - e2) + h) * sl,
    ]
}

/// A point's east, north and up (m) from the origin `(lat, lon, h)`.
pub fn enu(origin: (f64, f64, f64), p: (f64, f64, f64)) -> P3 {
    let (o, q) = (ecef(origin.0, origin.1, origin.2), ecef(p.0, p.1, p.2));
    let d = [q[0] - o[0], q[1] - o[1], q[2] - o[2]];
    let (sl, cl) = origin.0.to_radians().sin_cos();
    let (so, co) = origin.1.to_radians().sin_cos();
    [
        -so * d[0] + co * d[1],
        -sl * co * d[0] - sl * so * d[1] + cl * d[2],
        cl * co * d[0] + cl * so * d[1] + sl * d[2],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_offsets_are_metres() {
        let o = (41.2, -77.0, 150.0);
        assert_eq!(enu(o, o), [0.0, 0.0, 0.0]);
        let up = enu(o, (41.2, -77.0, 250.0));
        assert!(
            up[0].abs() < 1e-9 && up[1].abs() < 1e-9 && (up[2] - 100.0).abs() < 1e-9,
            "{up:?}"
        );
        // 0.001° of latitude is the meridian radius of curvature × 0.001° (to first order):
        // M = a (1 − e²) / (1 − e² sin²φ)^1.5.
        let e2 = F * (2.0 - F);
        let s = 41.2f64.to_radians().sin();
        let m = A * (1.0 - e2) / (1.0 - e2 * s * s).powf(1.5);
        let n = enu(o, (41.201, -77.0, 150.0));
        assert!((n[1] - m * 0.001f64.to_radians()).abs() < 0.01, "{n:?}");
        assert!(n[0].abs() < 1e-6);
        // 0.001° of longitude: N cos φ × 0.001°.
        let nn = A / (1.0 - e2 * s * s).sqrt();
        let e = enu(o, (41.2, -76.999, 150.0));
        assert!(
            (e[0] - nn * 41.2f64.to_radians().cos() * 0.001f64.to_radians()).abs() < 0.01,
            "{e:?}"
        );
    }
}
