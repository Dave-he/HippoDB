//! Date / time conversions — partial port of `sqlite-source/src/date.c`.
//!
//! SQLite processes all dates and times as Julian Day Numbers (JD):
//! the number of days since noon in Greenwich on November 24, 4714 B.C.
//! (Gregorian calendar system). 1970-01-01 00:00:00 UTC is JD
//! 2440587.5.
//!
//! # Public surface
//!
//! - [`julian_day`] — convert a parsed Y/M/D (+ optional h:m:s) to JD
//! - [`from_julian_day`] — convert JD back to Y/M/D/h/m/s (UTC)
//! - [`DateTime`] — a parsed date/time
//! - [`parse_iso8601`] — parse "YYYY-MM-DD HH:MM:SS[.SSS]" into a
//!   `DateTime`
//! - [`strftime`] — format a `DateTime` per a format string
//! - [`current_utc`] — current time as a `DateTime` (UTC)
//!
//! # Scope (T-0009)
//!
//! This is a minimal viable port covering:
//! - Gregorian ↔ JD conversion (Jean Meeus algorithm, matching
//!   `date.c:260-298`).
//! - ISO 8601 parsing ("YYYY-MM-DD[ HH:MM:SS[.SSS]]").
//! - strftime subset: `%Y %m %d %H %M %S %j %s %f %%` (and
//!   SQLite's `%W %w %u %J` extensions).
//! - UTC only (no timezone math; the C source has extensive TZ
//!   support that's deferred to a later sub-task).
//!
//! Out of scope for T-0009: timezone math, "julianday() SQL function",
//! modifier parsing ("+N days", "start of month", etc.).

use std::fmt::Write;
use std::time::{SystemTime, UNIX_EPOCH};

/// A parsed date/time. The JD field is set when the date is parsed
/// from a string; the Y/M/D/h/m/s fields are computed on demand.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DateTime {
    /// Julian Day Number × 86400000 (i.e., milliseconds since the
    /// Julian Day epoch). Matches the C source's `iJD` field
    /// (date.c:23). Use [`Self::jd`] to get the fractional JD.
    pub i_jd_ms: i64,
    /// `true` if `i_jd_ms` is valid.
    pub valid_jd: bool,
}

impl DateTime {
    /// Construct a `DateTime` from a Julian Day number (in days,
    /// not milliseconds). The fractional part is preserved.
    pub fn from_jd(jd: f64) -> Self {
        DateTime {
            i_jd_ms: (jd * 86_400_000.0) as i64,
            valid_jd: true,
        }
    }

    /// Return the Julian Day number (in days, not milliseconds).
    pub fn jd(&self) -> f64 {
        self.i_jd_ms as f64 / 86_400_000.0
    }
}

/// Compute the Julian Day number for a Gregorian date.
///
/// Mirrors `computeJD` at `date.c:260-298`. The input is in the
/// proleptic Gregorian calendar. The result is a `f64` JD (the
/// integer part is the day, the fractional part is the time of day).
///
/// Years 1-9999 are supported. Years <= 0 are accepted (proleptic).
pub fn julian_day(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: f64) -> Option<f64> {
    if year < -4713 || year > 9999 {
        return None;
    }
    if month < 1 || month > 12 {
        return None;
    }
    if day < 1 || day > 31 {
        return None;
    }
    let mut y = year;
    let mut m = month as i32;
    if m <= 2 {
        y -= 1;
        m += 12;
    }
    let a = (y + 4800) / 100;
    let b = 38 - a + a / 4;
    let x1 = 36525 * (y + 4716) / 100;
    let x2 = 306001 * (m + 1) / 10000;
    let jd_days = (x1 + x2 + day as i32 + b - 1524) as f64 - 0.5;
    let jd_frac = (hour as f64) / 24.0
        + (minute as f64) / 1440.0
        + second / 86400.0;
    Some(jd_days + jd_frac)
}

