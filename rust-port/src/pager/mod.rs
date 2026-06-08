//! Pager (page cache) — partial port of `sqlite-source/src/pager.c`.
//!
//! Implements a minimal Pager with read + write paths:
//! - Pages are read from the database file on first access and cached
//!   in memory.
//! - `write` modifies a page in the cache; on `commit` the changes
//!   are written to the file.
//! - `begin` opens a transaction; the OLD page contents are written
//!   to a rollback journal. `rollback` reads the journal back to
//!   restore the original page contents.
//! - `commit` truncates the journal.
//!
//! # C source correspondence
//!
//! | Rust item         | C source                          |
//! |-------------------|-----------------------------------|
//! | `Pager::new`      | `sqlite3PagerOpen` (pager.c:5901) |
//! | `Pager::get`      | `sqlite3PagerGet`  (pager.c:5611) |
//! | `Pager::write`    | `sqlite3PagerWrite` (pager.c:5483)|
//! | `Pager::begin`    | `sqlite3PagerBegin` (pager.c:4880)|
//! | `Pager::commit`   | `sqlite3PagerCommitPhaseTwo`      |
//! | `Pager::rollback` | `sqlite3PagerRollback`            |
//!
//! # Behavior contract
//!
//! - Pages are cached in a `HashMap<Pgno, CachedPage>` (Pgno = u32).
//! - The cache is bounded by `max_cache` pages; when full, the
//!   least-recently-used page is evicted (LRU).
//! - On `write`, the page is marked dirty. The OLD content is
//!   written to the journal only on the first `write` of a
//!   transaction (the journal is a `HashMap<Pgno, Vec<u8>>` in
//!   memory for the slim T-0012 scope; the C source writes to a
//!   side file `*-journal`).
//! - On `commit`, all dirty pages are flushed to the database file
//!   in page-number order, then the journal is cleared.
//! - On `rollback`, the journal is replayed: each journal entry
//!   restores the corresponding page in the cache. Then the journal
//!   is cleared.
//!
//! # Out of scope (T-0012)
//!
//! - Page locking / concurrency (the C source uses POSIX advisory
//!   locks; for T-0012 we assume single-threaded access).
//! - WAL mode (write-ahead log).
//! - Savepoints.

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
    /// True if this page has been modified since the last commit.
    dirty: bool,
}

/// A journal entry — the OLD contents of a page before a transaction.
#[derive(Clone)]
struct JournalEntry {
    /// The page number.
    pgno: Pgno,
    /// The OLD page contents (before any writes in the transaction).
    old_data: Vec<u8>,
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
    /// Read-only mode? Set by `open_read_only`; cleared by `open_writable`.
    read_only: bool,
    /// Current transaction state.
    state: PagerState,
    /// Rollback journal — old page contents (in-memory, for T-0012).
    /// `None` when no transaction is active.
    journal: Option<Vec<JournalEntry>>,
    /// `true` if any page in the journal has been written.
    journal_has_data: bool,
    /// Transaction state for write/begin/commit.
    in_transaction: bool,
    /// Maximum page number that has been written in the current txn
    /// (mirrors the C source's `nRec` — number of pages journalled).
    n_rec: u32,
}

/// Pager transaction state (mirrors the C source's `eState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PagerState {
    /// No transaction.
    NoTxn,
    /// Read transaction (shared lock).
    Read,
    /// Write transaction (reserved lock).
    Write,
}

