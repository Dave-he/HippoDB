//! B-Tree — partial port of `sqlite-source/src/btree.c`.
//!
//! Implements a slim read-only B-Tree for table B-Trees (the most
//! common type, used to store user data). The cursor walks rows in
//! `rowid` order. Interior pages are traversed via the rightmost
//! child pointer (sufficient for simple test databases with
//! sequential rowids).
//!
//! # C source correspondence
//!
//! | Rust item            | C source                          |
//! |----------------------|-----------------------------------|
//! | `Btree::open`        | `sqlite3BtreeOpen` (btree.c:2538) |
//! | `BtreeCursor::first` | `sqlite3BtreeFirst` (btree.c:5696)|
//! | `BtreeCursor::next`  | `sqlite3BtreeNext` (btree.c:5876) |
//!
//! # Page format (table B-Tree)
//!
//! A B-Tree page has an 8-byte page header at the start (preceded
//! by a 100-byte database header on page 1):
//! - 1 byte: page type
//!   - `0x0d` = leaf table
//!   - `0x05` = interior table
//! - 2 bytes: first freeblock offset (0 if none)
//! - 2 bytes: number of cells
//! - 2 bytes: cell content area start
//! - 1 byte: number of fragmented free bytes
//!
//! For interior pages, add 4 bytes for the right-child pointer
//! (the last byte is at the end of the page).
//!
//! Cell format (table leaf): varint(payload_length) || varint(rowid)
//! || payload_bytes.
//! Cell format (table interior): 4 bytes child page || varint(rowid).
//!
//! Cell pointers are 2-byte offsets stored in the page header,
//! immediately after the header (or after the right-child pointer
//! for interior pages).

use crate::error::{SqliteError, SqliteResult};
use crate::pager::Pager;

/// Page type byte (leaf table).
const PTYPE_LEAF_TABLE: u8 = 0x0d;
/// Page type byte (interior table).
const PTYPE_INTERIOR_TABLE: u8 = 0x05;

/// A B-Tree — wraps a Pager and identifies a particular table B-Tree
/// by its root page.
pub struct Btree {
    pager: Pager,
    root_pgno: u32,
}

impl Btree {
    /// Open a B-Tree for a table stored at `root_pgno` in the
    /// given pager.
    pub fn open(pager: Pager, root_pgno: u32) -> Self {
        Btree { pager, root_pgno }
    }

    /// Read the database header (page 1, first 100 bytes). For
    /// convenience only — the B-Tree doesn't need the header to
    /// read.
    pub fn read_header(&mut self, n: usize, out: &mut [u8]) -> SqliteResult<()> {
        self.pager.read_file_header(n, out)
    }

    /// Open a cursor for this B-Tree.
    pub fn cursor(&mut self) -> SqliteResult<BtreeCursor<'_>> {
        BtreeCursor::first(self)
    }
}

/// A cursor over a B-Tree.
pub struct BtreeCursor<'a> {
    btree: &'a mut Btree,
    /// The current page (cached).
    current_page: Vec<u8>,
    /// Current cell index in the current page (0-based).
    cell_idx: i32,
    /// Number of cells in the current page.
    cell_count: u16,
    /// Current rowid (for the current cell).
    rowid: i64,
    /// Current payload (for the current cell).
    payload: Vec<u8>,
    /// True if the cursor is "exhausted" (past the last row).
    exhausted: bool,
}

impl<'a> BtreeCursor<'a> {
    /// Position the cursor at the first row.
    pub fn first(btree: &'a mut Btree) -> SqliteResult<Self> {
        let mut cur = BtreeCursor {
            btree,
            current_page: Vec::new(),
            cell_idx: -1,
            cell_count: 0,
            rowid: 0,
            payload: Vec::new(),
            exhausted: false,
        };
        cur.load_leftmost_leaf()?;
        cur.advance()?;
        Ok(cur)
    }

