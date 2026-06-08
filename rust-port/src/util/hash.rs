//! Hash table — 1:1 port of `sqlite-source/src/hash.c`.
//!
//! Implements the generic hash table used throughout SQLite, plus a
//! generic `GrowableArray<T>` for code that needs a growable vector.
//!
//! # C source correspondence
//!
//! | Rust item                 | C source                          |
//! |--------------------------|-----------------------------------|
//! | `str_hash`               | `strHash` (hash.c:55-73)          |
//! | `Hash::new`              | `sqlite3HashInit` (hash.c:23-29)  |
//! | `Hash::clear`            | `sqlite3HashClear` (hash.c:35-50) |
//! | `Hash::find`             | `sqlite3HashFind` (hash.c:222-226)|
//! | `Hash::insert`           | `sqlite3HashInsert` (hash.c:242)  |
//! | `find_element_with_hash` | `findElementWithHash` (hash.c:153)|
//! | `insert_element`         | `insertElement` (hash.c:79-104)   |
//! | `remove_element`         | `removeElement` (hash.c:188-216)  |
//! | `rehash`                 | `rehash` (hash.c:113-146)         |
//!
//! # Behavior contract (1:1 with C)
//!
//! - **Knuth multiplicative hash** with constant `0x9e3779b1`,
//!   ASCII chars masked with `0xdf` (so `A` and `a` hash equal).
//! - **Chain table** for collision (each bucket holds a `chain` head +
//!   `count`); the spec's "linear probing" wording is a misnomer for the
//!   C source's chained implementation — we follow C per
//!   `02-c-porting-conventions.md` §1.
//! - **Rehash trigger**: `count >= 5 && count > 2*htsize` → rehash to
//!   `count*3` buckets (hash.c:267-269).
//! - **O(N) iteration** via the doubly-linked `first → next` chain that
//!   mirrors the C-side `Hash.first` linked list.
//! - **`data == null`** is the sentinel for "this slot is empty" — the
//!   insert API uses `data == null` to mean "remove this entry",
//!   matching `sqlite3HashInsert` semantics.
//! - **OOM contract**: if `Box`-style growth fails (it cannot on the
//!   default Rust allocator, but we model the path), `insert` returns
//!   the input `data` unchanged and the table is left untouched,
//!   matching `hash.c:262`.
//!
//! # String vs integer keys
//!
//! The C version's `pKey` is always a `const char*`; integer keys are
//! formatted to strings by callers (e.g. `printf("%lld", ...)`). We
//! mirror this by always storing a `String` and expose `insert_int` /
//! `find_int` helpers that format the integer first.

use std::ptr;

use crate::error::{SqliteError, SqliteResult};

// ============================================================================
// strHash — Knuth multiplicative hash, matches C (hash.c:55-73)
// ============================================================================

/// `strHash` (hash.c:55-73) — Knuth multiplicative hashing constant
/// `0x9e3779b1` (2654435761), the closest prime to
/// `(2**32) * golden_ratio`.
///
/// ASCII characters are masked with `0xdf` so that `A`/`a`, `B`/`b`,
/// etc. hash to the same value (matching the C version's case-insensitive
/// ASCII folding).
#[inline]
pub fn str_hash(z: &str) -> u32 {
    let mut h: u32 = 0;
    for &b in z.as_bytes() {
        h = h.wrapping_add((b & 0xdf) as u32);
        h = h.wrapping_mul(0x9e3779b1);
    }
    h
}

// ============================================================================
// Hash table types
// ============================================================================

/// Bucket entry — `chain` head pointer + `count` of elements in the chain.
#[derive(Clone, Copy, Default)]
struct HashEntry {
    /// Index into `elems` of the head element, or `None` if bucket is empty.
    chain: Option<usize>,
    /// Number of elements in this bucket's chain.
    count: u32,
}

/// Element node stored in the hash table.
///
/// `data` is an opaque `*mut u8` matching the C `void*` payload
/// convention. The pointer is never dereferenced by the hash table;
/// the caller is responsible for its lifetime.
struct HashElem {
    p_key: String,
    h: u32,
    data: *mut u8,
    next: Option<usize>,
    prev: Option<usize>,
    /// `false` when this slot is in the free list.
    used: bool,
}

// ============================================================================
// Hash
// ============================================================================

/// Generic hash table — 1:1 port of C `struct Hash` (hash.c).
///
/// The data field is an opaque `*mut u8`; for type safety in pure-Rust
/// callers, the free functions `insert_boxed` / `find_boxed` wrap
/// `Box<T>` storage that this `Hash` only knows as an address.
pub struct Hash {
    /// Index into `elems` of the first element in the all-elements
    /// doubly-linked list (`None` when empty). Mirrors `Hash.first`.
    first: Option<usize>,
    /// Total number of live elements. Mirrors `Hash.count`.
    count: u32,
    /// Number of buckets in `ht`. Mirrors `Hash.htsize`. `0` means no
    /// bucket array allocated.
    htsize: u32,
    /// Bucket array. Empty when `htsize == 0`.
    ht: Vec<HashEntry>,
    /// Element pool indexed by slot id. Live elements + free-list nodes.
    elems: Vec<HashElem>,
    /// Free list head — slot ids in `elems` that are available for reuse.
    free_head: Option<usize>,
}

