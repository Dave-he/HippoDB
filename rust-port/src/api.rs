//! FFI 公开 API 层。
//!
//! 对应 sqlite3.h 中的 C 函数。每个函数必须是 `#[no_mangle] pub unsafe extern "C"`,
//! 签名与官方保持 byte-for-byte 一致(指针宽度、整数宽度、调用约定)。

use std::ffi::{c_char, c_int, c_void, CStr, CString};

use crate::error::SqliteError;
use crate::handle::{SqliteDb, SqliteStmt};
use crate::util::alloc::Malloc;

/// 返回 SQLite 版本字符串。生命周期 = 进程,不需要 free。
///
/// C 契约:`const char *sqlite3_libversion(void);`
#[no_mangle]
pub unsafe extern "C" fn sqlite3_libversion() -> *const c_char {
    SQLITE_VERSION_STR.as_ptr() as *const c_char
}

/// 返回 SQLite 版本号(形如 3054000 = 3.54.0)。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_libversion_number() -> c_int {
    crate::SQLITE_VERSION_NUMBER
}

/// 打开/创建数据库。
///
/// C 契约:`int sqlite3_open(const char *filename, sqlite3 **ppDb);`
///
/// # Safety
/// - `filename` 必须是 NUL 结尾的 UTF-8 字符串(可为 null)
/// - `pp_db` 必须是非空指针
#[no_mangle]
pub unsafe extern "C" fn sqlite3_open(filename: *const c_char, pp_db: *mut *mut SqliteDb) -> c_int {
    if !filename.is_null() {
        // SAFETY: 调用方保证 filename 是 NUL 结尾的 UTF-8 字符串。
        if unsafe { CStr::from_ptr(filename) }.to_str().is_err() {
            return SqliteError::ERROR.code();
        }
    }
    if pp_db.is_null() {
        return SqliteError::MISUSE.code();
    }
    let db = SqliteDb::new();
    // SAFETY: pp_db 已校验非 null,写一个有效指针。
    unsafe { *pp_db = Box::into_raw(db) };
    SqliteError::OK.code()
}

/// 关闭数据库,释放 handle。
///
/// C 契约:`int sqlite3_close(sqlite3 *db);`
///
/// # Safety
/// `db` 必须是由 `sqlite3_open` 返回的指针,且不能被 close 两次。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_close(db: *mut SqliteDb) -> c_int {
    if db.is_null() {
        return SqliteError::OK.code();
    }
    // SAFETY: db 由 sqlite3_open 产生,这里取回所有权。
    let _ = unsafe { Box::from_raw(db) };
    SqliteError::OK.code()
}

/// 返回最近一次错误的 UTF-8 字符串。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_errmsg(db: *mut SqliteDb) -> *const c_char {
    if db.is_null() {
        return ERRSG_NOT_IMPL.as_ptr() as *const c_char;
    }
    let db_ref = unsafe { &*db };
    if let Ok(msg) = db_ref.last_err_msg.lock() {
        msg.as_ptr()
    } else {
        ERRSG_NOT_IMPL.as_ptr() as *const c_char
    }
}

/// 返回最近一次错误码。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_errcode(db: *mut SqliteDb) -> c_int {
    if db.is_null() {
        return SqliteError::OK.code();
    }
    let db_ref = unsafe { &*db };
    db_ref.last_err_code.load(std::sync::atomic::Ordering::Acquire)
}

/// `sqlite3_open_v2` — 带 flag 的打开。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_open_v2(
    filename: *const c_char,
    pp_db: *mut *mut SqliteDb,
    flags: c_int,
    z_vfs: *const c_char,
) -> c_int {
    let _ = flags;
    let _ = z_vfs;
    // SAFETY: 转发,所有 unsafe 在 sqlite3_open 内部处理。
    unsafe { sqlite3_open(filename, pp_db) }
}

/// `sqlite3_malloc` — 公开 API malloc。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_malloc(n: c_int) -> *mut c_void {
    Malloc::malloc(n as i64 as u64, None) as *mut c_void
}

/// `sqlite3_malloc64` — 同上但参数是 i64。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_malloc64(n: i64) -> *mut c_void {
    Malloc::malloc64(n as u64, None) as *mut c_void
}

/// `sqlite3_free` — 释放。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_free(p: *mut c_void) {
    Malloc::free(p as *mut u8)
}

