//! 内部内存分配 API — 1:1 移植 `sqlite-source/src/malloc.c`。
//!
//! 公共契约(对齐 C 源码):
//! - `malloc(n)`: n<=0 → NULL;n>MAX → NULL;OOM → NULL,设置 `db.malloc_failed`
//! - `malloc64(n)`: n==0 → NULL;n>MAX → NULL;OOM → NULL,设置 `db.malloc_failed`
//! - `realloc(p, n)`: p==NULL 退化为 malloc;n==0 释放 p 返回 NULL;
//!   n>MAX 返回 NULL, p 保留;OOM 返回 NULL, p 保留(原指针永不丢失)
//! - `free(p)`: NULL no-op
//! - 所有分配保证 8 字节对齐(对齐 C 端 `EIGHT_BYTE_ALIGNMENT` 断言)
//!
//! 实现:基于 `std::alloc::Global` + 显式 `Layout`,通过全局 `Mutex<HashMap>`
//! 跟踪每块分配的 size(因为 Rust 的 `GlobalAlloc` 本身不提供 xSize hook)。
//! 这模拟了 C 端 `sqlite3GlobalConfig.m.xSize` 的角色。

use std::alloc::{self, Layout};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::handle::SqliteDb;

/// SQLITE_MAX_ALLOCATION_SIZE = 2_147_483_391 — 见 `sqliteLimit.h:42`。
pub const SQLITE_MAX_ALLOCATION_SIZE: u64 = 2_147_483_391;

/// 所有内部分配的对齐 — 对齐 C 端 `EIGHT_BYTE_ALIGNMENT` 宏(8 字节)。
const MALLOC_ALIGN: usize = 8;

/// 全局分配 size 表(模拟 `sqlite3GlobalConfig.m.xSize`)。
///
/// `ptr_addr → size`。只在分配/释放时短暂持锁。
/// 用 `usize`(指针地址)作 key,避免 `*mut u8`/`NonNull<u8>` 的 `Send` 约束。
static ALLOC_SIZES: OnceLock<Mutex<HashMap<usize, usize>>> = OnceLock::new();

#[inline]
fn sizes() -> &'static Mutex<HashMap<usize, usize>> {
    ALLOC_SIZES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 内部 malloc 家族 — 对应 C 端 `sqlite3Malloc` / `sqlite3MallocZero` /
/// `sqlite3Realloc` / `sqlite3_free`。
///
/// `pub` 是为了让 `tests/util/alloc.rs` 的差分测试能直接调用;FFI 公开层
/// (`src/api.rs`)只是 thin wrapper。
pub struct Malloc;

impl Malloc {
    /// `sqlite3Malloc(n)` — 分配 n 字节。
    ///
    /// C 行为(`malloc.c:296-309`):`n==0 || n>SQLITE_MAX_ALLOCATION_SIZE` → NULL;
    /// OOM → NULL,设置 `db.malloc_failed`。
    // 我们遵循内部 API 全部 safe 的约定(参 `02-c-porting-conventions.md` §2):
    // 返回的 `*mut u8` 仍是裸指针,但调用方在 `unsafe` 上下文中使用。
    // 抑制 `not_unsafe_ptr_arg_deref` lint。
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn malloc(n: u64, db: Option<&SqliteDb>) -> *mut u8 {
        if n == 0 || n > SQLITE_MAX_ALLOCATION_SIZE {
            return std::ptr::null_mut();
        }
        let size = n as usize;
        // SAFE: n > 0, align 是 2 的幂,from_size_align_unchecked 不会失败。
        let layout = unsafe { Layout::from_size_align_unchecked(size, MALLOC_ALIGN) };
        // SAFE: layout 合法;`alloc` 仅在 OOM 时返回 null,不 panic。
        let p = unsafe { alloc::alloc(layout) };
        if p.is_null() {
            if let Some(db) = db {
                db.set_malloc_failed();
            }
            return std::ptr::null_mut();
        }
        record_alloc(p, size);
        p
    }

    /// `sqlite3Malloc64(n)` — 同 `malloc`,显式 i64/u64 签名。
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn malloc64(n: u64, db: Option<&SqliteDb>) -> *mut u8 {
        Self::malloc(n, db)
    }

    /// `sqlite3MallocZero(n)` — alloc + memset 0。
    #[allow(dead_code)] // 后续子任务(sqlite3DbMallocZero)会接入
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn malloc_zero(n: u64, db: Option<&SqliteDb>) -> *mut u8 {
        let p = Self::malloc(n, db);
        if !p.is_null() {
            // SAFE: p 是合法的 n 字节分配,memset 0 不越界。
            unsafe { std::ptr::write_bytes(p, 0u8, n as usize) };
        }
        p
    }