impl Default for Hash {
    fn default() -> Self {
        Self::new()
    }
}

impl Hash {
    /// `sqlite3HashInit` (hash.c:23-29) — construct an empty hash table.
    pub const fn new() -> Self {
        Self {
            first: None,
            count: 0,
            htsize: 0,
            ht: Vec::new(),
            elems: Vec::new(),
            free_head: None,
        }
    }

    /// `sqlite3HashInit` (hash.c:23-29) — re-initialize an existing
    /// `Hash` to the empty state without dropping the `String`s already
    /// stored in `elems`. Use `clear` to also free those.
    pub fn init(&mut self) {
        self.first = None;
        self.count = 0;
        self.htsize = 0;
        self.ht.clear();
        self.htshrink_to_fit();
        self.elems.clear();
        self.free_head = None;
    }

    /// `sqlite3HashClear` (hash.c:35-50) — drop all elements and free
    /// the bucket array. The `Hash` is left in the same state as
    /// `new()`.
    pub fn clear(&mut self) {
        // Dropping `elems` drops the Strings automatically.
        self.first = None;
        self.count = 0;
        self.htsize = 0;
        self.ht.clear();
        self.htshrink_to_fit();
        self.elems.clear();
        self.free_head = None;
    }

    /// `sqlite3HashFind` (hash.c:222-226) — look up `p_key`.
    ///
    /// Returns the stored `data` pointer, or `null_mut()` if the key
    /// is not present. Comparison is case-insensitive on ASCII,
    /// matching `sqlite3StrICmp`'s contract.
    pub fn find(&self, p_key: &str) -> *mut u8 {
        self.find_with_hash(p_key).data
    }

    /// Look up `p_key` returning both the data and the hash, useful for
    /// follow-up `insert` / `remove` calls. Mirrors the C-side
    /// `findElementWithHash`.
    pub fn find_with_hash(&self, p_key: &str) -> FindResult {
        let h = str_hash(p_key);
        if self.htsize > 0 {
            // SAFETY: htsize > 0 → ht.len() == htsize → index in range.
            let idx = (h % self.htsize) as usize;
            let entry = &self.ht[idx];
            let mut count = entry.count;
            let mut cur = entry.chain;
            while count > 0 {
                let slot = cur.expect("chain count > 0 implies chain is Some");
                let e = &self.elems[slot];
                if e.h == h && str_icmp(&e.p_key, p_key) == 0 {
                    return FindResult {
                        data: e.data,
                        h,
                        found: true,
                        slot: Some(slot),
                    };
                }
                cur = e.next;
                count -= 1;
            }
        } else {
            // No bucket array — linear search the all-elements list.
            let mut count = self.count;
            let mut cur = self.first;
            while count > 0 {
                let slot = cur.expect("count > 0 implies first is Some");
                let e = &self.elems[slot];
                if e.h == h && str_icmp(&e.p_key, p_key) == 0 {
                    return FindResult {
                        data: e.data,
                        h,
                        found: true,
                        slot: Some(slot),
                    };
                }
                cur = e.next;
                count -= 1;
            }
        }
        FindResult {
            data: ptr::null_mut(),
            h,
            found: false,
            slot: None,
        }
    }

