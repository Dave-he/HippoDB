//! T-0021 — WHERE optimizer + SELECT compiler.
//!
//! Slim subset: only full table scan with optional simple WHERE filter.
//! No index selection, no join, no ORDER BY, no GROUP BY, no LIMIT
//! (LIMIT is handled by an early-exit at the Next opcode).
//!
//! # C source correspondence
//!
//! | Rust item            | C source                            |
//! |----------------------|-------------------------------------|
//! | `compile_select`     | `selectAddSubqueryTypeInfo` + `codeSelect` |
//! | `compile_stmt`       | `sqlite3_exec`-style dispatch       |

use crate::error::SqliteError;
use crate::expr::{BinaryOp as B, EvalEnv, Expr as E, FunctionRegistry, NullRegistry, SqliteValue, eval as expr_eval};
use crate::parse::{parse_sql, ColumnDef, InsertStmt, SelectStmt, Stmt, Value, WhereExpr, WhereOp};
use crate::vdbe::{exec, Mem, Op, Schema, VdbeProgram};

/// Compile a SELECT statement into a VdbeProgram.
///
/// `reg_base` is the first register index the caller wants to use; we
/// add our own scratch registers above it.
pub fn compile_select(stmt: &SelectStmt, schema: &Schema, reg_base: u32) -> Result<VdbeProgram, SqliteError> {
    // Validate table exists
    if !schema.tables.contains_key(&stmt.from) {
        return Err(SqliteError::ERROR.with_msg(format!("no such table: {}", stmt.from)));
    }
    let table = schema.tables.get(&stmt.from).unwrap();
    let ncols = table.columns.len();

    // Resolve which columns to emit
    let emit_cols: Vec<usize> = if stmt.all {
        (0..ncols).collect()
    } else {
        let mut out = Vec::new();
        for col_name in &stmt.columns {
            let idx = table.columns.iter().position(|c| c == col_name).ok_or_else(|| {
                SqliteError::ERROR.with_msg(format!("no such column: {col_name}"))
            })?;
            out.push(idx);
        }
        out
    };

    // Build the program
    let mut ops: Vec<Op> = Vec::new();

    // OpenRead at pc 0
    let cursor_id: u32 = 0;
    ops.push(Op::OpenRead {
        cursor: cursor_id,
        table: stmt.from.clone(),
    });

    // Reserve registers: emit_cols[i] -> regs[reg_base + i]
    let emit_reg_base = reg_base;

    // Filter loop entry. PC = 1.
    let pc_top = ops.len() as u32;
    for (i, col_idx) in emit_cols.iter().enumerate() {
        ops.push(Op::Column {
            cursor: cursor_id,
            column: *col_idx as u32,
            dest: emit_reg_base + i as u32,
        });
    }

    // Apply WHERE filter (if any). The slim subset delegates the actual
    // comparison to the high-level runner (`run_select`), which walks the
    // WhereExpr tree and uses `expr::eval` against each row's Mem. So at the
    // Vm level we just emit an unconditional ResultRow for every cursor row;
    // the filter happens outside the Vm.
    let _ = &stmt.where_clause;

    // ResultRow
    ops.push(Op::ResultRow {
        start: emit_reg_base,
        count: emit_cols.len() as u32,
    });

    // Next at pc = ?
    let pc_next = ops.len() as u32;
    ops.push(Op::Next {
        cursor: cursor_id,
        target: pc_next + 2, // +1 for Goto, +1 for Halt
    });
    ops.push(Op::Goto { target: pc_top });
    let pc_halt = ops.len() as u32;
    ops.push(Op::Halt { code: 0 });

    let prog = VdbeProgram { ops };
    let _ = pc_halt;
    Ok(prog)
}