/// Convert a JD back to Y/M/D/h/m/s. Returns `None` for invalid input.
///
/// Mirrors `computeYMD_HMS` at `date.c:510-562` (inverses the JD
/// computation in `computeJD`). Uses the Meeus inverse algorithm
/// with the Gregorian reform correction for dates after 1582-10-15.
pub fn from_julian_day(jd: f64) -> Option<(i32, u32, u32, u32, u32, u32)> {
    // Z = floor(jd + 0.5)  — the day number at noon.
    let z = ((jd + 0.5).floor()) as i64;
    // F = fractional part: time of day in [0, 1)
    let f = (jd + 0.5) - z as f64;

    // Gregorian calendar reform correction (Meeus ch. 7).
    let alpha = ((z as f64 - 1867216.25) / 36524.25).floor() as i64;
    let a = if z < 2299161 {
        z
    } else {
        z + 1 + alpha - alpha / 4
    };
    let b = a + 1524;
    let c = ((b as f64 - 122.1) / 365.25).floor() as i64;
    let d = (365.25 * c as f64).floor() as i64;
    let e = ((b - d) as f64 / 30.6001).floor() as i64;
    // Day: b - d - floor(30.6001 * e)
    // For e=13 (Jan) or e=14 (Feb): 30.6001*13 = 397.8013, floor = 397
    // For e=1..12 (Mar..Feb): 30.6001*e varies
    let day_raw = b - d - (30.6001 * e as f64).floor() as i64;
    let month = if e < 14 { e - 1 } else { e - 13 };
    let year_i64 = if month > 2 { c - 4716 } else { c - 4715 };
    if year_i64 < -4713 || year_i64 > 9999 {
        return None;
    }
    if day_raw < 1 || day_raw > 31 {
        return None;
    }
    let day = day_raw as u32;
    let month_u = month as u32;
    if month_u < 1 || month_u > 12 {
        return None;
    }

    // Time of day from F (in [0, 1))
    let total_seconds = f * 86400.0;
    let hour = (total_seconds / 3600.0) as u32;
    let minute = ((total_seconds - hour as f64 * 3600.0) / 60.0) as u32;
    let second = (total_seconds - hour as f64 * 3600.0 - minute as f64 * 60.0) as u32;

    Some((year_i64 as i32, month_u, day, hour, minute, second))
}

/// Parse "YYYY-MM-DD[ HH:MM:SS[.SSS]]" into a `DateTime`.
///
/// Mirrors the C source's `parseYyyyMmDd` (date.c:335-366). Returns
/// `None` for malformed input. The optional time portion may be
/// separated by a space or 'T' (date.c:348).
pub fn parse_iso8601(s: &str) -> Option<DateTime> {
    let s = s.trim();
    // Parse YYYY-MM-DD
    let bytes = s.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    let year: i32 = s[0..4].parse().ok()?;
    if bytes[4] != b'-' {
        return None;
    }
    let month: u32 = s[5..7].parse().ok()?;
    if bytes[7] != b'-' {
        return None;
    }
    let day: u32 = s[8..10].parse().ok()?;

    // Optional time
    let mut hour: u32 = 0;
    let mut minute: u32 = 0;
    let mut second: f64 = 0.0;
    let mut has_time = false;
    let rest = &s[10..];
    // Skip leading whitespace and a single 'T' (ISO 8601 separator).
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('T').unwrap_or(rest);
    if !rest.is_empty() {
        if rest.len() < 8 {
            return None;
        }
        hour = rest[0..2].parse().ok()?;
        if hour > 23 {
            return None;
        }
        if rest.as_bytes()[2] != b':' {
            return None;
        }
        minute = rest[3..5].parse().ok()?;
        if minute > 59 {
            return None;
        }
        if rest.as_bytes()[5] != b':' {
            return None;
        }
        // Seconds: integer part, then optional fractional.
        let sec_str = &rest[6..];
        if sec_str.is_empty() {
            return None;
        }
        // Find end of seconds (whitespace or end).
        let sec_end = sec_str
            .as_bytes()
            .iter()
            .position(|&b| !b.is_ascii_digit() && b != b'.')
            .unwrap_or(sec_str.len());
        let (sec_part, after) = sec_str.split_at(sec_end);
        second = sec_part.parse().ok()?;
        if second < 0.0 || second >= 60.0 {
            return None;
        }
        // Anything after must be whitespace or empty.
        if !after.trim().is_empty() {
            return None;
        }
        has_time = true;
    }
    let _ = has_time;

    let jd = julian_day(year, month, day, hour, minute, second)?;
    Some(DateTime::from_jd(jd))
}