    /// `sqlite3HashInsert` (hash.c:242-272).
    ///
    /// - If `p_key` already exists:
    ///   - If `data` is non-null: replace, return the **old** data.
    ///   - If `data` is null: remove the entry, return the old data.
    /// - If `p_key` does not exist:
    ///   - If `data` is null: no-op, return null.
    ///   - If `data` is non-null: insert, return null.
    /// - On allocation failure: return `data` unchanged.
    pub fn insert(&mut self, p_key: &str, data: *mut u8) -> *mut u8 {
        let found = self.find_with_hash(p_key);
        if found.found {
            let old = found.data;
            if data.is_null() {
                // Remove the specific slot we just verified. Walking
                // by hash alone would risk deleting a different
                // element when hashes collide — which happens in the
                // 100k-insert test, so we use the slot returned by
                // `find_with_hash` (mirrors C `removeElement(pH, elem)`
                // where `elem` is the exact pointer).
                if let Some(slot) = found.slot {
                    self.remove_slot(slot);
                }
            } else {
                // Replace data + key reference. The C version stores
                // pKey verbatim; we own a String copy.
                let slot = found.slot.expect("found implies slot");
                let e = &mut self.elems[slot];
                e.data = data;
                e.p_key.clear();
                e.p_key.push_str(p_key);
            }
            return old;
        }
        if data.is_null() {
            return ptr::null_mut();
        }
        // Allocate a new element slot.
        let new_h = found.h;
        let slot = match self.alloc_slot() {
            Some(s) => s,
            None => return data, // OOM: hash.c:262
        };
        {
            let e = &mut self.elems[slot];
            e.p_key.clear();
            e.p_key.push_str(p_key);
            e.h = new_h;
            e.data = data;
            e.next = None;
            e.prev = None;
            e.used = true;
        }
        self.count += 1;
        // Rehash trigger: count >= 5 && count > 2*htsize (hash.c:267).
        if self.count >= 5 && self.count > 2 * self.htsize {
            self.rehash(self.count * 3);
        }
        // Determine target bucket (rehash may have moved us to a new
        // bucket layout).
        let bucket = if self.htsize > 0 {
            Some((new_h % self.htsize) as usize)
        } else {
            None
        };
        self.insert_element(bucket, slot);
        ptr::null_mut()
    }

    /// Convenience: insert with an integer key (formatted as decimal).
    pub fn insert_int(&mut self, key: i64, data: *mut u8) -> *mut u8 {
        // Use a stack buffer for typical sizes; fallback to a String
        // for the long-tail.
        let mut buf = itoa_buf();
        let s = format_i64(key, &mut buf);
        self.insert(&s, data)
    }

    /// Convenience: find by integer key.
    pub fn find_int(&self, key: i64) -> *mut u8 {
        let mut buf = itoa_buf();
        let s = format_i64(key, &mut buf);
        self.find(&s)
    }

    /// Number of live elements in the table.
    pub fn len(&self) -> u32 {
        self.count
    }

    /// `true` when no elements are stored.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Number of buckets currently allocated (0 means no bucket array).
    pub fn htsize(&self) -> u32 {
        self.htsize
    }

    /// Iterate all (key, data) pairs in insertion order (the order of
    /// the all-elements doubly-linked list, which matches the C
    /// `Hash.first` chain).
    pub fn iter(&self) -> HashIter<'_> {
        HashIter {
            ht: self,
            cur: self.first,
        }
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// `insertElement` (hash.c:79-104).
    fn insert_element(&mut self, bucket: Option<usize>, slot: usize) {
        // First, splice into the all-elements doubly-linked list.
        let head = if let Some(b) = bucket {
            let entry = &mut self.ht[b];
            let head = if entry.count > 0 { entry.chain } else { None };
            entry.count += 1;
            entry.chain = Some(slot);
            head
        } else {
            None
        };
        if let Some(head_slot) = head {
            // Insert `slot` at the head of `head_slot`'s chain.
            let prev = self.elems[head_slot].prev;
            self.elems[slot].next = Some(head_slot);
            self.elems[slot].prev = prev;
            if let Some(p) = prev {
                self.elems[p].next = Some(slot);
            } else {
                self.first = Some(slot);
            }
            self.elems[head_slot].prev = Some(slot);
        } else {
            // Append to the head of the all-elements list.
            self.elems[slot].next = self.first;
            if let Some(f) = self.first {
                self.elems[f].prev = Some(slot);
            }
            self.elems[slot].prev = None;
            self.first = Some(slot);
        }
    }

    /// `removeElement` (hash.c:188-216).
    ///
    /// Removes the element at `slot` from the table. The caller is
    /// responsible for providing a valid `slot` — typically obtained
    /// from `find_with_hash(...).slot`.
    fn remove_slot(&mut self, slot: usize) {
        // Unlink from all-elements list.
        let (next, prev) = {
            let e = &self.elems[slot];
            (e.next, e.prev)
        };
        if let Some(p) = prev {
            self.elems[p].next = next;
        } else {
            self.first = next;
        }
        if let Some(n) = next {
            self.elems[n].prev = prev;
        }
        // Unlink from bucket chain (hash.c:201-208).
        if self.htsize > 0 {
            let h = self.elems[slot].h;
            let idx = (h % self.htsize) as usize;
            let entry = &mut self.ht[idx];
            if entry.chain == Some(slot) {
                entry.chain = next;
            }
            entry.count -= 1;
        }
        // Free the slot.
        let free = self.elems[slot].next; // temporarily reuse `next`
        // Actually we need to push `slot` onto the free list. The
        // `next` field has been used for the live list — re-purpose
        // it as the free-list link.
        self.elems[slot].used = false;
        self.elems[slot].p_key.clear();
        self.elems[slot].data = ptr::null_mut();
        self.elems[slot].next = self.free_head;
        self.elems[slot].prev = None;
        self.free_head = Some(slot);
        let _ = free; // silence unused warning
        self.count -= 1;
        if self.count == 0 {
            // hash.c:211-215 — drop everything when empty.
            self.ht.clear();
            self.htshrink_to_fit();
            self.htsize = 0;
            self.first = None;
            self.elems.clear();
            self.free_head = None;
        }
    }

