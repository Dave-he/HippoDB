//! Virtual File System (VFS) abstraction — partial port of
//! `sqlite-source/src/os_unix.c` and the `sqlite3_vfs` /
//! `sqlite3_file` structures in `sqlite-source/src/sqlite.h.in`.
//!
//! For T-0010 we implement the slim subset: `xOpen` / `xClose` /
//! `xRead` / `xWrite` / `xSync` on `File`, and `xOpen` / `xDelete`
//! / `xAccess` / `xFullPathname` on `Vfs`. The Unix VFS is the
//! first concrete implementation.
//!
//! # Design
//!
//! The C source's `sqlite3_vfs` is a vtable; we mirror that with a
//! `Vfs` trait. The `sqlite3_file` is also a vtable; we mirror that
//! with a `File` trait. Each concrete VFS owns a registry of open
//! files (the C source's `OpenCounter` at os_unix.c:200-220) but
//! for T-0010 we use `std::sync::Mutex<HashMap<FileHandle, ...>>`.
//!
//! # Mapping C ↔ Rust
//!
//! | C                | Rust                        |
//! |------------------|------------------------------|
//! | `sqlite3_vfs`    | `trait Vfs`                  |
//! | `sqlite3_file`   | `trait File`                 |
//! | `unixFile`       | `struct UnixFile`            |
//! | `unixVfs`        | `struct UnixVfs`             |
//! | `open(2)`        | `std::fs::OpenOptions`      |
//! | `read(2)`/`pread`| `File::read_at` (Rust 1.78+) |
//! | `write(2)`/`pwrite` | `File::write_at`          |
//! | `fsync(2)`       | `File::sync_all`            |
//! | `close(2)`       | `drop`                      |

pub mod unix;

use crate::error::{SqliteError, SqliteResult};
use std::sync::atomic::{AtomicI32, Ordering};

/// Open flags for `Vfs::open` (subset of `SQLITE_OPEN_*` constants).
///
/// We define our own enum to keep the public surface Rust-idiomatic
/// while still being compatible with the C constants when translated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenFlags {
    /// `SQLITE_OPEN_READONLY` — open the file read-only.
    pub read_only: bool,
    /// `SQLITE_OPEN_READWRITE` — open the file for reading and writing.
    pub read_write: bool,
    /// `SQLITE_OPEN_CREATE` — create the file if it doesn't exist.
    /// Requires `read_write`.
    pub create: bool,
    /// `SQLITE_OPEN_DELETEONCLOSE` — delete the file when the last
    /// reference is closed. (Out of scope for T-0010.)
    pub delete_on_close: bool,
    /// `SQLITE_OPEN_EXCLUSIVE` — error if the file already exists
    /// (only with `create`). (Out of scope for T-0010.)
    pub exclusive: bool,
}

impl OpenFlags {
    /// Translate the `flags` integer to an `OpenFlags`. The C source
    /// uses bit flags; we decode the subset we care about.
    pub fn from_int(flags: i32) -> Self {
        OpenFlags {
            read_only: (flags & 0x01) != 0,
            read_write: (flags & 0x02) != 0,
            create: (flags & 0x04) != 0,
            delete_on_close: (flags & 0x08) != 0,
            exclusive: (flags & 0x10) != 0,
        }
    }
}

/// What kind of access to check (`xAccess` method).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    /// `SQLITE_ACCESS_EXISTS` — file exists.
    Exists,
    /// `SQLITE_ACCESS_READWRITE` — directory is readable and writable.
    ReadWrite,
    /// `SQLITE_ACCESS_READ` — file is readable.
    Read,
}

/// A trait representing an open file (the Rust analog of
/// `sqlite3_io_methods`).
///
/// The methods use `(buf, offset)` for `read`/`write` to match the
/// C source's `pread(2)` / `pwrite(2)` semantics (positional I/O,
/// not stream I/O). The `close` is implicit in `drop`.
pub trait File: Send {
    /// Read up to `buf.len()` bytes from `offset` into `buf`.
    /// Returns the number of bytes read (may be less than `buf.len()`).
    fn read(&mut self, buf: &mut [u8], offset: i64) -> SqliteResult<usize>;

