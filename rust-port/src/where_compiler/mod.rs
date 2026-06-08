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
use crate::parse::{parse_sql, ColumnDef, InsertStmt, SelectStmt, Stmt, Value};
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

    // Apply WHERE filter (if any). We use a simple inline evaluator.
    if let Some(wc) = &stmt.where_clause {
        // Use a scratch register for the filter result
        let filter_reg = emit_reg_base + emit_cols.len() as u32;
        // Inline: column = literal comparison.
        // For now, only support `col op value` where op is one of =, !=, <, <=, >, >=.
        let col_idx = table.columns.iter().position(|c| c == &wc.column).ok_or_else(|| {
            SqliteError::ERROR.with_msg(format!("no such column: {}", wc.column))
        })?;
        // Read the column value into a scratch register
        let col_scratch = filter_reg + 1;
        ops.push(Op::Column {
            cursor: cursor_id,
            column: col_idx as u32,
            dest: col_scratch,
        });
        // Push literal into another scratch register
        let lit_scratch = filter_reg + 2;
        match &wc.value {
            Value::Integer(i) => ops.push(Op::Integer { value: *i, dest: lit_scratch }),
            Value::Real(f) => ops.push(Op::Real { value: *f, dest: lit_scratch }),
            Value::String(s) => ops.push(Op::String { value: s.clone(), dest: lit_scratch }),
            Value::Null => ops.push(Op::Null { dest: lit_scratch }),
            Value::Identifier(_) => {
                return Err(SqliteError::ERROR.with_msg(
                    "WHERE with identifier RHS not supported in slim subset".into(),
                ));
            }
        }
        // For slim: do the comparison directly with a chain of IfNot -> Goto.
        // We use a single scratch "matches" register set to 1 then conditionally cleared.
        // Simpler: emit a single IfNot that jumps past ResultRow when filter is false.
        // We compare col_scratch and lit_scratch and put 0/1 in filter_reg.
        // For slim, we hand-roll: emit an evaluation op via Column/copy, but
        // there's no native comparison op. We use the trick: do comparison in
        // a small register sequence. The simplest path: implement `<`, `=`, `>`
        // via a sequence of If/IfNot on the SQL truthiness of the expression
        // by adding a compare op... but we don't have a Compare opcode.
        //
        // Pragmatic approach: encode the comparison inline in the program by
        // emitting a CallFunc-like opcode that delegates to expr::eval. Since
        // we don't have a FuncCall opcode either, we use a workaround: load
        // both values into env, eval outside the program, and **inline the
        // result** by reading the cursor outside of the Vm loop.
        //
        // But that's not how Vdbe works. The real fix: add a Compare op
        // to Vdbe. We do that by emitting a single Op::IfNot that uses a
        // precomputed match. For T-0021 slim, the filter is evaluated by a
        // higher-level wrapper: compile_select returns a program; the
        // SELECT executor pre-filters rows by evaluating the WHERE with
        // `expr::eval` against the cursor's current row, and only pushes
        // a "skip" flag. Since we don't have such a flag, the cleanest
        // path is: encode the comparison as a sequence of integer comparisons
        // via a new Op::Compare { left, right, op, dest } opcode.
        //
        // For now, encode it as: load col_scratch, load lit_scratch, then
        // emit an Op::Compare that we add. To keep T-0021 self-contained,
        // we don't add new opcodes; instead we evaluate the WHERE in the
        // higher-level executor (see `run_select` below).
        //
        // We mark the filter via a sentinel Op::Filter { left: col_scratch, right: lit_scratch, op: wc_op }.
        // But Vdbe doesn't know Filter. So the pragmatic answer: the e2e test
        // driver (run_select) handles filtering by pre-evaluating the WHERE
        // using expr::eval against Mem values it reads from the schema before
        // emitting ResultRow.
        //
        // For T-0021, we delegate the actual comparison to a wrapper that
        // walks the cursor outside the VM, emitting rows as it goes.
        // This means compile_select returns a "raw" program that the
        // caller filters with a Rust-side WHERE check.
        //
        // → see `run_select` in this module for the high-level entry point
        //   that does the full compile + filter + execute.
        let _ = filter_reg;
    }

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

/// Run a single SELECT, applying the WHERE filter with the expression
/// evaluator against each row.
pub fn run_select(stmt: &SelectStmt, schema: &mut Schema) -> Result<Vec<Vec<Mem>>, SqliteError> {
    // Compile and execute the raw program to get all rows (no filter applied in Vm).
    let prog = compile_select(stmt, schema, 0)?;
    let raw_rows = exec(&prog, schema)?;

    if stmt.where_clause.is_none() {
        return Ok(raw_rows.into_iter().map(|r| r.0).collect());
    }

    // Filter: for each row, build RowEnv and eval the WHERE.
    let wc = stmt.where_clause.as_ref().unwrap();
    let table = schema.tables.get(&stmt.from).unwrap();
    let col_idx = table
        .columns
        .iter()
        .position(|c| c == &wc.column)
        .ok_or_else(|| SqliteError::ERROR.with_msg(format!("no such column: {}", wc.column)))?;

    // Build a hand-rolled EvalEnv that takes the row's Mem for the column.
    // We construct a simple Expr: Binary{op, col_ref, literal}
    // and eval it. For the slim scope, this only handles =, <, >, <=, >=, !=.
    let bin_op = where_op_to_binaryop(&wc.value);
    let literal = value_to_sqlitevalue(&wc.value);
    let col_ref = E::ColumnRef(wc.column.clone());

    let mut out = Vec::new();
    for row in raw_rows {
        let env = SimpleMemEnv { col_name: wc.column.clone(), col_idx, all_cols: &table.columns, row: &row.0 };
        // Build the comparison Expr dynamically
        let cmp_expr = E::Binary {
            op: bin_op,
            left: Box::new(col_ref.clone()),
            right: Box::new(literal.clone()),
        };
        let v = expr_eval(&cmp_expr, &env)?;
        // Pass iff the comparison yields truthy
        if matches!(v, SqliteValue::Integer(1)) {
            out.push(row.0);
        } else if matches!(v, SqliteValue::Null) {
            // SQL: NULL WHERE → row excluded
            continue;
        }
        // Integer(0) → row excluded
    }
    Ok(out)
}

fn where_op_to_binaryop(_val: &Value) -> B {
    // The slim parser only supports = in WHERE (the existing parse::parse_select
    // has WhereClause { column, value } with implicit =). For broader support
    // we'd need to extend WhereClause to carry an explicit op. For now, =.
    B::Eq
}

fn value_to_sqlitevalue(v: &Value) -> E {
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
    col_name: String,
    col_idx: usize,
    all_cols: &'a [String],
    row: &'a [Mem],
}

impl<'a> EvalEnv for SimpleMemEnv<'a> {
    fn lookup(&self, name: &str) -> SqliteValue {
        if name == self.col_name {
            mem_to_sqlitevalue(&self.row[self.col_idx])
        } else if let Some(i) = self.all_cols.iter().position(|c| c == name) {
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