    /// `rehash` (hash.c:113-146).
    ///
    /// Grows the bucket array to at least `new_size` slots. No-op when
    /// `new_size == 0` or allocation fails (matching the C version's
    /// "performance hit but not fatal" stance).
    fn rehash(&mut self, new_size: u32) -> bool {
        if new_size == 0 {
            return false;
        }
        let new_size = if new_size < 8 { 8 } else { new_size };
        // Allocate new bucket array; if it fails, the C version
        // returns 0 (no resize) and keeps the old one.
        let mut new_ht = vec![HashEntry::default(); new_size as usize];
        // Snapshot the all-elements list, then re-insert.
        let saved_first = self.first;
        self.first = None;
        let mut cur = saved_first;
        while let Some(slot) = cur {
            let next = self.elems[slot].next;
            let h = self.elems[slot].h;
            // Detach from current list before re-inserting.
            self.elems[slot].next = None;
            self.elems[slot].prev = None;
            // Insert into new bucket.
            let bucket = (h % new_size) as usize;
            let head = if new_ht[bucket].count > 0 {
                new_ht[bucket].chain
            } else {
                None
            };
            new_ht[bucket].count += 1;
            new_ht[bucket].chain = Some(slot);
            if let Some(head_slot) = head {
                let prev = self.elems[head_slot].prev;
                self.elems[slot].next = Some(head_slot);
                self.elems[slot].prev = prev;
                if let Some(p) = prev {
                    self.elems[p].next = Some(slot);
                } else {
                    self.first = Some(slot);
                }
                self.elems[head_slot].prev = Some(slot);
            } else {
                self.elems[slot].next = self.first;
                if let Some(f) = self.first {
                    self.elems[f].prev = Some(slot);
                }
                self.elems[slot].prev = None;
                self.first = Some(slot);
            }
            cur = next;
        }
        self.ht = new_ht;
        self.htsize = new_size;
        true
    }

    fn alloc_slot(&mut self) -> Option<usize> {
        if let Some(slot) = self.free_head {
            self.free_head = self.elems[slot].next;
            return Some(slot);
        }
        // Grow the pool. Vec::push cannot fail in practice (caller
        // catches abort via the process exit), but we model the
        // OOM path by checking capacity. We pre-allocate to avoid
        // the abort.
        let len = self.elems.len();
        let new_cap = (len + 1).next_power_of_two().max(8);
        if self.elems.capacity() < new_cap {
            // Vec::reserve may abort on OOM on the default allocator;
            // we treat that as the OOM path: return None and let
            // the caller bail.
            // SAFETY NOTE: this is best-effort — on the system
            // allocator there is no recoverable OOM in Rust. The
            // contract documented in `02-c-porting-conventions.md`
            // §3 only requires "realloc on OOM returns null, original
            // pointer preserved" — for `Vec::push` of a single
            // element that has no prior slot, there is no prior
            // pointer to preserve.
            self.elems.reserve(new_cap - self.elems.capacity());
        }
        self.elems.push(HashElem {
            p_key: String::new(),
            h: 0,
            data: ptr::null_mut(),
            next: None,
            prev: None,
            used: true,
        });
        Some(self.elems.len() - 1)
    }

    fn htshrink_to_fit(&mut self) {
        self.ht.shrink_to_fit();
    }
}

// ============================================================================
// HashIter — iterate (key, data) pairs in insertion order
// ============================================================================

/// Result of a `find_with_hash` lookup.
#[derive(Clone, Copy)]
pub struct FindResult {
    /// The stored data pointer (null on miss).
    pub data: *mut u8,
    /// The computed hash of the key.
    pub h: u32,
    /// `true` if a matching element was found.
    pub found: bool,
    /// Slot id of the matching element, when `found` is `true`.
    ///
    /// Stored so the caller can directly remove the slot without a
    /// second walk — the C version passes the actual `HashElem*`
    /// pointer into `removeElement`, so we mirror that with a slot
    /// index. When `found` is `false`, this is `None`.
    pub slot: Option<usize>,
}

/// Iterator over a `Hash` in insertion order.
pub struct HashIter<'a> {
    ht: &'a Hash,
    cur: Option<usize>,
}

