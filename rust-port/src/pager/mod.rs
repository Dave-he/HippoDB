//! Pager (page cache) — partial port of `sqlite-source/src/pager.c`.
//!
//! Implements a minimal read-only Pager: pages are read from the
//! database file on first access and cached in memory. The page
//! size is fixed at construction time (default 4096, matching the
//! C source's `SQLITE_DEFAULT_PAGE_SIZE`).
//!
//! # C source correspondence
//!
//! | Rust item         | C source                          |
//! |-------------------|-----------------------------------|
//! | `Pager::new`      | `sqlite3PagerOpen` (pager.c:5901) |
//! | `Pager::get`      | `sqlite3PagerGet`  (pager.c:5611) |
//! | `Pager::page_size`| `pPager->pageSize`               |
//! | `Pager::db_size`  | `pPager->dbSize`                  |
//!
//! # Behavior contract
//!
//! - Pages are cached in a `HashMap<Pgno, Page>` (Pgno = u32).
//! - The cache is bounded by `max_cache` pages; when full, the
//!   least-recently-used page is evicted (LRU).
//! - `get` returns a `&[u8]` view of the page data, valid for
//!   the lifetime of the `Pager` (or until the page is evicted).
//! - The page data buffer is the C source's `PgHdr.pData`; we
//!   expose it as `&[u8]` for the read-only path.
//!
//! # Out of scope (T-0011)
//!
//! - Write path (rollback journal, write-ahead log)
//! - Page locking / concurrency
//! - Savepoints / transactions

use std::collections::HashMap;
use std::sync::Arc;

use crate::error::{SqliteError, SqliteResult};
use crate::vfs::{File, OpenFlags, Vfs};

/// Page number type (matches the C source's `Pgno`).
pub type Pgno = u32;

/// Default page size in bytes (matches `SQLITE_DEFAULT_PAGE_SIZE = 4096`).
pub const DEFAULT_PAGE_SIZE: usize = 4096;

/// Default maximum cache size (number of pages).
pub const DEFAULT_CACHE_SIZE: usize = 2000;

/// A cached page in the pager.
#[derive(Clone)]
struct CachedPage {
    /// The page data, owned by the cache. Length is always
    /// `page_size` (zero-padded if the file is shorter).
    data: Vec<u8>,
    /// Last access counter for LRU eviction.
    last_access: u64,
}

/// The Pager — page cache for a single database file.
pub struct Pager {
    /// The underlying VFS file.
    file: Box<dyn File>,
    /// Page size in bytes (fixed at open time).
    page_size: usize,
    /// Number of pages in the database file.
    db_size: Pgno,
    /// Maximum number of pages to cache.
    max_cache: usize,
    /// Page cache: `Pgno -> CachedPage`.
    cache: HashMap<Pgno, CachedPage>,
    /// LRU access counter (monotonically increasing).
    access_counter: u64,
    /// Cache hit / miss counters for testing.
    hits: u64,
    misses: u64,
    /// The path of the database file (kept for diagnostics).
    path: String,
}

impl Pager {
    /// Open a database file and create a `Pager` for it.
    ///
    /// Mirrors `sqlite3PagerOpen` (pager.c:5901). For T-0011 we
    /// always open read-only — the write path is a separate task.
    pub fn open(
        vfs: &dyn Vfs,
        path: &str,
        page_size: usize,
        max_cache: usize,
    ) -> SqliteResult<Self> {
        // Open the file read-only for the slim T-0011 scope.
        let file = vfs.open(path, OpenFlags::from_int(0x01))?; // SQLITE_OPEN_READONLY
        // Determine the file size to compute db_size.
        let file_size = file.file_size()?;
        let db_size = if page_size > 0 {
            ((file_size as usize + page_size - 1) / page_size) as Pgno
        } else {
            0
        };
        Ok(Pager {
            file,
            page_size,
            db_size,
            max_cache,
            cache: HashMap::new(),
            access_counter: 0,
            hits: 0,
            misses: 0,
            path: path.to_string(),
        })
    }

    /// Read the database file header (page 1, first 100 bytes).
    /// Mirrors `sqlite3PagerReadFileheader` (pager.c:3897).
    pub fn read_file_header(&mut self, n: usize, out: &mut [u8]) -> SqliteResult<()> {
        let n = n.min(out.len());
        let page = self.get(1)?;
        out[..n].copy_from_slice(&page[..n]);
        Ok(())
    }

