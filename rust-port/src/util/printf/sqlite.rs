//! SQLite-specific printf conversions — 1:1 port of `printf.c:952-1058`.
//!
//! Implements the body rendering for the SQLite extensions:
//! - `%q` — escape single quotes (double them); NULL → "(NULL)"
//! - `%Q` — escape single quotes and wrap in `'...'`; NULL → "NULL"
//! - `%w` — escape double quotes (double them); NULL → "(NULL)"
//! - `%z` — dynamic string (alias for `%s` for our port; the
//!   "dynamic" aspect is a C memory-management concern that does
//!   not apply to the Rust port)
//!
//! # `#` flag (alt-form) handling
//!
//! The C source's `#` flag enables backslash-escape mode (printf.c:989-1011):
//! - `%#q` — always unistr()-style escapes for control chars and backslash
//! - `%#Q` — same, but only if there's at least one control char
//! - `%#w` — same as `%#q` (printf.c:973 clears `flag_alternateform` for `%w`)
//!
//! Our port mirrors this behavior.
//!
//! # `!` flag (alt-form2) — UTF-8 character precision
//!
//! For `%q / %Q / %w`, the `!` flag converts the precision from
//! bytes to UTF-8 code points (printf.c:985-987). This is mostly
//! relevant for SQL string literals containing multi-byte chars.

use crate::error::SqliteResult;

use super::FormatSpec;

/// Render the body of `%q` (escape single quotes).
pub fn render_q(value: Option<&str>, spec: &FormatSpec) -> SqliteResult<String> {
    // C: `escarg == 0` → "(NULL)" (printf.c:966-967).
    let s = match value {
        Some(s) => s,
        None => return Ok("(NULL)".to_string()),
    };
    Ok(escape_quotes(s, '\'', spec, true))
}

/// Render the body of `%Q` (escape single quotes + wrap in '...').
pub fn render_big_q(value: Option<&str>, spec: &FormatSpec) -> SqliteResult<String> {
    // C: NULL → "NULL" (printf.c:966-967).
    let s = match value {
        Some(s) => s,
        None => return Ok("NULL".to_string()),
    };
    // %#Q with control chars produces `unistr('...')` (printf.c:1023-1027).
    if spec.alt_form && has_control_chars(s) {
        let inner = escape_quotes(s, '\'', spec, false);
        return Ok(format!("unistr('{}')", inner));
    }
    let inner = escape_quotes(s, '\'', spec, false);
    Ok(format!("'{}'", inner))
}

/// `true` if `s` contains any byte <= 0x1f (excluding 0 which is NUL).
fn has_control_chars(s: &str) -> bool {
    s.bytes().any(|b| b > 0 && b <= 0x1f)
}

/// Render the body of `%w` (escape double quotes).
pub fn render_w(value: Option<&str>, spec: &FormatSpec) -> SqliteResult<String> {
    // C: NULL → "(NULL)" (printf.c:966-967); the `#` flag is cleared
    // for %w so the alt-form backslash escaping does not apply.
    let s = match value {
        Some(s) => s,
        None => return Ok("(NULL)".to_string()),
    };
    let mut s2 = spec.clone();
    s2.alt_form = false; // %w: alt_form is always off (printf.c:973).
    Ok(escape_quotes(s, '"', &s2, false))
}

/// Render the body of `%z` (dynamic string).
///
/// The C source distinguishes `%z` from `%s` only in memory
/// management (printf.c:833-851: a `%z` may take ownership of the
/// malloced buffer). In the Rust port this is a no-op distinction —
/// `%z` behaves exactly like `%s`.
pub fn render_z(value: Option<&str>, spec: &FormatSpec) -> SqliteResult<String> {
    super::str::render_string(value, spec)
}

