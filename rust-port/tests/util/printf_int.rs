//! `tests/util/printf_int.rs` — integration tests for the integer
//! printf family.
//!
//! These tests exercise the public `printf_int` entry point and pin
//! the 1:1 behavior with the C reference implementation in
//! `sqlite-source/src/printf.c:416-532` (and the flag parsing in
//! `printf.c:264-366`).
//!
//! Each test case is annotated with the C source line range and the
//! behavior under test.

use libsqlite_rs::printf_int;

// ============================================================================
// 1. %d — basic signed decimal (printf.c:425-447, 495-509)
// ============================================================================
#[test]
fn d_positive_basic() {
    assert_eq!(printf_int("%d", &[42]).unwrap(), "42");
}

#[test]
fn d_zero() {
    assert_eq!(printf_int("%d", &[0]).unwrap(), "0");
}

#[test]
fn d_negative_basic() {
    assert_eq!(printf_int("%d", &[-42]).unwrap(), "-42");
}

#[test]
fn d_i64_min_boundary() {
    // printf("%d", INT64_MIN) must not overflow. The C code's
    // `longvalue = ~v; longvalue++;` trick is the canonical way to
    // handle this; we mirror that in render_decimal.
    assert_eq!(
        printf_int("%d", &[i64::MIN]).unwrap(),
        "-9223372036854775808"
    );
}

#[test]
fn d_i64_max_boundary() {
    assert_eq!(
        printf_int("%d", &[i64::MAX]).unwrap(),
        "9223372036854775807"
    );
}

// ============================================================================
// 2. %i — synonym for %d (printf.c:103, fmtinfo[5])
// ============================================================================
#[test]
fn i_is_alias_for_d() {
    assert_eq!(printf_int("%i", &[-1]).unwrap(), "-1");
    assert_eq!(printf_int("%i", &[0]).unwrap(), "0");
}

// ============================================================================
// 3. %u — unsigned decimal (printf.c:448-461)
// ============================================================================
#[test]
fn u_zero() {
    assert_eq!(printf_int("%u", &[0]).unwrap(), "0");
}

#[test]
fn u_positive() {
    assert_eq!(printf_int("%u", &[42]).unwrap(), "42");
}

#[test]
fn u_negative_interpreted_as_unsigned() {
    // -1 as i64 → 0xFFFF_FFFF_FFFF_FFFF as u64 → "18446744073709551615".
    assert_eq!(
        printf_int("%u", &[-1]).unwrap(),
        "18446744073709551615"
    );
}

// ============================================================================
// 4. %x / %X — hex (printf.c:495-509)
// ============================================================================
#[test]
fn x_lowercase() {
    assert_eq!(printf_int("%x", &[0xCAFE]).unwrap(), "cafe");
}

#[test]
fn x_uppercase() {
    assert_eq!(printf_int("%X", &[0xCAFE]).unwrap(), "CAFE");
}

#[test]
fn x_zero() {
    assert_eq!(printf_int("%x", &[0]).unwrap(), "0");
}

#[test]
fn x_alt_form_lowercase() {
    assert_eq!(printf_int("%#x", &[0xCAFE]).unwrap(), "0xcafe");
}

#[test]
fn x_alt_form_uppercase() {
    assert_eq!(printf_int("%#X", &[0xCAFE]).unwrap(), "0XCAFE");
}

#[test]
fn x_alt_form_zero_omitted() {
    // #x with 0 does NOT include the "0x" prefix.
    assert_eq!(printf_int("%#x", &[0]).unwrap(), "0");
}

// ============================================================================
// 5. %o — octal (printf.c:495-509)
// ============================================================================
#[test]
fn o_basic() {
    assert_eq!(printf_int("%o", &[8]).unwrap(), "10");
    assert_eq!(printf_int("%o", &[0o755]).unwrap(), "755");
}

#[test]
fn o_alt_form() {
    assert_eq!(printf_int("%#o", &[0o755]).unwrap(), "0755");
}

#[test]
fn o_alt_form_zero_omitted() {
    assert_eq!(printf_int("%#o", &[0]).unwrap(), "0");
}

// ============================================================================
// 6. %p — pointer (printf.c:525-530; fmtinfo[7] prefix=1)
// ============================================================================
#[test]
fn p_basic() {
    assert_eq!(printf_int("%p", &[0xCAFE]).unwrap(), "0xcafe");
}

#[test]
fn p_zero() {
    // %p is 16-based with "0x" prefix as a structural property, not
    // a flag. The C code's `if(longvalue==0) flag_alternateform = 0;`
    // only clears the flag when it was set by `#`; %p keeps its
    // prefix. So 0 renders as "0x0".
    assert_eq!(printf_int("%p", &[0]).unwrap(), "0x0");
}

// ============================================================================
// 7. %% — percent (printf.c:260-263, 408-410)
// ============================================================================
#[test]
fn percent_percent() {
    assert_eq!(printf_int("%%", &[]).unwrap(), "%");
    assert_eq!(printf_int("100%%", &[]).unwrap(), "100%");
}

#[test]
fn percent_percent_in_run() {
    assert_eq!(printf_int("[%d%%]", &[50]).unwrap(), "[50%]");
}

// ============================================================================
// 8. Field width (printf.c:291-310, 504-509)
// ============================================================================
#[test]
fn width_right_pad_spaces() {
    assert_eq!(printf_int("%5d", &[42]).unwrap(), "   42");
}

#[test]
fn width_left_justify_dash() {
    assert_eq!(printf_int("%-5d", &[42]).unwrap(), "42   ");
}

#[test]
fn width_wider_than_value() {
    assert_eq!(printf_int("%10d", &[-1]).unwrap(), "        -1");
}