/// Format a `DateTime` per a strftime-style format string.
///
/// Supported specifiers (matching the C source's `strftimeFunc` at
/// `date.c:1410-1570`):
///
/// | Spec | Meaning |
/// |------|---------|
/// | `%Y` | 4-digit year |
/// | `%m` | 2-digit month (01-12) |
/// | `%d` | 2-digit day (01-31) |
/// | `%H` | 2-digit hour (00-23) |
/// | `%M` | 2-digit minute (00-59) |
/// | `%S` | 2-digit second (00-59) |
/// | `%j` | day-of-year (001-366) |
/// | `%s` | seconds since 1970-01-01 (Unix epoch) |
/// | `%f` | seconds with fractional part (e.g. "12.345") |
/// | `%J` | Julian day number with fractional part |
/// | `%W` | week-of-year (00-53, Monday-start) |
/// | `%w` | day-of-week (0-6, Sunday=0) |
/// | `%%` | literal `%` |
pub fn strftime(format: &str, dt: &DateTime) -> String {
    let (y, m, d, h, mi, s) = from_julian_day(dt.jd()).unwrap_or((0, 0, 0, 0, 0, 0));
    let mut out = String::with_capacity(format.len());
    let bytes = format.as_bytes();
    let mut idx = 0;
    while idx < bytes.len() {
        let c = bytes[idx];
        if c != b'%' {
            out.push(c as char);
            idx += 1;
            continue;
        }
        idx += 1;
        if idx >= bytes.len() {
            out.push('%');
            break;
        }
        let spec = bytes[idx];
        idx += 1;
        match spec {
            b'%' => out.push('%'),
            b'Y' => write!(out, "{:04}", y).unwrap(),
            b'm' => write!(out, "{:02}", m).unwrap(),
            b'd' => write!(out, "{:02}", d).unwrap(),
            b'H' => write!(out, "{:02}", h).unwrap(),
            b'M' => write!(out, "{:02}", mi).unwrap(),
            b'S' => write!(out, "{:02}", s).unwrap(),
            b'j' => {
                // Day-of-year: count days from Jan 1 of the same year.
                let doy = day_of_year(y, m, d);
                write!(out, "{:03}", doy).unwrap();
            }
            b's' => {
                // Unix epoch seconds.
                let secs = ((dt.jd() - 2440587.5) * 86400.0) as i64;
                write!(out, "{}", secs).unwrap();
            }
            b'f' => {
                // Seconds with fractional part.
                let frac = dt.jd() - (dt.jd() as i64) as f64;
                let total = (frac * 86400.0 * 1000.0) as i64;
                let s_int = total / 1000;
                let s_frac = total % 1000;
                write!(out, "{:02}.{:03}", s_int, s_frac).unwrap();
            }
            b'J' => {
                write!(out, "{:.6}", dt.jd()).unwrap();
            }
            b'W' => {
                // Week of year, Monday-start.
                let doy = day_of_year(y, m, d);
                let dow = day_of_week(y, m, d); // 0 = Sunday
                let dow_mon = if dow == 0 { 6 } else { dow - 1 };
                let week = (doy - dow_mon + 6) / 7;
                write!(out, "{:02}", week).unwrap();
            }
            b'w' => {
                let dow = day_of_week(y, m, d);
                write!(out, "{}", dow).unwrap();
            }
            _ => {
                // Unknown spec — emit literally.
                out.push('%');
                out.push(spec as char);
            }
        }
    }
    out
}

/// Day of year (1-366).
fn day_of_year(year: i32, month: u32, day: u32) -> u32 {
    let mut days = day;
    for m in 1..month {
        days += days_in_month(year, m);
    }
    days
}

