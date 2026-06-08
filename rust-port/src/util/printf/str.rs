//! String printf conversions — 1:1 port of `printf.c:823-869`.
//!
//! Implements the body rendering for the `%s` family. Width / padding
//! is then applied by the parent `printf` module via [`apply_width`].
//!
//! # Spec → conversion
//!
//! | Spec | Behavior |
//! |------|----------|
//! | `s`  | Read `&str` from arg list. NULL → "". Precision = max bytes (or chars, with `!`). Width pads with spaces. |
//!
//! # NULL handling
//!
//! The C source at printf.c:831-832 maps `NULL` to the empty string.
//! The Rust port uses `Option<&str>` — `None` maps to the empty
//! string, matching the C behavior.
//!
//! # Precision modes
//!
//! Without `!` (`flag_altform2`): precision is the **maximum number
//! of bytes** (printf.c:863). With `!`: precision is the **maximum
//! number of UTF-8 characters** (printf.c:854-861 uses SQLITE_SKIP_UTF8
//! to walk the string char-by-char).

use crate::error::SqliteResult;

use super::FormatSpec;

/// Render the body of a `%s` directive.
///
/// `value` is `None` for the C `NULL` pointer case. The return value
/// is the body string (no width / padding).
pub fn render_string(value: Option<&str>, spec: &FormatSpec) -> SqliteResult<String> {
    // C: `if(bufpt==0){ bufpt = ""; }` — NULL pointer maps to "".
    let s = match value {
        Some(s) => s,
        None => "",
    };

    if spec.precision < 0 {
        // No precision limit: emit the whole string. C uses
        // 0x7fffffff as the upper bound; we just take the full slice.
        return Ok(s.to_string());
    }

    if spec.alt_form2 {
        // `!s` — precision is in characters, not bytes.
        // Walk the string char-by-char until we hit `precision`
        // characters or end of string.
        let mut bytes_taken = 0usize;
        let mut chars_taken = 0usize;
        for (i, _ch) in s.char_indices() {
            if chars_taken >= spec.precision as usize {
                break;
            }
            bytes_taken = i + _ch.len_utf8();
            chars_taken += 1;
        }
        if chars_taken >= spec.precision as usize {
            return Ok(s[..bytes_taken].to_string());
        }
        // Didn't reach precision: take the whole string.
        Ok(s.to_string())
    } else {
        // Plain `%s` with precision: limit to that many bytes (C
        // semantics — not necessarily char-aligned).
        let max = (spec.precision as usize).min(s.len());
        Ok(s[..max].to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::printf::FormatSpec;

    #[test]
    fn s_basic() {
        assert_eq!(render_string(Some("hello"), &FormatSpec::new()).unwrap(), "hello");
    }

    #[test]
    fn s_null_is_empty() {
        assert_eq!(render_string(None, &FormatSpec::new()).unwrap(), "");
    }

    #[test]
    fn s_empty() {
        assert_eq!(render_string(Some(""), &FormatSpec::new()).unwrap(), "");
    }

    #[test]
    fn s_precision_truncates() {
        let mut spec = FormatSpec::new();
        spec.precision = 3;
        assert_eq!(render_string(Some("hello"), &spec).unwrap(), "hel");
    }

    #[test]
    fn s_precision_larger_than_string() {
        let mut spec = FormatSpec::new();
        spec.precision = 100;
        assert_eq!(render_string(Some("hi"), &spec).unwrap(), "hi");
    }

    #[test]
    fn s_precision_zero_is_empty() {
        let mut spec = FormatSpec::new();
        spec.precision = 0;
        assert_eq!(render_string(Some("hello"), &spec).unwrap(), "");
    }

    #[test]
    fn s_alt_form2_truncates_by_char() {
        // !s with precision 2 → first 2 characters of "héllo" → "hé".
        let mut spec = FormatSpec::new();
        spec.precision = 2;
        spec.alt_form2 = true;
        let s = "héllo";
        assert_eq!(render_string(Some(s), &spec).unwrap(), "hé");
    }

    #[test]
    fn s_alt_form2_three_byte_codepoint() {
        // !s with precision 1 → first character (1 char = 3 bytes) → "中".
        let mut spec = FormatSpec::new();
        spec.precision = 1;
        spec.alt_form2 = true;
        assert_eq!(render_string(Some("中文"), &spec).unwrap(), "中");
    }
}