    /// Get a page by number. Returns a `&[u8]` view valid until the
    /// page is evicted from the cache.
    ///
    /// Mirrors `sqlite3PagerGet` (pager.c:5611) for the read path.
    pub fn get(&mut self, pgno: Pgno) -> SqliteResult<&[u8]> {
        if pgno < 1 {
            return Err(SqliteError(26)); // SQLITE_NOTADB
        }
        if pgno > self.db_size {
            return Err(SqliteError(26)); // SQLITE_NOTADB (out-of-bounds page)
        }
        // Check cache first.
        if self.cache.contains_key(&pgno) {
            self.hits += 1;
            self.access_counter += 1;
            self.cache.get_mut(&pgno).unwrap().last_access = self.access_counter;
            // SAFETY: we just confirmed the entry exists.
            let data = self.cache[&pgno].data.clone();
            // Stash the data in a side table to satisfy the borrow
            // checker: the returned `&[u8]` is borrowed from a Vec
            // stored in the cache, but the borrow can't escape the
            // method's scope because of how HashMap borrows work.
            // We use a different approach: store a small "scratch"
            // buffer that the caller can read.
            //
            // Actually the cleanest approach is to return a Vec<u8>
            // that the caller can use. But that defeats the purpose.
            //
            // For the read path we just return a fresh Vec<u8> from
            // the cache. The caller copies the data out before
            // the next `get` call. This is the simplest approach
            // for T-0011; a more sophisticated approach would use
            // a custom buffer pool.
            //
            // NOTE: This is a temporary implementation. T-0011 is
            // just the read path; the write path (T-0012) will
            // likely need a different design.
            return Ok(self.get_cached_data(pgno));
        }
        // Cache miss: read the page from disk.
        self.misses += 1;
        let offset = (pgno as i64 - 1) * self.page_size as i64;
        let mut data = vec![0u8; self.page_size];
        let n = self.file.read(&mut data, offset)?;
        // Zero-pad if the file is shorter than a full page.
        if n < self.page_size {
            for byte in &mut data[n..] {
                *byte = 0;
            }
        }
        // Evict if cache is full.
        if self.cache.len() >= self.max_cache {
            self.evict_lru();
        }
        self.access_counter += 1;
        let last_access = self.access_counter;
        self.cache.insert(
            pgno,
            CachedPage { data, last_access },
        );
        Ok(self.get_cached_data(pgno))
    }

    /// Internal: get a reference to the cached page's data.
    /// This uses a side buffer to escape the borrow checker.
    fn get_cached_data(&self, pgno: Pgno) -> &[u8] {
        // SAFETY: the page is in the cache because we just inserted
        // or confirmed it in `get`.
        let cached = self.cache.get(&pgno).expect("page in cache");
        &cached.data
    }

    /// Evict the least-recently-used page from the cache.
    fn evict_lru(&mut self) {
        if self.cache.is_empty() {
            return;
        }
        // Find the page with the smallest last_access.
        let victim = self
            .cache
            .iter()
            .min_by_key(|(_, p)| p.last_access)
            .map(|(k, _)| *k);
        if let Some(k) = victim {
            self.cache.remove(&k);
        }
    }

    /// The page size in bytes.
    pub fn page_size(&self) -> usize {
        self.page_size
    }

    /// The number of pages in the database.
    pub fn db_size(&self) -> Pgno {
        self.db_size
    }

    /// The path of the database file.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Number of cache hits (for testing).
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// Number of cache misses (for testing).
    pub fn misses(&self) -> u64 {
        self.misses
    }

    /// Number of pages currently in the cache.
    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::unix::UnixVfs;
    use std::fs;

    fn make_db(name: &str, page_size: usize, n_pages: usize) -> String {
        let mut p = std::env::temp_dir();
        p.push(format!("sqllite_rs_pager_{}_{}", std::process::id(), name));
        let path = p.to_str().unwrap().to_string();
        // Create a file with `n_pages * page_size` bytes of deterministic data.
        let mut data = Vec::with_capacity(n_pages * page_size);
        for i in 0..(n_pages * page_size) {
            data.push((i % 256) as u8);
        }
        fs::write(&path, &data).unwrap();
        path
    }

    #[test]
    fn open_small_db() {
        let path = make_db("small", DEFAULT_PAGE_SIZE, 4);
        let vfs = UnixVfs::new();
        let pager = Pager::open(&vfs, &path, DEFAULT_PAGE_SIZE, 100).unwrap();
        assert_eq!(pager.page_size(), DEFAULT_PAGE_SIZE);
        assert_eq!(pager.db_size(), 4);
        assert_eq!(pager.path(), path);
    }

