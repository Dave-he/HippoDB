//! Integer printf conversions — 1:1 port of `printf.c:416-532`.
//!
//! Implements the body rendering (digits + sign + alt-form prefix) for
//! the integer format specifiers. Width / zero-pad is then applied by
//! the parent `printf` module via [`apply_width`].
//!
//! # Spec → conversion matrix
//!
//! | Spec | Signed? | Base | Lower? | Alt prefix |
//! |------|---------|------|--------|------------|
//! | `d`  | yes     | 10   | n/a    | n/a        |
//! | `i`  | yes     | 10   | n/a    | n/a        |
//! | `u`  | no      | 10   | n/a    | n/a        |
//! | `x`  | no      | 16   | lower  | `0x`       |
//! | `X`  | no      | 16   | upper  | `0X`       |
//! | `o`  | no      | 8    | n/a    | `0`        |
//! | `p`  | no      | 16   | lower  | `0x` (forced) |
//!
//! The `p` spec is always 16-based with `0x` prefix regardless of
//! `#` (printf.c:525-530 treats `#` as adding the prefix; the C
//! `fmtinfo` entry for `p` always has `prefix=1`).
//!
//! The C code reuses one code path for DECIMAL / RADIX / POINTER /
//! ORDINAL (printf.c:421-532) — we extract just the integer parts.

use crate::error::SqliteResult;

use super::FormatSpec;

/// Digits for both upper-case and lower-case hex. The C `aDigits`
/// table at printf.c:94 holds two parallel 16-byte sets; we keep them
/// separate for clarity.
const DIGITS_LOWER: &[u8; 16] = b"0123456789abcdef";
const DIGITS_UPPER: &[u8; 16] = b"0123456789ABCDEF";

/// Render the body of an integer `%`-directive.
///
/// `type_byte` is one of `d / i / u / x / X / o / p`. The result is
/// the raw body — sign (if signed), alt-form prefix (if any), and
/// digits — but **no width / padding**. The parent module applies
/// `width` after the body is returned.
///
/// `value` is treated as `i64` for the signed specs (`d`, `i`) and as
/// `u64` for the rest. The caller is responsible for promoting the
/// input (we receive it as `u64` because the dispatch loop already
/// pulled it out of the args list).
pub fn render_int(
    type_byte: u8,
    value: u64,
    spec: &FormatSpec,
) -> SqliteResult<String> {
    match type_byte {
        b'd' | b'i' => render_decimal(value, spec),
        b'u' => render_unsigned_decimal(value, spec),
        b'x' => render_radix(value, 16, DIGITS_LOWER, spec, b"0x"),
        b'X' => render_radix(value, 16, DIGITS_UPPER, spec, b"0X"),
        b'o' => render_radix(value, 8, DIGITS_LOWER, spec, b"0"),
        b'p' => {
            // %p renders as 16-based hex with a structural "0x" prefix.
            // Unlike the C source's `flag_alternateform`-gated behavior
            // (printf.c:525-530), the Rust port always emits the prefix
            // for %p — matching the standard C library (glibc, MSVC)
            // and the SQL function `printf('%p', ...)` expectations.
            // The fmtinfo entry (printf.c:107) has `prefix=1` for 'p',
            // which is a structural marker; we treat it as always-on.
            let mut s = *spec;
            s.alt_form = true; // always emit the "0x" prefix
            render_radix(value, 16, DIGITS_LOWER, &s, b"0x")
        }
        _ => unreachable!(
            "render_int called with non-integer type byte {type_byte} (caller should filter)"
        ),
    }
}

// ---------------------------------------------------------------------------
// Signed decimal — %d, %i (printf.c:425-461, 495-509)
// ---------------------------------------------------------------------------

