//! String compare (case-insensitive ASCII) — 1:1 port of the `stricmp` /
//! `strnicmp` family in `sqlite-source/src/util.c`.
//!
//! Source mapping:
//! - `sqlite3_stricmp`  (util.c:415-422) — public, case-insensitive full compare
//! - `sqlite3StrICmp`   (util.c:423-441) — internal helper, full compare loop
//! - `sqlite3_strnicmp` (util.c:442-453) — public, case-insensitive bounded
//!   compare (up to N bytes)
//!
//! The 7-bit ASCII case fold uses the `sqlite3UpperToLower` table defined in
//! `sqlite-source/src/global.c:24-41`. The ASCII variant maps
//! `0x41..=0x5A` ('A'..'Z') to their lowercase counterparts `0x61..=0x7A`
//! ('a'..'z') and leaves every other byte unchanged.
//!
//! # Behavior contract (matching C)
//!
//! - `None` arguments model the C `NULL` pointer. The public APIs return:
//!   - `stricmp(NULL, NULL) == 0`
//!   - `stricmp(NULL, x)   < 0` (specifically `-1`)
//!   - `stricmp(x, NULL)   > 0` (specifically ` 1`)
//! - Comparison is byte-wise on the supplied `&[u8]` slice. A `0` byte
//!   inside the slice is treated as the C NUL terminator — both APIs stop
//!   at the first `0` they encounter in either input.
//! - Returns a signed `i32` (neg / zero / pos) suitable for feeding back
//!   into C-style comparison tables (`sqlite3aLTb` / `aEQb` / `aGTb`).
//! - Only the ASCII range is folded; UTF-8 multi-byte sequences pass
//!   through verbatim (and a stray `0x00` byte is treated as terminator).
//! - `sqlite3_strnicmp` with `N <= 0` returns 0 (loop never enters, the
//!   C `N-- > 0` check fails on the first iteration).
//! - `sqlite3_strnicmp` with `N` larger than either string's length
//!   behaves like `sqlite3_stricmp` — it stops at the first NUL / slice
//!   end and returns the byte-wise diff of the first differing position.

// ----------------------------------------------------------------------------
// sqlite3UpperToLower table (ASCII variant from global.c:24-41)
// ----------------------------------------------------------------------------

/// 256-byte ASCII case-fold table — equivalent of `sqlite3UpperToLower`.
///
/// Indexing `UPPER_TO_LOWER[b as usize]` returns `b` with the ASCII
/// uppercase range `0x41..=0x5A` mapped to lowercase `0x61..=0x7A`. All
/// other byte values are identity.
const UPPER_TO_LOWER: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i: usize = 0;
    while i < 256 {
        t[i] = if i >= 0x41 && i <= 0x5A { (i + 32) as u8 } else { i as u8 };
        i += 1;
    }
    t
};

/// Apply the `sqlite3UpperToLower` fold to a single byte.
#[inline]
fn to_lower(b: u8) -> u8 {
    // SAFETY: `b as usize` is in `0..=255`, so the index is in-bounds for
    // the 256-entry table. `UPPER_TO_LOWER` is a `const` array stored in
    // static memory; indexing it does not require `unsafe`.
    UPPER_TO_LOWER[b as usize]
}

// ----------------------------------------------------------------------------
// sqlite3StrICmp — internal full-length case-insensitive compare
// (util.c:423-441)
// ----------------------------------------------------------------------------

/// Internal case-insensitive full compare over two byte slices, treating
/// a `0` byte as the string terminator (matching the C `char*` semantics
/// used by the original `sqlite3StrICmp`).
///
/// The return value is the byte-wise difference of the first differing
/// position after ASCII case-folding, or `0` if the two strings are
/// equal up to the first NUL / slice end.
///
/// Translation of the C reference (util.c:423-441):
/// ```c
/// for(;;){
///     c = *a;  x = *b;
///     if( c==x ){
///         if( c==0 ) break;        // both NUL → equal
///     }else{
///         c = (int)UpperToLower[c] - (int)UpperToLower[x];
///         if( c ) break;            // folded diff non-zero → return diff
///         // else: case pair ('A' vs 'a') → fall through, advance
///     }
///     a++;  b++;
/// }
/// return c;
/// ```
pub fn str_icmp(a: &[u8], b: &[u8]) -> i32 {
    let mut i: usize = 0;
    loop {
        // Treat the end of either slice as an implicit NUL terminator —
        // mirrors C's NUL-terminated `char*` reads.
        let ca = if i < a.len() { a[i] } else { 0 };
        let cb = if i < b.len() { b[i] } else { 0 };
        if ca == cb {
            if ca == 0 {
                // Both strings ended together — equal.
                return 0;
            }
            // Raw bytes match (and are non-NUL) — advance.
        } else {
            let diff = (to_lower(ca) as i32) - (to_lower(cb) as i32);
            if diff != 0 {
                // Folded bytes differ — return the post-fold difference.
                // NOTE: this is `c` from the C code (overloaded), which
                // is now the folded diff, not the raw byte.
                return diff;
            }
            // Case pair (e.g. 'A' vs 'a') folded to the same byte; the C
            // code does NOT break here (the `if( c ) break` is false when
            // c == 0). It falls through to `a++; b++;` and continues.
            // Translating 1:1: just advance, do not return.
        }
        i += 1;
    }
}

// ----------------------------------------------------------------------------
// sqlite3_stricmp — public case-insensitive full compare
// (util.c:415-422)
// ----------------------------------------------------------------------------