#[test]
fn width_zero_pad() {
    assert_eq!(printf_int("%05d", &[42]).unwrap(), "00042");
}

#[test]
fn width_zero_pad_with_sign() {
    // Sign is preserved at the start; zeros pad between sign and digits.
    assert_eq!(printf_int("%06d", &[-42]).unwrap(), "-00042");
}

#[test]
fn width_smaller_than_value_unchanged() {
    assert_eq!(printf_int("%2d", &[12345]).unwrap(), "12345");
}

// ============================================================================
// 9. Precision (printf.c:332-363, 504-509)
// ============================================================================
#[test]
fn precision_min_digits() {
    assert_eq!(printf_int("%.5d", &[42]).unwrap(), "00042");
}

#[test]
fn precision_larger_than_value_no_truncate() {
    assert_eq!(printf_int("%.3d", &[12345]).unwrap(), "12345");
}

#[test]
fn precision_zero_emits_zero() {
    // SQLite's "%.0d" on 0 → "0" (the digit loop emits at least one
    // digit, per printf.c:498-501). C99 would say ""; SQLite diverges.
    assert_eq!(printf_int("%.0d", &[0]).unwrap(), "0");
}

#[test]
fn precision_combined_with_width() {
    // printf("%8.5d", 42) → "   00042" (precision dominates width
    // for zero-pad position).
    assert_eq!(printf_int("%8.5d", &[42]).unwrap(), "   00042");
}

#[test]
fn precision_x() {
    assert_eq!(printf_int("%.4x", &[0xAB]).unwrap(), "00ab");
}

#[test]
fn precision_o() {
    assert_eq!(printf_int("%.4o", &[0o7]).unwrap(), "0007");
}

#[test]
fn precision_u() {
    assert_eq!(printf_int("%.4u", &[7]).unwrap(), "0007");
}

// ============================================================================
// 10. Sign flags (printf.c:273-275, 438-447)
// ============================================================================
#[test]
fn force_sign_positive() {
    assert_eq!(printf_int("%+d", &[42]).unwrap(), "+42");
}

#[test]
fn force_sign_zero() {
    assert_eq!(printf_int("%+d", &[0]).unwrap(), "+0");
}

#[test]
fn force_sign_negative_unchanged() {
    assert_eq!(printf_int("%+d", &[-42]).unwrap(), "-42");
}

#[test]
fn space_prefix() {
    assert_eq!(printf_int("% d", &[42]).unwrap(), " 42");
}

#[test]
fn force_sign_wins_over_space() {
    assert_eq!(printf_int("%+ d", &[42]).unwrap(), "+42");
}

// ============================================================================
// 11. Alt-form flags (printf.c:276, 470, 525-530)
// ============================================================================
#[test]
fn alt_o_with_zero_value() {
    // Already covered by `o_alt_form_zero_omitted`, but pinning
    // explicit %o + # + 0 here for the matrix.
    assert_eq!(printf_int("%#o", &[0]).unwrap(), "0");
}

#[test]
fn alt_x_with_negative_value() {
    // Negative values get re-interpreted as u64 in render_radix.
    assert_eq!(printf_int("%#x", &[-1]).unwrap(), "0xffffffffffffffff");
}

#[test]
fn alt_x_uppercase_with_value() {
    assert_eq!(printf_int("%#X", &[0xDEAD_BEEF]).unwrap(), "0XDEADBEEF");
}

// ============================================================================
// 12. * (width / precision from arg) (printf.c:311-331, 332-345)
// ============================================================================
#[test]
fn width_from_arg_star() {
    assert_eq!(printf_int("%*d", &[5, 42]).unwrap(), "   42");
}

#[test]
fn width_from_arg_star_negative_is_left_justified() {
    // printf("%*d", -5, 42) → "42   " (negative width = left).
    assert_eq!(printf_int("%*d", &[-5, 42]).unwrap(), "42   ");
}

#[test]
fn precision_from_arg_star() {
    assert_eq!(printf_int("%.*d", &[5, 42]).unwrap(), "00042");
}

#[test]
fn width_and_precision_from_star() {
    assert_eq!(printf_int("%*.*d", &[8, 5, 42]).unwrap(), "   00042");
}

// ============================================================================
// 13. Length modifiers (printf.c:281-289)
// ============================================================================
#[test]
fn l_long_no_effect_on_i64_args() {
    // In the Rust port all ints are i64, so `%ld` and `%lld` are
    // equivalent to `%d`. The C version's flag_long only changes
    // which va_arg slot is read; for us there is only one slot.
    assert_eq!(printf_int("%ld", &[42]).unwrap(), "42");
    assert_eq!(printf_int("%lld", &[-42]).unwrap(), "-42");
}

// ============================================================================
// 14. Multi-spec, mixed types (printf.c:246-258)
// ============================================================================
#[test]
fn multi_spec_run() {
    assert_eq!(
        printf_int("d=%d u=%u x=%x X=%X o=%o p=%p %%",
                   &[10, 10, 0xAB, 0xAB, 8, 0xCAFE]).unwrap(),
        "d=10 u=10 x=ab X=AB o=10 p=0xcafe %"
    );
}

#[test]
fn literal_run_with_one_spec() {
    assert_eq!(
        printf_int("count=%d items", &[3]).unwrap(),
        "count=3 items"
    );
}

#[test]
fn missing_arg_returns_zero() {
    // C's getIntArg returns 0 when out of args (printf.c:144-147).
    // The Rust port models this by returning 0 from the closure.
    assert_eq!(printf_int("%d", &[]).unwrap(), "0");
}
