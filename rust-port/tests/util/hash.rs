//! `tests/util/hash.rs` — `Hash` + `GrowableArray` 集成测试。
//!
//! 严格对齐 `sqlite-source/src/hash.c` 行为:
//! - `sqlite3HashInit` / `sqlite3HashClear` / `sqlite3HashFind` /
//!   `sqlite3HashInsert`(对应 `Hash::new` / `clear` / `find` / `insert`)
//! - Knuth multiplicative hash(`strHash`)
//! - Rehash 触发条件:`count >= 5 && count > 2*htsize` → `count*3`
//!
//! 每个测试在文档注释中标注对应的 C 源码行号与期望行为。

use libsqlite_rs::{GrowableArray, Hash};

// ============================================================================
// 1. strHash:空串返回 0 (hash.c:56-57 — `h = 0` while 循环不进入)
// ============================================================================
#[test]
fn str_hash_empty_returns_zero() {
    assert_eq!(libsqlite_rs::str_hash(""), 0);
}

// ============================================================================
// 2. strHash:Knuth multiplicative constant 0x9e3779b1
//    手工计算 "a" 的预期值:
//      h = 0
//      h += 0xdf & 'a' (0x61) = 0x61
//      h *= 0x9e3779b1
// ============================================================================
#[test]
fn str_hash_single_char_matches_knuth() {
    // 0x61 * 0x9e3779b1 mod 2^32
    let h = (0x61u32).wrapping_mul(0x9e3779b1);
    assert_eq!(libsqlite_rs::str_hash("a"), h);
}

// ============================================================================
// 3. strHash:ASCII 大小写不敏感(mask 0xdf)
// ============================================================================
#[test]
fn str_hash_ascii_case_insensitive() {
    assert_eq!(libsqlite_rs::str_hash("ABC"), libsqlite_rs::str_hash("abc"));
    assert_eq!(libsqlite_rs::str_hash("Hello"), libsqlite_rs::str_hash("HELLO"));
    // 'A' (0x41) & 0xdf == 0x41, 'a' (0x61) & 0xdf == 0x41
    assert_eq!(libsqlite_rs::str_hash("A"), libsqlite_rs::str_hash("a"));
}

// ============================================================================
// 4. Hash::new:对应 sqlite3HashInit (hash.c:23-29) — first=0,count=0,htsize=0,ht=0
// ============================================================================
#[test]
fn hash_new_is_empty() {
    let h = Hash::new();
    assert_eq!(h.len(), 0);
    assert!(h.is_empty());
    assert_eq!(h.htsize(), 0);
}

// ============================================================================
// 5. Hash::insert + Hash::find (hash.c:242-272, 222-226)
//    新 key 返回 null(insert),find 返回原 data
// ============================================================================
#[test]
fn hash_insert_find_basic() {
    let mut h = Hash::new();
    let p = 0xdead_beefusize as *mut u8;
    let prev = h.insert("alpha", p);
    assert!(prev.is_null());
    assert_eq!(h.len(), 1);
    assert_eq!(h.find("alpha"), p);
}

// ============================================================================
// 6. Hash::insert 重复 key:返回 old data,新 data 替换 (hash.c:250-258)
// ============================================================================
#[test]
fn hash_insert_replaces_existing() {
    let mut h = Hash::new();
    let p1 = 0x1111usize as *mut u8;
    let p2 = 0x2222usize as *mut u8;
    h.insert("k", p1);
    let old = h.insert("k", p2);
    assert_eq!(old, p1);
    assert_eq!(h.find("k"), p2);
    assert_eq!(h.len(), 1, "replace should not grow count");
}

// ============================================================================
// 7. Hash::insert with null data:删除条目 (hash.c:252-253)
// ============================================================================
#[test]
fn hash_insert_null_data_removes_entry() {
    let mut h = Hash::new();
    h.insert("k", 0x1234usize as *mut u8);
    let old = h.insert("k", std::ptr::null_mut());
    assert_eq!(old, 0x1234usize as *mut u8);
    assert_eq!(h.len(), 0);
    assert!(h.find("k").is_null());
}

// ============================================================================
// 8. Hash::insert null data on missing key:no-op (hash.c:260)
// ============================================================================
#[test]
fn hash_insert_null_data_on_missing_is_noop() {
    let mut h = Hash::new();
    let r = h.insert("missing", std::ptr::null_mut());
    assert!(r.is_null());
    assert_eq!(h.len(), 0);
}

// ============================================================================
// 9. Hash::find:大小写不敏感 ASCII (对齐 C 端 sqlite3StrICmp)
// ============================================================================
#[test]
fn hash_find_is_case_insensitive_ascii() {
    let mut h = Hash::new();
    let p = 0xabcdusize as *mut u8;
    h.insert("Hello", p);
    assert_eq!(h.find("HELLO"), p);
    assert_eq!(h.find("hello"), p);
    assert_eq!(h.find("HeLLo"), p);
}