/// Compile an INSERT statement.
pub fn compile_insert(stmt: &InsertStmt, schema: &Schema, reg_base: u32) -> Result<VdbeProgram, SqliteError> {
    if !schema.tables.contains_key(&stmt.table) {
        return Err(SqliteError::ERROR.with_msg(format!("no such table: {}", stmt.table)));
    }
    let table = schema.tables.get(&stmt.table).unwrap();
    if stmt.values.len() != table.columns.len() {
        return Err(SqliteError::ERROR.with_msg(format!(
            "INSERT: {} values for {} columns",
            stmt.values.len(),
            table.columns.len()
        )));
    }

    let mut ops: Vec<Op> = Vec::new();
    ops.push(Op::OpenWrite {
        cursor: 0,
        table: stmt.table.clone(),
    });
    for (i, v) in stmt.values.iter().enumerate() {
        let dest = reg_base + i as u32;
        match v {
            Value::Integer(n) => ops.push(Op::Integer { value: *n, dest }),
            Value::Real(f) => ops.push(Op::Real { value: *f, dest }),
            Value::String(s) => ops.push(Op::String { value: s.clone(), dest }),
            Value::Null => ops.push(Op::Null { dest }),
            Value::Identifier(_) => {
                return Err(SqliteError::ERROR.with_msg(
                    "INSERT identifier value not supported".into(),
                ));
            }
        }
    }
    let n_values = stmt.values.len() as u32;
    ops.push(Op::Insert {
        cursor: 0,
        start: reg_base,
        count: n_values,
    });
    ops.push(Op::Halt { code: 0 });
    Ok(VdbeProgram { ops })
}

/// Compile a CREATE TABLE statement.
pub fn compile_create(name: &str, columns: &[ColumnDef]) -> VdbeProgram {
    let names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
    VdbeProgram {
        ops: vec![
            Op::CreateTable { name: name.to_string(), columns: names },
            Op::Halt { code: 0 },
        ],
    }
}

/// Compile a DROP TABLE statement.
pub fn compile_drop(name: &str) -> VdbeProgram {
    VdbeProgram {
        ops: vec![Op::DropTable { name: name.to_string() }, Op::Halt { code: 0 }],
    }
}

/// Top-level dispatch.
pub fn compile_stmt(stmt: &Stmt, schema: &Schema) -> Result<VdbeProgram, SqliteError> {
    match stmt {
        Stmt::Select(s) => compile_select(s, schema, 0),
        Stmt::Insert(i) => compile_insert(i, schema, 0),
        Stmt::CreateTable(ct) => Ok(compile_create(&ct.table, &ct.columns)),
        Stmt::DropTable(dt) => Ok(compile_drop(&dt.table)),
        Stmt::Empty => Ok(VdbeProgram { ops: vec![Op::Halt { code: 0 }] }),
    }
}

// ─── High-level runner: applies WHERE filter via expr::eval ──────────────

/// Run a single SELECT, applying the WHERE filter (a `WhereExpr` tree of
/// comparisons + AND/OR) via `expr::eval` against each row's Mem.
pub fn run_select(stmt: &SelectStmt, schema: &mut Schema) -> Result<Vec<Vec<Mem>>, SqliteError> {
    // Validate WHERE column references first (mirrors resolve_where in the
    // resolve layer). This catches typos like `WHERE y = 1` against a table
    // that only has `x`, instead of silently returning NULL.
    if let Some(w) = &stmt.where_clause {
        if let Some(table) = schema.tables.get(&stmt.from) {
            crate::resolve::validate_where_tree(w, &table.columns)?;
        }
    }

    // Compile and execute the raw program to get all rows (no filter applied in Vm).
    let prog = compile_select(stmt, schema, 0)?;
    let raw_rows = exec(&prog, schema)?;

    if stmt.where_clause.is_none() {
        return Ok(raw_rows.into_iter().map(|r| r.0).collect());
    }

    // Pre-collect table info before the borrow on raw_rows.
    let table_columns: Vec<String> = schema
        .tables
        .get(&stmt.from)
        .map(|t| t.columns.clone())
        .unwrap_or_default();

    let where_expr = stmt.where_clause.as_ref().unwrap();

    let mut out = Vec::new();
    for row in raw_rows {
        let env = SimpleMemEnv {
            all_cols: &table_columns,
            row: &row.0,
        };
        let v = expr_eval(&where_expr_to_expr(where_expr), &env)?;
        // SQL truthiness: Integer(1) → keep; Null → exclude (NULL WHERE);
        // Integer(0) or anything else → exclude.
        if matches!(v, SqliteValue::Integer(1)) {
            out.push(row.0);
        }
    }
    Ok(out)
}