/// 辅助函数：根据 SELECT 语句和当前 Schema 确定结果列。
fn get_result_columns(stmt: &crate::parse::Stmt, schema: &crate::vdbe::Schema) -> Vec<String> {
    match stmt {
        crate::parse::Stmt::Select(s) => {
            if let Some(table) = schema.tables.get(&s.from) {
                if s.all {
                    table.columns.clone()
                } else {
                    s.columns.clone()
                }
            } else {
                s.columns.clone()
            }
        }
        _ => Vec::new(),
    }
}

/// `sqlite3_prepare_v2` — 将 SQL 编译为 prepared statement。
///
/// # Safety
/// - `db` 必须指向有效 `SqliteDb` 结构体。
/// - `z_sql` 必须是有效 C 字符串或字符序列。
/// - `pp_stmt` 和 `pz_tail` 如果非空，必须是有效且可写的指针。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_prepare_v2(
    db: *mut SqliteDb,
    z_sql: *const c_char,
    n_byte: c_int,
    pp_stmt: *mut *mut SqliteStmt,
    pz_tail: *mut *const c_char,
) -> c_int {
    if db.is_null() || z_sql.is_null() || pp_stmt.is_null() {
        return SqliteError::MISUSE.code();
    }

    let db_ref = unsafe { &*db };
    db_ref.clear_error();
    unsafe { *pp_stmt = std::ptr::null_mut() };
    if !pz_tail.is_null() {
        unsafe { *pz_tail = z_sql };
    }

    // 将输入转换为 Rust &str
    let sql_str = if n_byte < 0 {
        match unsafe { CStr::from_ptr(z_sql) }.to_str() {
            Ok(s) => s,
            Err(_) => {
                db_ref.set_error(SqliteError::ERROR.code(), "invalid utf-8 in SQL");
                return SqliteError::ERROR.code();
            }
        }
    } else {
        let slice = unsafe { std::slice::from_raw_parts(z_sql as *const u8, n_byte as usize) };
        match std::str::from_utf8(slice) {
            Ok(s) => s,
            Err(_) => {
                db_ref.set_error(SqliteError::ERROR.code(), "invalid utf-8 in SQL");
                return SqliteError::ERROR.code();
            }
        }
    };

    // 分词
    let tokens = match crate::tokenize::tokenize(sql_str) {
        Ok(t) => t,
        Err(e) => {
            db_ref.set_error(e.code(), &e.message());
            return e.code();
        }
    };

    if tokens.is_empty() {
        return SqliteError::OK.code();
    }

    // 解析第一条 SQL 语句
    let mut parser = crate::parse::Parser::new(&tokens);
    let stmt = match parser.parse_stmt() {
        Ok(s) => s,
        Err(e) => {
            db_ref.set_error(e.code(), &e.message());
            return e.code();
        }
    };

    if matches!(stmt, crate::parse::Stmt::Empty) {
        return SqliteError::OK.code();
    }

    // 编译语句并获取列信息
    let schema = db_ref.schema.lock().unwrap();
    let program = match crate::where_compiler::compile_stmt(&stmt, &*schema) {
        Ok(p) => p,
        Err(e) => {
            db_ref.set_error(e.code(), &e.message());
            return e.code();
        }
    };
    let cols = get_result_columns(&stmt, &*schema);
    drop(schema);

    // 创建 SqliteStmt 并返回它的裸指针
    let stmt_box = SqliteStmt::new(db, stmt, program, cols);
    unsafe { *pp_stmt = Box::into_raw(stmt_box) };

    // 如果请求了，设置 pz_tail 到未被消费的下一个 token 起始位置
    if !pz_tail.is_null() {
        let next_offset = if parser.pos < tokens.len() {
            tokens[parser.pos].offset
        } else {
            sql_str.len()
        };
        unsafe { *pz_tail = z_sql.add(next_offset) };
    }

    SqliteError::OK.code()
}