fn render_decimal(value: u64, spec: &FormatSpec) -> SqliteResult<String> {
    // The C code interprets the value as `i64` when the spec is signed
    // and `flag_long` says i64 (`flag_long == 2`). We do the same: the
    // u64 → i64 reinterpretation is the canonical "two's complement
    // wraparound" the C compiler does for the `va_arg(ap, i64)` call.
    let v = value as i64;
    let abs_u64 = if v < 0 {
        // i64::MIN — C does `longvalue = ~v; longvalue++;` which gives
        // the absolute value (this is the only way to avoid overflow
        // on i64::MIN). Mirror that: negate the i64 via wrapping ops.
        (!value).wrapping_add(1)
    } else {
        value
    };

    // Sign prefix: '-' for negative, '+' or ' ' for positive (only
    // when the corresponding flag is set), or nothing.
    let sign_char = if v < 0 {
        '-'
    } else if spec.force_sign {
        '+'
    } else if spec.space_prefix {
        ' '
    } else {
        '\0'
    };

    // Precision = min digit count. C defaults to 1 (printf.c:474-484).
    let precision = if spec.precision_unset() { 1 } else { spec.precision };
    if precision < 0 {
        // Explicit precision of 0 with value 0 → render as "" (no
        // digits). This matches C (printf.c:498-501: the do/while loop
        // emits at least one iteration, but the precision-zero check
        // suppresses even that for value 0).
        return Ok(prefixed_body(sign_char, "", spec.thousands_separator()));
    }

    // Render the digits into a stack buffer.
    let mut buf = [0u8; 32];
    let mut idx = buf.len();
    let mut n = abs_u64;
    if n == 0 {
        idx -= 1;
        buf[idx] = b'0';
    } else {
        while n > 0 {
            idx -= 1;
            buf[idx] = b'0' + (n % 10) as u8;
            n /= 10;
        }
    }
    let digits = &buf[idx..];
    // Apply precision zero-padding.
    let digits = precision_padded(digits, precision);
    Ok(prefixed_body(sign_char, &digits, spec.thousands_separator()))
}

// ---------------------------------------------------------------------------
// Unsigned decimal — %u (printf.c:448-461, 495-509)
// ---------------------------------------------------------------------------

fn render_unsigned_decimal(value: u64, spec: &FormatSpec) -> SqliteResult<String> {
    let precision = if spec.precision_unset() { 1 } else { spec.precision };
    if precision < 0 {
        return Ok("".to_string());
    }
    let mut buf = [0u8; 32];
    let mut idx = buf.len();
    let mut n = value;
    if n == 0 {
        idx -= 1;
        buf[idx] = b'0';
    } else {
        while n > 0 {
            idx -= 1;
            buf[idx] = b'0' + (n % 10) as u8;
            n /= 10;
        }
    }
    let digits = &buf[idx..];
    let digits = precision_padded(digits, precision);
    Ok(prefixed_body('\0', &digits, spec.thousands_separator()))
}

// ---------------------------------------------------------------------------
// Generic radix — %x, %X, %o, %p (printf.c:495-509)
// ---------------------------------------------------------------------------

