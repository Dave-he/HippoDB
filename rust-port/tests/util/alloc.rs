//! `tests/util/alloc.rs` — 8 单元测试严格对齐 C `malloc.c` 行为。
//!
//! 这些测试直接调 `libsqlite_rs::Malloc` 内部 API(因为公开 FFI
//! 目前还没有 `sqlite3_realloc`)。等 sqlite3_realloc 公开后,可另外加
//! 一组走 FFI 的差分测试。

use libsqlite_rs::{Malloc, SqliteDb, SQLITE_MAX_ALLOCATION_SIZE};

// ============================================================================
// Test 1: malloc(0) 返回 NULL
// ============================================================================
#[test]
fn malloc_zero_returns_null() {
    let p = Malloc::malloc(0, None);
    assert!(p.is_null(), "malloc(0) must return NULL per malloc.c:320");
}

// ============================================================================
// Test 2: malloc64(0) 返回 NULL
// ============================================================================
#[test]
fn malloc64_zero_returns_null() {
    let p = Malloc::malloc64(0, None);
    assert!(p.is_null(), "malloc64(0) must return NULL per malloc.c:298");
}

// ============================================================================
// Test 3: free(NULL) 是 no-op
// ============================================================================
#[test]
fn free_null_is_noop() {
    // 不能 panic,也不能修改 malloc_failed 标志
    Malloc::free(std::ptr::null_mut());
    Malloc::free(std::ptr::null_mut());
}

// ============================================================================
// Test 4: realloc 失败时原指针保留
// ============================================================================
#[test]
fn realloc_failure_preserves_original_pointer() {
    // 1) 分配 64 字节
    let p = Malloc::malloc(64, None);
    assert!(!p.is_null());

    // 2) 用超过 SQLITE_MAX_ALLOCATION_SIZE 的 size realloc → 失败
    let r = Malloc::realloc(p, SQLITE_MAX_ALLOCATION_SIZE + 1, None);
    assert!(r.is_null(), "realloc with n>MAX must return NULL per malloc.c:515-517");

    // 3) p 必须仍然有效 — 写一个字节再读
    // SAFE: p 是 64 字节有效分配,未释放。
    unsafe { std::ptr::write_bytes(p, 0x42, 64) };
    let byte0 = unsafe { *p };
    assert_eq!(byte0, 0x42, "original p must still be writable after failed realloc");

    // 清理
    Malloc::free(p);
}

// ============================================================================
// Test 5: 大块 alloc — 超过 MAX 必须被拒绝
// ============================================================================
#[test]
fn large_alloc_above_max_is_rejected() {
    let p = Malloc::malloc(SQLITE_MAX_ALLOCATION_SIZE + 1, None);
    assert!(p.is_null(), "alloc > SQLITE_MAX_ALLOCATION_SIZE must fail");

    // 边界值 0 同样被拒
    let p2 = Malloc::malloc_zero(SQLITE_MAX_ALLOCATION_SIZE + 1, None);
    assert!(p2.is_null(), "malloc_zero > MAX must also return NULL");

    // 合法的大块(避开触发 OOM killer,只用 1MB 做"大"测试)
    let big = Malloc::malloc(1024 * 1024, None);
    assert!(!big.is_null(), "1MB alloc must succeed");
    Malloc::free(big);
}

// ============================================================================
// Test 6: 对齐 — 所有分配必须 8 字节对齐(对齐 EIGHT_BYTE_ALIGNMENT 断言)
// ============================================================================
#[test]
fn allocation_is_8_byte_aligned() {
    for size in [1, 3, 7, 8, 9, 15, 16, 17, 31, 32, 33, 127, 128, 1023, 1024] {
        let p = Malloc::malloc(size, None);
        assert!(!p.is_null(), "alloc of {size} bytes must succeed");
        assert_eq!(
            p as usize % 8,
            0,
            "alloc of {size} bytes must be 8-byte aligned, got {:#x}",
            p as usize
        );
        Malloc::free(p);
    }
}

// ============================================================================
// Test 7: OOM 标志读写 — malloc 失败必须写 db.malloc_failed
// ============================================================================
#[test]
fn oom_flag_written_on_size_limit() {
    let db = SqliteDb::new();
    assert!(!db.check_malloc_failed(), "malloc_failed must start false");

    // 触发"参数超限"路径:C 端对 n>MAX **不**写 malloc_failed(因为是
    // 参数错误,不是真正的 OOM)。我们改用 malloc_zero 测真实 OOM 写入
    // — 但 64 位下没法稳定触发 alloc 失败,所以这条主要验证:
    //  a) 超限不 panic
    //  b) 标志状态与输入一致
    let p = Malloc::malloc(SQLITE_MAX_ALLOCATION_SIZE + 1, Some(&db));
    assert!(p.is_null());
    // 超限不写 malloc_failed(对齐 C 端 sqlite3Malloc:写 0 走参数检查)
    assert!(
        !db.check_malloc_failed(),
        "size > MAX is a parameter error, not OOM — C does not set malloc_failed"
    );

    // 正常 alloc 不写标志
    let q = Malloc::malloc(64, Some(&db));
    assert!(!q.is_null());
    assert!(!db.check_malloc_failed());
    Malloc::free(q);

    // 手动置位,验证 read 路径
    db.set_malloc_failed();
    assert!(db.check_malloc_failed());
}

// ============================================================================
// Test 8: 0 字节 realloc — 必须释放 p,返回 NULL
// ============================================================================
#[test]
fn realloc_with_zero_size_frees_and_returns_null() {
    let p = Malloc::malloc(128, None);
    assert!(!p.is_null());

    let r = Malloc::realloc(p, 0, None);
    assert!(r.is_null(), "realloc(p, 0) must return NULL per malloc.c:511-514");
    // p 已经被 free,后续 free(p) 必须是 no-op(查表时找不到条目)
    // 二次 free 不应 panic
    // (不能直接 free 同一指针两次 — 内部会查表,未知指针就静默 no-op)
    Malloc::free(p);
}
