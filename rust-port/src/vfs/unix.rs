//! Unix VFS — concrete implementation of the `Vfs` trait using
//! `std::fs`.
//!
//! Mirrors `sqlite-source/src/os_unix.c`'s `unixVfs` (the default VFS
//! SQLite registers). For T-0010 we implement the slim subset:
//! `xOpen` / `xClose` / `xRead` / `xWrite` / `xSync` / `xTruncate` /
//! `xFileSize` / `xDelete` / `xAccess` / `xFullPathname`.

use crate::error::{SqliteError, SqliteResult};
use crate::vfs::{
    decrement_open_files, increment_open_files, map_io_error, AccessKind, File, IoOp, OpenFlags,
    Vfs,
};
use std::fs::{File as StdFile, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Concrete Unix file handle.
pub struct UnixFile {
    /// The underlying Rust file. We hold the file open for the
    /// lifetime of the `UnixFile`.
    file: StdFile,
    /// Cached file size (mirrors the C source's `unixFile.nByte`).
    cached_size: i64,
}

impl UnixFile {
    /// Open a Unix file. Mirrors `unixOpen` at os_unix.c (the slim
    /// subset: read/write/create modes).
    fn open(path: &str, flags: OpenFlags) -> SqliteResult<Self> {
        let mut opts = OpenOptions::new();
        // The C source's flag-to-open-mode translation is complex;
        // we use the simple subset that matches the test cases.
        if flags.read_only {
            opts.read(true);
        } else if flags.read_write {
            opts.read(true).write(true);
            if flags.create {
                opts.create(true);
            }
        } else {
            // Default: read-only if neither flag is set.
            opts.read(true);
        }
        // Open the file. If `create` is set and the file exists, the
        // default behavior is to open it (not truncate).
        let file = opts.open(Path::new(path)).map_err(|e| map_io_error(e, IoOp::Open))?;
        // Compute the file size.
        let metadata = file
            .metadata()
            .map_err(|e| map_io_error(e, IoOp::FileSize))?;
        let cached_size = metadata.len() as i64;
        increment_open_files();
        Ok(UnixFile { file, cached_size })
    }
}

impl File for UnixFile {
    fn read(&mut self, buf: &mut [u8], offset: i64) -> SqliteResult<usize> {
        if offset < 0 {
            return Err(SqliteError(22)); // SQLITE_IOERR
        }
        // Seek to the offset, then read.
        self.file
            .seek(SeekFrom::Start(offset as u64))
            .map_err(|e| map_io_error(e, IoOp::Read))?;
        let mut handle = Read::by_ref(&mut self.file);
        let mut total = 0;
        while total < buf.len() {
            match handle.read(&mut buf[total..]) {
                Ok(0) => break, // EOF
                Ok(n) => total += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(map_io_error(e, IoOp::Read)),
            }
        }
        Ok(total)
    }

    fn write(&mut self, buf: &[u8], offset: i64) -> SqliteResult<usize> {
        if offset < 0 {
            return Err(SqliteError(22));
        }
        self.file
            .seek(SeekFrom::Start(offset as u64))
            .map_err(|e| map_io_error(e, IoOp::Write))?;
        let mut handle = Write::by_ref(&mut self.file);
        let mut total = 0;
        while total < buf.len() {
            match handle.write(&mut &buf[total..]) {
                Ok(0) => break, // shouldn't happen, but guard anyway
                Ok(n) => total += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(map_io_error(e, IoOp::Write)),
            }
        }
        // Update cached size if we extended the file.
        let end = offset + total as i64;
        if end > self.cached_size {
            self.cached_size = end;
        }
        Ok(total)
    }

    fn sync(&mut self) -> SqliteResult<()> {
        self.file
            .sync_all()
            .map_err(|e| map_io_error(e, IoOp::Sync))
    }

    fn file_size(&self) -> SqliteResult<i64> {
        Ok(self.cached_size)
    }

    fn truncate(&mut self, size: i64) -> SqliteResult<()> {
        self.file
            .set_len(size as u64)
            .map_err(|e| map_io_error(e, IoOp::Truncate))?;
        self.cached_size = size;
        Ok(())
    }
}

impl Drop for UnixFile {
    fn drop(&mut self) {
        decrement_open_files();
    }
}

/// The Unix VFS instance.
pub struct UnixVfs {
    /// Maximum pathname length. Mirrors the C source's `MAX_PATHNAME`
    /// (os_unix.c:217).
    max_pathname: usize,
}

impl UnixVfs {
    /// Construct a new Unix VFS. The `max_pathname` is typically 512
    /// or 1024; the C source uses 512.
    pub const fn new() -> Self {
        UnixVfs {
            max_pathname: 512,
        }
    }
}

impl Default for UnixVfs {
    fn default() -> Self {
        Self::new()
    }
}

impl Vfs for UnixVfs {
    fn name(&self) -> &str {
        "unix"
    }

    fn max_pathname(&self) -> usize {
        self.max_pathname
    }

    fn open(&self, path: &str, flags: OpenFlags) -> SqliteResult<Box<dyn File>> {
        UnixFile::open(path, flags).map(|f| Box::new(f) as Box<dyn File>)
    }

    fn delete(&self, path: &str) -> SqliteResult<()> {
        std::fs::remove_file(Path::new(path)).map_err(|e| map_io_error(e, IoOp::Delete))?;
        Ok(())
    }

    fn access(&self, path: &str, kind: AccessKind) -> SqliteResult<bool> {
        let p = Path::new(path);
        let exists = p.exists();
        match kind {
            AccessKind::Exists => Ok(exists),
            AccessKind::Read => Ok(exists && std::fs::metadata(p).is_ok()),
            AccessKind::ReadWrite => {
                // Check parent directory's writability (matching the
                // C source's xAccess behavior for SQLITE_ACCESS_READWRITE).
                let parent = p.parent().unwrap_or(Path::new("."));
                let writable = if let Ok(md) = std::fs::metadata(parent) {
                    md.is_dir() && !md.permissions().readonly()
                } else {
                    false
                };
                Ok(writable)
            }
        }
    }

    fn full_pathname(&self, path: &str) -> SqliteResult<String> {
        // For Unix, the C source returns the path as-is if it's
        // already absolute, or prepends the cwd otherwise. For T-0010
        // we just return the path verbatim (no cwd resolution).
        if path.len() > self.max_pathname {
            return Err(SqliteError(18)); // SQLITE_TOOBIG
        }
        Ok(path.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as IoWrite;

    fn temp_path(name: &str) -> String {
        let mut p = std::env::temp_dir();
        p.push(format!("sqllite_rs_test_{}_{}", std::process::id(), name));
        p.to_str().unwrap().to_string()
    }

    #[test]
    fn open_close_basic() {
        let path = temp_path("open_close");
        // Create the file first.
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, b"hello").unwrap();
        let vfs = UnixVfs::new();
        let f = vfs.open(&path, OpenFlags::from_int(0x01)).unwrap(); // READONLY
        let _ = f.file_size();
    }

    #[test]
    fn read_write_round_trip() {
        let path = temp_path("read_write");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, b"hello").unwrap();
        let vfs = UnixVfs::new();
        let mut f = vfs.open(&path, OpenFlags::from_int(0x02)).unwrap(); // READWRITE
        // Read what was written.
        let mut buf = [0u8; 5];
        let n = f.read(&mut buf, 0).unwrap();
        assert_eq!(n, 5);
        assert_eq!(&buf, b"hello");
        // Write at offset 0.
        let n = f.write(b"world", 0).unwrap();
        assert_eq!(n, 5);
        // Re-read.
        let mut buf = [0u8; 5];
        f.read(&mut buf, 0).unwrap();
        assert_eq!(&buf, b"world");
        // File size should now be 5 (unchanged).
        assert_eq!(f.file_size().unwrap(), 5);
    }

    #[test]
    fn read_past_end_returns_zero() {
        let path = temp_path("read_past_end");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, b"hi").unwrap();
        let vfs = UnixVfs::new();
        let mut f = vfs.open(&path, OpenFlags::from_int(0x01)).unwrap();
        let mut buf = [0u8; 100];
        let n = f.read(&mut buf, 0).unwrap();
        assert_eq!(n, 2);
    }

    #[test]
    fn write_extends_file() {
        let path = temp_path("write_extends");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, b"hi").unwrap();
        let vfs = UnixVfs::new();
        let mut f = vfs.open(&path, OpenFlags::from_int(0x02)).unwrap();
        f.write(b"hello", 1).unwrap(); // overwrite from offset 1
        // File size = max(2, 1+5) = 6.
        assert_eq!(f.file_size().unwrap(), 6);
    }

    #[test]
    fn truncate_shortens() {
        let path = temp_path("truncate");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, b"hello world").unwrap();
        let vfs = UnixVfs::new();
        let mut f = vfs.open(&path, OpenFlags::from_int(0x02)).unwrap();
        assert_eq!(f.file_size().unwrap(), 11);
        f.truncate(5).unwrap();
        assert_eq!(f.file_size().unwrap(), 5);
    }

    #[test]
    fn access_exists() {
        let path = temp_path("access_exists");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, b"x").unwrap();
        let vfs = UnixVfs::new();
        assert!(vfs.access(&path, AccessKind::Exists).unwrap());
        assert!(!vfs.access("/no/such/path", AccessKind::Exists).unwrap());
    }

    #[test]
    fn delete_removes_file() {
        let path = temp_path("delete");
        std::fs::write(&path, b"x").unwrap();
        let vfs = UnixVfs::new();
        assert!(vfs.access(&path, AccessKind::Exists).unwrap());
        vfs.delete(&path).unwrap();
        assert!(!vfs.access(&path, AccessKind::Exists).unwrap());
    }

    #[test]
    fn sync_no_error() {
        let path = temp_path("sync");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, b"x").unwrap();
        let vfs = UnixVfs::new();
        let mut f = vfs.open(&path, OpenFlags::from_int(0x02)).unwrap();
        f.sync().unwrap();
    }

    #[test]
    fn open_file_count_tracks() {
        let path1 = temp_path("cnt1");
        let path2 = temp_path("cnt2");
        let _ = std::fs::remove_file(&path1);
        let _ = std::fs::remove_file(&path2);
        std::fs::write(&path1, b"1").unwrap();
        std::fs::write(&path2, b"2").unwrap();
        let vfs = UnixVfs::new();
        let before = crate::vfs::open_file_count();
        {
            let _f1 = vfs.open(&path1, OpenFlags::from_int(0x01)).unwrap();
            let _f2 = vfs.open(&path2, OpenFlags::from_int(0x01)).unwrap();
            assert_eq!(crate::vfs::open_file_count(), before + 2);
        }
        // After drop, the count should return to before.
        assert_eq!(crate::vfs::open_file_count(), before);
    }
}