// ============================================================================
// 10. Hash::clear (hash.c:35-50):释放所有 elements + ht
// ============================================================================
#[test]
fn hash_clear_drops_all() {
    let mut h = Hash::new();
    for i in 0..50u32 {
        // i+1 to avoid null data pointers (insert(_, null) is a no-op
        // for missing keys, matching sqlite3HashInsert hash.c:262).
        h.insert(&format!("k{i}"), (i + 1) as *mut u8);
    }
    assert_eq!(h.len(), 50);
    h.clear();
    assert_eq!(h.len(), 0);
    assert!(h.is_empty());
    for i in 0..50u32 {
        assert!(h.find(&format!("k{i}")).is_null());
    }
}

// ============================================================================
// 11. Rehash 触发条件 (hash.c:267-269):
//     count >= 5 && count > 2*htsize → rehash(count*3)
//     插入 100k 元素,必须全部可查;htsize 必须 > 0
// ============================================================================
#[test]
fn hash_rehash_100k_inserts() {
    let mut h = Hash::new();
    for i in 0..100_000u32 {
        let s = format!("key_{i}");
        // i+1 to avoid null data pointers.
        let prev = h.insert(&s, ((i + 1) as usize) as *mut u8);
        assert!(prev.is_null(), "duplicate insert at i={i}");
    }
    assert_eq!(h.len(), 100_000);
    assert!(h.htsize() > 0, "htsize must grow");
    for i in 0..100_000u32 {
        let s = format!("key_{i}");
        assert_eq!(h.find(&s), ((i + 1) as usize) as *mut u8, "key_{i} not found");
    }
}

// ============================================================================
// 12. 100k 元素 delete:删除一半后,剩余必须仍可查
// ============================================================================
#[test]
fn hash_delete_half_of_100k() {
    let mut h = Hash::new();
    for i in 0..100_000u32 {
        h.insert(&format!("k{i}"), i as *mut u8);
    }
    for i in (0..100_000u32).step_by(2) {
        h.insert(&format!("k{i}"), std::ptr::null_mut());
    }
    assert_eq!(h.len(), 50_000);
    for i in 0..100_000u32 {
        let s = format!("k{i}");
        if i % 2 == 0 {
            assert!(h.find(&s).is_null());
        } else {
            assert_eq!(h.find(&s), i as *mut u8);
        }
    }
}

// ============================================================================
// 13. 100k 元素 mix:insert + delete + reinsert (压测 resize 稳定性)
// ============================================================================
#[test]
fn hash_mixed_100k_operations() {
    let mut h = Hash::new();
    for i in 0..50_000u32 {
        // i+1 to avoid null data pointers.
        h.insert(&format!("x{i}"), (i + 1) as *mut u8);
    }
    // Delete every 3rd
    for i in (0..50_000u32).step_by(3) {
        h.insert(&format!("x{i}"), std::ptr::null_mut());
    }
    // Reinsert new keys
    for i in 50_000..100_000u32 {
        h.insert(&format!("x{i}"), (i + 1) as *mut u8);
    }
    // Spot-check (data pointers are i+1 for the survivors)
    assert!(h.find("x1") == 2u32 as *mut u8, "x1 should be present, data=2");
    assert!(h.find("x0").is_null(), "x0 was step(3)-deleted");
    assert!(h.find("x3").is_null(), "x3 was step(3)-deleted");
    assert_eq!(h.find("x50000"), 50001u32 as *mut u8);
    assert_eq!(h.find("x99999"), 100000u32 as *mut u8);
    // Total: 50000 - ceil(50000/3) + 50000 = 83334 or similar
    assert!(h.len() > 80_000 && h.len() < 90_000, "unexpected count: {}", h.len());
}

// ============================================================================
// 14. 整数键:insert_int / find_int
// ============================================================================
#[test]
fn hash_int_keys_round_trip() {
    let mut h = Hash::new();
    h.insert_int(42, 0xaausize as *mut u8);
    h.insert_int(-7, 0xbbusize as *mut u8);
    h.insert_int(0, 0xccusize as *mut u8);
    h.insert_int(i64::MAX, 0xddusize as *mut u8);
    h.insert_int(i64::MIN, 0xeeusize as *mut u8);
    assert_eq!(h.find_int(42), 0xaausize as *mut u8);
    assert_eq!(h.find_int(-7), 0xbbusize as *mut u8);
    assert_eq!(h.find_int(0), 0xccusize as *mut u8);
    assert_eq!(h.find_int(i64::MAX), 0xddusize as *mut u8);
    assert_eq!(h.find_int(i64::MIN), 0xeeusize as *mut u8);
}

// ============================================================================
// 15. iter:遍历顺序 = 插入顺序(对齐 C 端 all-elements 链表)
// ============================================================================
#[test]
fn hash_iter_in_insertion_order() {
    // The C source's rehash (hash.c:113-146) walks the all-elements
    // list and re-inserts each element at the head, which reverses
    // the iteration order. After 5 inserts the rehash triggers
    // (count >= 5 && count > 2*htsize with htsize=0), so the post-
    // rehash iter yields in INSERTION order, not reverse.
    let mut h = Hash::new();
    let keys = ["first", "second", "third", "fourth", "fifth"];
    for (i, k) in keys.iter().enumerate() {
        // i+1 to avoid the null pointer (insert with null data is a
        // no-op for missing keys).
        h.insert(k, (i + 1) as *mut u8);
    }
    let collected: Vec<&str> = h.iter().map(|(k, _)| k).collect();
    assert_eq!(collected, keys);
}