    /// `sqlite3Realloc(p, n)` — 调整 p 的大小。
    ///
    /// C 行为(`malloc.c:503-556`):
    /// - `p==NULL` → 退化为 `malloc(n)`
    /// - `n==0` → 释放 p,返回 NULL
    /// - `n>MAX` → 返回 NULL,**p 保留**
    /// - OOM → 返回 NULL,**p 保留**(C 永不释放失败时的原指针)
    #[allow(dead_code)] // 公开 API 转换在后续子任务(sqlite3_realloc)接入
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn realloc(p: *mut u8, n: u64, db: Option<&SqliteDb>) -> *mut u8 {
        if p.is_null() {
            return Self::malloc(n, db);
        }
        if n == 0 {
            Self::free(p);
            return std::ptr::null_mut();
        }
        if n > SQLITE_MAX_ALLOCATION_SIZE {
            return std::ptr::null_mut();
        }
        let old_size = match lookup_size(p) {
            Some(s) => s,
            // p 不是我们分配的(无法 resize) → 视为 OOM。
            None => {
                if let Some(db) = db {
                    db.set_malloc_failed();
                }
                return std::ptr::null_mut();
            }
        };
        let new_size = n as usize;
        // 分配新块;若 OOM,**保留 p**(对齐 C 行为)。
        let new_p = Self::malloc(n, db);
        if new_p.is_null() {
            return std::ptr::null_mut();
        }
        let copy_len = old_size.min(new_size);
        // SAFE: new_p 是 new_size 字节有效分配;src p 仍有效;copy_len 不越界任一方。
        unsafe { std::ptr::copy_nonoverlapping(p, new_p, copy_len) };
        // 释放旧块(不影响 new_p)。
        Self::free(p);
        new_p
    }

    /// `sqlite3_free(p)` — 释放指针。NULL 是 no-op。
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn free(p: *mut u8) {
        if p.is_null() {
            return;
        }
        let size = match lookup_size(p) {
            Some(s) => s,
            None => return, // 未知指针,静默忽略(对齐 C:assert-only 检查)
        };
        forget_alloc(p);
        // SAFE: p 是我们之前 alloc 的,size 与之匹配,align 恒为 8。
        let layout = unsafe { Layout::from_size_align_unchecked(size, MALLOC_ALIGN) };
        // SAFETY: layout 与 alloc 时一致;`dealloc` 对此指针不 panic。
        unsafe { alloc::dealloc(p, layout) };
    }
}

/// 记录一次分配。`p` 必须非 null。
fn record_alloc(p: *mut u8, size: usize) {
    sizes().lock().unwrap().insert(p as usize, size);
}

/// 释放时移除记录。
fn forget_alloc(p: *mut u8) {
    sizes().lock().unwrap().remove(&(p as usize));
}

/// 查表获取分配 size。`p` 必须非 null。
fn lookup_size(p: *mut u8) -> Option<usize> {
    sizes().lock().unwrap().get(&(p as usize)).copied()
}

// SAFETY: ALLOC_SIZES 只在分配/释放路径短暂持锁,NonNull<u8> 是 Send。
unsafe impl Send for Malloc {}
unsafe impl Sync for Malloc {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_malloc_zero_returns_null() {
        let p = Malloc::malloc(0, None);
        assert!(p.is_null());
    }

    #[test]
    fn internal_realloc_grow_preserves_data() {
        let p = Malloc::malloc(16, None);
        assert!(!p.is_null());
        // SAFE: p 是 16 字节有效分配。
        unsafe { std::ptr::write_bytes(p, 0xab, 16) };
        let np = Malloc::realloc(p, 64, None);
        assert!(!np.is_null());
        // SAFE: np 是 64 字节分配,前 16 字节已从 p 复制。
        for i in 0..16 {
            assert_eq!(unsafe { *np.add(i) }, 0xab);
        }
        Malloc::free(np);
    }

    #[test]
    fn internal_alignment_is_8_bytes() {
        for size in [1, 7, 8, 9, 16, 17, 128, 1023] {
            let p = Malloc::malloc(size, None);
            assert!(!p.is_null());
            assert_eq!(p as usize % 8, 0, "alloc of {size} not 8-aligned");
            Malloc::free(p);
        }
    }
}