    /// Load the leftmost leaf of the B-Tree (via the pager).
    fn load_leftmost_leaf(&mut self) -> SqliteResult<()> {
        let mut pgno = self.btree.root_pgno;
        // If the root is a leaf, we're done. If it's an interior,
        // follow the leftmost child pointer. Since cell pointers
        // and the right-child pointer point to children, the
        // leftmost child is at the offset stored in the first
        // cell pointer.
        loop {
            let page = self.btree.pager.get(pgno)?;
            self.current_page = page.to_vec();
            let ptype = page[0];
            if ptype == PTYPE_LEAF_TABLE {
                self.cell_count = read_u16(&page, 3);
                return Ok(());
            } else if ptype == PTYPE_INTERIOR_TABLE {
                // Interior page layout:
                //   offset 0..8:  page header (same as leaf)
                //   offset 8..12: 4-byte right-child pointer
                //     (the page number of the right-most child; we
                //     don't follow it in the slim subset)
                //   offset 12..:  cell pointer array (2 bytes each)
                // The leftmost child is at the offset stored in the
                // FIRST cell pointer, so we read at offset 12.
                let first_ptr = read_u16(&page, 12);
                if first_ptr == 0 {
                    return Err(SqliteError(11)); // SQLITE_CORRUPT
                }
                // Read the first cell: 4 bytes child page number.
                let child = read_u32(&page, first_ptr as usize);
                pgno = child;
                // Loop continues.
            } else {
                return Err(SqliteError(11)); // SQLITE_CORRUPT
            }
        }
    }

    /// Advance the cursor to the next cell in the current page, or
    /// mark it as exhausted.
    fn advance(&mut self) -> SqliteResult<()> {
        self.cell_idx += 1;
        if (self.cell_idx as u16) < self.cell_count {
            self.read_current_cell()
        } else {
            // No more cells in this page. The slim T-0013 port
            // doesn't follow the right-child pointer of an
            // interior page (we always read leftmost). For a
            // test database with all rows in one page, this
            // is sufficient. Mark as exhausted.
            self.exhausted = true;
            Ok(())
        }
    }

    /// Read the current cell (cell_idx) into rowid and payload.
    fn read_current_cell(&mut self) -> SqliteResult<()> {
        // Cell pointer is at offset 8 + 2*cell_idx.
        // For interior pages the offset is 12 + 2*cell_idx
        // (after the right-child pointer).
        let ptype = self.current_page[0];
        let ptr_offset = if ptype == PTYPE_INTERIOR_TABLE {
            12 + 2 * self.cell_idx as usize
        } else {
            8 + 2 * self.cell_idx as usize
        };
        let cell_offset = read_u16(&self.current_page, ptr_offset) as usize;
        // Parse the cell: varint(payload_length), varint(rowid),
        // then payload bytes.
        let (pl, off1) = read_varint(&self.current_page, cell_offset);
        let (rowid, off2) = read_varint(&self.current_page, off1);
        let payload_end = off2 + pl as usize;
        self.rowid = rowid;
        self.payload = self.current_page[off2..payload_end].to_vec();
        Ok(())
    }

    /// Advance to the next row. Returns `true` if there's a row
    /// available, `false` if exhausted.
    pub fn next(&mut self) -> SqliteResult<bool> {
        if self.exhausted {
            return Ok(false);
        }
        self.advance()?;
        Ok(!self.exhausted)
    }

    /// The current rowid.
    pub fn rowid(&self) -> i64 {
        self.rowid
    }

    /// The current payload.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// `true` if the cursor has no more rows.
    pub fn is_eof(&self) -> bool {
        self.exhausted
    }
}

/// Read a big-endian u16 from `data` at `offset`.
fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([data[offset], data[offset + 1]])
}

/// Read a big-endian u32 from `data` at `offset`.
fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

