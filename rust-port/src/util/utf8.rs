//! UTF-8 helpers — 1:1 port of `sqlite-source/src/utf.c` core functions.
//!
//! Implements the building blocks used throughout SQLite for UTF-8
//! encoding/decoding: `utf8_read`, `utf8_write`, `utf8_char_count`.
//! The decoder corresponds to `sqlite3Utf8Read` (utf.c:175-194) and the
//! encoder to `sqlite3AppendOneUtf8Character` (utf.c:114-135).
//!
//! # Behavior contract
//!
//! - 1-4 byte UTF-8 sequences decode as the corresponding codepoint
//! - `0x80..=0xBF` as a first byte is treated as a single literal byte
//!   (per `utf.c:155-158`); the over-long form of any codepoint ≥ 0x80
//!   is *accepted* (`utf.c:160-162`), not replaced with U+FFFD
//! - Surrogate values (0xD800..=0xDFFF) and noncharacters (0xFFFE/0xFFFF)
//!   are normalized to U+FFFD on read
//! - 5+ byte leading bytes (0xF8+) and overlong NUL/7-bit forms are
//!   all folded to U+FFFD (the C decoder reaches this outcome via
//!   `sqlite3Utf8Trans1` and the post-decode validity check)
//! - All input ranges are handled without panicking; the result type
//!   for the decoder is `(u32, usize)` because U+FFFD is a valid `char`
//!   but the caller may want the raw 32-bit value for diagnostic paths

/// `sqlite3Utf8Trans1` lookup table (utf.c:52-61).
///
/// Index `i` corresponds to a leading byte of `0xC0 + i` and stores the
/// initial value of the codepoint accumulator *before* the continuation
/// bytes are folded in. Two-byte leading bytes map to 0x02..=0x1F,
/// three-byte to 0x00..=0x0F, and four-byte to 0x00..=0x07. Indices
/// for 5+-byte leading bytes (0xF8+) are 0x00..=0x05, which combined
/// with the validity check below guarantees such sequences end up as
/// either a sub-0x80 value or a valid (non-surrogate, non-FFFE) number.
const UTF8_TRANS1: [u8; 64] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, // 0xC0 - 0xC7
    0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, // 0xC8 - 0xCF
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, // 0xD0 - 0xD7
    0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, // 0xD8 - 0xDF
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, // 0xE0 - 0xE7
    0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, // 0xE8 - 0xEF
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, // 0xF0 - 0xF7
    0x00, 0x01, 0x02, 0x03, 0x00, 0x01, 0x00, 0x00, // 0xF8 - 0xFF
];

// ============================================================================
// utf8_read — corresponds to sqlite3Utf8Read (utf.c:175-194)
// ============================================================================

/// Read a single UTF-8 character from the start of `bytes`.
///
/// Returns `(codepoint, byte_len)`. `byte_len` is the number of bytes
/// consumed from `bytes` (1-4) — for empty input the result is `(0, 0)`.
/// On any malformed input the codepoint is U+FFFD; the byte length is
/// still 1 (the leading byte is consumed) for all non-empty inputs,
/// matching `sqlite3Utf8Read`.
///
/// # Example
///
/// ```
/// use libsqlite_rs::util::utf8::utf8_read;
/// assert_eq!(utf8_read(b"A"),     (0x41, 1));
/// assert_eq!(utf8_read("\u{4e2d}".as_bytes()), (0x4e2d, 3)); // 中
/// assert_eq!(utf8_read("\u{1f600}".as_bytes()), (0x1F600, 4)); // 😀
/// ```
pub fn utf8_read(bytes: &[u8]) -> (u32, usize) {
    if bytes.is_empty() {
        return (0, 0);
    }
    let first = bytes[0];
    if first < 0xc0 {
        // ASCII fast path. Also handles continuation bytes used as
        // leaders (0x80..=0xBF) per utf.c:155-158: returned as-is.
        return (first as u32, 1);
    }
    // Multi-byte: initial accumulator from the trans1 table, then
    // greedy fold of continuation bytes (0x80..=0xBF).
    let mut codepoint = UTF8_TRANS1[(first - 0xc0) as usize] as u32;
    let mut len = 1usize;
    while len < bytes.len() && (bytes[len] & 0xc0) == 0x80 {
        // SAFE-equivalent: index is bounded by bytes.len() (loop guard).
        codepoint = (codepoint << 6) + (0x3f & bytes[len] as u32);
        len += 1;
    }
    // utf.c:189-191: post-decode validity check.
    if codepoint < 0x80
        || (codepoint & 0xFFFF_F800) == 0xD800
        || (codepoint & 0xFFFF_FFFE) == 0xFFFE
    {
        codepoint = 0xFFFD;
    }
    (codepoint, len)
}