/// Public case-insensitive UTF-8 string compare (1:1 port of
/// `sqlite3_stricmp` from `util.c:415-422`).
///
/// `None` models the C `NULL` pointer:
/// - `(None, None)` → `0`
/// - `(None, Some(_))` → `-1`
/// - `(Some(_), None)` → ` 1`
///
/// Otherwise the function returns the signed difference of the first
/// differing byte after ASCII case-folding (`< 0` if `z_left < z_right`,
/// `> 0` if `z_left > z_right`, `0` if equal).
///
/// Note: a `0` byte inside either slice is treated as the string
/// terminator, matching the C `char*` semantics.
pub fn sqlite3_stricmp(z_left: Option<&[u8]>, z_right: Option<&[u8]>) -> i32 {
    match (z_left, z_right) {
        (None, None) => 0,
        (None, Some(_)) => -1,
        (Some(_), None) => 1,
        (Some(l), Some(r)) => str_icmp(l, r),
    }
}

// ----------------------------------------------------------------------------
// sqlite3_strnicmp — public case-insensitive bounded compare
// (util.c:442-453)
// ----------------------------------------------------------------------------

/// Public case-insensitive compare of at most `n` bytes (1:1 port of
/// `sqlite3_strnicmp` from `util.c:442-453`).
///
/// NULL handling mirrors `sqlite3_stricmp`:
/// - `(None, None)` → `0`
/// - `(None, Some(_))` → `-1`
/// - `(Some(_), None)` → ` 1`
///
/// Non-NULL inputs:
/// - `n <= 0` → returns `0` (the C `while (N-- > 0 ...)` never enters)
/// - Walks both strings in lockstep, stopping when:
///   1. `n` iterations have been performed, OR
///   2. a `0` byte is seen in `z_left` (NUL terminator), OR
///   3. the ASCII-folded bytes differ
/// - If `n` iterations completed, returns `0` (this matches the C
///   `return N<0 ? 0 : ...` — once we have compared the full budget
///   the C code declares the strings equal regardless of what comes
///   after).
/// - Otherwise returns the byte-wise difference of the current
///   positions after ASCII case-folding.
pub fn sqlite3_strnicmp(z_left: Option<&[u8]>, z_right: Option<&[u8]>, n: i32) -> i32 {
    match (z_left, z_right) {
        (None, None) => 0,
        (None, Some(_)) => -1,
        (Some(_), None) => 1,
        (Some(a), Some(b)) => {
            let mut remaining = n;
            let mut i: usize = 0;
            loop {
                if remaining <= 0 {
                    break;
                }
                let ca = if i < a.len() { a[i] } else { 0 };
                if ca == 0 {
                    // NUL in `a` — C exits the loop here before the
                    // case-folded comparison, then returns
                    // `UpperToLower[*a] - UpperToLower[*b]` below.
                    break;
                }
                let cb = if i < b.len() { b[i] } else { 0 };
                if to_lower(ca) != to_lower(cb) {
                    break;
                }
                i += 1;
                remaining -= 1;
            }
            if remaining <= 0 {
                // The C code returns 0 here (`N<0`) only if N became
                // negative during the post-decrement in the loop header.
                // In our formulation `remaining` is decremented AFTER
                // the work, so we mirror by checking `remaining <= 0`
                // which captures both "N=0 initially" and "N iterations
                // completed exactly".
                0
            } else {
                let ca = if i < a.len() { a[i] } else { 0 };
                let cb = if i < b.len() { b[i] } else { 0 };
                (to_lower(ca) as i32) - (to_lower(cb) as i32)
            }
        }
    }
}

// ----------------------------------------------------------------------------
// 单元测试(冒烟,完整测试在 tests/util/str.rs)
// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_str_icmp_equal() {
        assert_eq!(str_icmp(b"hello", b"hello"), 0);
    }

    #[test]
    fn internal_str_icmp_case_diff_returns_zero() {
        // Case-fold makes 'A' and 'a' equal — diff is 0.
        assert_eq!(str_icmp(b"Hello", b"hello"), 0);
    }

    #[test]
    fn internal_str_icmp_stops_at_nul() {
        // 'a' '\0' 'b' should compare equal to 'a' '\0' 'c' (stop at NUL).
        assert_eq!(str_icmp(&[b'a', 0, b'b'], &[b'a', 0, b'c']), 0);
    }

    #[test]
    fn public_stricmp_basic() {
        assert_eq!(sqlite3_stricmp(Some(b"ABC"), Some(b"abc")), 0);
    }

    #[test]
    fn public_stricmp_null_handling() {
        assert_eq!(sqlite3_stricmp(None, None), 0);
        assert_eq!(sqlite3_stricmp(None, Some(b"x")), -1);
        assert_eq!(sqlite3_stricmp(Some(b"x"), None), 1);
    }

    #[test]
    fn public_strnicmp_n_limits() {
        // N=3, both start with "abc" — should match.
        assert_eq!(sqlite3_strnicmp(Some(b"abcdef"), Some(b"abcxyz"), 3), 0);
        // N=2, second char differs after case-fold.
        assert!(sqlite3_strnicmp(Some(b"aB"), Some(b"aC"), 2) < 0);
    }

    #[test]
    fn public_strnicmp_n_zero() {
        // N=0 → always 0.
        assert_eq!(sqlite3_strnicmp(Some(b"a"), Some(b"b"), 0), 0);
    }
}