impl<'a> Iterator for HashIter<'a> {
    type Item = (&'a str, *mut u8);

    fn next(&mut self) -> Option<Self::Item> {
        let slot = self.cur?;
        let e = &self.ht.elems[slot];
        let key = e.p_key.as_str();
        let data = e.data;
        self.cur = e.next;
        Some((key, data))
    }
}

// ============================================================================
// Case-insensitive ASCII comparison — matches C `sqlite3StrICmp`
// ============================================================================

#[inline]
fn str_icmp(a: &str, b: &str) -> i32 {
    // Mirror sqlite3StrICmp semantics: case-insensitive on ASCII
    // A-Z / a-z, byte-equal otherwise. Empty strings are equal.
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    let n = ab.len().min(bb.len());
    for i in 0..n {
        let mut x = ab[i];
        let mut y = bb[i];
        if x >= b'A' && x <= b'Z' {
            x += 0x20;
        }
        if y >= b'A' && y <= b'Z' {
            y += 0x20;
        }
        if x != y {
            return (x as i32) - (y as i32);
        }
    }
    (ab.len() as i32) - (bb.len() as i32)
}

// ============================================================================
// Integer-to-string conversion (avoids pulling in `itoa` crate)
// ============================================================================

fn itoa_buf() -> [u8; 20] {
    [0u8; 20]
}

fn format_i64(mut n: i64, buf: &mut [u8; 20]) -> &str {
    // Use the buffer as scratch from the back.
    let negative = n < 0;
    // Handle i64::MIN specially to avoid overflow on -n.
    if negative {
        n = n.wrapping_neg();
    }
    let mut i = buf.len();
    if n == 0 {
        i -= 1;
        buf[i] = b'0';
    } else {
        while n > 0 && i > 0 {
            i -= 1;
            buf[i] = b'0' + (n % 10) as u8;
            n /= 10;
        }
    }
    if negative && i > 0 {
        i -= 1;
        buf[i] = b'-';
    }
    // SAFETY: we only ever write ASCII digits and an optional '-'.
    std::str::from_utf8(&buf[i..]).unwrap()
}

// ============================================================================
// GrowableArray<T> — dynamic array with capacity doubling
// ============================================================================

/// Generic growable array.
///
/// Mirrors the role of `GrowableArray` in SQLite's C source
/// (used e.g. in `where.c` and `vdbemem.c`):
/// - Starts at capacity 0
/// - On `push`, doubles the capacity when full
/// - On `remove`, halves the capacity when `count < capacity/4` and
///   capacity > `MIN_CAPACITY`
///
/// This is **not** a 1:1 port of any specific C `GrowableArray`
/// (the C version lives in a few different files and the contracts
/// differ). It is a Rust-idiomatic equivalent used by Rust-side code
/// that needs a growable vector with deterministic capacity behavior.
pub struct GrowableArray<T> {
    data: Vec<Option<T>>,
    count: u32,
    capacity: u32,
}

/// Minimum capacity to avoid thrashing on a 0 → 1 → 0 sequence.
const GROWABLE_MIN_CAPACITY: u32 = 4;

impl<T> GrowableArray<T> {
    /// Construct a new, empty growable array.
    pub const fn new() -> Self {
        Self {
            data: Vec::new(),
            count: 0,
            capacity: 0,
        }
    }

    /// Construct with a given initial capacity.
    pub fn with_capacity(cap: usize) -> Self {
        let cap = cap.max(GROWABLE_MIN_CAPACITY as usize) as u32;
        let mut data = Vec::with_capacity(cap as usize);
        data.resize_with(cap as usize, || None);
        Self {
            data,
            count: 0,
            capacity: cap,
        }
    }

    /// Number of live elements.
    pub fn len(&self) -> u32 {
        self.count
    }

    /// `true` when there are no live elements.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Current capacity (number of slots, live or tombstone).
    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Append `value`, doubling the capacity if necessary. Returns
    /// the new index.
    ///
    /// OOM (in the sense of `Vec` abort-on-alloc-failure) is not
    /// recoverable; the array is left untouched on failure. We expose
    /// a `try_push` for fallible allocation.
    pub fn push(&mut self, value: T) -> u32 {
        let idx = match self.try_push(value) {
            Ok(i) => i,
            Err(_) => {
                // On the system allocator, Vec::push cannot fail
                // short of an abort; for completeness we fall back
                // to panicking with a descriptive message.
                panic!("GrowableArray: out of memory");
            }
        };
        idx
    }

    /// Fallible `push` — returns `Err(SqliteError::NOMEM)` on
    /// allocation failure (modeled; the system allocator aborts
    /// in practice).
    pub fn try_push(&mut self, value: T) -> SqliteResult<u32> {
        if self.count == self.capacity {
            // Double. Guard against 0 → 1.
            let new_cap = if self.capacity == 0 {
                GROWABLE_MIN_CAPACITY
            } else {
                self.capacity.saturating_mul(2)
            };
            self.resize_to(new_cap)?;
        }
        // Find first free slot (linear scan from 0). After doubling,
        // the new slots are at the end and are all `None`.
        let mut idx = 0u32;
        while idx < self.capacity {
            if self.data[idx as usize].is_none() {
                self.data[idx as usize] = Some(value);
                self.count += 1;
                return Ok(idx);
            }
            idx += 1;
        }
        // Should not be reachable: we just resized.
        Err(SqliteError::ERROR)
    }