/// Common escape routine — doubles the quote character `q` and applies
/// the alt-form (`#`) backslash escapes if enabled. `wrap_q` controls
/// whether the alt-form's wrapping logic should match the C source's
/// `%q` (always) vs `%Q` (only with control chars).
fn escape_quotes(s: &str, q: char, spec: &FormatSpec, _wrap_q: bool) -> String {
    // Precision handling. The C source at printf.c:982-988:
    //   - precision < 0  → walk to NUL terminator
    //   - precision >= 0 → walk up to that many bytes (or chars, with !)
    //
    // We model this as `Option<usize>`: `None` = no limit (use whole
    // string), `Some(n)` = stop at n bytes / chars.
    let byte_limit: Option<usize> = if spec.alt_form2 {
        None // char-mode: byte_limit not used
    } else if spec.precision < 0 {
        None
    } else {
        Some(spec.precision as usize)
    };
    let char_limit: Option<usize> = if !spec.alt_form2 {
        None
    } else if spec.precision < 0 {
        None
    } else {
        Some(spec.precision as usize)
    };

    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    let mut chars_taken = 0usize;
    while i < s.len() {
        if let Some(bl) = byte_limit {
            if i >= bl {
                break;
            }
        }
        if let Some(cl) = char_limit {
            if chars_taken >= cl {
                break;
            }
        }
        let ch = s[i..].chars().next().unwrap();
        let ch_len = ch.len_utf8();
        if spec.alt_form {
            // Backslash-escape mode (printf.c:1030-1045).
            if ch == '\\' {
                out.push('\\');
                out.push('\\');
            } else if (ch as u32) <= 0x1f {
                out.push('\\');
                out.push('u');
                out.push('0');
                out.push('0');
                if (ch as u32) >= 0x10 {
                    out.push('1');
                } else {
                    out.push('0');
                }
                let h = (ch as u32) & 0xf;
                out.push(HEX_DIGITS[h as usize] as char);
            } else {
                out.push(ch);
            }
            if ch == q {
                out.push(q);
            }
        } else {
            // Plain mode (printf.c:1046-1051).
            out.push(ch);
            if ch == q {
                out.push(q);
            }
        }
        i += ch_len;
        chars_taken += 1;
    }
    out
}

const HEX_DIGITS: &[u8] = b"0123456789abcdef";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::printf::FormatSpec;

    fn spec() -> FormatSpec {
        FormatSpec::new()
    }

    #[test]
    fn q_basic() {
        assert_eq!(render_q(Some("hello"), &spec()).unwrap(), "hello");
    }

    #[test]
    fn q_doubles_single_quotes() {
        assert_eq!(render_q(Some("it's"), &spec()).unwrap(), "it''s");
    }

    #[test]
    fn q_multiple_quotes() {
        assert_eq!(render_q(Some("'a'"), &spec()).unwrap(), "''a''");
    }

    #[test]
    fn q_null_is_literal_null() {
        assert_eq!(render_q(None, &spec()).unwrap(), "(NULL)");
    }

    #[test]
    fn q_precision_truncates() {
        // The C source (printf.c:982-988) uses precision to limit the
        // number of input bytes read, NOT the output bytes. The
        // doubling happens on the bytes within the precision window.
        // So for "it's" with precision 3 we read 3 input bytes
        // ('i', 't', '\'') and the quote gets doubled → "it''"
        // (4 output bytes).
        let mut s = spec();
        s.precision = 3;
        assert_eq!(render_q(Some("it's"), &s).unwrap(), "it''");
    }

    #[test]
    fn big_q_wraps_in_single_quotes() {
        assert_eq!(render_big_q(Some("hi"), &spec()).unwrap(), "'hi'");
    }

    #[test]
    fn big_q_escapes_inner_quotes() {
        assert_eq!(
            render_big_q(Some("it's"), &spec()).unwrap(),
            "'it''s'"
        );
    }

    #[test]
    fn big_q_null_is_sql_null() {
        assert_eq!(render_big_q(None, &spec()).unwrap(), "NULL");
    }

    #[test]
    fn w_doubles_double_quotes() {
        assert_eq!(render_w(Some("a\"b"), &spec()).unwrap(), "a\"\"b");
    }

    #[test]
    fn w_null_is_literal_null() {
        assert_eq!(render_w(None, &spec()).unwrap(), "(NULL)");
    }

    #[test]
    fn w_no_alt_form_for_q() {
        // %w ignores the # flag (printf.c:973).
        let mut s = spec();
        s.alt_form = true;
        // The control char in the input would normally be
        // backslash-escaped, but for %w the # flag is suppressed.
        assert_eq!(render_w(Some("a\nb"), &s).unwrap(), "a\nb");
    }

    #[test]
    fn z_is_alias_for_s() {
        assert_eq!(render_z(Some("hi"), &spec()).unwrap(), "hi");
    }

    #[test]
    fn z_null_is_empty() {
        assert_eq!(render_z(None, &spec()).unwrap(), "");
    }
}