    /// Write `buf` to `offset`. Returns the number of bytes written.
    fn write(&mut self, buf: &[u8], offset: i64) -> SqliteResult<usize>;

    /// Flush any buffered writes to disk.
    fn sync(&mut self) -> SqliteResult<()>;

    /// Return the current file size in bytes.
    fn file_size(&self) -> SqliteResult<i64>;

    /// Truncate the file to `size` bytes.
    fn truncate(&mut self, size: i64) -> SqliteResult<()>;
}

/// A trait representing a Virtual File System (the Rust analog of
/// `sqlite3_vfs`).
///
/// For T-0010 we only implement the slim subset: `open`, `delete`,
/// `access`, `full_pathname`. The remaining methods (`xRandomness`,
/// `xSleep`, `xCurrentTime`, `xDlOpen`, etc.) are deferred.
pub trait Vfs: Send + Sync {
    /// The VFS name (e.g. "unix").
    fn name(&self) -> &str;

    /// The maximum pathname length supported (typically 1024).
    fn max_pathname(&self) -> usize;

    /// Open a file. Returns a boxed `File` on success.
    fn open(&self, path: &str, flags: OpenFlags) -> SqliteResult<Box<dyn File>>;

    /// Delete a file. Returns `SQLITE_OK` on success.
    fn delete(&self, path: &str) -> SqliteResult<()>;

    /// Check accessibility. Returns `true` if the access check passes.
    fn access(&self, path: &str, kind: AccessKind) -> SqliteResult<bool>;

    /// Convert a relative path to absolute. Returns the full path.
    fn full_pathname(&self, path: &str) -> SqliteResult<String>;
}

/// Global counter for the number of currently-open files. The C
/// source uses this for the `SQLITE_OPEN_NOFOLLOW` semantics and
/// for `sqlite3_open_count` (used by tests). For T-0010 we expose
/// it for test purposes only.
static OPEN_FILE_COUNT: AtomicI32 = AtomicI32::new(0);

/// Return the current number of open files (for testing).
pub fn open_file_count() -> i32 {
    OPEN_FILE_COUNT.load(Ordering::Relaxed)
}

/// Increment the open file counter (called by VFS impls on `open`).
pub(crate) fn increment_open_files() {
    OPEN_FILE_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// Decrement the open file counter (called by VFS impls on `close`).
pub(crate) fn decrement_open_files() {
    OPEN_FILE_COUNT.fetch_sub(1, Ordering::Relaxed);
}

/// Map a Rust I/O error to a `SqliteError`. We use `SQLITE_IOERR_READ`
/// for read errors, `SQLITE_IOERR_WRITE` for write errors, and
/// `SQLITE_IOERR` as the default.
pub fn map_io_error(_err: std::io::Error, op: IoOp) -> SqliteError {
    let code = match op {
        IoOp::Open => 14,   // SQLITE_CANTOPEN
        IoOp::Close => 10,  // SQLITE_IOERR
        IoOp::Read => 266,  // SQLITE_IOERR_READ
        IoOp::Write => 778, // SQLITE_IOERR_WRITE
        IoOp::Sync => 1034, // SQLITE_IOERR_FSYNC
        IoOp::Delete => 10, // SQLITE_IOERR_DELETE
        IoOp::Access => 10, // SQLITE_IOERR
        IoOp::FileSize => 10,
        IoOp::Truncate => 10,
    };
    SqliteError(code)
}

/// What I/O operation failed.
pub enum IoOp {
    Open,
    Close,
    Read,
    Write,
    Sync,
    Delete,
    Access,
    FileSize,
    Truncate,
}
