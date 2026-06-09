//! SqliteError — 1:1 复刻官方 C 头文件中的 `i32` 错误码。
//!
//! C 端代码用返回 `i32` 表达错误。Rust 端我们用 newtype 包装,但保持
//! `SqliteError == 0` 即 `OK` 的语义,使得 `as i32` 后 byte-for-byte
//! 与 C ABI 一致。

use core::fmt;
use std::error::Error;

/// 包装官方 `i32` 错误码的 newtype。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(transparent)]
pub struct SqliteError(pub i32);

impl SqliteError {
    /// SQLITE_OK = 0
    pub const OK: SqliteError = SqliteError(0);
    /// SQLITE_ERROR = 1
    pub const ERROR: SqliteError = SqliteError(1);
    /// SQLITE_INTERNAL = 2
    pub const INTERNAL: SqliteError = SqliteError(2);
    /// SQLITE_PERM = 3
    pub const PERM: SqliteError = SqliteError(3);
    /// SQLITE_ABORT = 4
    pub const ABORT: SqliteError = SqliteError(4);
    /// SQLITE_BUSY = 5
    pub const BUSY: SqliteError = SqliteError(5);
    /// SQLITE_LOCKED = 6
    pub const LOCKED: SqliteError = SqliteError(6);
    /// SQLITE_NOMEM = 7
    pub const NOMEM: SqliteError = SqliteError(7);
    /// SQLITE_READONLY = 8
    pub const READONLY: SqliteError = SqliteError(8);
    /// SQLITE_INTERRUPT = 9
    pub const INTERRUPT: SqliteError = SqliteError(9);
    /// SQLITE_IOERR = 10
    pub const IOERR: SqliteError = SqliteError(10);
    /// SQLITE_CORRUPT = 11
    pub const CORRUPT: SqliteError = SqliteError(11);
    /// SQLITE_NOTFOUND = 12
    pub const NOTFOUND: SqliteError = SqliteError(12);
    /// SQLITE_FULL = 13
    pub const FULL: SqliteError = SqliteError(13);
    /// SQLITE_CANTOPEN = 14
    pub const CANTOPEN: SqliteError = SqliteError(14);
    /// SQLITE_PROTOCOL = 15
    pub const PROTOCOL: SqliteError = SqliteError(15);
    /// SQLITE_EMPTY = 16
    pub const EMPTY: SqliteError = SqliteError(16);
    /// SQLITE_SCHEMA = 17
    pub const SCHEMA: SqliteError = SqliteError(17);
    /// SQLITE_TOOBIG = 18
    pub const TOOBIG: SqliteError = SqliteError(18);
    /// SQLITE_CONSTRAINT = 19
    pub const CONSTRAINT: SqliteError = SqliteError(19);
    /// SQLITE_MISMATCH = 20
    pub const MISMATCH: SqliteError = SqliteError(20);
    /// SQLITE_MISUSE = 21
    pub const MISUSE: SqliteError = SqliteError(21);
    /// SQLITE_NOLFS = 22
    pub const NOLFS: SqliteError = SqliteError(22);
    /// SQLITE_AUTH = 23
    /// SQLITE_AUTH=23
    pub const AUTH: SqliteError = SqliteError(23);
    /// SQLITE_FORMAT = 24
    pub const FORMAT: SqliteError = SqliteError(24);
    /// SQLITE_RANGE = 25
    pub const RANGE: SqliteError = SqliteError(25);
    /// SQLITE_NOTADB = 26
    pub const NOTADB: SqliteError = SqliteError(26);
    /// SQLITE_NOTICE = 27
    pub const NOTICE: SqliteError = SqliteError(27);
    /// SQLITE_WARNING = 28
    pub const WARNING: SqliteError = SqliteError(28);
    /// SQLITE_ROW = 100
    pub const ROW: SqliteError = SqliteError(100);
    /// SQLITE_DONE = 101
    pub const DONE: SqliteError = SqliteError(101);

    /// 返回底层整数(等价于 C 端 `r` 的值)。
    #[inline]
    pub const fn code(self) -> i32 {
        self.0
    }

    /// 返回错误码对应的默认错误消息字符串。
    pub const fn message(self) -> &'static str {
        match self.0 {
            0 => "not an error",
            1 => "SQL logic error",
            2 => "internal error",
            3 => "permission denied",
            4 => "callback requested query abort",
            5 => "database is locked",
            7 => "out of memory",
            10 => "disk I/O error",
            11 => "database disk image is malformed",
            12 => "table or record not found",
            13 => "database is full",
            14 => "unable to open database file",
            19 => "constraint failed",
            20 => "datatype mismatch",
            21 => "bad parameter or other API misuse",
            22 => "large file support is disabled",
            _ => "unknown error",
        }
    }

    /// true 当且仅当为 SQLITE_OK(0)。
    #[inline]
    pub const fn is_ok(self) -> bool {
        self.0 == 0
    }

    /// true 当且仅当不为 SQLITE_OK(0)。
    #[inline]
    pub const fn is_err(self) -> bool {
        self.0 != 0
    }
}

impl fmt::Debug for SqliteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 对齐官方 sqlite3ErrName 的输出形式
        write!(f, "SqliteError({})", self.0)
    }
}

impl fmt::Display for SqliteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sqlite error code {}", self.0)
    }
}

impl Error for SqliteError {}

impl From<i32> for SqliteError {
    #[inline]
    fn from(v: i32) -> Self {
        SqliteError(v)
    }
}

impl From<SqliteError> for i32 {
    #[inline]
    fn from(e: SqliteError) -> Self {
        e.0
    }
}

impl From<std::ffi::NulError> for SqliteError {
    fn from(_: std::ffi::NulError) -> Self {
        SqliteError::ERROR
    }
}

impl<T> From<Result<T, SqliteError>> for SqliteError {
    fn from(r: Result<T, SqliteError>) -> Self {
        match r {
            Ok(_) => SqliteError::OK,
            Err(e) => e,
        }
    }
}

/// `Result<T, SqliteError>` 的便利别名。
pub type SqliteResult<T> = Result<T, SqliteError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_is_zero() {
        assert_eq!(SqliteError::OK.0, 0);
        assert_eq!(SqliteError::OK.code(), 0);
        assert!(SqliteError::OK.is_ok());
        assert!(!SqliteError::OK.is_err());
    }

    #[test]
    fn error_codes_match_c() {
        // 对照 sqlite3.h 验证 1:1
        assert_eq!(SqliteError::ERROR.0, 1);
        assert_eq!(SqliteError::INTERNAL.0, 2);
        assert_eq!(SqliteError::NOMEM.0, 7);
        assert_eq!(SqliteError::ROW.0, 100);
        assert_eq!(SqliteError::DONE.0, 101);
    }

    #[test]
    fn conversions() {
        let e: SqliteError = 7i32.into();
        assert_eq!(e, SqliteError::NOMEM);
        let n: i32 = e.into();
        assert_eq!(n, 7);
    }

    #[test]
    fn result_to_error() {
        let ok: SqliteError = Ok::<(), SqliteError>(()).into();
        assert!(ok.is_ok());
        let err: SqliteError = Err::<(), SqliteError>(SqliteError::BUSY).into();
        assert_eq!(err, SqliteError::BUSY);
    }
}
