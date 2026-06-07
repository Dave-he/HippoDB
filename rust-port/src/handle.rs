//! `sqlite3` 句柄 — 对齐 C 端 `struct sqlite3` 的内存布局。
//!
//! C 端 FFI 消费者通过 `sqlite3*` 拿到不透明指针,函数对它的字段做有
//! 限的访问(主要通过 `db->aDb[]` / `db->nDb` / `db->mallocFailed` 等)。
//! 真实实现中本结构体将膨胀到 ~600 字段,当前 P0 阶段只放核心字段,
//! 后续按 Pager/Schema/Vdbe 等子任务逐字段补齐。

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Mutex;

/// 不透明的 sqlite3 数据库连接句柄。
///
/// `#[repr(C)]` 是必要的:C 端消费者可能直接 cast 已知偏移,我们要
/// 保证 `SqliteDb` 在内存中与 C `struct sqlite3` 的字段顺序/大小一致。
/// 当前 P0 阶段为最小布局;随 Pager/Schema 接入会补字段。
#[repr(C)]
pub struct SqliteDb {
    /// 已分配的 schema 数组(后续 P3 阶段补)。当前为 null。
    pub a_db: *mut SchemaEntry,
    /// schema 数量。
    pub n_db: i32,
    /// malloc 失败标志(对齐 C 端 `db->mallocFailed`)。
    pub malloc_failed: AtomicBool,
    /// 活跃语句数(防止在 finalize 中误关)。
    pub active_stmt_count: AtomicI32,
    /// 内部锁。
    pub mutex: Mutex<DbMutexInner>,
}

/// 内部互斥体持有的内容(P0 阶段为空)。
#[derive(Default)]
pub struct DbMutexInner {
    /// 占位
    _placeholder: (),
}

/// Schema 数组元素 — 占位,P3 阶段会展开。
#[repr(C)]
pub struct SchemaEntry {
    /// 占位
    _placeholder: (),
}

impl SqliteDb {
    /// 分配一个全新的、空的 db 句柄。
    pub fn new() -> Box<Self> {
        Box::new(Self {
            a_db: std::ptr::null_mut(),
            n_db: 0,
            malloc_failed: AtomicBool::new(false),
            active_stmt_count: AtomicI32::new(0),
            mutex: Mutex::new(DbMutexInner::default()),
        })
    }

    /// 标记 malloc 失败(对齐 C 端 `db->mallocFailed = 1`)。
    pub fn set_malloc_failed(&self) {
        self.malloc_failed.store(true, Ordering::Release);
    }

    /// 检查 malloc 失败标志(对齐 `db->mallocFailed != 0`)。
    pub fn check_malloc_failed(&self) -> bool {
        self.malloc_failed.load(Ordering::Acquire)
    }
}

// SAFETY: SqliteDb 内部同步通过 AtomicBool/Mutex;指针字段在
// 当前阶段不跨线程共享(P0)。
unsafe impl Send for SqliteDb {}
unsafe impl Sync for SqliteDb {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_db_starts_clean() {
        let db = SqliteDb::new();
        assert!(!db.check_malloc_failed());
        assert_eq!(db.active_stmt_count.load(Ordering::Acquire), 0);
    }

    #[test]
    fn malloc_failed_round_trip() {
        let db = SqliteDb::new();
        db.set_malloc_failed();
        assert!(db.check_malloc_failed());
    }
}