    #[test]
    fn read_page_contents() {
        let path = make_db("read", DEFAULT_PAGE_SIZE, 4);
        let vfs = UnixVfs::new();
        let mut pager = Pager::open(&vfs, &path, DEFAULT_PAGE_SIZE, 100).unwrap();
        let page = pager.get(1).unwrap();
        // Page 1 starts with `data[0..page_size]`.
        for i in 0..page.len() {
            assert_eq!(page[i], (i % 256) as u8);
        }
    }

    #[test]
    fn cache_hit_on_second_access() {
        let path = make_db("hit", DEFAULT_PAGE_SIZE, 4);
        let vfs = UnixVfs::new();
        let mut pager = Pager::open(&vfs, &path, DEFAULT_PAGE_SIZE, 100).unwrap();
        let _ = pager.get(1).unwrap();
        let _ = pager.get(1).unwrap();
        assert_eq!(pager.hits(), 1);
        assert_eq!(pager.misses(), 1);
    }

    #[test]
    fn cache_eviction_lru() {
        let path = make_db("evict", 256, 10);
        let vfs = UnixVfs::new();
        let mut pager = Pager::open(&vfs, &path, 256, 3).unwrap();
        // Touch pages 1, 2, 3 → cache holds {1, 2, 3}.
        let _ = pager.get(1).unwrap();
        let _ = pager.get(2).unwrap();
        let _ = pager.get(3).unwrap();
        assert_eq!(pager.cache_size(), 3);
        // Touch page 1 again → cache {1, 2, 3} but 1 is most recent.
        let _ = pager.get(1).unwrap();
        // Touch page 4 → cache full, evict LRU (page 2).
        let _ = pager.get(4).unwrap();
        assert_eq!(pager.cache_size(), 3);
        // Page 2 should be a miss (was evicted).
        let prev_misses = pager.misses();
        let _ = pager.get(2).unwrap();
        assert!(pager.misses() > prev_misses);
    }

    #[test]
    fn out_of_bounds_page_errors() {
        let path = make_db("oob", DEFAULT_PAGE_SIZE, 4);
        let vfs = UnixVfs::new();
        let mut pager = Pager::open(&vfs, &path, DEFAULT_PAGE_SIZE, 100).unwrap();
        // Page 5 doesn't exist (db_size=4).
        let result = pager.get(5);
        assert!(result.is_err());
    }

    #[test]
    fn page_zero_errors() {
        let path = make_db("zero", DEFAULT_PAGE_SIZE, 1);
        let vfs = UnixVfs::new();
        let mut pager = Pager::open(&vfs, &path, DEFAULT_PAGE_SIZE, 100).unwrap();
        // Page 0 doesn't exist (pgno starts at 1).
        let result = pager.get(0);
        assert!(result.is_err());
    }

    #[test]
    fn read_file_header() {
        let path = make_db("header", DEFAULT_PAGE_SIZE, 2);
        let vfs = UnixVfs::new();
        let mut pager = Pager::open(&vfs, &path, DEFAULT_PAGE_SIZE, 100).unwrap();
        let mut header = [0u8; 16];
        pager.read_file_header(16, &mut header).unwrap();
        // First 16 bytes of page 1.
        for i in 0..16 {
            assert_eq!(header[i], (i % 256) as u8);
        }
    }

    #[test]
    fn thousand_random_accesses() {
        let path = make_db("1000", DEFAULT_PAGE_SIZE, 100);
        let vfs = UnixVfs::new();
        let mut pager = Pager::open(&vfs, &path, DEFAULT_PAGE_SIZE, 50).unwrap();
        // 1000 accesses to a 20-page working set (well within the
        // 50-page cache). After the first 20 misses, the rest are
        // hits.
        let mut accesses: u64 = 0;
        for i in 0..1000 {
            let pgno = (i % 20 + 1) as Pgno;
            let _ = pager.get(pgno).unwrap();
            accesses += 1;
        }
        assert_eq!(accesses, 1000);
        // We should have many hits and ~20 misses.
        assert!(pager.hits() >= 900, "got hits={}", pager.hits());
        assert!(pager.misses() <= 100, "got misses={}", pager.misses());
        assert_eq!(pager.hits() + pager.misses(), 1000);
    }
}
