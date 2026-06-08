//! T-0020 — VdbeProgram + Op enum + Vm executor.
//!
//! Partial port of `sqlite-source/src/vdbe.c`. Slim subset: a register
//! machine with ~15 opcodes, executing against an in-memory `Schema`.
//! No cursors over B-trees (in-memory row vectors), no subqueries, no
//! triggers, no savepoints, no autoincrement, no journaling.
//!
//! # C source correspondence
//!
//! | Rust item            | C source                              |
//! |----------------------|---------------------------------------|
//! | `VdbeProgram`        | `Vdbe *`                              |
//! | `Op` variants        | `OP_*` in `vdbe.c`                    |
//! | `Vm::exec`           | `sqlite3VdbeExec`                    |
//! | `Schema` / `Table`   | `sqlite3` + `Btree` (simplified)     |
//! | `Mem`                | `Mem` (simplified)                    |

use crate::error::SqliteError;
use std::collections::BTreeMap;

/// A VDBE program: a flat list of opcodes to execute.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VdbeProgram {
    pub ops: Vec<Op>,
}

/// Vdbe opcodes (slim subset).
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// Halt with the given code (0 = OK, 1 = ERROR, etc).
    Halt { code: i32 },
    /// Load an integer literal into register `dest`.
    Integer { value: i64, dest: u32 },
    /// Load a real literal into register `dest`.
    Real { value: f64, dest: u32 },
    /// Load a text literal into register `dest`.
    String { value: String, dest: u32 },
    /// Load NULL into register `dest`.
    Null { dest: u32 },
    /// Copy register `src` into register `dest`.
    Copy { src: u32, dest: u32 },
    /// Emit the registers in [start..start+count) as one result row.
    ResultRow { start: u32, count: u32 },
    /// Jump to `target` if register is NULL or Integer(0).
    IfNot { reg: u32, target: u32 },
    /// Jump to `target` if register is non-NULL and non-zero.
    If { reg: u32, target: u32 },
    /// Unconditional jump.
    Goto { target: u32 },
    /// Open a read cursor for the given table.
    OpenRead { cursor: u32, table: String },
    /// Read column `column` of the current cursor row into register `dest`.
    Column { cursor: u32, column: u32, dest: u32 },
    /// Advance the cursor; jump to `target` when it goes past the last row.
    Next { cursor: u32, target: u32 },
    /// Open a write cursor (we treat it identically to OpenRead for in-memory).
    OpenWrite { cursor: u32, table: String },
    /// Insert a row from registers [start..start+count).
    Insert { cursor: u32, start: u32, count: u32 },
    /// Create a new table in the schema.
    CreateTable { name: String, columns: Vec<String> },
    /// Drop a table from the schema.
    DropTable { name: String },
}

/// A runtime value, mirroring SQLite's `Mem` union.
#[derive(Debug, Clone, PartialEq)]
pub enum Mem {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Mem {
    /// Coerce to an integer (for comparison / bitwise ops).
    pub fn to_integer(&self) -> Option<i64> {
        match self {
            Mem::Null => None,
            Mem::Integer(i) => Some(*i),
            Mem::Real(f) => Some(*f as i64),
            Mem::Text(s) => s.parse::<i64>().ok(),
            Mem::Blob(b) if b.len() == 8 => Some(i64::from_be_bytes(b[..8].try_into().unwrap())),
            _ => None,
        }
    }

    /// SQL truthiness.
    pub fn to_bool(&self) -> Option<bool> {
        match self {
            Mem::Null => None,
            Mem::Integer(0) => Some(false),
            Mem::Integer(_) => Some(true),
            Mem::Real(f) if *f == 0.0 => Some(false),
            Mem::Real(_) => Some(true),
            Mem::Text(s) if s.is_empty() => Some(false),
            Mem::Text(s) => match s.parse::<f64>() {
                Ok(f) if f == 0.0 => Some(false),
                Ok(_) => Some(true),
                Err(_) => Some(false),
            },
            Mem::Blob(b) if b.is_empty() => Some(false),
            Mem::Blob(_) => Some(true),
        }
    }

    /// SQL strict equality with NULL propagation.
    /// NULL == anything → false. Text vs Int coerces if Int is parseable.
    pub fn eq_sql(&self, other: &Mem) -> bool {
        if matches!(self, Mem::Null) || matches!(other, Mem::Null) {
            return false;
        }
        match (self, other) {
            (Mem::Integer(a), Mem::Integer(b)) => a == b,
            (Mem::Real(a), Mem::Real(b)) => a == b,
            (Mem::Integer(a), Mem::Real(b)) => (*a as f64) == *b,
            (Mem::Real(a), Mem::Integer(b)) => *a == (*b as f64),
            (Mem::Text(a), Mem::Text(b)) => a == b,
            (Mem::Text(a), Mem::Integer(b)) => a.parse::<i64>().map(|i| i == *b).unwrap_or(false),
            (Mem::Integer(a), Mem::Text(b)) => b.parse::<i64>().map(|i| i == *a).unwrap_or(false),
            (Mem::Blob(a), Mem::Blob(b)) => a == b,
            _ => false,
        }
    }