/// Convert a parsed `WhereExpr` (slim: cmp/AND/OR over a single literal
/// RHS) into the generic `expr::Expr` AST that `expr::eval` consumes.
fn where_expr_to_expr(w: &WhereExpr) -> E {
    match w {
        WhereExpr::Cmp { op, column, value } => E::Binary {
            op: where_op_to_binaryop(*op),
            left: Box::new(E::ColumnRef(column.clone())),
            right: Box::new(value_to_expr(value)),
        },
        WhereExpr::And(a, b) => E::Binary {
            op: B::And,
            left: Box::new(where_expr_to_expr(a)),
            right: Box::new(where_expr_to_expr(b)),
        },
        WhereExpr::Or(a, b) => E::Binary {
            op: B::Or,
            left: Box::new(where_expr_to_expr(a)),
            right: Box::new(where_expr_to_expr(b)),
        },
    }
}

fn where_op_to_binaryop(op: WhereOp) -> B {
    match op {
        WhereOp::Eq => B::Eq,
        WhereOp::Ne => B::Ne,
        WhereOp::Lt => B::Lt,
        WhereOp::Le => B::Le,
        WhereOp::Gt => B::Gt,
        WhereOp::Ge => B::Ge,
    }
}

fn value_to_expr(v: &Value) -> E {
    match v {
        Value::Integer(i) => E::Literal(crate::expr::Literal::Integer(*i)),
        Value::Real(f) => E::Literal(crate::expr::Literal::Real(*f)),
        Value::String(s) => E::Literal(crate::expr::Literal::String(s.clone())),
        Value::Null => E::Literal(crate::expr::Literal::Null),
        Value::Identifier(s) => E::ColumnRef(s.clone()),
    }
}

/// EvalEnv backed by a single row of Mem.
struct SimpleMemEnv<'a> {
    all_cols: &'a [String],
    row: &'a [Mem],
}

impl<'a> EvalEnv for SimpleMemEnv<'a> {
    fn lookup(&self, name: &str) -> SqliteValue {
        if let Some(i) = self.all_cols.iter().position(|c| c == name) {
            if i < self.row.len() {
                mem_to_sqlitevalue(&self.row[i])
            } else {
                SqliteValue::Null
            }
        } else {
            SqliteValue::Null
        }
    }
    fn call_func(&self, _name: &str, _args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
        Err(SqliteError::ERROR)
    }
}

fn mem_to_sqlitevalue(m: &Mem) -> SqliteValue {
    match m {
        Mem::Null => SqliteValue::Null,
        Mem::Integer(i) => SqliteValue::Integer(*i),
        Mem::Real(f) => SqliteValue::Real(*f),
        Mem::Text(s) => SqliteValue::Text(s.clone()),
        Mem::Blob(b) => SqliteValue::Blob(b.clone()),
    }
}

/// Run a sequence of SQL statements.
pub fn run_sql(sql: &str, schema: &mut Schema) -> Result<Vec<Vec<Mem>>, SqliteError> {
    let stmts = parse_sql(sql)?;
    let mut all_rows: Vec<Vec<Mem>> = Vec::new();
    for stmt in stmts {
        match &stmt {
            Stmt::Select(s) => {
                let rows = run_select(s, schema)?;
                all_rows.extend(rows);
            }
            _ => {
                let prog = compile_stmt(&stmt, schema)?;
                exec(&prog, schema)?;
            }
        }
    }
    Ok(all_rows)
}
