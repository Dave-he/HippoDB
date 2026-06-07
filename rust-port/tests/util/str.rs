//! `tests/util/str.rs` — `sqlite3_stricmp` / `sqlite3_strnicmp` integration
//! tests verifying 1:1 behavior parity with the C reference
//! implementation in `sqlite-source/src/util.c:415-453`.
//!
//! Each test documents the expected outcome in terms of the C behavior
//! and asserts against the Rust port. Where the C code has a subtle
//! boundary case (e.g. `N=0` returns 0 even when the first bytes
//! differ), the test pins that exact contract.

use libsqlite_rs::{sqlite3_stricmp, sqlite3_strnicmp};

// ============================================================================
// 1. sqlite3_stricmp — exact ASCII equality (util.c:415-422)
// ============================================================================
#[test]
fn stricmp_exact_equal_ascii() {
    assert_eq!(sqlite3_stricmp(Some(b"hello"), Some(b"hello")), 0);
    assert_eq!(sqlite3_stricmp(Some(b""), Some(b"")), 0);
}

// ============================================================================
// 2. sqlite3_stricmp — mixed case is equal (util.c:423-441, the fold loop)
// ============================================================================
#[test]
fn stricmp_mixed_case_is_equal() {
    assert_eq!(sqlite3_stricmp(Some(b"Hello"), Some(b"hello")), 0);
    assert_eq!(sqlite3_stricmp(Some(b"HELLO"), Some(b"hello")), 0);
    assert_eq!(sqlite3_stricmp(Some(b"hElLo"), Some(b"HeLlO")), 0);
}

// ============================================================================
// 3. sqlite3_stricmp — unequal lengths, common prefix
// ============================================================================
#[test]
fn stricmp_unequal_lengths_common_prefix() {
    // "abc" < "abcd" → negative
    assert!(sqlite3_stricmp(Some(b"abc"), Some(b"abcd")) < 0);
    // "abcd" > "abc" → positive
    assert!(sqlite3_stricmp(Some(b"abcd"), Some(b"abc")) > 0);
    // Case-folded version: "ABCd" vs "abc" — first 3 match, then 'd' vs NUL.
    assert!(sqlite3_stricmp(Some(b"ABCd"), Some(b"abc")) > 0);
    assert!(sqlite3_stricmp(Some(b"abc"), Some(b"ABCd")) < 0);
}

// ============================================================================
// 4. sqlite3_stricmp — embedded NUL terminates comparison (C `char*` semantics)
// ============================================================================
#[test]
fn stricmp_stops_at_embedded_nul() {
    // After the NUL, the C code never sees those bytes — both inputs
    // appear equal to `sqlite3_stricmp`.
    let a: &[u8] = &[b'a', 0, b'X', b'Y', b'Z'];
    let b: &[u8] = &[b'a', 0, b'1', b'2', b'3'];
    assert_eq!(sqlite3_stricmp(Some(a), Some(b)), 0);
}

// ============================================================================
// 5. sqlite3_stricmp — NULL pointer handling (util.c:415-420)
// ============================================================================
#[test]
fn stricmp_null_pointer_semantics() {
    // (None, None) → 0
    assert_eq!(sqlite3_stricmp(None, None), 0);
    // (None, Some(_)) → -1
    assert_eq!(sqlite3_stricmp(None, Some(b"x")), -1);
    // (Some(_), None) → +1
    assert_eq!(sqlite3_stricmp(Some(b"x"), None), 1);
    // Same with empty slice on the other side.
    assert_eq!(sqlite3_stricmp(None, Some(b"")), -1);
    assert_eq!(sqlite3_stricmp(Some(b""), None), 1);
}

// ============================================================================
// 6. sqlite3_strnicmp — N limits the comparison (util.c:442-453)
// ============================================================================
#[test]
fn strnicmp_n_caps_match() {
    // N=3 covers the common prefix "abc"; even though "abcdef" vs
    // "abcxyz" differ past that, the function returns 0.
    assert_eq!(sqlite3_strnicmp(Some(b"abcdef"), Some(b"abcxyz"), 3), 0);
    // Case-fold within the N-window: still equal.
    assert_eq!(sqlite3_strnicmp(Some(b"ABCdef"), Some(b"abcXYZ"), 3), 0);
}

// ============================================================================
// 7. sqlite3_strnicmp — N larger than input; the NUL/slice end stops it
// ============================================================================
#[test]
fn strnicmp_n_larger_than_input() {
    // "ab" vs "abc" with N=100 — first 2 match, then 'a' is NUL/end and
    // 'b' is 'c' which fold-distinct → negative.
    assert!(sqlite3_strnicmp(Some(b"ab"), Some(b"abc"), 100) < 0);
    assert!(sqlite3_strnicmp(Some(b"abc"), Some(b"ab"), 100) > 0);
    // Exact equal inputs regardless of N.
    assert_eq!(sqlite3_strnicmp(Some(b"abc"), Some(b"abc"), 100), 0);
    // N larger than both: empty strings are equal.
    assert_eq!(sqlite3_strnicmp(Some(b""), Some(b""), 100), 0);
}

// ============================================================================
// 8. sqlite3_strnicmp — UTF-8 bytes pass through; N counts bytes, not chars
//    (util.c:442-453 — comparison is byte-wise on the raw input)
// ============================================================================
#[test]
fn strnicmp_utf8_byte_window() {
    // "中" is 3 bytes (0xE4 0xB8 0xAD) in UTF-8.
    let zhang: &[u8] = "中".as_bytes();
    let zhong: &[u8] = "中abc".as_bytes();
    // First 3 bytes are identical UTF-8 sequence; N=3 → equal.
    assert_eq!(sqlite3_strnicmp(Some(zhang), Some(zhong), 3), 0);
    // N=2 cuts the UTF-8 sequence mid-byte; first 2 bytes happen to be
    // identical, so still 0.
    assert_eq!(sqlite3_strnicmp(Some(zhang), Some(zhong), 2), 0);
    // N=4: "中abc" has more bytes; after the UTF-8 char the next byte
    // is 'a' on one side and 0 (end of slice) on the other →
    // 0x61 - 0 = positive.
    assert!(sqlite3_strnicmp(Some(zhong), Some(zhang), 4) > 0);
    // Mixed: "中" vs "Z" — first bytes are 0xE4 vs 0x5A; they are NOT
    // equal under the case-fold (0xE4 unchanged, 0x5A → 0x7A). With
    // N=1 we compare one byte: 0xE4 - 0x7A = 106 - 122 = -16.
    assert_eq!(sqlite3_strnicmp(Some(zhang), Some(b"Z"), 1), 0xE4 - 0x7A);
    // N=0 — the C `N-- > 0` check fails on the first iteration, the
    // function returns 0 regardless of input.
    assert_eq!(sqlite3_strnicmp(Some(b"a"), Some(b"b"), 0), 0);
    // Negative N — same behavior: 0.
    assert_eq!(sqlite3_strnicmp(Some(b"a"), Some(b"b"), -5), 0);
    // NULL handling for strnicmp.
    assert_eq!(sqlite3_strnicmp(None, None, 10), 0);
    assert_eq!(sqlite3_strnicmp(None, Some(b"x"), 10), -1);
    assert_eq!(sqlite3_strnicmp(Some(b"x"), None, 10), 1);
}
