//! T-0001 集成测试:通过 libsqlite_rs crate 调 FFI 公开 API,验证行为与官方一致。
//!
//! 直接 `use libsqlite_rs::...` 调我们自己导出的 `unsafe extern "C"` 函数,
//! 因为 crate-type 是 rlib,这等价于用 libloading 调动态库,但更简单。

use libsqlite_rs::{
    sqlite3_close, sqlite3_errcode, sqlite3_errmsg, sqlite3_libversion,
    sqlite3_libversion_number, sqlite3_open, SqliteDb, SQLITE_OK,
};
use std::ffi::{c_char, CStr};

#[test]
fn libversion_matches_3_54_0() {
    // SAFETY: sqlite3_libversion 返回 'static 字符串
    let ptr = unsafe { sqlite3_libversion() };
    assert!(!ptr.is_null());
    let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
    assert_eq!(s, "3.54.0", "libversion must match sqlite-source/VERSION");
}

#[test]
fn libversion_number_is_3054000() {
    let n = unsafe { sqlite3_libversion_number() };
    assert_eq!(n, 3_054_000);
}

#[test]
fn open_memory_returns_ok_and_nonnull_handle() {
    let mut db: *mut SqliteDb = std::ptr::null_mut();
    let name = b":memory:\0";
    let rc = unsafe { sqlite3_open(name.as_ptr() as *const c_char, &mut db) };
    assert_eq!(rc, SQLITE_OK);
    assert!(!db.is_null());

    // 关闭
    let rc = unsafe { sqlite3_close(db) };
    assert_eq!(rc, SQLITE_OK);
}

#[test]
fn open_with_null_pp_db_returns_misuse() {
    let name = b":memory:\0";
    let rc = unsafe { sqlite3_open(name.as_ptr() as *const c_char, std::ptr::null_mut()) };
    assert_eq!(rc, 21 /* SQLITE_MISUSE */);
}

#[test]
fn close_null_is_ok() {
    let rc = unsafe { sqlite3_close(std::ptr::null_mut()) };
    assert_eq!(rc, SQLITE_OK);
}

#[test]
fn round_trip_open_close_100x() {
    for _ in 0..100 {
        let mut db: *mut SqliteDb = std::ptr::null_mut();
        let name = b":memory:\0";
        let rc = unsafe { sqlite3_open(name.as_ptr() as *const c_char, &mut db) };
        assert_eq!(rc, SQLITE_OK);
        let rc = unsafe { sqlite3_close(db) };
        assert_eq!(rc, SQLITE_OK);
    }
}

#[test]
fn errmsg_returns_valid_utf8() {
    let mut db: *mut SqliteDb = std::ptr::null_mut();
    let name = b":memory:\0";
    let _ = unsafe { sqlite3_open(name.as_ptr() as *const c_char, &mut db) };
    let msg = unsafe { sqlite3_errmsg(db) };
    assert!(!msg.is_null());
    let s = unsafe { CStr::from_ptr(msg) }.to_str();
    assert!(s.is_ok());
    let _ = unsafe { sqlite3_close(db) };
}

#[test]
fn errcode_returns_ok_for_fresh_handle() {
    let mut db: *mut SqliteDb = std::ptr::null_mut();
    let name = b":memory:\0";
    let _ = unsafe { sqlite3_open(name.as_ptr() as *const c_char, &mut db) };
    let code = unsafe { sqlite3_errcode(db) };
    assert_eq!(code, SQLITE_OK);
    let _ = unsafe { sqlite3_close(db) };
}
