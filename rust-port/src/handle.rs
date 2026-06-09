//! `sqlite3` 句柄和 `sqlite3_stmt` 句柄 — 对齐 C 端内存布局。
//!
//! C 端 FFI 消费者通过 `sqlite3*` 和 `sqlite3_stmt*` 拿到不透明指针。
//! 真实实现中，这些结构体非常庞大，我们在 slim subset 阶段只放核心字段，
//! 尤其是内存数据库的 Schema 状态以及 Statement 缓存的结果行。

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Mutex;

/// 不透明的 sqlite3 数据库连接句柄。
#[repr(C)]
pub struct SqliteDb {
    /// 已分配的 schema 数组(对齐 C 布局)。当前为 null。
    pub a_db: *mut SchemaEntry,
    /// schema 数量。
    pub n_db: i32,
    /// malloc 失败标志(对齐 C 端 `db->mallocFailed`)。
    pub malloc_failed: AtomicBool,
    /// 活跃语句数(防止在 finalize 中误关)。
    pub active_stmt_count: AtomicI32,
    /// 内部锁。
    pub mutex: Mutex<DbMutexInner>,

    /// 存储数据库的所有表和数据 (在内存中)
    pub schema: Mutex<crate::vdbe::Schema>,
    /// 最近一次错误码
    pub last_err_code: AtomicI32,
    /// 最近一次错误消息 (CString 保证 FFI 兼容)
    pub last_err_msg: Mutex<std::ffi::CString>,
}

/// 内部互斥体持有的内容。
#[derive(Default)]
pub struct DbMutexInner {
    _placeholder: (),
}

/// Schema 数组元素 — 占位。
#[repr(C)]
pub struct SchemaEntry {
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
            schema: Mutex::new(crate::vdbe::Schema::new()),
            last_err_code: AtomicI32::new(0),
            last_err_msg: Mutex::new(std::ffi::CString::new("").unwrap()),
        })
    }

    /// 标记 malloc 失败。
    pub fn set_malloc_failed(&self) {
        self.malloc_failed.store(true, Ordering::Release);
    }

    /// 检查 malloc 失败标志。
    pub fn check_malloc_failed(&self) -> bool {
        self.malloc_failed.load(Ordering::Acquire)
    }

    /// 设置最近一次的错误信息。
    pub fn set_error(&self, code: i32, msg: &str) {
        self.last_err_code.store(code, Ordering::Release);
        if let Ok(mut m) = self.last_err_msg.lock() {
            *m = std::ffi::CString::new(msg).unwrap_or_else(|_| std::ffi::CString::new("").unwrap());
        }
    }

    /// 清理最近一次的错误信息。
    pub fn clear_error(&self) {
        self.last_err_code.store(0, Ordering::Release);
        if let Ok(mut m) = self.last_err_msg.lock() {
            *m = std::ffi::CString::new("").unwrap();
        }
    }
}

// SAFETY: SqliteDb 内部同步通过 Atomic/Mutex 进行保护。
unsafe impl Send for SqliteDb {}
unsafe impl Sync for SqliteDb {}

/// 不透明的 sqlite3_stmt 语句句柄。
#[repr(C)]
pub struct SqliteStmt {
    /// 指向所属 of db 句柄
    pub db: *mut SqliteDb,
    /// 编译后的 VdbeProgram
    pub program: crate::vdbe::VdbeProgram,
    /// 解析出来的 AST 语句 (用于 run_select 等 high-level runner)
    pub stmt: crate::parse::Stmt,
    /// 缓存的执行结果行 (None 表示未执行)
    pub rows: Option<Vec<Vec<crate::vdbe::Mem>>>,
    /// 当前迭代到的行索引 (0-based)
    pub row_idx: usize,
    /// 当前活跃行的索引 (None 表示未开始或已结束)
    pub current_row: Option<usize>,
    /// 结果列名称
    pub columns: Vec<String>,
    /// 缓存的结果列名称 CString 指针 (用于 sqlite3_column_name 延长生命周期)
    pub column_names_c: Vec<std::ffi::CString>,
    /// 缓存当前行的列值 CString (用于 sqlite3_column_text 延长生命周期)
    pub cached_col_texts: Vec<Option<std::ffi::CString>>,
}

impl SqliteStmt {
    /// 创建一个新的 Statement 句柄。
    pub fn new(
        db: *mut SqliteDb,
        stmt: crate::parse::Stmt,
        program: crate::vdbe::VdbeProgram,
        columns: Vec<String>,
    ) -> Box<Self> {
        let column_names_c = columns
            .iter()
            .map(|c| std::ffi::CString::new(c.clone()).unwrap_or_else(|_| std::ffi::CString::new("").unwrap()))
            .collect();
        let cached_col_texts = vec![None; columns.len()];
        Box::new(Self {
            db,
            stmt,
            program,
            rows: None,
            row_idx: 0,
            current_row: None,
            columns,
            column_names_c,
            cached_col_texts,
        })
    }
}

// SAFETY: 跨 FFI 传递
unsafe impl Send for SqliteStmt {}
unsafe impl Sync for SqliteStmt {}

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

    #[test]
    fn error_handling() {
        let db = SqliteDb::new();
        db.set_error(1, "some error");
        assert_eq!(db.last_err_code.load(Ordering::Acquire), 1);
        let msg = db.last_err_msg.lock().unwrap();
        assert_eq!(msg.to_str().unwrap(), "some error");
    }
}
