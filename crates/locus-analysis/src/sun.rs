//! Where the sun is: azimuth and elevation from place and time, by the NOAA solar position
//! algorithm (after Meeus, *Astronomical Algorithms*), as used by the NOAA Global Monitoring
//! Laboratory's solar calculator. NOAA states its results are accurate to about one
//! arcminute (0.0167°) for dates between 1800 and 2100; atmospheric refraction near the
//! horizon is less certain (it depends on temperature and pressure), so the uncertainty is
//! larger there.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct SunPosition {
    /// Clockwise from true north, degrees (0–360).
    pub azimuth: f64,
    /// Above the horizon, degrees, without refraction.
    pub elevation: f64,
    /// Above the horizon, degrees, as seen through a standard atmosphere.
    pub apparent_elevation: f64,
    /// Declination of the sun, degrees.
    pub declination: f64,
    /// Equation of time, minutes (apparent minus mean solar time).
    pub equation_of_time: f64,
    /// Uncertainty of azimuth and apparent elevation, degrees (see the module note).
    pub uncertainty: f64,
}

/// Uncertainty NOAA gives for the algorithm, degrees.
pub const NOAA_ACCURACY: f64 = 1.0 / 60.0;

fn rad(d: f64) -> f64 {
    d.to_radians()
}
fn deg(r: f64) -> f64 {
    r.to_degrees()
}

/// The sun's position seen from `lat`, `lon` (degrees; north and east positive) at `unix`
/// seconds (UTC).
pub fn sun_position(lat: f64, lon: f64, unix: f64) -> SunPosition {
    let jd = unix / 86_400.0 + 2_440_587.5;
    let t = (jd - 2_451_545.0) / 36_525.0; // Julian centuries since J2000.0
    let l0 = (280.46646 + t * (36000.76983 + t * 0.0003032)).rem_euclid(360.0);
    let m = 357.52911 + t * (35999.05029 - 0.0001537 * t);
    let e = 0.016708634 - t * (0.000042037 + 0.0000001267 * t);
    let c = rad(m).sin() * (1.914602 - t * (0.004817 + 0.000014 * t))
        + rad(2.0 * m).sin() * (0.019993 - 0.000101 * t)
        + rad(3.0 * m).sin() * 0.000289;
    let true_long = l0 + c;
    let omega = 125.04 - 1934.136 * t;
    let app_long = true_long - 0.00569 - 0.00478 * rad(omega).sin();
    let obliq0 =
        23.0 + (26.0 + (21.448 - t * (46.815 + t * (0.00059 - t * 0.001813))) / 60.0) / 60.0;
    let obliq = obliq0 + 0.00256 * rad(omega).cos();
    let decl = deg((rad(obliq).sin() * rad(app_long).sin()).asin());
    let y = rad(obliq / 2.0).tan().powi(2);
    let eot = 4.0
        * deg(y * rad(2.0 * l0).sin() - 2.0 * e * rad(m).sin()
            + 4.0 * e * y * rad(m).sin() * rad(2.0 * l0).cos()
            - 0.5 * y * y * rad(4.0 * l0).sin()
            - 1.25 * e * e * rad(2.0 * m).sin());
    let minutes = unix.rem_euclid(86_400.0) / 60.0;
    let tst = (minutes + eot + 4.0 * lon).rem_euclid(1440.0);
    let ha = if tst / 4.0 < 0.0 {
        tst / 4.0 + 180.0
    } else {
        tst / 4.0 - 180.0
    };
    let cos_zen = (rad(lat).sin() * rad(decl).sin()
        + rad(lat).cos() * rad(decl).cos() * rad(ha).cos())
    .clamp(-1.0, 1.0);
    let zen = deg(cos_zen.acos());
    let az = {
        let den = rad(lat).cos() * rad(zen).sin();
        if den.abs() < 1e-12 {
            // At a pole, or the sun at the zenith: azimuth is undefined; report 180°.
            180.0
        } else {
            let a = deg(((rad(lat).sin() * rad(zen).cos() - rad(decl).sin()) / den)
                .clamp(-1.0, 1.0)
                .acos());
            if ha > 0.0 {
                (a + 180.0).rem_euclid(360.0)
            } else {
                (540.0 - a).rem_euclid(360.0)
            }
        }
    };
    let elev = 90.0 - zen;
    let refraction = refraction(elev);
    SunPosition {
        azimuth: az,
        elevation: elev,
        apparent_elevation: elev + refraction,
        declination: decl,
        equation_of_time: eot,
        // Refraction's own uncertainty is roughly a tenth of it (weather); near the horizon
        // that dominates NOAA's stated accuracy.
        uncertainty: NOAA_ACCURACY.max(refraction.abs() * 0.1),
    }
}

