//! `tests/util/printf_str.rs` — integration tests for the string
//! printf family.
//!
//! These tests exercise the public `printf_str` entry point and pin
//! the 1:1 behavior with the C reference implementation in
//! `sqlite-source/src/printf.c:823-869` (etSTRING / etDYNSTRING).

use libsqlite_rs::printf_str;

// ============================================================================
// 1. %s — basic (printf.c:823-867)
// ============================================================================
#[test]
fn s_basic() {
    assert_eq!(printf_str("%s", &[Some("hello")]).unwrap(), "hello");
}

#[test]
fn s_empty() {
    assert_eq!(printf_str("%s", &[Some("")]).unwrap(), "");
}

#[test]
fn s_null_pointer_is_empty_string() {
    // C: `if(bufpt==0){ bufpt = ""; }` — NULL pointer → "".
    assert_eq!(printf_str("%s", &[None]).unwrap(), "");
}

#[test]
fn s_in_run() {
    assert_eq!(
        printf_str("count=%s items", &[Some("three")]).unwrap(),
        "count=three items"
    );
}

#[test]
fn s_missing_arg_is_null_pointer() {
    // C's getTextArg returns NULL when out of args. None here is
    // treated as NULL → "".
    assert_eq!(printf_str("[%s]", &[]).unwrap(), "[]");
}

// ============================================================================
// 2. %s with precision (printf.c:853-867)
// ============================================================================
#[test]
fn s_precision_truncates_bytes() {
    // Without `!` flag, precision is in bytes.
    assert_eq!(printf_str("%.3s", &[Some("hello")]).unwrap(), "hel");
}

#[test]
fn s_precision_larger_than_input() {
    assert_eq!(printf_str("%.10s", &[Some("hi")]).unwrap(), "hi");
}

#[test]
fn s_precision_zero_is_empty() {
    assert_eq!(printf_str("%.0s", &[Some("hello")]).unwrap(), "");
}

#[test]
fn s_precision_clamps_at_nul() {
    // C: `for(length=0; length<precision && bufpt[length]; length++)`
    // — precision is the max, but NUL terminates.
    assert_eq!(printf_str("%.10s", &[Some("hi")]).unwrap(), "hi");
}

#[test]
fn s_precision_at_utf8_boundary() {
    // "héllo" — 'h' (1), 'é' (2), 'l' (1), 'l' (1), 'o' (1) = 6 bytes.
    // The C source truncates by raw byte count (printf.c:863). For
    // precision 4, the output is the first 4 bytes: 'h' + 'é' (2 bytes)
    // + 'l' = "hél" (3 chars, 4 bytes). The 'é' is included whole
    // because its 2 bytes fit within the precision window.
    let s = "héllo";
    assert_eq!(printf_str("%.4s", &[Some(s)]).unwrap(), "hél");
}

// ============================================================================
// 3. %!s — UTF-8 character precision (printf.c:854-861)
// ============================================================================
#[test]
fn s_alt_form2_two_chars() {
    // !s with precision 2 → first 2 chars → "hé".
    let s = "héllo";
    assert_eq!(printf_str("%!.2s", &[Some(s)]).unwrap(), "hé");
}

#[test]
fn s_alt_form2_three_byte_codepoint() {
    // "中文" — first 1 char = "中" (3 bytes).
    let s = "中文";
    assert_eq!(printf_str("%!.1s", &[Some(s)]).unwrap(), "中");
}

#[test]
fn s_alt_form2_with_width() {
    // "%!5.2s" with "héllo" → "   hé" (left-pad to width 5, with
    // width measured in chars because of the `!` flag). The C source
    // left-pads by default (adjust_width_for_utf8 path at printf.c:868).
    let s = "héllo";
    assert_eq!(printf_str("%!5.2s", &[Some(s)]).unwrap(), "   hé");
}

// ============================================================================
// 4. %s with field width (printf.c:868-880)
// ============================================================================
#[test]
fn s_width_right_pad() {
    assert_eq!(printf_str("%5s", &[Some("hi")]).unwrap(), "   hi");
}

#[test]
fn s_width_left_justify() {
    assert_eq!(printf_str("%-5s", &[Some("hi")]).unwrap(), "hi   ");
}

#[test]
fn s_width_smaller_than_value() {
    assert_eq!(printf_str("%2s", &[Some("hello")]).unwrap(), "hello");
}

#[test]
fn s_width_null_pointer() {
    // NULL with width 5 → 5 spaces.
    assert_eq!(printf_str("%5s", &[None]).unwrap(), "     ");
}

#[test]
fn s_width_null_left_justify() {
    assert_eq!(printf_str("%-5s", &[None]).unwrap(), "     ");
}

// ============================================================================
// 5. Combined (printf.c:863-880)
// ============================================================================
#[test]
fn s_precision_and_width() {
    // "%5.3s" with "hello" → "  hel" (precision trims to 3, then pad).
    assert_eq!(printf_str("%5.3s", &[Some("hello")]).unwrap(), "  hel");
}

#[test]
fn s_precision_and_left_justify() {
    // "%-5.3s" with "hello" → "hel  " (precision trims, then left-pad).
    assert_eq!(printf_str("%-5.3s", &[Some("hello")]).unwrap(), "hel  ");
}

#[test]
fn s_multiple_args() {
    assert_eq!(
        printf_str("%s + %s", &[Some("foo"), Some("bar")]).unwrap(),
        "foo + bar"
    );
}

#[test]
fn s_no_format_just_literal() {
    // Sanity: empty args list, format with no %s — the literal
    // passes through.
    assert_eq!(printf_str("no args here", &[]).unwrap(), "no args here");
}

#[test]
fn s_literal_passthrough_with_one_format() {
    assert_eq!(
        printf_str("[%s]", &[Some("hi")]).unwrap(),
        "[hi]"
    );
}