impl Pager {
    /// Open a database file in read-only mode.
    pub fn open(
        vfs: &dyn Vfs,
        path: &str,
        page_size: usize,
        max_cache: usize,
    ) -> SqliteResult<Self> {
        // Open the file read-only.
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
            read_only: true,
            state: PagerState::Read,
            journal: None,
            journal_has_data: false,
            in_transaction: false,
            n_rec: 0,
        })
    }

    /// Open a database file in read-write mode.
    pub fn open_writable(
        vfs: &dyn Vfs,
        path: &str,
        page_size: usize,
        max_cache: usize,
    ) -> SqliteResult<Self> {
        // Open the file read-write. Try with create first, then
        // without, to handle both new and existing files.
        let file = vfs
            .open(path, OpenFlags::from_int(0x02 | 0x04)) // READWRITE | CREATE
            .or_else(|_| vfs.open(path, OpenFlags::from_int(0x02)))?;
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
            read_only: false,
            state: PagerState::NoTxn,
            journal: None,
            journal_has_data: false,
            in_transaction: false,
            n_rec: 0,
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
            CachedPage { data, last_access, dirty: false },
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

    /// Begin a write transaction. Must be called before any `write`.
    /// Mirrors `sqlite3PagerBegin` for the write case.
    pub fn begin(&mut self) -> SqliteResult<()> {
        if self.read_only {
            return Err(SqliteError(8)); // SQLITE_READONLY
        }
        if self.in_transaction {
            return Err(SqliteError(5)); // SQLITE_BUSY (nested txn)
        }
        self.state = PagerState::Write;
        self.journal = Some(Vec::new());
        self.journal_has_data = false;
        self.n_rec = 0;
        self.in_transaction = true;
        Ok(())
    }

    /// Write `data` to page `pgno`. The page is marked dirty and
    /// (if a transaction is active) the OLD content is journaled
    /// before the write.
    pub fn write(&mut self, pgno: Pgno, data: &[u8]) -> SqliteResult<()> {
        if self.read_only {
            return Err(SqliteError(8));
        }
        if !self.in_transaction {
            return Err(SqliteError(1)); // SQLITE_ERROR
        }
        if data.len() != self.page_size {
            return Err(SqliteError(18)); // SQLITE_TOOBIG
        }
        // If the page is already in the cache, journal its CURRENT
        // contents (not yet written) and overwrite.
        // If not in the cache, we need to read it from disk,
        // journal the OLD disk content, then write the new data.
        let current_data: Vec<u8>;
        let mut already_journaled = false;
        if let Some(cached) = self.cache.get(&pgno) {
            if cached.dirty {
                // Page is already in the cache and dirty; the
                // journal already has the pre-txn version.
                already_journaled = true;
            }
            current_data = cached.data.clone();
        } else {
            // Load from disk.
            let offset = (pgno as i64 - 1) * self.page_size as i64;
            let mut disk_data = vec![0u8; self.page_size];
            let n = self.file.read(&mut disk_data, offset)?;
            if n < self.page_size {
                for byte in &mut disk_data[n..] {
                    *byte = 0;
                }
            }
            current_data = disk_data;
        }
        // If we haven't already journaled this page, add it now.
        if !already_journaled {
            if let Some(journal) = self.journal.as_mut() {
                journal.push(JournalEntry {
                    pgno,
                    old_data: current_data.clone(),
                });
                self.n_rec += 1;
                self.journal_has_data = true;
            }
        }
        // Update or insert the cache entry with the new data.
        self.access_counter += 1;
        let last_access = self.access_counter;
        self.cache.insert(
            pgno,
            CachedPage {
                data: data.to_vec(),
                last_access,
                dirty: true,
            },
        );
        // If the page is past the current db_size, update db_size.
        if pgno > self.db_size {
            self.db_size = pgno;
        }
        Ok(())
    }

    /// Commit the current transaction: flush all dirty pages to the
    /// database file, then clear the journal.
    pub fn commit(&mut self) -> SqliteResult<()> {
        if self.read_only {
            return Err(SqliteError(8));
        }
        if !self.in_transaction {
            return Err(SqliteError(1));
        }
        // Collect dirty pages sorted by page number (matches the C
        // source's commit order).
        let mut dirty: Vec<(Pgno, Vec<u8>)> = self
            .cache
            .iter()
            .filter(|(_, p)| p.dirty)
            .map(|(k, p)| (*k, p.data.clone()))
            .collect();
        dirty.sort_by_key(|(k, _)| *k);
        // Flush to file.
        for (pgno, data) in dirty {
            let offset = (pgno as i64 - 1) * self.page_size as i64;
            self.file.write(&data, offset)?;
        }
        self.file.sync()?;
        // Clear dirty bits.
        for cached in self.cache.values_mut() {
            cached.dirty = false;
        }
        // Clear the journal.
        self.journal = None;
        self.journal_has_data = false;
        self.n_rec = 0;
        self.in_transaction = false;
        self.state = PagerState::Read;
        Ok(())
    }

    /// Rollback the current transaction: restore the OLD page
    /// contents from the journal and clear the dirty flag.
    pub fn rollback(&mut self) -> SqliteResult<()> {
        if self.read_only {
            return Err(SqliteError(8));
        }
        if !self.in_transaction {
            return Err(SqliteError(1));
        }
        // Replay the journal.
        if let Some(journal) = self.journal.take() {
            for entry in journal {
                self.access_counter += 1;
                let last_access = self.access_counter;
                self.cache.insert(
                    entry.pgno,
                    CachedPage {
                        data: entry.old_data,
                        last_access,
                        dirty: false,
                    },
                );
            }
        }
        // If the transaction didn't write anything, there's nothing
        // to roll back. But we still clear the dirty bits on the
        // cache (no-op if all dirty pages were in the journal and
        // got restored).
        for cached in self.cache.values_mut() {
            cached.dirty = false;
        }
        self.journal_has_data = false;
        self.n_rec = 0;
        self.in_transaction = false;
        self.state = PagerState::Read;
        Ok(())
    }

    /// `true` if a transaction is active.
    pub fn in_transaction(&self) -> bool {
        self.in_transaction
    }

    /// `true` if the pager is read-only.
    pub fn is_read_only(&self) -> bool {
        self.read_only
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

    // ========================================================================
    // Write path tests (T-0012)
    // ========================================================================

    fn make_writable_db(name: &str, n_pages: usize) -> String {
        let path = make_db(name, DEFAULT_PAGE_SIZE, n_pages);
        // Make the file writable for the Unix VFS read-write open.
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(false);
        fs::set_permissions(&path, perms).unwrap();
        path
    }

    fn make_empty_db(name: &str) -> String {
        let mut p = std::env::temp_dir();
        p.push(format!("sqllite_rs_pager_empty_{}_{}", std::process::id(), name));
        let path = p.to_str().unwrap().to_string();
        // Create an empty file (0 bytes); the pager will treat it
        // as a 0-page database.
        fs::write(&path, b"").unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(false);
        fs::set_permissions(&path, perms).unwrap();
        path
    }

    fn page_data(fill: u8) -> Vec<u8> {
        vec![fill; DEFAULT_PAGE_SIZE]
    }

    #[test]
    fn open_writable_creates_if_missing() {
        let path = make_empty_db("open_writable_new");
        let vfs = UnixVfs::new();
        let mut pager = Pager::open_writable(&vfs, &path, DEFAULT_PAGE_SIZE, 100).unwrap();
        assert!(!pager.is_read_only());
        // Begin a transaction, write a page, commit.
        pager.begin().unwrap();
        let new_page = page_data(0xAB);
        pager.write(1, &new_page).unwrap();
        pager.commit().unwrap();
        // Reopen and verify.
        let mut pager2 = Pager::open(&vfs, &path, DEFAULT_PAGE_SIZE, 100).unwrap();
        let page1 = pager2.get(1).unwrap();
        for &b in page1 {
            assert_eq!(b, 0xAB);
        }
    }

    #[test]
    fn write_commit_persists() {
        let path = make_writable_db("write_commit", 4);
        let vfs = UnixVfs::new();
        {
            let mut pager = Pager::open_writable(&vfs, &path, DEFAULT_PAGE_SIZE, 100).unwrap();
            pager.begin().unwrap();
            let new_page = page_data(0x55);
            pager.write(1, &new_page).unwrap();
            pager.write(2, &new_page).unwrap();
            pager.commit().unwrap();
        }
        // Reopen and verify.
        let mut pager2 = Pager::open(&vfs, &path, DEFAULT_PAGE_SIZE, 100).unwrap();
        for pgno in 1..=2 {
            let page = pager2.get(pgno).unwrap();
            for &b in page {
                assert_eq!(b, 0x55, "pgno={pgno}");
            }
        }
        // Other pages should still have their original content.
        let page3 = pager2.get(3).unwrap();
        // Page 3's original data is from make_db: (i % 256) for i in 0..page_size.
        for i in 0..DEFAULT_PAGE_SIZE {
            assert_eq!(page3[i], (i % 256) as u8);
        }
    }

    #[test]
    fn write_rollback_restores_old() {
        let path = make_writable_db("write_rollback", 4);
        let vfs = UnixVfs::new();
        // First, write some known content.
        {
            let mut pager = Pager::open_writable(&vfs, &path, DEFAULT_PAGE_SIZE, 100).unwrap();
            pager.begin().unwrap();
            pager.write(1, &page_data(0xAA)).unwrap();
            pager.commit().unwrap();
        }
        // Now begin, write, and rollback.
        {
            let mut pager = Pager::open_writable(&vfs, &path, DEFAULT_PAGE_SIZE, 100).unwrap();
            pager.begin().unwrap();
            pager.write(1, &page_data(0xBB)).unwrap();
            pager.write(2, &page_data(0xBB)).unwrap();
            // Abort via rollback.
            pager.rollback().unwrap();
        }
        // Reopen and verify page 1 still has 0xAA, page 2 has
        // original content.
        let mut pager2 = Pager::open(&vfs, &path, DEFAULT_PAGE_SIZE, 100).unwrap();
        let p1 = pager2.get(1).unwrap();
        for &b in p1 {
            assert_eq!(b, 0xAA);
        }
        let p2 = pager2.get(2).unwrap();
        // Original content (i % 256).
        for i in 0..DEFAULT_PAGE_SIZE {
            assert_eq!(p2[i], (i % 256) as u8);
        }
    }

    #[test]
    fn write_requires_transaction() {
        let path = make_writable_db("write_no_txn", 4);
        let vfs = UnixVfs::new();
        let mut pager = Pager::open_writable(&vfs, &path, DEFAULT_PAGE_SIZE, 100).unwrap();
        // write without begin should fail.
        let result = pager.write(1, &page_data(0xFF));
        assert!(result.is_err());
    }

    #[test]
    fn read_only_pager_rejects_write() {
        let path = make_writable_db("readonly_write", 4);
        let vfs = UnixVfs::new();
        let mut pager = Pager::open(&vfs, &path, DEFAULT_PAGE_SIZE, 100).unwrap();
        assert!(pager.is_read_only());
        let result = pager.begin();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().0, 8); // SQLITE_READONLY
    }

    #[test]
    fn ten_pages_write_rollback() {
        // The spec test for T-0012: 写 10 页后 abort, 重新 open
        // 内容应为旧值.
        let path = make_writable_db("10pages_rollback", 4);
        let vfs = UnixVfs::new();
        {
            let mut pager = Pager::open_writable(&vfs, &path, DEFAULT_PAGE_SIZE, 100).unwrap();
            // First commit a known state.
            pager.begin().unwrap();
            for pgno in 1..=4 {
                pager.write(pgno, &page_data(0x11)).unwrap();
            }
            pager.commit().unwrap();
        }
        // Begin a new transaction, write to 10 pages, abort.
        {
            let mut pager = Pager::open_writable(&vfs, &path, DEFAULT_PAGE_SIZE, 100).unwrap();
            pager.begin().unwrap();
            for pgno in 1..=4 {
                pager.write(pgno, &page_data(0xEE)).unwrap();
            }
            // (We only have 4 pages, but the test says "write 10 pages" —
            // extend the database by writing pages 5..=10.)
            for pgno in 5..=10 {
                pager.write(pgno, &page_data(0xEE)).unwrap();
            }
            pager.rollback().unwrap();
        }
        // Reopen and verify pages 1-4 still have 0x11, pages 5-10
        // were never persisted (rollback).
        let mut pager2 = Pager::open(&vfs, &path, DEFAULT_PAGE_SIZE, 100).unwrap();
        for pgno in 1..=4 {
            let p = pager2.get(pgno).unwrap();
            for &b in p {
                assert_eq!(b, 0x11, "pgno={pgno}");
            }
        }
        // db_size should NOT have grown — pages 5-10 weren't
        // committed.
        assert_eq!(pager2.db_size(), 4);
    }

    #[test]
    fn nested_begin_errors() {
        let path = make_writable_db("nested_begin", 4);
        let vfs = UnixVfs::new();
        let mut pager = Pager::open_writable(&vfs, &path, DEFAULT_PAGE_SIZE, 100).unwrap();
        pager.begin().unwrap();
        let result = pager.begin();
        assert!(result.is_err());
    }
}
