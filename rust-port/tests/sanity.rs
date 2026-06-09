//! T-0001 集成测试:通过 libsqlite_rs crate 调 FFI 公开 API,验证行为与官方一致。

use libsqlite_rs::{
    sqlite3_close, sqlite3_errcode, sqlite3_errmsg, sqlite3_libversion,
    sqlite3_libversion_number, sqlite3_open, SqliteDb, SqliteStmt, SQLITE_OK,
    sqlite3_prepare_v2, sqlite3_step, sqlite3_finalize,
    sqlite3_column_count, sqlite3_column_name, sqlite3_column_type,
    sqlite3_column_int, sqlite3_column_int64, sqlite3_column_double,
    sqlite3_column_text, sqlite3_column_bytes,
    sqlite3_exec,
};
use std::ffi::{c_char, c_int, c_void, CStr};
use std::sync::Mutex;

#[test]
fn libversion_matches_3_54_0() {
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

#[test]
fn prepare_and_step_create_insert_select() {
    let mut db: *mut SqliteDb = std::ptr::null_mut();
    let name = b":memory:\0";
    let rc = unsafe { sqlite3_open(name.as_ptr() as *const c_char, &mut db) };
    assert_eq!(rc, SQLITE_OK);

    // 1. CREATE TABLE t(id INT, val TEXT)
    let sql_create = b"CREATE TABLE t(id INT, val TEXT);\0";
    let mut stmt: *mut SqliteStmt = std::ptr::null_mut();
    let rc = unsafe {
        sqlite3_prepare_v2(
            db,
            sql_create.as_ptr() as *const c_char,
            -1,
            &mut stmt,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, SQLITE_OK);
    assert!(!stmt.is_null());

    let rc = unsafe { sqlite3_step(stmt) };
    assert_eq!(rc, 101 /* SQLITE_DONE */);

    let rc = unsafe { sqlite3_finalize(stmt) };
    assert_eq!(rc, SQLITE_OK);

    // 2. INSERT INTO t VALUES(42, 'hello')
    let sql_insert = b"INSERT INTO t VALUES(42, 'hello');\0";
    let mut stmt: *mut SqliteStmt = std::ptr::null_mut();
    let rc = unsafe {
        sqlite3_prepare_v2(
            db,
            sql_insert.as_ptr() as *const c_char,
            -1,
            &mut stmt,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, SQLITE_OK);
    assert!(!stmt.is_null());

    let rc = unsafe { sqlite3_step(stmt) };
    assert_eq!(rc, 101 /* SQLITE_DONE */);

    let rc = unsafe { sqlite3_finalize(stmt) };
    assert_eq!(rc, SQLITE_OK);

    // 3. SELECT id, val FROM t
    let sql_select = b"SELECT id, val FROM t;\0";
    let mut stmt: *mut SqliteStmt = std::ptr::null_mut();
    let rc = unsafe {
        sqlite3_prepare_v2(
            db,
            sql_select.as_ptr() as *const c_char,
            -1,
            &mut stmt,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, SQLITE_OK);
    assert!(!stmt.is_null());

    // Check column metadata before step
    let col_count = unsafe { sqlite3_column_count(stmt) };
    assert_eq!(col_count, 2);

    let name0 = unsafe { sqlite3_column_name(stmt, 0) };
    let name1 = unsafe { sqlite3_column_name(stmt, 1) };
    assert_eq!(unsafe { CStr::from_ptr(name0) }.to_str().unwrap(), "id");
    assert_eq!(unsafe { CStr::from_ptr(name1) }.to_str().unwrap(), "val");

    // Step to the row
    let rc = unsafe { sqlite3_step(stmt) };
    assert_eq!(rc, 100 /* SQLITE_ROW */);

    // Check column types
    assert_eq!(unsafe { sqlite3_column_type(stmt, 0) }, 1 /* SQLITE_INTEGER */);
    assert_eq!(unsafe { sqlite3_column_type(stmt, 1) }, 3 /* SQLITE_TEXT */);

    // Check column values
    assert_eq!(unsafe { sqlite3_column_int(stmt, 0) }, 42);
    assert_eq!(unsafe { sqlite3_column_int64(stmt, 0) }, 42);
    assert_eq!(unsafe { sqlite3_column_double(stmt, 0) }, 42.0);

    let text_val = unsafe { sqlite3_column_text(stmt, 1) };
    assert_eq!(unsafe { CStr::from_ptr(text_val as *const c_char) }.to_str().unwrap(), "hello");
    assert_eq!(unsafe { sqlite3_column_bytes(stmt, 1) }, 5);

    // Step to the end
    let rc = unsafe { sqlite3_step(stmt) };
    assert_eq!(rc, 101 /* SQLITE_DONE */);

    let rc = unsafe { sqlite3_finalize(stmt) };
    assert_eq!(rc, SQLITE_OK);

    let rc = unsafe { sqlite3_close(db) };
    assert_eq!(rc, SQLITE_OK);
}

#[test]
fn prepare_multiple_statements_with_tail() {
    let mut db: *mut SqliteDb = std::ptr::null_mut();
    let name = b":memory:\0";
    let rc = unsafe { sqlite3_open(name.as_ptr() as *const c_char, &mut db) };
    assert_eq!(rc, SQLITE_OK);

    let sql = b"CREATE TABLE x(y); INSERT INTO x VALUES(100); SELECT y FROM x;\0";
    let mut stmt: *mut SqliteStmt = std::ptr::null_mut();
    let mut tail: *const c_char = std::ptr::null();

    // 1. Prepare CREATE TABLE
    let rc = unsafe {
        sqlite3_prepare_v2(
            db,
            sql.as_ptr() as *const c_char,
            -1,
            &mut stmt,
            &mut tail,
        )
    };
    assert_eq!(rc, SQLITE_OK);
    assert!(!stmt.is_null());
    assert!(!tail.is_null());

    let rc = unsafe { sqlite3_step(stmt) };
    assert_eq!(rc, 101 /* SQLITE_DONE */);
    unsafe { sqlite3_finalize(stmt) };

    // 2. Prepare INSERT using tail
    let rc = unsafe {
        sqlite3_prepare_v2(
            db,
            tail,
            -1,
            &mut stmt,
            &mut tail,
        )
    };
    assert_eq!(rc, SQLITE_OK);
    assert!(!stmt.is_null());

    let rc = unsafe { sqlite3_step(stmt) };
    assert_eq!(rc, 101 /* SQLITE_DONE */);
    unsafe { sqlite3_finalize(stmt) };

    // 3. Prepare SELECT using tail
    let rc = unsafe {
        sqlite3_prepare_v2(
            db,
            tail,
            -1,
            &mut stmt,
            &mut tail,
        )
    };
    assert_eq!(rc, SQLITE_OK);
    assert!(!stmt.is_null());

    let rc = unsafe { sqlite3_step(stmt) };
    assert_eq!(rc, 100 /* SQLITE_ROW */);
    assert_eq!(unsafe { sqlite3_column_int(stmt, 0) }, 100);

    let rc = unsafe { sqlite3_step(stmt) };
    assert_eq!(rc, 101 /* SQLITE_DONE */);
    unsafe { sqlite3_finalize(stmt) };

    let rc = unsafe { sqlite3_close(db) };
    assert_eq!(rc, SQLITE_OK);
}

// Thread-safe structures for exec callback
static CALLBACK_COUNTER: Mutex<i32> = Mutex::new(0);
static CALLBACK_VALS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

unsafe extern "C" fn exec_callback(
    _arg: *mut c_void,
    argc: c_int,
    argv: *mut *mut c_char,
    az_col_name: *mut *mut c_char,
) -> c_int {
    if let Ok(mut counter) = CALLBACK_COUNTER.lock() {
        *counter += 1;
    }
    if let Ok(mut vals) = CALLBACK_VALS.lock() {
        for i in 0..argc as usize {
            let val_ptr = *argv.add(i);
            let col_ptr = *az_col_name.add(i);
            let val = if val_ptr.is_null() {
                "NULL".to_string()
            } else {
                CStr::from_ptr(val_ptr).to_str().unwrap().to_string()
            };
            let col = CStr::from_ptr(col_ptr).to_str().unwrap().to_string();
            vals.push((col, val));
        }
    }
    0 // Continue
}

#[test]
fn exec_with_callback() {
    let mut db: *mut SqliteDb = std::ptr::null_mut();
    let name = b":memory:\0";
    let rc = unsafe { sqlite3_open(name.as_ptr() as *const c_char, &mut db) };
    assert_eq!(rc, SQLITE_OK);

    // Exec CREATE and INSERT
    let rc = unsafe {
        sqlite3_exec(
            db,
            b"CREATE TABLE test_exec(a, b); INSERT INTO test_exec VALUES(1, 'apple'); INSERT INTO test_exec VALUES(2, 'banana');\0".as_ptr() as *const c_char,
            None,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, SQLITE_OK);

    // Reset globals
    if let Ok(mut counter) = CALLBACK_COUNTER.lock() {
        *counter = 0;
    }
    if let Ok(mut vals) = CALLBACK_VALS.lock() {
        vals.clear();
    }

    // Exec SELECT
    let rc = unsafe {
        sqlite3_exec(
            db,
            b"SELECT a, b FROM test_exec;\0".as_ptr() as *const c_char,
            Some(exec_callback),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, SQLITE_OK);

    if let (Ok(counter), Ok(vals)) = (CALLBACK_COUNTER.lock(), CALLBACK_VALS.lock()) {
        assert_eq!(*counter, 2);
        assert_eq!(vals.len(), 4);
        assert_eq!(vals[0], ("a".to_string(), "1".to_string()));
        assert_eq!(vals[1], ("b".to_string(), "apple".to_string()));
        assert_eq!(vals[2], ("a".to_string(), "2".to_string()));
        assert_eq!(vals[3], ("b".to_string(), "banana".to_string()));
    }

    let rc = unsafe { sqlite3_close(db) };
    assert_eq!(rc, SQLITE_OK);
}
