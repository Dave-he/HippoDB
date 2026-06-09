//! FFI smoke tests for the public `sqlite3_malloc` / `sqlite3_malloc64`
//! / `sqlite3_free` family. These mirror the C contract at
//! `sqlite-source/src/malloc.c:316-404`.
//!
//! Run with: `cargo test --test ffi_alloc`

use libsqlite_rs::{sqlite3_free, sqlite3_malloc, sqlite3_malloc64};
use std::ffi::c_void;
use std::ptr;

#[test]
fn malloc_zero_returns_null() {
    // malloc.c:316 — malloc(0) returns NULL
    let p = unsafe { sqlite3_malloc(0) };
    assert!(p.is_null(), "malloc(0) must return NULL");
}

#[test]
fn malloc_negative_returns_null() {
    // malloc.c:316 — malloc(n<=0) returns NULL
    let p = unsafe { sqlite3_malloc(-1) };
    assert!(p.is_null(), "malloc(-1) must return NULL");
}

#[test]
fn malloc_small_returns_writable() {
    let p = unsafe { sqlite3_malloc(64) };
    assert!(!p.is_null());
    // SAFETY: sqlite3_malloc(64) returns 64 writable bytes (or more).
    unsafe {
        let buf: &mut [u8] = std::slice::from_raw_parts_mut(p as *mut u8, 64);
        for (i, b) in buf.iter_mut().enumerate() {
            *b = i as u8;
        }
        for (i, b) in buf.iter().enumerate() {
            assert_eq!(*b, i as u8);
        }
    }
    unsafe { sqlite3_free(p) };
}

#[test]
fn malloc64_zero_returns_null() {
    // malloc.c:322 — malloc64(0) returns NULL
    let p = unsafe { sqlite3_malloc64(0) };
    assert!(p.is_null(), "malloc64(0) must return NULL");
}

#[test]
fn malloc64_small_returns_writable() {
    let p = unsafe { sqlite3_malloc64(1024) };
    assert!(!p.is_null());
    unsafe { sqlite3_free(p) };
}

#[test]
fn malloc64_huge_returns_null() {
    // Above SQLITE_MAX_ALLOCATION_SIZE: 0x7fffff00 (about 2 GB) returns NULL.
    // 1i64 << 62 is well above that; malloc64 should return NULL.
    let huge: i64 = 1i64 << 62;
    let p = unsafe { sqlite3_malloc64(huge) };
    assert!(p.is_null(), "malloc64 of {} bytes must return NULL", huge);
}

#[test]
fn free_null_is_noop() {
    // malloc.c:391 — free(NULL) is a no-op
    unsafe { sqlite3_free(ptr::null_mut()) };
    unsafe { sqlite3_free(ptr::null_mut::<c_void>()) };
}

#[test]
fn malloc_free_round_trip_many() {
    // 1000 alloc / free pairs to ensure the bookkeeping is balanced.
    for _ in 0..1000 {
        let p = unsafe { sqlite3_malloc(128) };
        assert!(!p.is_null());
        unsafe { sqlite3_free(p) };
    }
}

#[test]
fn malloc_zero_then_free_is_safe() {
    let p = unsafe { sqlite3_malloc(0) };
    assert!(p.is_null());
    // free(NULL) is safe; pass it through to make sure the FFI surface
    // doesn't choke on a NULL pointer.
    unsafe { sqlite3_free(p) };
}