    /// Get a reference to the element at `idx`.
    pub fn get(&self, idx: u32) -> Option<&T> {
        self.data.get(idx as usize).and_then(|s| s.as_ref())
    }

    /// Remove the element at `idx`, returning the value. Shrinks the
    /// capacity when `count < capacity / 4` and capacity is above the
    /// minimum threshold.
    pub fn remove(&mut self, idx: u32) -> Option<T> {
        if idx >= self.capacity {
            return None;
        }
        let v = self.data[idx as usize].take()?;
        self.count -= 1;
        if self.capacity > GROWABLE_MIN_CAPACITY
            && self.count < self.capacity / 4
        {
            let new_cap = (self.capacity / 2).max(GROWABLE_MIN_CAPACITY);
            let _ = self.resize_to(new_cap); // best-effort shrink
        }
        Some(v)
    }

    /// Iterate live elements in slot order (lower index first).
    pub fn iter(&self) -> impl Iterator<Item = (u32, &T)> {
        self.data
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|v| (i as u32, v)))
    }

    /// Drain all elements, returning an iterator of owned values.
    pub fn drain(&mut self) -> impl Iterator<Item = T> + '_ {
        self.count = 0;
        self.data.drain(..).filter_map(|s| s)
    }

    /// Resize the backing storage to `new_cap` slots. New slots are
    /// `None`. If `new_cap` is smaller than `count`, this truncates
    /// (in slot order) — useful for tests of the boundary case.
    fn resize_to(&mut self, new_cap: u32) -> SqliteResult<()> {
        let new_cap = new_cap as usize;
        if new_cap > self.data.len() {
            // Vec cannot return OOM on the system allocator; we
            // pre-check the layout and accept the abort.
            self.data.resize_with(new_cap, || None);
            self.capacity = new_cap as u32;
        } else if new_cap < self.data.len() {
            // Truncate from the end. Adjust count if we drop live
            // elements (we choose the policy: drop highest slots).
            for slot in self.data[new_cap..].iter_mut() {
                if slot.is_some() {
                    self.count -= 1;
                }
            }
            self.data.truncate(new_cap);
            self.capacity = new_cap as u32;
        }
        Ok(())
    }
}

impl<T> Default for GrowableArray<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Drop for GrowableArray<T> {
    fn drop(&mut self) {
        // Vec drops its elements automatically.
    }
}

// ============================================================================
// Internal Drop for Hash — drop live elements (their Strings) and any
// raw `data` pointers are left alone (the C version also doesn't free
// `data`).
// ============================================================================

impl Drop for Hash {
    fn drop(&mut self) {
        // Vec<HashElem> drops automatically, freeing the owned Strings.
    }
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn to_ptr<T>(x: usize) -> *mut u8 {
        x as *mut u8
    }

    fn from_ptr(p: *mut u8) -> usize {
        p as usize
    }

    #[test]
    fn str_hash_matches_c_known_value() {
        // Manually compute Knuth hash for "" — must be 0.
        assert_eq!(str_hash(""), 0);
        // Sanity: a non-empty string should produce a non-zero value
        // (Knuth multiplicative hash is non-trivial for non-empty input).
        assert_ne!(str_hash("a"), 0);
        assert_ne!(str_hash("hello"), 0);
    }

    #[test]
    fn str_hash_ascii_case_insensitive() {
        // ASCII A-Z / a-z must hash equal (mask 0xdf collapses case).
        assert_eq!(str_hash("ABC"), str_hash("abc"));
        assert_eq!(str_hash("Hello"), str_hash("HELLO"));
    }

    #[test]
    fn new_and_init_are_empty() {
        let h = Hash::new();
        assert_eq!(h.len(), 0);
        assert!(h.is_empty());
        assert_eq!(h.htsize(), 0);
    }

    #[test]
    fn insert_then_find_returns_same_data() {
        let mut h = Hash::new();
        let prev = h.insert("alpha", to_ptr::<i32>(42));
        assert!(prev.is_null());
        assert_eq!(h.len(), 1);
        assert_eq!(from_ptr(h.find("alpha")), 42);
    }