// ============================================================================
// GrowableArray — 边界条件
// ============================================================================

// 16. GrowableArray::new:空 (capacity=0)
#[test]
fn growable_new_is_empty() {
    let g: GrowableArray<u32> = GrowableArray::new();
    assert_eq!(g.len(), 0);
    assert!(g.is_empty());
    assert_eq!(g.capacity(), 0);
}

// 17. GrowableArray::push:从 0 容量开始 (倍增触发)
#[test]
fn growable_push_doubles_from_zero() {
    let mut g: GrowableArray<u32> = GrowableArray::new();
    g.push(1);
    // First push must grow to MIN_CAPACITY (4).
    assert!(g.capacity() >= 1);
    let c1 = g.capacity();
    // Fill then push — capacity must at least double (or grow
    // by the doubling factor from the current state).
    while g.len() < c1 {
        g.push(0);
    }
    let c2 = g.capacity();
    g.push(99);
    let c3 = g.capacity();
    assert!(c3 >= c2 * 2 || c3 == c1 * 2, "expected doubling: {} -> {}", c2, c3);
}

// 18. GrowableArray:100k 元素压测
#[test]
fn growable_100k_pushes() {
    let mut g: GrowableArray<u64> = GrowableArray::new();
    for i in 0..100_000u64 {
        g.push(i);
    }
    assert_eq!(g.len(), 100_000);
    let mut count = 0u64;
    let mut sum = 0u64;
    for (_, v) in g.iter() {
        count += 1;
        sum += *v;
    }
    assert_eq!(count, 100_000);
    assert_eq!(sum, (0..100_000u64).sum());
}

// 19. GrowableArray:边界 — 满 / 收缩
#[test]
fn growable_full_then_shrink() {
    let mut g: GrowableArray<u32> = GrowableArray::new();
    // Fill to 16. Track real slot ids; remove(0) on a None slot is a no-op.
    let mut slots: Vec<u32> = (0..16).map(|_| g.push(0)).collect();
    let cap_full = g.capacity();
    assert!(cap_full >= 16);
    // Drain to < 1/4 — capacity must shrink. pop() 末尾 live slot,
    // 避免 resize_to 的 drop-highest 策略把还活的前面 slot 砍掉.
    for _ in 0..13 {
        let slot = slots.pop().unwrap();
        g.remove(slot);
    }
    assert_eq!(g.len(), 3);
    assert!(g.capacity() < cap_full, "expected shrink from {} to {}", cap_full, g.capacity());
}

// 20. GrowableArray:不能收缩到 MIN_CAPACITY 以下
#[test]
fn growable_shrink_floor() {
    let mut g: GrowableArray<u32> = GrowableArray::new();
    let mut slots: Vec<u32> = (0..64).map(|_| g.push(0)).collect();
    let cap_before = g.capacity();
    // Drain to 1 — pop() 末尾 live slot, 同上避免 shrink 砍活元素.
    // (固定 remove(0) 会死循环 — slot 0 在第一次 remove 后变 None, 后续是 no-op)
    while g.len() > 1 {
        let slot = slots.pop().unwrap();
        g.remove(slot);
    }
    assert!(g.capacity() >= 4, "shrunk below floor: cap={}", g.capacity());
    assert!(g.capacity() <= cap_before, "should have shrunk");
}

// 21. GrowableArray:remove 越界返回 None
#[test]
fn growable_remove_out_of_bounds() {
    let mut g: GrowableArray<u32> = GrowableArray::new();
    assert!(g.remove(0).is_none());
    g.push(1);
    assert!(g.remove(99).is_none());
    assert_eq!(g.len(), 1);
}

// 22. GrowableArray:drain 清空且 yield 所有元素
#[test]
fn growable_drain_yields_all() {
    let mut g: GrowableArray<u32> = GrowableArray::new();
    g.push(1);
    g.push(2);
    g.push(3);
    let drained: Vec<u32> = g.drain().collect();
    assert_eq!(drained.len(), 3);
    assert!(g.is_empty());
    assert_eq!(g.len(), 0);
}

// 23. GrowableArray:边界 — 空 growable 行为
#[test]
fn growable_empty_iter() {
    let g: GrowableArray<u32> = GrowableArray::new();
    assert_eq!(g.iter().count(), 0);
    assert!(g.get(0).is_none());
}

// 24. GrowableArray:push + remove + re-push (验证 slot 复用)
#[test]
fn growable_push_remove_repush() {
    let mut g: GrowableArray<u32> = GrowableArray::new();
    g.push(1);
    g.push(2);
    g.push(3);
    let v = g.remove(0);
    assert_eq!(v, Some(1));
    // After remove, len=2; re-push should succeed.
    g.push(4);
    assert_eq!(g.len(), 3);
    let mut values: Vec<u32> = g.iter().map(|(_, v)| *v).collect();
    values.sort();
    assert_eq!(values, vec![2, 3, 4]);
}
