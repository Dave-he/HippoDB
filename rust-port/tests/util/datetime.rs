//! `tests/util/datetime.rs` — integration tests for the date/time
//! functions (julianday, strftime, current_*).
//!
//! These tests verify the 1:1 behavior with the C reference
//! implementation in `sqlite-source/src/date.c` for the basic
//! Gregorian ↔ JD conversion, ISO 8601 parsing, and strftime
//! formatting.

use libsqlite_rs::util::datetime::{
    current_date_str, current_time_str, current_timestamp_str, from_julian_day, julian_day,
    parse_iso8601, strftime,
};

// ============================================================================
// 1. Julian Day conversion (date.c:260-298)
// ============================================================================
#[test]
fn jd_2000_01_01_is_well_known() {
    // 2000-01-01 00:00:00 UTC = JD 2451544.5 (well-known reference
    // point used throughout astronomy / SQLite tests).
    let jd = julian_day(2000, 1, 1, 0, 0, 0.0).unwrap();
    assert!((jd - 2451544.5).abs() < 1e-6, "got jd={jd}");
}

#[test]
fn jd_1970_01_01_is_unix_epoch() {
    // 1970-01-01 00:00:00 UTC = JD 2440587.5 (Unix epoch).
    let jd = julian_day(1970, 1, 1, 0, 0, 0.0).unwrap();
    assert!((jd - 2440587.5).abs() < 1e-6, "got jd={jd}");
}

#[test]
fn jd_1900_01_01() {
    // 1900-01-01 00:00:00 UTC = JD 2415020.5
    let jd = julian_day(1900, 1, 1, 0, 0, 0.0).unwrap();
    assert!((jd - 2415020.5).abs() < 1e-6, "got jd={jd}");
}

#[test]
fn jd_invalid_year() {
    // Years > 9999 are not supported.
    assert!(julian_day(10000, 1, 1, 0, 0, 0.0).is_none());
}

#[test]
fn jd_invalid_month() {
    assert!(julian_day(2024, 13, 1, 0, 0, 0.0).is_none());
    assert!(julian_day(2024, 0, 1, 0, 0, 0.0).is_none());
}

// ============================================================================
// 2. Round-trip (JD → Y/M/D/h/m/s) — date.c:510-562
// ============================================================================
#[test]
fn round_trip_2024_jan_1() {
    let jd = julian_day(2024, 1, 1, 0, 0, 0.0).unwrap();
    let (y, m, d, h, mi, s) = from_julian_day(jd).unwrap();
    assert_eq!(y, 2024);
    assert_eq!(m, 1);
    assert_eq!(d, 1);
    assert_eq!(h, 0);
    assert_eq!(mi, 0);
    assert_eq!(s, 0);
}

#[test]
fn round_trip_2024_jun_15_with_time() {
    let jd = julian_day(2024, 6, 15, 12, 30, 45.0).unwrap();
    let (y, m, d, h, mi, s) = from_julian_day(jd).unwrap();
    assert_eq!(y, 2024);
    assert_eq!(m, 6);
    assert_eq!(d, 15);
    assert_eq!(h, 12);
    assert_eq!(mi, 30);
    // Allow ±1 second rounding from f64 arithmetic.
    assert!(s == 45 || s == 44, "got s={s}");
}

#[test]
fn round_trip_2024_dec_31() {
    let jd = julian_day(2024, 12, 31, 23, 59, 59.0).unwrap();
    let (y, m, d, _h, _mi, _s) = from_julian_day(jd).unwrap();
    assert_eq!(y, 2024);
    assert_eq!(m, 12);
    assert_eq!(d, 31);
}

#[test]
fn round_trip_leap_year() {
    // 2024-02-29 (leap year) round-trips.
    let jd = julian_day(2024, 2, 29, 0, 0, 0.0).unwrap();
    let (y, m, d, _, _, _) = from_julian_day(jd).unwrap();
    assert_eq!((y, m, d), (2024, 2, 29));
}

#[test]
fn round_trip_century_boundary() {
    // 2000-01-01 (Y2K) — 2000 is a leap year (divisible by 400).
    let jd = julian_day(2000, 1, 1, 0, 0, 0.0).unwrap();
    let (y, m, d, _, _, _) = from_julian_day(jd).unwrap();
    assert_eq!((y, m, d), (2000, 1, 1));
}

// ============================================================================
// 3. ISO 8601 parsing (date.c:335-366)
// ============================================================================
#[test]
fn parse_date_only() {
    let dt = parse_iso8601("2024-06-15").unwrap();
    let (y, m, d, h, mi, s) = from_julian_day(dt.jd()).unwrap();
    assert_eq!((y, m, d), (2024, 6, 15));
    assert_eq!(h, 0);
    assert_eq!(mi, 0);
    assert_eq!(s, 0);
}

#[test]
fn parse_with_space_separator() {
    let dt = parse_iso8601("2024-06-15 12:30:45").unwrap();
    let (_y, _m, _d, h, mi, s) = from_julian_day(dt.jd()).unwrap();
    assert_eq!((h, mi, s), (12, 30, 45));
}