/// `sqlite3_step` — 执行 prepared statement。
///
/// # Safety
/// `stmt` 必须是有效的 `SqliteStmt` 指针。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_step(stmt: *mut SqliteStmt) -> c_int {
    if stmt.is_null() {
        return SqliteError::MISUSE.code();
    }

    let stmt_ref = unsafe { &mut *stmt };
    let db_ref = unsafe { &*stmt_ref.db };
    db_ref.clear_error();

    if stmt_ref.rows.is_none() {
        let mut schema = match db_ref.schema.lock() {
            Ok(s) => s,
            Err(_) => {
                db_ref.set_error(SqliteError::ERROR.code(), "failed to lock schema");
                return SqliteError::ERROR.code();
            }
        };

        match &stmt_ref.stmt {
            crate::parse::Stmt::Select(select_stmt) => {
                match crate::where_compiler::run_select(select_stmt, &mut *schema) {
                    Ok(result_rows) => {
                        stmt_ref.rows = Some(result_rows);
                        stmt_ref.row_idx = 0;
                    }
                    Err(e) => {
                        db_ref.set_error(e.code(), &e.message());
                        return e.code();
                    }
                }
            }
            other_stmt => {
                let prog = match crate::where_compiler::compile_stmt(other_stmt, &*schema) {
                    Ok(p) => p,
                    Err(e) => {
                        db_ref.set_error(e.code(), &e.message());
                        return e.code();
                    }
                };
                match crate::vdbe::exec(&prog, &mut *schema) {
                    Ok(_) => {
                        stmt_ref.rows = Some(Vec::new());
                        stmt_ref.row_idx = 0;
                    }
                    Err(e) => {
                        db_ref.set_error(e.code(), &e.message());
                        return e.code();
                    }
                }
            }
        }
    }

    let rows = stmt_ref.rows.as_ref().unwrap();
    if stmt_ref.row_idx < rows.len() {
        stmt_ref.current_row = Some(stmt_ref.row_idx);
        stmt_ref.row_idx += 1;
        // 清理上一个行的 CString 缓存
        for cache in &mut stmt_ref.cached_col_texts {
            *cache = None;
        }
        100 // SQLITE_ROW
    } else {
        stmt_ref.current_row = None;
        101 // SQLITE_DONE
    }
}

/// `sqlite3_finalize` — 释放 prepared statement。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_finalize(stmt: *mut SqliteStmt) -> c_int {
    if stmt.is_null() {
        return SqliteError::OK.code();
    }
    let _ = unsafe { Box::from_raw(stmt) };
    SqliteError::OK.code()
}

/// `sqlite3_reset` — 重置 prepared statement 状态。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_reset(stmt: *mut SqliteStmt) -> c_int {
    if stmt.is_null() {
        return SqliteError::OK.code();
    }
    let stmt_ref = unsafe { &mut *stmt };
    stmt_ref.rows = None;
    stmt_ref.row_idx = 0;
    stmt_ref.current_row = None;
    for cache in &mut stmt_ref.cached_col_texts {
        *cache = None;
    }
    SqliteError::OK.code()
}

/// `sqlite3_column_count` — 结果列数。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_column_count(stmt: *mut SqliteStmt) -> c_int {
    if stmt.is_null() {
        return 0;
    }
    let stmt_ref = unsafe { &*stmt };
    stmt_ref.columns.len() as c_int
}

/// `sqlite3_column_name` — 结果列名。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_column_name(stmt: *mut SqliteStmt, n: c_int) -> *const c_char {
    if stmt.is_null() || n < 0 {
        return std::ptr::null();
    }
    let stmt_ref = unsafe { &*stmt };
    let idx = n as usize;
    if idx >= stmt_ref.column_names_c.len() {
        return std::ptr::null();
    }
    stmt_ref.column_names_c[idx].as_ptr()
}

/// `sqlite3_column_type` — 结果列数据类型。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_column_type(stmt: *mut SqliteStmt, i_col: c_int) -> c_int {
    if stmt.is_null() || i_col < 0 {
        return 5; // SQLITE_NULL
    }
    let stmt_ref = unsafe { &*stmt };
    let col = i_col as usize;
    if col >= stmt_ref.columns.len() {
        return 5; // SQLITE_NULL
    }
    let current_row_idx = match stmt_ref.current_row {
        Some(idx) => idx,
        None => return 5, // SQLITE_NULL
    };
    let rows = match &stmt_ref.rows {
        Some(r) => r,
        None => return 5, // SQLITE_NULL
    };
    if current_row_idx >= rows.len() {
        return 5; // SQLITE_NULL
    }
    let row = &rows[current_row_idx];
    if col >= row.len() {
        return 5; // SQLITE_NULL
    }
    match &row[col] {
        crate::vdbe::Mem::Integer(_) => 1, // SQLITE_INTEGER
        crate::vdbe::Mem::Real(_) => 2,    // SQLITE_FLOAT
        crate::vdbe::Mem::Text(_) => 3,    // SQLITE_TEXT
        crate::vdbe::Mem::Blob(_) => 4,    // SQLITE_BLOB
        crate::vdbe::Mem::Null => 5,       // SQLITE_NULL
    }
}