// ============================================================================
// utf8_write — corresponds to sqlite3AppendOneUtf8Character (utf.c:114-135)
// ============================================================================

/// Write a single UTF-8 character with codepoint `c` into `buf`.
///
/// `n` is the caller's stated buffer capacity; the write is clamped to
/// `min(n, buf.len())` to prevent buffer overruns. The return value is
/// the number of bytes actually written — `0` if `n` is smaller than
/// the encoding size of `c`, otherwise `1`, `2`, `3`, or `4` depending
/// on the codepoint, matching `sqlite3AppendOneUtf8Character`.
///
/// Codepoint ranges (mirroring C):
/// - `c < 0x0080`  → 1 byte
/// - `c < 0x0800`  → 2 bytes
/// - `c < 0x10000` → 3 bytes
/// - `c >= 0x10000`→ 4 bytes
///
/// The encoder does not validate `c` (it is just a bit-pattern source).
/// Callers that have decoded via `utf8_read` can rely on `c` being
/// either a valid codepoint or U+FFFD.
pub fn utf8_write(buf: &mut [u8], c: u32, n: usize) -> usize {
    let cap = n.min(buf.len());
    if c < 0x0080 && cap >= 1 {
        buf[0] = c as u8;
        return 1;
    }
    if c < 0x0800 && cap >= 2 {
        buf[0] = 0xC0 | ((c >> 6) as u8 & 0x1F);
        buf[1] = 0x80 | (c as u8 & 0x3F);
        return 2;
    }
    if c < 0x10000 && cap >= 3 {
        buf[0] = 0xE0 | ((c >> 12) as u8 & 0x0F);
        buf[1] = 0x80 | ((c >> 6) as u8 & 0x3F);
        buf[2] = 0x80 | (c as u8 & 0x3F);
        return 3;
    }
    if cap >= 4 {
        buf[0] = 0xF0 | ((c >> 18) as u8 & 0x07);
        buf[1] = 0x80 | ((c >> 12) as u8 & 0x3F);
        buf[2] = 0x80 | ((c >> 6) as u8 & 0x3F);
        buf[3] = 0x80 | (c as u8 & 0x3F);
        return 4;
    }
    0
}

// ============================================================================
// utf8_char_count — corresponds to sqlite3Utf8CharLen (utf.c:475-490)
// ============================================================================

/// Count UTF-8 characters in `s`, stopping at the first 0x00 byte or
/// the end of the slice, whichever comes first.
///
/// This mirrors `sqlite3Utf8CharLen(zIn, -1)`. The `-1` nByte in C means
/// "scan until NUL"; the Rust equivalent stops at `s.len()` because Rust
/// slices do not have a guaranteed NUL terminator — the slice boundary
/// is also a valid stop condition.
pub fn utf8_char_count(s: &[u8]) -> usize {
    let mut count: usize = 0;
    let mut i: usize = 0;
    while i < s.len() && s[i] != 0 {
        // SQLITE_SKIP_UTF8(z) — utf.c forward-declared in sqliteInt.h.
        // For first < 0x80, one byte; otherwise consume leading + all
        // continuation bytes.
        let first = s[i];
        if first < 0x80 {
            i += 1;
        } else {
            i += 1;
            while i < s.len() && (s[i] & 0xc0) == 0x80 {
                i += 1;
            }
        }
        count += 1;
    }
    count
}

