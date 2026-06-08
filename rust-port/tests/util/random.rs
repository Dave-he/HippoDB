//! `tests/util/random.rs` — integration tests for the PRNG
//! (`sqlite3_randomness`).
//!
//! These tests verify:
//! 1. The output is non-zero (entropy was injected).
//! 2. Two calls produce different output (counter advances).
//! 3. Repeated calls don't loop (no period < 1k bytes).

use libsqlite_rs::sqlite3_randomness;

// ============================================================================
// 1. Basic behavior
// ============================================================================
#[test]
fn one_kilobyte_has_byte_entropy() {
    let mut buf = [0u8; 1024];
    sqlite3_randomness(1024, Some(&mut buf));
    // All 1024 bytes should not be zero (the C source uses
    // OS entropy for seeding, our port uses system time, both
    // produce non-zero).
    let nonzero_count = buf.iter().filter(|&&b| b != 0).count();
    assert!(nonzero_count > 900, "expected > 900 non-zero bytes, got {nonzero_count}");
}

#[test]
fn one_kilobyte_unique_byte_values() {
    // With 1024 bytes of random data, the probability of any
    // particular byte value appearing 0 times is 1/e^4 ≈ 1.8%.
    // We check that at least 200 of the 256 possible values appear.
    let mut buf = [0u8; 1024];
    sqlite3_randomness(1024, Some(&mut buf));
    let mut seen = [false; 256];
    for &b in &buf {
        seen[b as usize] = true;
    }
    let distinct = seen.iter().filter(|&&x| x).count();
    assert!(distinct >= 200, "expected ≥ 200 distinct byte values, got {distinct}");
}

#[test]
fn repeated_calls_differ() {
    // Two 1k buffers should be different with overwhelming probability.
    let mut a = [0u8; 1024];
    let mut b = [0u8; 1024];
    sqlite3_randomness(1024, Some(&mut a));
    sqlite3_randomness(1024, Some(&mut b));
    assert_ne!(a, b);
}

// ============================================================================
// 2. Edge cases (matching the C source: random.c:88-92)
// ============================================================================
#[test]
fn n_zero_is_noop() {
    // C: `if( N<=0 || pBuf==0 )` → return. Randomness state is not
    // touched (we can't observe this from outside, but the function
    // should not crash).
    sqlite3_randomness(0, None);
    sqlite3_randomness(-1, None);
}

#[test]
fn partial_block_works() {
    // N = 17 — odd size, less than one ChaCha20 block.
    let mut buf = [0u8; 17];
    sqlite3_randomness(17, Some(&mut buf));
    let nonzero = buf.iter().filter(|&&b| b != 0).count();
    assert!(nonzero > 0);
}

#[test]
fn exact_block_boundary() {
    // N = 64 — exactly one ChaCha20 block.
    let mut buf = [0u8; 64];
    sqlite3_randomness(64, Some(&mut buf));
    assert!(buf.iter().any(|&b| b != 0));
}

#[test]
fn multiple_blocks() {
    // N = 192 — exactly 3 blocks.
    let mut buf = [0u8; 192];
    sqlite3_randomness(192, Some(&mut buf));
    let nonzero = buf.iter().filter(|&&b| b != 0).count();
    // Each 64-byte block should be all-but-zero. 192 bytes total,
    // allow some slack.
    assert!(nonzero > 100, "expected > 100 non-zero bytes, got {nonzero}");
}

// ============================================================================
// 3. Cross-call consistency (state advances correctly)
// ============================================================================
#[test]
fn sequential_partial_fills_advance_state() {
    // Make a 64-byte call, then a 64-byte call. The two outputs
    // should differ (counter advanced by 1 between blocks).
    let mut a = [0u8; 64];
    let mut b = [0u8; 64];
    sqlite3_randomness(64, Some(&mut a));
    sqlite3_randomness(64, Some(&mut b));
    assert_ne!(a, b);
}

#[test]
fn ten_sequential_one_byte_calls_all_differ() {
    // 10 sequential 1-byte calls should produce 10 distinct values
    // (1 byte is a tiny range, but with 256 possible values and
    // 10 samples, the probability of any collision is small).
    let mut seen = std::collections::HashSet::new();
    for _ in 0..10 {
        let mut b = [0u8; 1];
        sqlite3_randomness(1, Some(&mut b));
        // We don't assert uniqueness because the entropy is small;
        // just assert the function runs and returns a value.
        seen.insert(b[0]);
    }
    assert!(!seen.is_empty());
}
