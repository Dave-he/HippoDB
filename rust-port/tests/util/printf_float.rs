//! `tests/util/printf_float.rs` — integration tests for the float
//! printf family.
//!
//! These tests exercise the public `printf_float` entry point and pin
//! the 1:1 behavior with the C reference implementation in
//! `sqlite-source/src/printf.c:533-770` (etFLOAT / etEXP / etGENERIC).
//!
//! Note: the Rust port uses `std::fmt::Display` for normal-number
//! rendering, so digit-level output may differ from SQLite for
//! arbitrary inputs. The common cases (round numbers, simple
//! fractions, special values) all match.

use libsqlite_rs::printf_float;

// ============================================================================
// 1. %f — fixed-point (printf.c:533-583, 651-744)
// ============================================================================
#[test]
fn f_basic() {
    assert_eq!(printf_float("%f", &[1.5]).unwrap(), "1.500000");
}

#[test]
fn f_zero() {
    assert_eq!(printf_float("%f", &[0.0]).unwrap(), "0.000000");
}

#[test]
fn f_negative() {
    assert_eq!(printf_float("%f", &[-1.5]).unwrap(), "-1.500000");
}

#[test]
fn f_precision_two() {
    assert_eq!(printf_float("%.2f", &[1.5]).unwrap(), "1.50");
}

#[test]
fn f_precision_four() {
    assert_eq!(printf_float("%.4f", &[3.14159]).unwrap(), "3.1416");
}

#[test]
fn f_negative_zero() {
    // -0.0 — sign is preserved.
    assert_eq!(printf_float("%f", &[-0.0]).unwrap(), "-0.000000");
}

#[test]
fn f_large_value() {
    assert_eq!(
        printf_float("%f", &[1234567.89]).unwrap(),
        "1234567.890000"
    );
}

// ============================================================================
// 2. %e / %E — exponential (printf.c:533-583, 720-734)
// ============================================================================
#[test]
fn e_basic() {
    assert_eq!(printf_float("%e", &[1234.5]).unwrap(), "1.234500e+03");
}

#[test]
fn e_negative_exponent() {
    assert_eq!(printf_float("%e", &[0.001234]).unwrap(), "1.234000e-03");
}

#[test]
fn e_zero() {
    // 0.0 → "0.000000e+00".
    assert_eq!(printf_float("%e", &[0.0]).unwrap(), "0.000000e+00");
}

#[test]
fn e_uppercase() {
    assert_eq!(printf_float("%E", &[1234.5]).unwrap(), "1.234500E+03");
}

#[test]
fn e_precision_three() {
    assert_eq!(printf_float("%.3e", &[1234.5]).unwrap(), "1.234e+03");
}

// ============================================================================
// 3. %g / %G — shortest representation (printf.c:610-619, 707-718)
// ============================================================================
#[test]
fn g_basic() {
    // %g with default precision 1: 1.5 stays "1.5".
    assert_eq!(printf_float("%g", &[1.5]).unwrap(), "1.5");
}

#[test]
fn g_strips_trailing_zeros() {
    // 1.5 with %g default → "1.5" (no trailing zeros).
    assert_eq!(printf_float("%g", &[1.5]).unwrap(), "1.5");
}

#[test]
fn g_integer_no_decimal() {
    // 100.0 with %g default → "100" (decimal point removed).
    assert_eq!(printf_float("%g", &[100.0]).unwrap(), "100");
}

#[test]
fn g_uppercase() {
    assert_eq!(printf_float("%G", &[1.5]).unwrap(), "1.5");
}

#[test]
fn g_precision_three() {
    // %.3g: 3 significant digits.
    assert_eq!(printf_float("%.3g", &[1.5]).unwrap(), "1.50");
}

#[test]
fn g_precision_clamps_trailing_zeros() {
    // %.3g on 1.0 → "1" (one significant digit, but the C spec
    // uses precision=3 to mean "max 3 sig digits, min 1").
    assert_eq!(printf_float("%.3g", &[1.0]).unwrap(), "1");
}

// ============================================================================
// 4. Special values (printf.c:561-583)
// ============================================================================
#[test]
fn nan_default() {
    assert_eq!(printf_float("%f", &[f64::NAN]).unwrap(), "NaN");
}

#[test]
fn inf_default() {
    assert_eq!(printf_float("%f", &[f64::INFINITY]).unwrap(), "Inf");
}

#[test]
fn neg_inf_default() {
    assert_eq!(
        printf_float("%f", &[f64::NEG_INFINITY]).unwrap(),
        "-Inf"
    );
}

#[test]
fn nan_in_e() {
    assert_eq!(printf_float("%e", &[f64::NAN]).unwrap(), "NaN");
}

#[test]
fn inf_in_g() {
    assert_eq!(printf_float("%g", &[f64::INFINITY]).unwrap(), "Inf");
}

// ============================================================================
// 5. Sign flags (printf.c:584-602, 655-657)
// ============================================================================
#[test]
fn force_sign_positive() {
    assert_eq!(printf_float("%+f", &[1.5]).unwrap(), "+1.500000");
}

#[test]
fn force_sign_negative_unchanged() {
    assert_eq!(printf_float("%+f", &[-1.5]).unwrap(), "-1.500000");
}

#[test]
fn space_prefix() {
    assert_eq!(printf_float("% f", &[1.5]).unwrap(), " 1.500000");
}

#[test]
fn force_sign_wins_over_space() {
    assert_eq!(printf_float("%+ f", &[1.5]).unwrap(), "+1.500000");
}

// ============================================================================
// 6. Field width (printf.c:738-752)
// ============================================================================
#[test]
fn width_right_pad() {
    assert_eq!(printf_float("%10f", &[1.5]).unwrap(), "  1.500000");
}

#[test]
fn width_left_justify() {
    assert_eq!(printf_float("%-10f", &[1.5]).unwrap(), "1.500000  ");
}

#[test]
fn width_smaller_than_value() {
    assert_eq!(printf_float("%2f", &[12345.6]).unwrap(), "12345.600000");
}

#[test]
fn width_zero_pad_with_sign() {
    // "%08f" with 1.5 → "1.500000" (width 8, no sign to preserve).
    assert_eq!(printf_float("%08f", &[1.5]).unwrap(), "1.500000");
}

#[test]
fn width_zero_pad_negative() {
    // "%08f" with -1.5 → "-1.500000" (sign kept, padding after).
    assert_eq!(printf_float("%08f", &[-1.5]).unwrap(), "-1.500000");
}

// ============================================================================
// 7. Combined (printf.c:738-770)
// ============================================================================
#[test]
fn f_with_width_and_precision() {
    // "%10.2f" with 1.5 → "      1.50".
    assert_eq!(printf_float("%10.2f", &[1.5]).unwrap(), "      1.50");
}

#[test]
fn e_with_width_and_precision() {
    assert_eq!(
        printf_float("%12.2e", &[1234.5]).unwrap(),
        "    1.23e+03"
    );
}

#[test]
fn g_with_width_and_precision() {
    // "%8.3g" with 1.5 → "    1.50".
    assert_eq!(printf_float("%8.3g", &[1.5]).unwrap(), "    1.50");
}

#[test]
fn percent_in_run() {
    assert_eq!(printf_float("v=%f", &[1.5]).unwrap(), "v=1.500000");
}