/// 内部辅助函数：获取当前行的列 `Mem`。
unsafe fn get_column_mem(stmt: *mut SqliteStmt, i_col: c_int) -> Option<*const crate::vdbe::Mem> {
    if stmt.is_null() || i_col < 0 {
        return None;
    }
    let stmt_ref = unsafe { &*stmt };
    let col = i_col as usize;
    if col >= stmt_ref.columns.len() {
        return None;
    }
    let current_row_idx = stmt_ref.current_row?;
    let rows = stmt_ref.rows.as_ref()?;
    if current_row_idx >= rows.len() {
        return None;
    }
    let row = &rows[current_row_idx];
    if col >= row.len() {
        return None;
    }
    Some(&row[col] as *const crate::vdbe::Mem)
}

/// `sqlite3_column_int` — 结果列整数值 (i32)。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_column_int(stmt: *mut SqliteStmt, i_col: c_int) -> c_int {
    unsafe { sqlite3_column_int64(stmt, i_col) as c_int }
}

/// `sqlite3_column_int64` — 结果列长整数值 (i64)。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_column_int64(stmt: *mut SqliteStmt, i_col: c_int) -> i64 {
    let mem_ptr = unsafe { get_column_mem(stmt, i_col) };
    if mem_ptr.is_none() {
        return 0;
    }
    let mem = unsafe { &*mem_ptr.unwrap() };
    match mem {
        crate::vdbe::Mem::Integer(i) => *i,
        crate::vdbe::Mem::Real(f) => *f as i64,
        crate::vdbe::Mem::Text(s) => s.parse::<i64>().unwrap_or(0),
        crate::vdbe::Mem::Blob(b) => {
            if b.len() == 8 {
                i64::from_be_bytes(b[..8].try_into().unwrap())
            } else {
                0
            }
        }
        crate::vdbe::Mem::Null => 0,
    }
}

/// `sqlite3_column_double` — 结果列双精度浮点值 (f64)。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_column_double(stmt: *mut SqliteStmt, i_col: c_int) -> f64 {
    let mem_ptr = unsafe { get_column_mem(stmt, i_col) };
    if mem_ptr.is_none() {
        return 0.0;
    }
    let mem = unsafe { &*mem_ptr.unwrap() };
    match mem {
        crate::vdbe::Mem::Integer(i) => *i as f64,
        crate::vdbe::Mem::Real(f) => *f,
        crate::vdbe::Mem::Text(s) => s.parse::<f64>().unwrap_or(0.0),
        crate::vdbe::Mem::Blob(_) => 0.0,
        crate::vdbe::Mem::Null => 0.0,
    }
}

/// `sqlite3_column_text` — 结果列文本。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_column_text(stmt: *mut SqliteStmt, i_col: c_int) -> *const u8 {
    let mem_ptr = unsafe { get_column_mem(stmt, i_col) };
    if mem_ptr.is_none() {
        return std::ptr::null();
    }
    let mem = unsafe { &*mem_ptr.unwrap() };
    let text_str = match mem {
        crate::vdbe::Mem::Integer(i) => i.to_string(),
        crate::vdbe::Mem::Real(f) => f.to_string(),
        crate::vdbe::Mem::Text(s) => s.clone(),
        crate::vdbe::Mem::Blob(b) => String::from_utf8_lossy(b).into_owned(),
        crate::vdbe::Mem::Null => return std::ptr::null(),
    };

    let stmt_ref = unsafe { &mut *stmt };
    let col = i_col as usize;
    if col >= stmt_ref.cached_col_texts.len() {
        return std::ptr::null();
    }

    let c_str = CString::new(text_str).unwrap_or_else(|_| CString::new("").unwrap());
    let ptr = c_str.as_ptr() as *const u8;
    stmt_ref.cached_col_texts[col] = Some(c_str);
    ptr
}

