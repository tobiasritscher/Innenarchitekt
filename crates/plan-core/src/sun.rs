//! Sun position (NOAA General Solar Position algorithm).

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SunPosition {
    /// Degrees clockwise from geographic north.
    pub azimuth_deg: f64,
    /// Degrees above the horizon (refraction-corrected).
    pub elevation_deg: f64,
}

fn julian_day(year: i32, month: u32, day: u32) -> f64 {
    let (mut y, mut m) = (year as f64, month as f64);
    if m <= 2.0 {
        y -= 1.0;
        m += 12.0;
    }
    let a = (y / 100.0).floor();
    let b = 2.0 - a + (a / 4.0).floor();
    (365.25 * (y + 4716.0)).floor() + (30.6001 * (m + 1.0)).floor() + day as f64 + b - 1524.5
}

/// `minutes` = local clock time in minutes after midnight; `utc_offset_min` = local − UTC (e.g. +120 for CEST).
pub fn sun_position(lat: f64, lon: f64, year: i32, month: u32, day: u32, minutes: f64, utc_offset_min: f64) -> SunPosition {
    let tz_h = utc_offset_min / 60.0;
    let jd = julian_day(year, month, day) + (minutes / 60.0 - tz_h) / 24.0;
    let t = (jd - 2451545.0) / 36525.0;
    let l0 = (280.46646 + t * (36000.76983 + t * 0.0003032)).rem_euclid(360.0);
    let m = 357.52911 + t * (35999.05029 - 0.0001537 * t);
    let e = 0.016708634 - t * (0.000042037 + 0.0000001267 * t);
    let mr = m.to_radians();
    let c = mr.sin() * (1.914602 - t * (0.004817 + 0.000014 * t))
        + (2.0 * mr).sin() * (0.019993 - 0.000101 * t)
        + (3.0 * mr).sin() * 0.000289;
    let true_long = l0 + c;
    let omega = 125.04 - 1934.136 * t;
    let app_long = true_long - 0.00569 - 0.00478 * omega.to_radians().sin();
    let mean_obliq = 23.0 + (26.0 + (21.448 - t * (46.815 + t * (0.00059 - t * 0.001813))) / 60.0) / 60.0;
    let obliq = mean_obliq + 0.00256 * omega.to_radians().cos();
    let decl = (obliq.to_radians().sin() * app_long.to_radians().sin()).asin();
    let y = (obliq.to_radians() / 2.0).tan().powi(2);
    let l0r = l0.to_radians();
    let eq_time = 4.0
        * (y * (2.0 * l0r).sin() - 2.0 * e * mr.sin() + 4.0 * e * y * mr.sin() * (2.0 * l0r).cos()
            - 0.5 * y * y * (4.0 * l0r).sin()
            - 1.25 * e * e * (2.0 * mr).sin())
        .to_degrees();
    let true_solar = (minutes + eq_time + 4.0 * lon - 60.0 * tz_h).rem_euclid(1440.0);
    let hour_angle = if true_solar / 4.0 < 0.0 { true_solar / 4.0 + 180.0 } else { true_solar / 4.0 - 180.0 };
    let latr = lat.to_radians();
    let har = hour_angle.to_radians();
    let cos_zen = (latr.sin() * decl.sin() + latr.cos() * decl.cos() * har.cos()).clamp(-1.0, 1.0);
    let zenith = cos_zen.acos();
    let elev = 90.0 - zenith.to_degrees();
    let refraction = if elev > 85.0 {
        0.0
    } else {
        let te = elev.to_radians().tan();
        let r = if elev > 5.0 {
            58.1 / te - 0.07 / te.powi(3) + 0.000086 / te.powi(5)
        } else if elev > -0.575 {
            1735.0 + elev * (-518.2 + elev * (103.4 + elev * (-12.79 + elev * 0.711)))
        } else {
            -20.772 / te
        };
        r / 3600.0
    };
    let az_den = latr.cos() * zenith.sin();
    let azimuth = if az_den.abs() > 1e-9 {
        let a = ((latr.sin() * zenith.cos() - decl.sin()) / az_den).clamp(-1.0, 1.0).acos().to_degrees();
        if hour_angle > 0.0 {
            (a + 180.0).rem_euclid(360.0)
        } else {
            (540.0 - a).rem_euclid(360.0)
        }
    } else if lat > 0.0 {
        180.0
    } else {
        0.0
    };
    SunPosition { azimuth_deg: azimuth, elevation_deg: elev + refraction }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zurich_summer_noon() {
        // Zürich 21.06., 13:30 CEST: sun ~ south, elevation ~ 66°
        let s = sun_position(47.3769, 8.5417, 2026, 6, 21, 13.0 * 60.0 + 30.0, 120.0);
        assert!((s.elevation_deg - 66.0).abs() < 1.5, "{s:?}");
        assert!((s.azimuth_deg - 180.0).abs() < 10.0, "{s:?}");
    }

    #[test]
    fn zurich_winter_morning() {
        // 21.12. 09:00 CET: low sun in the south-east
        let s = sun_position(47.3769, 8.5417, 2026, 12, 21, 9.0 * 60.0, 60.0);
        assert!(s.elevation_deg > 2.0 && s.elevation_deg < 12.0, "{s:?}");
        assert!(s.azimuth_deg > 125.0 && s.azimuth_deg < 145.0, "{s:?}");
    }
}
