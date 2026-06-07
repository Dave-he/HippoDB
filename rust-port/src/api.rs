//! FFI 公开 API 层。
//!
//! 对应 sqlite3.h 中的 C 函数。每个函数必须是 `#[no_mangle] pub unsafe extern "C"`,
//! 签名与官方保持 byte-for-byte 一致(指针宽度、整数宽度、调用约定)。
//!
//! 当前 P0 阶段只实现 4 个最小可验证函数:
//!  - `sqlite3_libversion`
//!  - `sqlite3_libversion_number`
//!  - `sqlite3_open`            (桩 — 分配 handle,返回 OK)
//!  - `sqlite3_close`           (桩 — 释放 handle,返回 OK)
//! 后续子任务会把它们替换为真实实现。

use std::ffi::{c_char, c_int, c_void, CStr};

use crate::error::SqliteError;
use crate::handle::SqliteDb;

/// 返回 SQLite 版本字符串。生命周期 = 进程,不需要 free。
///
/// C 契约:`const char *sqlite3_libversion(void);`
#[no_mangle]
pub unsafe extern "C" fn sqlite3_libversion() -> *const c_char {
    // 'static 字符串嵌入二进制
    SQLITE_VERSION_STR.as_ptr() as *const c_char
}

/// 返回 SQLite 版本号(形如 3054000 = 3.54.0)。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_libversion_number() -> c_int {
    crate::SQLITE_VERSION_NUMBER
}

/// 打开/创建数据库。P0 桩:接受任意 filename,分配一个 handle 写回。
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

/// 关闭数据库,释放 handle。P0 桩。
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
    // SAFETY: db 由 sqlite3_open 通过 Box::into_raw 产生,这里取回所有权。
    let _ = unsafe { Box::from_raw(db) };
    SqliteError::OK.code()
}

/// 返回最近一次错误的 UTF-8 字符串。P0 桩:返回占位。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_errmsg(db: *mut SqliteDb) -> *const c_char {
    let _ = db;
    ERRSG_NOT_IMPL.as_ptr() as *const c_char
}

/// 返回最近一次错误码。P0 桩:总是返回 OK。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_errcode(db: *mut SqliteDb) -> c_int {
    let _ = db;
    SqliteError::OK.code()
}

/// `sqlite3_open_v2` — 带 flag 的打开。P0 桩。
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

/// `sqlite3_malloc` — 公开 API malloc。P0 桩:用 std::alloc。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_malloc(n: c_int) -> *mut c_void {
    if n <= 0 {
        // C 端: malloc(0) 返回非 null 的"可 free"指针
        return 1usize as *mut c_void; // 假的非 null 指针,后续 T-0002 会替换
    }
    // SAFETY: n > 0 保证 Layout::from_size_align 不会失败(非零 size)。
    let layout = unsafe { std::alloc::Layout::from_size_align_unchecked(n as usize, 8) };
    // SAFETY: layout 合法(n>0, 8-byte 对齐),alloc 不 panic 仅返回 null on OOM。
    unsafe { std::alloc::alloc(layout) as *mut c_void }
}

/// `sqlite3_malloc64` — 同上但参数是 i64。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_malloc64(n: i64) -> *mut c_void {
    // SAFETY: 同 sqlite3_malloc
    unsafe { sqlite3_malloc(n as c_int) }
}

/// `sqlite3_free` — 释放。
#[no_mangle]
pub unsafe extern "C" fn sqlite3_free(p: *mut c_void) {
    if p.is_null() || p as usize == 1 {
        return;
    }
    // 桩:不实际释放(我们没有跟踪 size)。T-0002 会替换。
}

// 'static 字符串(嵌入二进制,末尾 \0)
static SQLITE_VERSION_STR: &[u8] = b"3.54.0\0";
static ERRSG_NOT_IMPL: &[u8] = b"not implemented\0";
