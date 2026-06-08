//! `tests/util/printf_sqlite.rs` — integration tests for the
//! SQLite-specific printf family (`%q / %Q / %w / %z`).
//!
//! These tests exercise the public `printf_sqlite` entry point and
//! pin the 1:1 behavior with the C reference implementation in
//! `sqlite-source/src/printf.c:952-1058`.

use libsqlite_rs::printf_sqlite;

// ============================================================================
// 1. %q — escape single quotes (printf.c:952-967, 1030-1051)
// ============================================================================
#[test]
fn q_basic() {
    assert_eq!(printf_sqlite("%q", &[Some("hello")]).unwrap(), "hello");
}

#[test]
fn q_doubles_inner_quote() {
    // The single quote in "it's" gets doubled → "it''s".
    assert_eq!(printf_sqlite("%q", &[Some("it's")]).unwrap(), "it''s");
}

#[test]
fn q_only_quotes() {
    // The whole string is just quotes — every one gets doubled.
    assert_eq!(printf_sqlite("%q", &[Some("''")]).unwrap(), "''''");
}

#[test]
fn q_null_pointer() {
    // C: NULL → "(NULL)" (printf.c:967).
    assert_eq!(printf_sqlite("%q", &[None]).unwrap(), "(NULL)");
}

#[test]
fn q_in_run() {
    assert_eq!(
        printf_sqlite("name=%q", &[Some("O'Brien")]).unwrap(),
        "name=O''Brien"
    );
}

#[test]
fn q_precision_truncates_input() {
    // %.3q with "it's" → reads 3 input bytes (i, t, ') and the
    // trailing quote gets doubled → "it''".
    assert_eq!(printf_sqlite("%.3q", &[Some("it's")]).unwrap(), "it''");
}

#[test]
fn q_precision_clamps_at_nul() {
    assert_eq!(printf_sqlite("%.10q", &[Some("hi")]).unwrap(), "hi");
}

// ============================================================================
// 2. %Q — escape + wrap in '...' (printf.c:953, 968-970, 1021-1027)
// ============================================================================
#[test]
fn big_q_basic() {
    assert_eq!(printf_sqlite("%Q", &[Some("hi")]).unwrap(), "'hi'");
}

#[test]
fn big_q_doubles_quotes_inside() {
    assert_eq!(
        printf_sqlite("%Q", &[Some("it's")]).unwrap(),
        "'it''s'"
    );
}

#[test]
fn big_q_null_pointer_is_sql_null() {
    // C: NULL → "NULL" (printf.c:967).
    assert_eq!(printf_sqlite("%Q", &[None]).unwrap(), "NULL");
}

#[test]
fn big_q_always_wraps_non_null() {
    // Even an empty string gets the quotes.
    assert_eq!(printf_sqlite("%Q", &[Some("")]).unwrap(), "''");
}

// ============================================================================
// 3. %w — escape double quotes (printf.c:954, 971-973)
// ============================================================================
#[test]
fn w_basic() {
    assert_eq!(printf_sqlite("%w", &[Some("hello")]).unwrap(), "hello");
}

#[test]
fn w_doubles_inner_double_quote() {
    // The double quote in `a"b` gets doubled → `a""b`.
    assert_eq!(printf_sqlite("%w", &[Some("a\"b")]).unwrap(), "a\"\"b");
}

#[test]
fn w_null_pointer() {
    assert_eq!(printf_sqlite("%w", &[None]).unwrap(), "(NULL)");
}

#[test]
fn w_alt_form_suppressed() {
    // The C source clears flag_alternateform for %w (printf.c:973),
    // so the # flag has no effect.
    assert_eq!(
        printf_sqlite("%#w", &[Some("a\"b")]).unwrap(),
        "a\"\"b"
    );
}

// ============================================================================
// 4. %z — dynamic string, alias for %s (printf.c:833-851)
// ============================================================================
#[test]
fn z_basic() {
    assert_eq!(printf_sqlite("%z", &[Some("hi")]).unwrap(), "hi");
}

#[test]
fn z_null_pointer_is_empty() {
    // %z treats NULL like %s → "".
    assert_eq!(printf_sqlite("%z", &[None]).unwrap(), "");
}

#[test]
fn z_precision_truncates() {
    assert_eq!(printf_sqlite("%.3z", &[Some("hello")]).unwrap(), "hel");
}

#[test]
fn z_width_pads() {
    assert_eq!(printf_sqlite("%5z", &[Some("hi")]).unwrap(), "   hi");
}

// ============================================================================
// 5. Combined (printf.c:925-944 — width / padding for the q-family)
// ============================================================================
#[test]
fn q_with_width() {
    // "%5q" with "hi" → "   hi" (the body is "hi", pad to width 5).
    assert_eq!(printf_sqlite("%5q", &[Some("hi")]).unwrap(), "   hi");
}

#[test]
fn big_q_with_width() {
    // "%8Q" with "hi" → "     'hi'" (the body is "'hi'" = 4 chars,
    // pad to width 8).
    assert_eq!(
        printf_sqlite("%8Q", &[Some("hi")]).unwrap(),
        "    'hi'"
    );
}

#[test]
fn w_with_width() {
    assert_eq!(printf_sqlite("%5w", &[Some("hi")]).unwrap(), "   hi");
}

#[test]
fn q_in_complex_run() {
    // Note: the integer spec is not supported by printf_sqlite, so
    // it passes through literally. The string spec is processed.
    assert_eq!(
        printf_sqlite("name=%Q id=%d", &[Some("O'Brien")]).unwrap(),
        "name='O''Brien' id=%d"
    );
}

// ============================================================================
// 6. Alt-form backslash escaping (printf.c:989-1011, 1030-1045)
// ============================================================================
#[test]
fn q_alt_form_escapes_backslash() {
    // %#q escapes backslash as "\\".
    assert_eq!(printf_sqlite("%#q", &[Some("a\\b")]).unwrap(), "a\\\\b");
}

#[test]
fn q_alt_form_escapes_control_chars() {
    assert_eq!(printf_sqlite("%#q", &[Some("a\nb")]).unwrap(), "a\\u000ab");
}

#[test]
fn big_q_alt_form_escapes_when_control() {
    // %#Q escapes control chars AND wraps in single quotes
    // (needQuote == 2 means "unistr('...')"-style).
    // Actually looking at the C source: %#Q with control chars
    // produces `unistr('...')` (printf.c:1023-1027).
    let out = printf_sqlite("%#Q", &[Some("a\nb")]).unwrap();
    // The escape form: 'unistr(' + escaped content + ')'.
    assert!(out.contains("unistr("));
    assert!(out.ends_with(')'));
}

#[test]
fn big_q_alt_form_plain_when_no_control() {
    // %#Q without control chars falls back to plain %Q.
    assert_eq!(
        printf_sqlite("%#Q", &[Some("clean text")]).unwrap(),
        "'clean text'"
    );
}