// ============================================================================
// 单元测试(快速冒烟,完整测试在 tests/util/utf8.rs)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_ascii() {
        assert_eq!(utf8_read(b"a"), (0x61, 1));
        assert_eq!(utf8_read(b"\0"), (0, 1)); // NUL treated as ASCII
    }

    #[test]
    fn read_two_byte() {
        // U+00A0 = 0xC2 0xA0
        assert_eq!(utf8_read(&[0xC2, 0xA0]), (0xA0, 2));
    }

    #[test]
    fn read_three_byte() {
        // U+4E2D (中) = 0xE4 0xB8 0xAD
        assert_eq!(utf8_read("中".as_bytes()), (0x4E2D, 3));
    }

    #[test]
    fn read_four_byte() {
        // U+1F600 (😀) = 0xF0 0x9F 0x98 0x80
        assert_eq!(utf8_read("😀".as_bytes()), (0x1F600, 4));
    }

    #[test]
    fn read_surrogate_replaced_with_fffd() {
        // U+D800 in canonical 3-byte form = 0xED 0xA0 0x80
        assert_eq!(utf8_read(&[0xED, 0xA0, 0x80]), (0xFFFD, 3));
        // U+DFFF = 0xED 0xBF 0xBF
        assert_eq!(utf8_read(&[0xED, 0xBF, 0xBF]), (0xFFFD, 3));
    }

    #[test]
    fn read_overlong_nul_replaced_with_fffd() {
        // 0xC0 0x80 is overlong NUL → c=0 → <0x80 → 0xFFFD
        assert_eq!(utf8_read(&[0xC0, 0x80]), (0xFFFD, 2));
    }

    #[test]
    fn read_continuation_byte_as_leader() {
        // 0x80..=0xBF as first byte: returned as literal single byte.
        assert_eq!(utf8_read(&[0x80, 0x80]), (0x80, 1));
        assert_eq!(utf8_read(&[0xBF]), (0xBF, 1));
    }

    #[test]
    fn read_empty() {
        assert_eq!(utf8_read(b""), (0, 0));
    }

    #[test]
    fn write_ascii() {
        let mut buf = [0u8; 4];
        let n = utf8_write(&mut buf, 0x41, 4);
        assert_eq!(n, 1);
        assert_eq!(buf[0], 0x41);
    }

    #[test]
    fn write_two_byte() {
        let mut buf = [0u8; 4];
        let n = utf8_write(&mut buf, 0xA0, 4);
        assert_eq!(n, 2);
        assert_eq!(buf, [0xC2, 0xA0, 0, 0]);
    }

    #[test]
    fn write_three_byte() {
        let mut buf = [0u8; 4];
        let n = utf8_write(&mut buf, 0x4E2D, 4);
        assert_eq!(n, 3);
        assert_eq!(buf, [0xE4, 0xB8, 0xAD, 0]);
    }

    #[test]
    fn write_four_byte() {
        let mut buf = [0u8; 4];
        let n = utf8_write(&mut buf, 0x1F600, 4);
        assert_eq!(n, 4);
        assert_eq!(buf, [0xF0, 0x9F, 0x98, 0x80]);
    }

    #[test]
    fn write_truncated_buffer_returns_zero() {
        // Need 4 bytes for U+1F600, only 2 available → 0
        let mut buf = [0u8; 2];
        let n = utf8_write(&mut buf, 0x1F600, 2);
        assert_eq!(n, 0);
        assert_eq!(buf, [0, 0]);
    }

    #[test]
    fn write_read_roundtrip() {
        for &c in &[0x00u32, 0x41, 0x7F, 0x80, 0xA0, 0x7FF, 0x800, 0xFFF, 0xFFFD, 0x10000, 0x1F600, 0x10FFFF] {
            let mut buf = [0u8; 4];
            let n = utf8_write(&mut buf, c, 4);
            assert!(n >= 1 && n <= 4, "c={c:x} n={n}");
            let (decoded, n2) = utf8_read(&buf[..n]);
            // Surrogate (0xD800-0xDFFF) and noncharacter (0xFFFE/0xFFFF)
            // are normalized to U+FFFD on read per utf.c:189-191.
            if (0xD800..=0xDFFF).contains(&c) || c == 0xFFFE || c == 0xFFFF {
                assert_eq!(decoded, 0xFFFD, "surrogate/FFFE c={c:x}");
            } else {
                assert_eq!(decoded, c, "roundtrip c={c:x}");
            }
            assert_eq!(n, n2);
        }
    }

    #[test]
    fn char_count_empty() {
        assert_eq!(utf8_char_count(b""), 0);
    }

    #[test]
    fn char_count_ascii() {
        assert_eq!(utf8_char_count(b"hello"), 5);
    }

    #[test]
    fn char_count_mixed() {
        // "a中b😀c" — 5 chars
        let s = "a中b😀c".as_bytes();
        assert_eq!(utf8_char_count(s), 5);
    }

    #[test]
    fn char_count_stops_at_nul() {
        let s: &[u8] = &[b'a', b'b', 0, b'c'];
        assert_eq!(utf8_char_count(s), 2);
    }
}