/// `sqlite3_column_blob` — 结果列二进制大对象 (Blob)。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_column_blob(stmt: *mut SqliteStmt, i_col: c_int) -> *const c_void {
    let mem_ptr = unsafe { get_column_mem(stmt, i_col) };
    if mem_ptr.is_none() {
        return std::ptr::null();
    }
    let mem = unsafe { &*mem_ptr.unwrap() };
    match mem {
        crate::vdbe::Mem::Blob(b) => b.as_ptr() as *const c_void,
        crate::vdbe::Mem::Text(s) => s.as_ptr() as *const c_void,
        _ => std::ptr::null(),
    }
}

/// `sqlite3_column_bytes` — 结果列字节长度。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_column_bytes(stmt: *mut SqliteStmt, i_col: c_int) -> c_int {
    let mem_ptr = unsafe { get_column_mem(stmt, i_col) };
    if mem_ptr.is_none() {
        return 0;
    }
    let mem = unsafe { &*mem_ptr.unwrap() };
    match mem {
        crate::vdbe::Mem::Blob(b) => b.len() as c_int,
        crate::vdbe::Mem::Text(s) => s.len() as c_int,
        crate::vdbe::Mem::Integer(i) => i.to_string().len() as c_int,
        crate::vdbe::Mem::Real(f) => f.to_string().len() as c_int,
        crate::vdbe::Mem::Null => 0,
    }
}