/// Approximate atmospheric refraction, degrees, for a true elevation in degrees (NOAA's
/// piecewise formula for a standard atmosphere).
pub fn refraction(elev: f64) -> f64 {
    if elev > 85.0 {
        return 0.0;
    }
    let te = rad(elev).tan();
    let arcsec = if elev > 5.0 {
        58.1 / te - 0.07 / te.powi(3) + 0.000086 / te.powi(5)
    } else if elev > -0.575 {
        1735.0 + elev * (-518.2 + elev * (103.4 + elev * (-12.79 + elev * 0.711)))
    } else {
        -20.772 / te
    };
    arcsec / 3600.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unix seconds for a UTC date and time (proleptic Gregorian; days from civil).
    fn unix(y: i64, mo: i64, d: i64, h: f64) -> f64 {
        let (y, mo) = if mo <= 2 { (y - 1, mo + 12) } else { (y, mo) };
        let days = 365 * y + y / 4 - y / 100 + y / 400 + (153 * (mo - 3) + 2) / 5 + d - 719_469;
        days as f64 * 86_400.0 + h * 3600.0
    }

    #[test]
    fn the_date_helper_is_right() {
        assert_eq!(unix(1970, 1, 1, 0.0), 0.0);
        assert_eq!(unix(2000, 1, 1, 12.0), 946_728_000.0);
    }

    #[test]
    fn matches_the_published_spa_example() {
        // Reda & Andreas, "Solar Position Algorithm for Solar Radiation Applications"
        // (NREL/TP-560-34302), worked example: 17 Oct 2003 12:30:30 at UTC−7, Golden, CO,
        // 39.742476° N, 105.1786° W. Published: topocentric zenith 50.11162° (refracted),
        // azimuth 194.34024°. The NOAA algorithm is simpler (no parallax, standard
        // atmosphere), so it agrees to within its stated accuracy, not to SPA's.
        let s = sun_position(39.742476, -105.1786, unix(2003, 10, 17, 19.0 + 30.5 / 60.0));
        assert!((s.azimuth - 194.34024).abs() < 0.02, "{}", s.azimuth);
        assert!(
            (90.0 - s.apparent_elevation - 50.11162).abs() < 0.03,
            "{}",
            s.apparent_elevation
        );
    }

    #[test]
    fn declination_and_equation_of_time_follow_the_year() {
        // June solstice 2024 (20 Jun 20:51 UTC): declination at its maximum, 23.44°.
        let s = sun_position(0.0, 0.0, unix(2024, 6, 20, 20.85));
        assert!((s.declination - 23.44).abs() < 0.01, "{}", s.declination);
        // Early November the sun runs about 16.4 minutes fast (the equation of time's peak).
        let s = sun_position(0.0, 0.0, unix(2024, 11, 3, 12.0));
        assert!(
            (s.equation_of_time - 16.4).abs() < 0.1,
            "{}",
            s.equation_of_time
        );
        // Mid-February about 14.2 minutes slow.
        let s = sun_position(0.0, 0.0, unix(2024, 2, 11, 12.0));
        assert!(
            (s.equation_of_time + 14.2).abs() < 0.1,
            "{}",
            s.equation_of_time
        );
    }

    #[test]
    fn solar_noon_puts_the_sun_due_south_at_the_expected_height() {
        // 51.5° N, 0° E on the March equinox 2024 (20 Mar): at solar noon the sun is due
        // south, at 90° − 51.5° + declination (≈ 0).
        let noon_utc =
            12.0 - sun_position(51.5, 0.0, unix(2024, 3, 20, 12.0)).equation_of_time / 60.0;
        let s = sun_position(51.5, 0.0, unix(2024, 3, 20, noon_utc));
        assert!((s.azimuth - 180.0).abs() < 0.05, "{}", s.azimuth);
        assert!(
            (s.elevation - (38.5 + s.declination)).abs() < 0.01,
            "{}",
            s.elevation
        );
    }

    #[test]
    fn refraction_is_about_half_a_degree_at_the_horizon_and_nothing_overhead() {
        assert!((refraction(0.0) - 1735.0 / 3600.0).abs() < 1e-12);
        assert_eq!(refraction(89.0), 0.0);
        assert!((refraction(45.0) - 58.03 / 3600.0).abs() < 0.001 / 3600.0 * 100.0);
        // Near the horizon the stated uncertainty grows beyond NOAA's arcminute.
        let s = sun_position(51.5, 0.0, unix(2024, 3, 20, 6.1));
        assert!(s.elevation.abs() < 2.0 && s.uncertainty > NOAA_ACCURACY);
    }
}