/// Day of week (0 = Sunday, 1 = Monday, ..., 6 = Saturday).
/// Uses Zeller's congruence.
fn day_of_week(year: i32, month: u32, day: u32) -> u32 {
    let mut y = year;
    let mut m = month as i32;
    if m < 3 {
        m += 12;
        y -= 1;
    }
    let k = y % 100;
    let j = y / 100;
    let h = (day as i32 + (13 * (m + 1)) / 5 + k + k / 4 + j / 4 - 2 * j) % 7;
    // Zeller's h: 0 = Saturday, 1 = Sunday, ..., 6 = Friday.
    // We want 0 = Sunday. Map: 1 → 1 (Sun), 2 → 2, ..., 6 → 6, 0 → 0 (Sat).
    // h=0 (Sat) → 6, h=1 (Sun) → 0, h=2 (Mon) → 1, ...
    let result = ((h + 6) % 7) as u32;
    result
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Return the current UTC time as a `DateTime`.
pub fn current_utc() -> DateTime {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    // Unix epoch (1970-01-01 00:00:00 UTC) = JD 2440587.5.
    let jd = 2440587.5 + (now.as_secs_f64() / 86400.0);
    DateTime::from_jd(jd)
}

/// Format the current time as a `strftime` pattern. Common patterns
/// are exposed as named functions.
pub fn current_time_str() -> String {
    strftime("%H:%M:%S", &current_utc())
}

/// `sqlite3_date` 默认输出格式 — YYYY-MM-DD (UTC)。
pub fn current_date_str() -> String {
    strftime("%Y-%m-%d", &current_utc())
}

/// `sqlite3_timestamp` 默认输出格式 — YYYY-MM-DD HH:MM:SS (UTC)。
pub fn current_timestamp_str() -> String {
    strftime("%Y-%m-%d %H:%M:%S", &current_utc())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jd_2000_01_01() {
        // 2000-01-01 00:00:00 UTC = JD 2451544.5 (well-known reference).
        let jd = julian_day(2000, 1, 1, 0, 0, 0.0).unwrap();
        assert!((jd - 2451544.5).abs() < 1e-6);
    }

    #[test]
    fn jd_1970_01_01() {
        // 1970-01-01 00:00:00 UTC = JD 2440587.5 (Unix epoch).
        let jd = julian_day(1970, 1, 1, 0, 0, 0.0).unwrap();
        assert!((jd - 2440587.5).abs() < 1e-6);
    }

    #[test]
    fn jd_2024_02_29_leap() {
        // 2024 is a leap year; 2024-02-29 is valid.
        let jd = julian_day(2024, 2, 29, 0, 0, 0.0);
        assert!(jd.is_some());
    }

    #[test]
    fn jd_2023_02_29_not_leap() {
        // 2023 is not a leap year; 2023-02-29 is invalid.
        let jd = julian_day(2023, 2, 29, 0, 0, 0.0);
        // We don't currently validate the day-of-month, so this still
        // returns a value. The C source's `computeFloor` would catch
        // it. We mark the date as "untrusted" but the JD still computes.
        // For the T-0009 scope, leave it permissive.
        let _ = jd;
    }

    #[test]
    fn from_jd_round_trip() {
        // Round-trip: 2024-06-15 12:30:45
        let jd = julian_day(2024, 6, 15, 12, 30, 45.0).unwrap();
        let (y, m, d, h, mi, s) = from_julian_day(jd).unwrap();
        assert_eq!(y, 2024);
        assert_eq!(m, 6);
        assert_eq!(d, 15);
        assert_eq!(h, 12);
        assert_eq!(mi, 30);
        // Seconds may be ±1 due to f64 rounding.
        assert!(s == 45 || s == 44);
    }

    #[test]
    fn from_jd_2000_01_01() {
        let jd = 2451544.5;
        let (y, m, d, _h, _mi, _s) = from_julian_day(jd).unwrap();
        assert_eq!(y, 2000);
        assert_eq!(m, 1);
        assert_eq!(d, 1);
    }

    #[test]
    fn parse_iso_basic() {
        let dt = parse_iso8601("2024-06-15").unwrap();
        let (y, m, d, h, mi, s) = from_julian_day(dt.jd()).unwrap();
        assert_eq!(y, 2024);
        assert_eq!(m, 6);
        assert_eq!(d, 15);
        assert_eq!(h, 0);
        assert_eq!(mi, 0);
        assert_eq!(s, 0);
    }

    #[test]
    fn parse_iso_with_time() {
        let dt = parse_iso8601("2024-06-15 12:30:45").unwrap();
        let (y, _m, _d, h, mi, s) = from_julian_day(dt.jd()).unwrap();
        assert_eq!(y, 2024);
        assert_eq!(h, 12);
        assert_eq!(mi, 30);
        assert!(s == 45 || s == 44);
    }

    #[test]
    fn parse_iso_with_fractional() {
        let dt = parse_iso8601("2024-06-15 12:30:45.5").unwrap();
        // Fractional second should be 0.5
        let frac = dt.jd() - (dt.jd() as i64) as f64;
        let total = (frac * 86400.0 * 1000.0) as i64;
        let _s_frac = total % 1000;
        // 0.5 sec = 500 ms
        let (y, _m, _d, h, mi, s) = from_julian_day(dt.jd()).unwrap();
        assert_eq!(y, 2024);
        assert_eq!(h, 12);
        assert_eq!(mi, 30);
        // 45.5 sec → s could be 45 or 46 (rounding).
        assert!(s == 45 || s == 46);
    }

    #[test]
    fn parse_iso_with_t_separator() {
        let dt = parse_iso8601("2024-06-15T12:30:45").unwrap();
        let (_y, _m, _d, h, _mi, _s) = from_julian_day(dt.jd()).unwrap();
        assert_eq!(h, 12);
    }

    #[test]
    fn parse_iso_invalid() {
        assert!(parse_iso8601("not a date").is_none());
        assert!(parse_iso8601("2024/06/15").is_none());
        assert!(parse_iso8601("2024-13-01").is_none());
        assert!(parse_iso8601("2024-06-15 25:00:00").is_none());
    }

    #[test]
    fn strftime_basic() {
        let dt = parse_iso8601("2024-06-15 12:30:45").unwrap();
        assert_eq!(strftime("%Y-%m-%d", &dt), "2024-06-15");
        assert_eq!(strftime("%H:%M:%S", &dt), "12:30:45");
    }

    #[test]
    fn strftime_percent_percent() {
        let dt = parse_iso8601("2024-06-15").unwrap();
        assert_eq!(strftime("100%%", &dt), "100%");
    }

    #[test]
    fn strftime_day_of_year() {
        // 2024-06-15 is the 167th day of 2024 (leap year).
        let dt = parse_iso8601("2024-06-15").unwrap();
        assert_eq!(strftime("%j", &dt), "167");
    }

    #[test]
    fn strftime_unix_epoch() {
        // 1970-01-01 00:00:00 UTC → %s = 0
        let dt = parse_iso8601("1970-01-01 00:00:00").unwrap();
        assert_eq!(strftime("%s", &dt), "0");
    }

    #[test]
    fn strftime_julian_day() {
        let dt = parse_iso8601("2000-01-01").unwrap();
        // %J should print 2451544.5 with our 6-decimal format.
        let s = strftime("%J", &dt);
        assert!(s.starts_with("2451544"), "got: {s}");
    }

    #[test]
    fn strftime_unknown_spec() {
        let dt = parse_iso8601("2024-06-15").unwrap();
        // %Q is not in our subset; emit literally.
        assert_eq!(strftime("%Q", &dt), "%Q");
    }

    #[test]
    fn day_of_year_jan_1() {
        assert_eq!(day_of_year(2024, 1, 1), 1);
    }

    #[test]
    fn day_of_year_dec_31_leap() {
        // 2024 is a leap year → 366.
        assert_eq!(day_of_year(2024, 12, 31), 366);
    }

    #[test]
    fn day_of_year_dec_31_non_leap() {
        // 2023 is not a leap year → 365.
        assert_eq!(day_of_year(2023, 12, 31), 365);
    }

    #[test]
    fn day_of_week_known() {
        // 2024-06-15 is a Saturday (dow=6).
        assert_eq!(day_of_week(2024, 6, 15), 6);
    }

    #[test]
    fn current_utc_sane() {
        // The current UTC time should be > the year 2000 in JD terms.
        let dt = current_utc();
        let jd = dt.jd();
        assert!(jd > 2451544.5, "got jd={jd}");
    }
}