    /// Convert to a display string (used for result row printing).
    pub fn to_display_string(&self) -> String {
        match self {
            Mem::Null => "NULL".to_string(),
            Mem::Integer(i) => i.to_string(),
            Mem::Real(f) => format!("{f}"),
            Mem::Text(s) => s.clone(),
            Mem::Blob(b) => format!("X'{}'", hex_encode(b)),
        }
    }
}

fn hex_encode(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for byte in b {
        s.push_str(&format!("{byte:02x}"));
    }
    s
}

/// A table schema entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Mem>>,
}

/// Column metadata (index, name).
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnInfo {
    pub index: u32,
    pub name: String,
}

/// The full database schema (in-memory).
#[derive(Debug, Default, Clone)]
pub struct Schema {
    pub tables: BTreeMap<String, Table>,
}

impl Schema {
    pub fn new() -> Self {
        Self::default()
    }
}

/// A cursor state.
#[derive(Debug, Clone)]
struct Cursor {
    table: String,
    pos: usize, // -1 means EOF; we use pos >= len() for EOF
}

/// A single emitted row.
#[derive(Debug, Clone, PartialEq)]
pub struct Row(pub Vec<Mem>);

/// Execute a VdbeProgram, collecting all ResultRow emissions.
///
/// `reg` is sized to the maximum dest register referenced in the program.
pub fn exec(prog: &VdbeProgram, schema: &mut Schema) -> Result<Vec<Row>, SqliteError> {
    // Compute max register index
    let max_reg = max_register(prog).unwrap_or(0);
    let mut regs: Vec<Mem> = vec![Mem::Null; (max_reg + 1) as usize];
    let mut cursors: BTreeMap<u32, Cursor> = BTreeMap::new();
    let mut rows: Vec<Row> = Vec::new();

    let mut pc: usize = 0;
    loop {
        if pc >= prog.ops.len() {
            // Ran off the end without Halt: treat as halt(0)
            break;
        }
        let op = &prog.ops[pc];
        match op {
            Op::Halt { code } => {
                if *code != 0 {
                    return Err(SqliteError(*code));
                }
                break;
            }
            Op::Integer { value, dest } => {
                regs[*dest as usize] = Mem::Integer(*value);
                pc += 1;
            }
            Op::Real { value, dest } => {
                regs[*dest as usize] = Mem::Real(*value);
                pc += 1;
            }
            Op::String { value, dest } => {
                regs[*dest as usize] = Mem::Text(value.clone());
                pc += 1;
            }
            Op::Null { dest } => {
                regs[*dest as usize] = Mem::Null;
                pc += 1;
            }
            Op::Copy { src, dest } => {
                regs[*dest as usize] = regs[*src as usize].clone();
                pc += 1;
            }
            Op::ResultRow { start, count } => {
                let s = *start as usize;
                let n = *count as usize;
                let row = regs[s..s + n].to_vec();
                rows.push(Row(row));
                pc += 1;
            }
            Op::IfNot { reg, target } => {
                let v = &regs[*reg as usize];
                if !matches!(v.to_bool(), Some(true)) {
                    pc = *target as usize;
                } else {
                    pc += 1;
                }
            }
            Op::If { reg, target } => {
                let v = &regs[*reg as usize];
                if matches!(v.to_bool(), Some(true)) {
                    pc = *target as usize;
                } else {
                    pc += 1;
                }
            }
            Op::Goto { target } => {
                pc = *target as usize;
            }
            Op::OpenRead { cursor, table } | Op::OpenWrite { cursor, table } => {
                if !schema.tables.contains_key(table) {
                    return Err(SqliteError::ERROR.with_msg(format!("no such table: {table}")));
                }
                cursors.insert(
                    *cursor,
                    Cursor {
                        table: table.clone(),
                        pos: 0,
                    },
                );
                pc += 1;
            }
            Op::Column { cursor, column, dest } => {
                let cur = cursors.get(cursor).ok_or_else(|| {
                    SqliteError::ERROR.with_msg(format!("no such cursor: {cursor}"))
                })?;
                let table = schema
                    .tables
                    .get(&cur.table)
                    .ok_or_else(|| SqliteError::ERROR)?;
                if cur.pos >= table.rows.len() {
                    regs[*dest as usize] = Mem::Null;
                } else {
                    let row = &table.rows[cur.pos];
                    let col = *column as usize;
                    if col < row.len() {
                        regs[*dest as usize] = row[col].clone();
                    } else {
                        regs[*dest as usize] = Mem::Null;
                    }
                }
                pc += 1;
            }
            Op::Next { cursor, target } => {
                let cur = cursors.get_mut(cursor).ok_or_else(|| {
                    SqliteError::ERROR.with_msg(format!("no such cursor: {cursor}"))
                })?;
                cur.pos += 1;
                let table = schema.tables.get(&cur.table).unwrap();
                if cur.pos >= table.rows.len() {
                    pc = *target as usize;
                } else {
                    pc += 1;
                }
            }
            Op::Insert { cursor, start, count } => {
                let cur = cursors.get(cursor).ok_or_else(|| {
                    SqliteError::ERROR.with_msg(format!("no such cursor: {cursor}"))
                })?;
                let s = *start as usize;
                let n = *count as usize;
                let new_row: Vec<Mem> = regs[s..s + n].to_vec();
                let table_name = cur.table.clone();
                let table = schema
                    .tables
                    .get_mut(&table_name)
                    .ok_or_else(|| SqliteError::ERROR)?;
                table.rows.push(new_row);
                pc += 1;
            }
            Op::CreateTable { name, columns } => {
                schema.tables.insert(
                    name.clone(),
                    Table {
                        columns: columns.clone(),
                        rows: Vec::new(),
                    },
                );
                pc += 1;
            }
            Op::DropTable { name } => {
                schema.tables.remove(name);
                pc += 1;
            }
        }
    }
    Ok(rows)
}