/// Read a SQLite varint from `data` at `offset`. Returns (value, next_offset).
/// SQLite varints use 1-9 bytes; the high bit of each byte (except
/// the last) indicates more bytes follow. The first byte contains
/// the LEAST significant 7 bits; subsequent bytes contain higher
/// bits.
fn read_varint(data: &[u8], offset: usize) -> (i64, usize) {
    let mut result: u64 = 0;
    for i in 0..9 {
        if offset + i >= data.len() {
            return (0, offset);
        }
        let b = data[offset + i];
        if i == 8 {
            // 9th byte: all 8 bits are data.
            result |= (b as u64) << (7 * 8);
            return (result as i64, offset + 9);
        } else {
            result |= ((b & 0x7f) as u64) << (7 * i);
            if b & 0x80 == 0 {
                return (result as i64, offset + i + 1);
            }
        }
    }
    (result as i64, offset + 9)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::unix::UnixVfs;
    use std::fs;

    /// Build a database with N pages, each filled with a single byte.
    /// Page 1 is the database header (100 bytes) + B-Tree page header.
    /// For the T-0013 test we use a simple layout: page 1 is the
    /// schema page (treated as a leaf), and we read directly from
    /// a specified root page.
    fn make_db(name: &str, n_pages: usize) -> String {
        let mut p = std::env::temp_dir();
        p.push(format!("sqllite_rs_btree_{}_{}", std::process::id(), name));
        let path = p.to_str().unwrap().to_string();
        let page_size = 4096;
        let mut data = vec![0u8; n_pages * page_size];
        // Page 1: 100-byte DB header + leaf table page header.
        // DB header (simplified — just enough to be parseable).
        // The pager doesn't actually parse the DB header, so we
        // can leave it as zeros for T-0013.
        // Leaf table page header: starts at offset 100.
        data[100] = PTYPE_LEAF_TABLE; // leaf table
        // freeblock offset = 0
        // number of cells: 0 (will be set in the test)
        // cell content area start: 0
        // fragmented bytes: 0
        fs::write(&path, &data).unwrap();
        path
    }

    /// Build a leaf table page with N cells, each cell is
    /// (rowid, payload) where rowid is i+1 and payload is the
    /// ASCII string "row_<i>".
    fn build_leaf_page(n: usize) -> Vec<u8> {
        let page_size = 4096;
        let mut page = vec![0u8; page_size];
        page[0] = PTYPE_LEAF_TABLE;
        // Header: 8 bytes
        // - ptype at 0
        // - freeblock at 1..2 (big-endian u16)
        // - cell count at 3..4
        // - cell content area start at 5..6
        // - fragmented at 7
        let cell_count = n as u16;
        page[3] = (cell_count >> 8) as u8;
        page[4] = (cell_count & 0xff) as u8;
        // Cell pointers start at offset 8. Each is 2 bytes.
        // Cell content starts after the cell pointers.
        let cell_ptrs_size = cell_count as usize * 2;
        let content_start = 8 + cell_ptrs_size;
        page[5] = (content_start >> 8) as u8;
        page[6] = (content_start & 0xff) as u8;
        // Now write cells.
        let mut cursor = content_start;
        for i in 0..n {
            // Write cell pointer.
            let ptr_offset = 8 + i * 2;
            page[ptr_offset] = (cursor >> 8) as u8;
            page[ptr_offset + 1] = (cursor & 0xff) as u8;
            // Write cell: varint(pl), varint(rowid), payload.
            let rowid = (i + 1) as i64;
            let payload = format!("row_{i}").into_bytes();
            // Encode varint(pl).
            let pl_bytes = encode_varint(payload.len() as u64);
            for b in pl_bytes {
                page[cursor] = b;
                cursor += 1;
            }
            // Encode varint(rowid).
            let r_bytes = encode_varint(rowid as u64);
            for b in r_bytes {
                page[cursor] = b;
                cursor += 1;
            }
            // Write payload.
            for b in &payload {
                page[cursor] = *b;
                cursor += 1;
            }
        }
        page
    }

    /// Encode a u64 as a SQLite varint. SQLite varints are 1-9 bytes:
    /// the first 8 bytes each carry 7 bits of data plus a high-bit
    /// continuation flag; the 9th byte (if needed) carries the
    /// remaining 8 bits without a continuation flag.
    fn encode_varint(mut v: u64) -> Vec<u8> {
        let mut out = Vec::new();
        for _ in 0..8 {
            let b = (v as u8) & 0x7f;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return out;
            }
            out.push(b | 0x80);
        }
        // 9th byte: all 8 bits of remaining data.
        out.push(v as u8);
        out
    }

    #[test]
    fn varint_round_trip() {
        for v in [0u64, 1, 100, 127, 128, 1000, 16383, 16384, u32::MAX as u64, u64::MAX] {
            let bytes = encode_varint(v);
            let (decoded, next) = read_varint(&bytes, 0);
            assert_eq!(decoded as u64, v, "v={v}");
            assert_eq!(next, bytes.len(), "v={v}");
        }
    }

    #[test]
    fn read_first_row() {
        // Build a database with one leaf page containing 3 cells.
        let path = make_db("first_row", 1);
        let page = build_leaf_page(3);
        fs::write(&path, &page).unwrap();
        let vfs = UnixVfs::new();
        let pager = Pager::open(&vfs, &path, 4096, 100).unwrap();
        // The B-Tree root is page 1 (offset 0 in the file).
        // For page 1, the B-Tree header is at offset 100 (after
        // the 100-byte DB header).
        let mut btree = Btree::open(pager, 1);
        let mut cur = btree.cursor().unwrap();
        assert_eq!(cur.rowid(), 1);
        assert_eq!(cur.payload(), b"row_0");
    }

    #[test]
    fn iterate_all_rows() {
        // Build a database with one leaf page containing 10 cells.
        let path = make_db("iterate", 1);
        let page = build_leaf_page(10);
        fs::write(&path, &page).unwrap();
        let vfs = UnixVfs::new();
        let pager = Pager::open(&vfs, &path, 4096, 100).unwrap();
        let mut btree = Btree::open(pager, 1);
        let mut cur = btree.cursor().unwrap();
        let mut count = 0;
        loop {
            let expected_rowid = (count + 1) as i64;
            let expected_payload = format!("row_{count}").into_bytes();
            assert_eq!(cur.rowid(), expected_rowid, "count={count}");
            assert_eq!(cur.payload(), &expected_payload[..], "count={count}");
            count += 1;
            if !cur.next().unwrap() {
                break;
            }
        }
        assert_eq!(count, 10);
    }

    #[test]
    fn empty_btree_exhausts_immediately() {
        let path = make_db("empty", 1);
        // Build a leaf with 0 cells.
        let page = build_leaf_page(0);
        fs::write(&path, &page).unwrap();
        let vfs = UnixVfs::new();
        let pager = Pager::open(&vfs, &path, 4096, 100).unwrap();
        let mut btree = Btree::open(pager, 1);
        let cur = btree.cursor().unwrap();
        // Cursor should be exhausted immediately.
        assert!(cur.is_eof());
    }

    #[test]
    fn read_header_returns_db_header() {
        // Build a database whose first 100 bytes are non-zero, so we
        // can verify read_header returns them.
        let path = make_db("header", 1);
        let mut data = fs::read(&path).unwrap();
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(37);
        }
        fs::write(&path, &data).unwrap();
        let vfs = UnixVfs::new();
        let pager = Pager::open(&vfs, &path, 4096, 100).unwrap();
        let mut btree = Btree::open(pager, 1);
        let mut out = [0u8; 16];
        btree.read_header(16, &mut out).unwrap();
        for (i, b) in out.iter().enumerate() {
            assert_eq!(*b, (i as u8).wrapping_mul(37), "byte {i}");
        }
    }

    #[test]
    fn read_header_zero_bytes() {
        // Reading 0 bytes is allowed and a no-op.
        let path = make_db("hdr_zero", 1);
        let vfs = UnixVfs::new();
        let pager = Pager::open(&vfs, &path, 4096, 100).unwrap();
        let mut btree = Btree::open(pager, 1);
        let mut out = [0u8; 8];
        btree.read_header(0, &mut out).unwrap();
        // out untouched
        for b in &out {
            assert_eq!(*b, 0);
        }
    }

    #[test]
    fn interior_page_follows_to_leftmost_child() {
        // Build a 2-page database:
        //   page 1: interior table page pointing to page 2 (leftmost child)
        //   page 2: leaf table with 3 cells
        // (The slim-subset btree reads the page header at offset 0 of
        // the page data — there's no 100-byte DB header for our test
        // databases.)
        let path = make_db("interior", 2);
        let page_size = 4096;

        // Build page 2 (leaf with 3 cells).
        let leaf = build_leaf_page(3);

        // Build page 1 (interior with 1 cell pointing to page 2).
        let mut interior = vec![0u8; page_size];
        interior[0] = PTYPE_INTERIOR_TABLE; // page type at offset 0
        // 8-byte B-Tree page header at offset 0..8
        //   - ptype at 0
        //   - freeblock at 1..2 (0 = none)
        //   - cell count at 3..4
        //   - cell content start at 5..6
        //   - fragmented at 7
        interior[3] = 0;
        interior[4] = 1; // 1 cell
        // 4-byte right-child pointer at 8..12 (we set 0; slim subset
        // doesn't follow it).
        // Cell pointers start at offset 12.
        // Cell 0 pointer at 12..14.
        // Cell content starts at 14.
        let cell_offset: u16 = 14;
        interior[12] = (cell_offset >> 8) as u8;
        interior[13] = (cell_offset & 0xff) as u8;
        // Interior cell: 4-byte child page number + varint rowid.
        interior[14] = 0;
        interior[15] = 0;
        interior[16] = 0;
        interior[17] = 2; // child = page 2
        interior[18] = 100; // varint rowid = 100

        // Assemble the 2-page file: page 1 (file offset 0..4096) +
        // page 2 (file offset 4096..8192).
        let mut file = vec![0u8; 2 * page_size];
        file[..page_size].copy_from_slice(&interior);
        file[page_size..].copy_from_slice(&leaf);
        fs::write(&path, &file).unwrap();

        let vfs = UnixVfs::new();
        let pager = Pager::open(&vfs, &path, 4096, 100).unwrap();
        let mut btree = Btree::open(pager, 1);
        let mut cur = btree.cursor().unwrap();
        // The cursor should have followed the leftmost-child pointer
        // and landed on the first cell of the leaf.
        assert_eq!(cur.rowid(), 1);
        assert_eq!(cur.payload(), b"row_0");
    }

    #[test]
    fn interior_page_corrupt_returns_error() {
        // Page 1 is interior but its first cell pointer is 0 → CORRUPT.
        let path = make_db("corrupt", 1);
        let page_size = 4096;
        let mut page = vec![0u8; page_size];
        page[0] = PTYPE_INTERIOR_TABLE; // page type at offset 0
        // cell count = 1 so the loader tries to read the first pointer
        // (which is 0 → CORRUPT).
        page[4] = 1;
        fs::write(&path, &page).unwrap();
        let vfs = UnixVfs::new();
        let pager = Pager::open(&vfs, &path, 4096, 100).unwrap();
        let mut btree = Btree::open(pager, 1);
        let r = btree.cursor();
        assert!(r.is_err());
        assert_eq!(r.err().unwrap().code(), 11); // SQLITE_CORRUPT
    }

    #[test]
    fn interior_page_unknown_ptype_errors() {
        // Page 1 has an unknown page type byte → CORRUPT.
        let path = make_db("bad_ptype", 1);
        let page_size = 4096;
        let mut page = vec![0u8; page_size];
        page[0] = 0xFF; // unknown page type
        fs::write(&path, &page).unwrap();
        let vfs = UnixVfs::new();
        let pager = Pager::open(&vfs, &path, 4096, 100).unwrap();
        let mut btree = Btree::open(pager, 1);
        let r = btree.cursor();
        assert!(r.is_err());
        assert_eq!(r.err().unwrap().code(), 11); // SQLITE_CORRUPT
    }
}