/// `sqlite3_exec` — 高级封装，直接执行多条语句并通过回调函数获取查询结果。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_exec(
    db: *mut SqliteDb,
    sql: *const c_char,
    callback: Option<
        unsafe extern "C" fn(
            *mut c_void,
            c_int,
            *mut *mut c_char,
            *mut *mut c_char,
        ) -> c_int,
    >,
    arg: *mut c_void,
    errmsg: *mut *mut c_char,
) -> c_int {
    if db.is_null() || sql.is_null() {
        return SqliteError::MISUSE.code();
    }

    let db_ref = unsafe { &*db };
    db_ref.clear_error();

    if !errmsg.is_null() {
        unsafe { *errmsg = std::ptr::null_mut() };
    }

    let sql_str = match unsafe { CStr::from_ptr(sql) }.to_str() {
        Ok(s) => s,
        Err(_) => {
            let err_msg = "invalid utf-8 in SQL";
            db_ref.set_error(SqliteError::ERROR.code(), err_msg);
            if !errmsg.is_null() {
                let msg_c = CString::new(err_msg).unwrap();
                unsafe { *errmsg = sqlite3_malloc(msg_c.as_bytes_with_nul().len() as c_int) as *mut c_char };
                if !unsafe { *errmsg }.is_null() {
                    unsafe { std::ptr::copy_nonoverlapping(msg_c.as_ptr(), *errmsg, msg_c.as_bytes_with_nul().len()) };
                }
            }
            return SqliteError::ERROR.code();
        }
    };

    let stmts = match crate::parse::parse_sql(sql_str) {
        Ok(s) => s,
        Err(e) => {
            db_ref.set_error(e.code(), &e.message());
            if !errmsg.is_null() {
                let msg_c = CString::new(e.message()).unwrap_or_else(|_| CString::new("").unwrap());
                unsafe { *errmsg = sqlite3_malloc(msg_c.as_bytes_with_nul().len() as c_int) as *mut c_char };
                if !unsafe { *errmsg }.is_null() {
                    unsafe { std::ptr::copy_nonoverlapping(msg_c.as_ptr(), *errmsg, msg_c.as_bytes_with_nul().len()) };
                }
            }
            return e.code();
        }
    };

    for stmt in stmts {
        if matches!(stmt, crate::parse::Stmt::Empty) {
            continue;
        }

        let mut schema = match db_ref.schema.lock() {
            Ok(s) => s,
            Err(_) => {
                let err_msg = "failed to lock schema";
                db_ref.set_error(SqliteError::ERROR.code(), err_msg);
                return SqliteError::ERROR.code();
            }
        };

        match &stmt {
            crate::parse::Stmt::Select(select_stmt) => {
                let cols = get_result_columns(&stmt, &*schema);
                match crate::where_compiler::run_select(select_stmt, &mut *schema) {
                    Ok(result_rows) => {
                        if let Some(cb) = callback {
                            let col_c_strs: Vec<CString> = cols
                                .iter()
                                .map(|c| CString::new(c.clone()).unwrap_or_else(|_| CString::new("").unwrap()))
                                .collect();
                            let mut col_ptrs: Vec<*mut c_char> = col_c_strs.iter().map(|c| c.as_ptr() as *mut c_char).collect();

                            for row in result_rows {
                                let val_c_strs: Vec<Option<CString>> = row
                                    .iter()
                                    .map(|m| match m {
                                        crate::vdbe::Mem::Null => None,
                                        crate::vdbe::Mem::Integer(i) => Some(CString::new(i.to_string()).unwrap()),
                                        crate::vdbe::Mem::Real(f) => Some(CString::new(f.to_string()).unwrap()),
                                        crate::vdbe::Mem::Text(s) => Some(CString::new(s.clone()).unwrap()),
                                        crate::vdbe::Mem::Blob(b) => Some(CString::new(String::from_utf8_lossy(b).into_owned()).unwrap()),
                                    })
                                    .collect();

                                let mut val_ptrs: Vec<*mut c_char> = val_c_strs
                                    .iter()
                                    .map(|opt| opt.as_ref().map_or(std::ptr::null_mut(), |c| c.as_ptr() as *mut c_char))
                                    .collect();

                                let rc = unsafe {
                                    cb(
                                        arg,
                                        cols.len() as c_int,
                                        val_ptrs.as_mut_ptr(),
                                        col_ptrs.as_mut_ptr(),
                                    )
                                };
                                if rc != 0 {
                                    let err_msg = "callback requested abort";
                                    db_ref.set_error(SqliteError::ABORT.code(), err_msg);
                                    return SqliteError::ABORT.code();
                                }
                            }
                        }
                    }
                    Err(e) => {
                        db_ref.set_error(e.code(), &e.message());
                        if !errmsg.is_null() {
                            let msg_c = CString::new(e.message()).unwrap_or_else(|_| CString::new("").unwrap());
                            unsafe { *errmsg = sqlite3_malloc(msg_c.as_bytes_with_nul().len() as c_int) as *mut c_char };
                            if !unsafe { *errmsg }.is_null() {
                                unsafe { std::ptr::copy_nonoverlapping(msg_c.as_ptr(), *errmsg, msg_c.as_bytes_with_nul().len()) };
                            }
                        }
                        return e.code();
                    }
                }
            }
            other_stmt => {
                let prog = match crate::where_compiler::compile_stmt(other_stmt, &*schema) {
                    Ok(p) => p,
                    Err(e) => {
                        db_ref.set_error(e.code(), &e.message());
                        if !errmsg.is_null() {
                            let msg_c = CString::new(e.message()).unwrap_or_else(|_| CString::new("").unwrap());
                            unsafe { *errmsg = sqlite3_malloc(msg_c.as_bytes_with_nul().len() as c_int) as *mut c_char };
                            if !unsafe { *errmsg }.is_null() {
                                unsafe { std::ptr::copy_nonoverlapping(msg_c.as_ptr(), *errmsg, msg_c.as_bytes_with_nul().len()) };
                            }
                        }
                        return e.code();
                    }
                };
                if let Err(e) = crate::vdbe::exec(&prog, &mut *schema) {
                    db_ref.set_error(e.code(), &e.message());
                    if !errmsg.is_null() {
                        let msg_c = CString::new(e.message()).unwrap_or_else(|_| CString::new("").unwrap());
                        unsafe { *errmsg = sqlite3_malloc(msg_c.as_bytes_with_nul().len() as c_int) as *mut c_char };
                        if !unsafe { *errmsg }.is_null() {
                            unsafe { std::ptr::copy_nonoverlapping(msg_c.as_ptr(), *errmsg, msg_c.as_bytes_with_nul().len()) };
                        }
                    }
                    return e.code();
                }
            }
        }
    }

    SqliteError::OK.code()
}

// 'static 字符串(嵌入二进制,末尾 \0)
static SQLITE_VERSION_STR: &[u8] = b"3.54.0\0";
static ERRSG_NOT_IMPL: &[u8] = b"not implemented\0";