fn max_register(prog: &VdbeProgram) -> Option<u32> {
    let mut max: Option<u32> = None;
    for op in &prog.ops {
        let candidates: &[u32] = match op {
            Op::Integer { dest, .. }
            | Op::Real { dest, .. }
            | Op::String { dest, .. }
            | Op::Null { dest }
            | Op::Column { dest, .. } => std::slice::from_ref(dest),
            Op::Copy { src, dest } => return Some(*src.max(dest)),
            Op::IfNot { reg, .. } | Op::If { reg, .. } => std::slice::from_ref(reg),
            _ => &[],
        };
        for c in candidates {
            max = Some(max.map_or(*c, |m| m.max(*c)));
        }
    }
    max
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(v: i64) -> Mem {
        Mem::Integer(v)
    }
    fn text(s: &str) -> Mem {
        Mem::Text(s.into())
    }
    fn null() -> Mem {
        Mem::Null
    }

    #[test]
    fn exec_simple_halt() {
        let prog = VdbeProgram {
            ops: vec![Op::Halt { code: 0 }],
        };
        let mut s = Schema::new();
        let rows = exec(&prog, &mut s).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn exec_integer_to_reg() {
        let prog = VdbeProgram {
            ops: vec![
                Op::Integer { value: 42, dest: 0 },
                Op::ResultRow { start: 0, count: 1 },
                Op::Halt { code: 0 },
            ],
        };
        let mut s = Schema::new();
        let rows = exec(&prog, &mut s).unwrap();
        assert_eq!(rows, vec![Row(vec![int(42)])]);
    }

    #[test]
    fn exec_create_insert_select() {
        let prog = VdbeProgram {
            ops: vec![
                Op::CreateTable {
                    name: "t".into(),
                    columns: vec!["a".into(), "b".into()],
                },
                Op::OpenWrite { cursor: 0, table: "t".into() },
                Op::Integer { value: 1, dest: 0 },
                Op::String {
                    value: "x".into(),
                    dest: 1,
                },
                Op::Insert {
                    cursor: 0,
                    start: 0,
                    count: 2,
                },
                Op::OpenRead { cursor: 0, table: "t".into() },
                Op::Column { cursor: 0, column: 0, dest: 0 },
                Op::Column { cursor: 0, column: 1, dest: 1 },
                Op::ResultRow { start: 0, count: 2 },
                Op::Next { cursor: 0, target: 12 },
                Op::Goto { target: 6 },
                Op::Halt { code: 0 },
                Op::Halt { code: 0 }, // pc=12: Halt
            ],
        };
        let mut s = Schema::new();
        let rows = exec(&prog, &mut s).unwrap();
        assert_eq!(rows, vec![Row(vec![int(1), text("x")])]);
    }

    #[test]
    fn mem_eq_sql() {
        assert!(int(5).eq_sql(&int(5)));
        assert!(!int(5).eq_sql(&int(6)));
        assert!(!null().eq_sql(&int(5)));
        assert!(!int(5).eq_sql(&null()));
        assert!(text("a").eq_sql(&text("a")));
        assert!(!text("a").eq_sql(&text("b")));
    }

    #[test]
    fn mem_to_bool() {
        assert_eq!(int(0).to_bool(), Some(false));
        assert_eq!(int(1).to_bool(), Some(true));
        assert_eq!(int(-1).to_bool(), Some(true));
        assert_eq!(null().to_bool(), None);
        assert_eq!(text("0").to_bool(), Some(false));
        assert_eq!(text("").to_bool(), Some(false));
    }
}