#[test]
fn parse_with_t_separator() {
    let dt = parse_iso8601("2024-06-15T12:30:45").unwrap();
    let (_y, _m, _d, h, mi, s) = from_julian_day(dt.jd()).unwrap();
    assert_eq!((h, mi, s), (12, 30, 45));
}

#[test]
fn parse_with_fractional_seconds() {
    let dt = parse_iso8601("2024-06-15 12:30:45.5").unwrap();
    let jd = dt.jd();
    let frac = jd - jd.floor();
    // 0.5 sec = 0.5/86400 of a day.
    let sec_of_day = frac * 86400.0;
    assert!((sec_of_day - (12.0 * 3600.0 + 30.0 * 60.0 + 45.5)).abs() < 0.01);
}

#[test]
fn parse_rejects_garbage() {
    assert!(parse_iso8601("hello").is_none());
    assert!(parse_iso8601("").is_none());
    assert!(parse_iso8601("2024").is_none());
    assert!(parse_iso8601("2024-06").is_none());
    assert!(parse_iso8601("2024/06/15").is_none());
}

#[test]
fn parse_rejects_invalid_time() {
    assert!(parse_iso8601("2024-06-15 25:00:00").is_none());
    assert!(parse_iso8601("2024-06-15 12:60:00").is_none());
    assert!(parse_iso8601("2024-06-15 12:30:60").is_none());
}

#[test]
fn parse_handles_leading_whitespace() {
    let dt = parse_iso8601("  2024-06-15").unwrap();
    let (y, _m, _d, _, _, _) = from_julian_day(dt.jd()).unwrap();
    assert_eq!(y, 2024);
}

// ============================================================================
// 4. strftime (date.c:1410-1570)
// ============================================================================
#[test]
fn strftime_year_month_day() {
    let dt = parse_iso8601("2024-06-15 12:30:45").unwrap();
    assert_eq!(strftime("%Y-%m-%d", &dt), "2024-06-15");
}

#[test]
fn strftime_hour_minute_second() {
    let dt = parse_iso8601("2024-06-15 12:30:45").unwrap();
    assert_eq!(strftime("%H:%M:%S", &dt), "12:30:45");
}

#[test]
fn strftime_percent_literal() {
    let dt = parse_iso8601("2024-06-15").unwrap();
    assert_eq!(strftime("100%% done", &dt), "100% done");
}

#[test]
fn strftime_day_of_year_known() {
    // 2024-06-15 is the 167th day of 2024 (leap year).
    let dt = parse_iso8601("2024-06-15").unwrap();
    assert_eq!(strftime("%j", &dt), "167");
}

#[test]
fn strftime_day_of_year_jan_1() {
    let dt = parse_iso8601("2024-01-01").unwrap();
    assert_eq!(strftime("%j", &dt), "001");
}

#[test]
fn strftime_day_of_year_dec_31_leap() {
    // 2024 is leap → 366.
    let dt = parse_iso8601("2024-12-31").unwrap();
    assert_eq!(strftime("%j", &dt), "366");
}

#[test]
fn strftime_unix_epoch_seconds() {
    // 1970-01-01 00:00:00 → %s = 0.
    let dt = parse_iso8601("1970-01-01 00:00:00").unwrap();
    assert_eq!(strftime("%s", &dt), "0");
}

#[test]
fn strftime_julian_day_format() {
    let dt = parse_iso8601("2000-01-01").unwrap();
    // 2000-01-01 00:00:00 = JD 2451544.5
    let s = strftime("%J", &dt);
    assert!(s.starts_with("2451544"), "got: {s}");
}

#[test]
fn strftime_unknown_spec_kept_literal() {
    let dt = parse_iso8601("2024-06-15").unwrap();
    // %Q is not in our subset — emit literally.
    assert_eq!(strftime("%Q", &dt), "%Q");
}

#[test]
fn strftime_combined() {
    let dt = parse_iso8601("2024-06-15 12:30:45").unwrap();
    assert_eq!(strftime("%Y-%m-%d %H:%M:%S", &dt), "2024-06-15 12:30:45");
}

#[test]
fn strftime_day_of_week_known() {
    // 2024-06-15 is a Saturday.
    let dt = parse_iso8601("2024-06-15").unwrap();
    assert_eq!(strftime("%w", &dt), "6");
}

// ============================================================================
// 5. current_time / current_date / current_timestamp
// ============================================================================
#[test]
fn current_time_format() {
    // Should be "HH:MM:SS" — 8 chars.
    let s = current_time_str();
    assert_eq!(s.len(), 8, "got: {s}");
    assert!(s.contains(':'));
}

#[test]
fn current_date_format() {
    // Should be "YYYY-MM-DD" — 10 chars.
    let s = current_date_str();
    assert_eq!(s.len(), 10, "got: {s}");
    assert!(s.starts_with("20"), "got: {s}");
}

#[test]
fn current_timestamp_format() {
    // Should be "YYYY-MM-DD HH:MM:SS" — 19 chars.
    let s = current_timestamp_str();
    assert_eq!(s.len(), 19, "got: {s}");
    assert!(s.contains(' '));
}