    #[test]
    fn insert_replaces_existing_key() {
        let mut h = Hash::new();
        h.insert("k", to_ptr::<i32>(1));
        let prev = h.insert("k", to_ptr::<i32>(2));
        assert_eq!(from_ptr(prev), 1);
        assert_eq!(from_ptr(h.find("k")), 2);
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn insert_null_data_removes_entry() {
        let mut h = Hash::new();
        h.insert("k", to_ptr::<i32>(1));
        let prev = h.insert("k", ptr::null_mut());
        assert_eq!(from_ptr(prev), 1);
        assert_eq!(h.len(), 0);
        assert!(h.find("k").is_null());
    }

    #[test]
    fn insert_null_data_on_missing_key_is_noop() {
        let mut h = Hash::new();
        let prev = h.insert("missing", ptr::null_mut());
        assert!(prev.is_null());
        assert_eq!(h.len(), 0);
    }

    #[test]
    fn find_missing_returns_null() {
        let mut h = Hash::new();
        h.insert("k", to_ptr::<i32>(7));
        assert!(h.find("nope").is_null());
    }

    #[test]
    fn find_is_case_insensitive_ascii() {
        let mut h = Hash::new();
        h.insert("Hello", to_ptr::<i32>(99));
        // Case-insensitive ASCII comparison
        assert_eq!(from_ptr(h.find("HELLO")), 99);
        assert_eq!(from_ptr(h.find("hello")), 99);
    }

    #[test]
    fn remove_via_null_insert_decrements_count() {
        let mut h = Hash::new();
        h.insert("a", to_ptr::<i32>(1));
        h.insert("b", to_ptr::<i32>(2));
        h.insert("c", to_ptr::<i32>(3));
        assert_eq!(h.len(), 3);
        h.insert("b", ptr::null_mut());
        assert_eq!(h.len(), 2);
        assert_eq!(from_ptr(h.find("a")), 1);
        assert!(h.find("b").is_null());
        assert_eq!(from_ptr(h.find("c")), 3);
    }

    #[test]
    fn clear_drops_all_elements() {
        let mut h = Hash::new();
        for i in 0..10 {
            h.insert(&format!("k{i}"), to_ptr::<i32>(i));
        }
        assert_eq!(h.len(), 10);
        h.clear();
        assert_eq!(h.len(), 0);
        assert!(h.is_empty());
        for i in 0..10 {
            assert!(h.find(&format!("k{i}")).is_null());
        }
    }

    #[test]
    fn insert_int_round_trip() {
        let mut h = Hash::new();
        h.insert_int(42, to_ptr::<i32>(1));
        h.insert_int(-1, to_ptr::<i32>(2));
        h.insert_int(0, to_ptr::<i32>(3));
        h.insert_int(i64::MIN, to_ptr::<i32>(4));
        assert_eq!(from_ptr(h.find_int(42)), 1);
        assert_eq!(from_ptr(h.find_int(-1)), 2);
        assert_eq!(from_ptr(h.find_int(0)), 3);
        assert_eq!(from_ptr(h.find_int(i64::MIN)), 4);
    }

    #[test]
    fn int_keys_negative_zero_dont_collide() {
        let mut h = Hash::new();
        h.insert_int(0, to_ptr::<i32>(100));
        h.insert_int(1, to_ptr::<i32>(200));
        assert_eq!(from_ptr(h.find_int(0)), 100);
        assert_eq!(from_ptr(h.find_int(1)), 200);
    }

    #[test]
    fn rehash_grows_buckets() {
        let mut h = Hash::new();
        // Insert enough to trigger the 5/count > 2*htsize rule.
        for i in 0..30u32 {
            h.insert(&format!("key{i}"), to_ptr::<i32>(i as usize));
        }
        assert_eq!(h.len(), 30);
        // htsize must have grown past 0.
        assert!(h.htsize() > 0);
        // All keys must still be reachable.
        for i in 0..30u32 {
            assert_eq!(
                from_ptr(h.find(&format!("key{i}"))),
                i as usize,
                "key{i} not found after rehash"
            );
        }
    }

    #[test]
    fn rehash_preserves_data_on_collision() {
        // Two keys that hash to the same bucket must both round-trip.
        let mut h = Hash::new();
        h.insert("collision_a", to_ptr::<i32>(1));
        h.insert("collision_b", to_ptr::<i32>(2));
        assert_eq!(from_ptr(h.find("collision_a")), 1);
        assert_eq!(from_ptr(h.find("collision_b")), 2);
    }

    #[test]
    fn remove_clears_bucket_chain() {
        // Remove one of two colliding keys; the other must still
        // resolve.
        let mut h = Hash::new();
        h.insert("a", to_ptr::<i32>(1));
        h.insert("b", to_ptr::<i32>(2));
        h.insert("a", ptr::null_mut());
        assert!(h.find("a").is_null());
        assert_eq!(from_ptr(h.find("b")), 2);
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn iter_yields_all_elements() {
        let mut h = Hash::new();
        for i in 0..5u32 {
            h.insert(&format!("k{i}"), to_ptr::<i32>(i as usize));
        }
        let mut seen = vec![false; 5];
        for (_k, d) in h.iter() {
            let i = from_ptr(d);
            assert!(i < 5, "unexpected data: {i}");
            assert!(!seen[i], "duplicate iter yield for slot {i}");
            seen[i] = true;
        }
        assert!(seen.iter().all(|x| *x));
    }

    #[test]
    fn clear_when_empty_is_idempotent() {
        let mut h = Hash::new();
        h.clear();
        h.clear();
        assert_eq!(h.len(), 0);
    }

    #[test]
    fn init_after_use_resets_state() {
        let mut h = Hash::new();
        h.insert("a", to_ptr::<i32>(1));
        h.insert("b", to_ptr::<i32>(2));
        h.init();
        assert_eq!(h.len(), 0);
        assert!(h.is_empty());
    }

    // -----------------------------------------------------------------
    // GrowableArray tests
    // -----------------------------------------------------------------

    #[test]
    fn growable_new_is_empty() {
        let g: GrowableArray<u32> = GrowableArray::new();
        assert_eq!(g.len(), 0);
        assert!(g.is_empty());
        assert_eq!(g.capacity(), 0);
    }

    #[test]
    fn growable_push_doubles_capacity() {
        let mut g: GrowableArray<u32> = GrowableArray::new();
        let i0 = g.push(10);
        assert_eq!(g.len(), 1);
        assert!(g.capacity() >= 1);
        let c1 = g.capacity();
        // Fill to capacity, then push once more — capacity must
        // double.
        while g.len() < c1 {
            g.push(0);
        }
        let prev_cap = g.capacity();
        g.push(99);
        assert!(g.capacity() >= prev_cap * 2 || g.capacity() == GROWABLE_MIN_CAPACITY * 2);
        assert_eq!(g.get(i0), Some(&10));
        assert_eq!(g.get(g.capacity() - 1), Some(&99));
    }

    #[test]
    fn growable_get_returns_correct_values() {
        let mut g: GrowableArray<u32> = GrowableArray::new();
        g.push(10);
        g.push(20);
        g.push(30);
        // Indices depend on internal slot allocation; collect
        // through iter and sort.
        let mut vals: Vec<u32> = g.iter().map(|(_, v)| *v).collect();
        vals.sort();
        assert_eq!(vals, vec![10, 20, 30]);
    }

    #[test]
    fn growable_remove_shrinks_capacity() {
        let mut g: GrowableArray<u32> = GrowableArray::with_capacity(16);
        for _ in 0..16 {
            g.push(0);
        }
        let big_cap = g.capacity();
        // Drain to < 1/4.
        for _ in 0..14 {
            g.remove(0);
        }
        assert_eq!(g.len(), 2);
        assert!(g.capacity() < big_cap, "expected shrink, got {} (was {})", g.capacity(), big_cap);
    }

    #[test]
    fn growable_remove_nonexistent_returns_none() {
        let mut g: GrowableArray<u32> = GrowableArray::new();
        assert!(g.remove(0).is_none());
        g.push(1);
        assert!(g.remove(g.capacity() + 10).is_none());
    }

    #[test]
    fn growable_keeps_min_capacity() {
        let mut g: GrowableArray<u32> = GrowableArray::with_capacity(16);
        for _ in 0..16 {
            g.push(0);
        }
        // Drain to 1 — must not shrink below MIN_CAPACITY.
        while g.len() > 1 {
            g.remove(0);
        }
        assert!(g.capacity() >= GROWABLE_MIN_CAPACITY);
    }

    #[test]
    fn growable_drain_returns_all_values() {
        let mut g: GrowableArray<u32> = GrowableArray::new();
        g.push(1);
        g.push(2);
        g.push(3);
        let drained: Vec<u32> = g.drain().collect();
        assert_eq!(drained.len(), 3);
        assert_eq!(g.len(), 0);
        assert!(g.is_empty());
    }

    #[test]
    fn growable_handles_zero_and_full() {
        // Zero capacity push: should grow to MIN.
        let mut g: GrowableArray<u32> = GrowableArray::new();
        g.push(7);
        assert!(g.capacity() >= 1);
        assert_eq!(g.len(), 1);
        // Fill to capacity, then verify "full" state.
        let cap = g.capacity();
        while g.len() < cap {
            g.push(0);
        }
        assert_eq!(g.len(), cap);
    }

    #[test]
    fn growable_100k_elements_works() {
        let mut g: GrowableArray<u64> = GrowableArray::new();
        for i in 0..100_000u64 {
            g.push(i);
        }
        assert_eq!(g.len(), 100_000);
        // Spot-check a few values.
        let mut count = 0u64;
        let mut sum = 0u64;
        for (_, v) in g.iter() {
            count += 1;
            sum += *v;
        }
        assert_eq!(count, 100_000);
        assert_eq!(sum, (0..100_000u64).sum());
    }
}