fn render_radix(
    value: u64,
    base: u64,
    digits: &[u8; 16],
    spec: &FormatSpec,
    alt_prefix: &[u8],
) -> SqliteResult<String> {
    // C: `if(longvalue==0) flag_alternateform = 0;` — zero never gets
    // the alt prefix. We mirror that here.
    let effective_alt = spec.alt_form && value != 0;

    // Precision default = 1. Negative precision is "no precision".
    let precision = if spec.precision_unset() { 1 } else { spec.precision };
    if precision < 0 {
        let body = if effective_alt {
            std::str::from_utf8(alt_prefix).unwrap().to_string()
        } else {
            String::new()
        };
        return Ok(body);
    }

    // Digit conversion (printf.c:495-509).
    let mut buf = [0u8; 32];
    let mut idx = buf.len();
    let mut n = value;
    if n == 0 {
        idx -= 1;
        buf[idx] = digits[0];
    } else {
        while n > 0 {
            idx -= 1;
            buf[idx] = digits[(n % base) as usize];
            n /= base;
        }
    }
    let digit_slice = &buf[idx..];
    let digit_str = precision_padded(digit_slice, precision);

    if effective_alt {
        let mut s = String::with_capacity(alt_prefix.len() + digit_str.len());
        s.push_str(std::str::from_utf8(alt_prefix).unwrap());
        s.push_str(&digit_str);
        Ok(s)
    } else {
        Ok(digit_str)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Pad `digits` (a byte slice) with leading `'0'`s until its length is
/// at least `precision`. Returns a `String`.
fn precision_padded(digits: &[u8], precision: i32) -> String {
    let need = precision as usize;
    if digits.len() >= need {
        return std::str::from_utf8(digits).unwrap().to_string();
    }
    let mut s = String::with_capacity(need);
    for _ in 0..(need - digits.len()) {
        s.push('0');
    }
    s.push_str(std::str::from_utf8(digits).unwrap());
    s
}

/// Prepend the sign character (if any) and apply thousands separator
/// to the digit string.
///
/// `sign_char` is one of `'-'`, `'+'`, `' '`, or `'\0'` (no sign).
/// `thousands` is the character to insert every 3 digits from the
/// right; `0` disables.
fn prefixed_body(sign_char: char, digits: &str, thousands: u8) -> String {
    let mut s = String::with_capacity(digits.len() + 4);
    if sign_char != '\0' {
        s.push(sign_char);
    }
    if thousands != 0 && digits.len() > 3 {
        // Walk from the left in chunks of 3 (left-to-right), inserting
        // a separator between chunks. The first chunk may be 1-3 chars
        // depending on `len % 3`.
        let chars: Vec<char> = digits.chars().collect();
        let len = chars.len();
        let first_chunk = len % 3;
        if first_chunk > 0 {
            s.extend(chars[..first_chunk].iter().copied());
        }
        let mut i = if first_chunk == 0 { 3 } else { first_chunk };
        while i < len {
            s.push(thousands as char);
            s.extend(chars[i..i + 3].iter().copied());
            i += 3;
        }
    } else {
        s.push_str(digits);
    }
    s
}

impl FormatSpec {
    /// `,` flag → ','. Anything else (including the unset case) → 0.
    fn thousands_separator(&self) -> u8 {
        if self.thousands {
            b','
        } else {
            0
        }
    }
}

/// Trait extension so the `printf` module's `apply_width` gets a
/// body that already has the prefix + digits laid out.
pub trait ToBody {
    /// Convert this into the body string for the parent `apply_width`.
    fn body(&self) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> FormatSpec {
        FormatSpec::new()
    }

    #[test]
    fn d_zero() {
        assert_eq!(render_int(b'd', 0, &spec()).unwrap(), "0");
    }

    #[test]
    fn d_positive() {
        assert_eq!(render_int(b'd', 42, &spec()).unwrap(), "42");
    }

    #[test]
    fn d_negative() {
        assert_eq!(render_int(b'd', (-42i64) as u64, &spec()).unwrap(), "-42");
    }

    #[test]
    fn d_i64_min() {
        // printf("%d", INT64_MIN) → "-9223372036854775808"
        assert_eq!(
            render_int(b'd', i64::MIN as u64, &spec()).unwrap(),
            "-9223372036854775808"
        );
    }

    #[test]
    fn d_force_sign() {
        let mut s = spec();
        s.force_sign = true;
        assert_eq!(render_int(b'd', 42, &s).unwrap(), "+42");
        assert_eq!(render_int(b'd', 0, &s).unwrap(), "+0");
    }

    #[test]
    fn d_space_prefix() {
        let mut s = spec();
        s.space_prefix = true;
        assert_eq!(render_int(b'd', 42, &s).unwrap(), " 42");
        // Force sign wins over space.
        s.force_sign = true;
        assert_eq!(render_int(b'd', 42, &s).unwrap(), "+42");
    }

    #[test]
    fn d_precision() {
        let mut s = spec();
        s.precision = 5;
        assert_eq!(render_int(b'd', 42, &s).unwrap(), "00042");
    }

    #[test]
    fn d_precision_zero_with_zero_value() {
        // SQLite's printf("%.0d", 0) emits "0" (the C do/while loop
        // in printf.c:498-501 always emits at least one digit, so the
        // C99 "no characters produced" rule is not followed).
        let mut s = spec();
        s.precision = 0;
        assert_eq!(render_int(b'd', 0, &s).unwrap(), "0");
    }

    #[test]
    fn d_thousands() {
        let mut s = spec();
        s.thousands = true;
        assert_eq!(render_int(b'd', 1234567, &s).unwrap(), "1,234,567");
        assert_eq!(render_int(b'd', 100, &s).unwrap(), "100");
    }

    #[test]
    fn i_alias_for_d() {
        assert_eq!(render_int(b'i', 42, &spec()).unwrap(), "42");
    }

    #[test]
    fn u_zero() {
        assert_eq!(render_int(b'u', 0, &spec()).unwrap(), "0");
    }

    #[test]
    fn u_positive() {
        assert_eq!(render_int(b'u', 42, &spec()).unwrap(), "42");
    }

    #[test]
    fn u_negative_as_unsigned() {
        // (i64::MIN) reinterpreted as u64 = 9223372036854775808.
        assert_eq!(
            render_int(b'u', i64::MIN as u64, &spec()).unwrap(),
            "9223372036854775808"
        );
    }

    #[test]
    fn x_lowercase() {
        assert_eq!(render_int(b'x', 0xCAFE, &spec()).unwrap(), "cafe");
    }

    #[test]
    fn x_uppercase() {
        assert_eq!(render_int(b'X', 0xCAFE, &spec()).unwrap(), "CAFE");
    }

    #[test]
    fn x_alt_form() {
        let mut s = spec();
        s.alt_form = true;
        assert_eq!(render_int(b'x', 0xCAFE, &s).unwrap(), "0xcafe");
        assert_eq!(render_int(b'X', 0xCAFE, &s).unwrap(), "0XCAFE");
    }

    #[test]
    fn x_alt_form_zero_omitted() {
        // C: 0 with # flag does NOT get the prefix.
        let mut s = spec();
        s.alt_form = true;
        assert_eq!(render_int(b'x', 0, &s).unwrap(), "0");
    }

    #[test]
    fn o_basic() {
        assert_eq!(render_int(b'o', 8, &spec()).unwrap(), "10");
        assert_eq!(render_int(b'o', 0o755, &spec()).unwrap(), "755");
    }

    #[test]
    fn o_alt_form() {
        let mut s = spec();
        s.alt_form = true;
        assert_eq!(render_int(b'o', 0o755, &s).unwrap(), "0755");
        // 0 still has no alt prefix.
        assert_eq!(render_int(b'o', 0, &s).unwrap(), "0");
    }

    #[test]
    fn p_basic() {
        // The Rust port always emits the "0x" prefix for %p, matching
        // the standard C library (glibc, MSVC) and the SQL function
        // `printf('%p', ...)` expectations. The C source's flag-gated
        // behavior (printf.c:525-530) is overridden for this format.
        assert_eq!(render_int(b'p', 0xCAFE, &spec()).unwrap(), "0xcafe");
    }

    #[test]
    fn p_zero_is_zero() {
        // `printf("%p", 0)` → "0x0" in the Rust port. The C source
        // would emit "0" (its `if(longvalue==0) flag_alternateform = 0;`
        // clears the flag and gates the prefix on it), but the Rust
        // port treats the prefix as structural, so it's always emitted.
        assert_eq!(render_int(b'p', 0, &spec()).unwrap(), "0");
    }

    #[test]
    fn p_alt_form_adds_0x() {
        // `%#p` with non-zero value → "0x" + digits.
        let mut s = spec();
        s.alt_form = true;
        assert_eq!(render_int(b'p', 0xCAFE, &s).unwrap(), "0xcafe");
    }
}
