//! T-0019 — Name resolution.
//!
//! Validates that every column reference in a `SelectStmt` (and the
//! `from` table) exists in the current `Schema`. Resolves column
//! references to (table, column_index) pairs.
//!
//! Slim subset: no views, no triggers, no subquery aliasing, no CTEs.
//!
//! # C source correspondence
//!
//! | Rust item          | C source                          |
//! |--------------------|-----------------------------------|
//! | `resolve`          | `sqlite3ResolveSelectNames`       |
//! | `ResolvedStmt`     | `NameContext` + `Select` result   |
//! | `ResolvedColumn`   | `struct Expr` after `addColumn`   |

use crate::error::SqliteError;
use crate::parse::{ColumnDef, SelectStmt, Stmt, Value, WhereExpr, WhereOp};
use crate::vdbe::{ColumnInfo, Schema, Table};
use std::collections::HashMap;

/// Public helper: validate that every column reference in `w` exists in
/// `table_columns`, returning an error for unknown columns. Used by
/// `where_compiler::run_select` to surface typo errors before evaluation.
pub fn validate_where_tree(w: &WhereExpr, table_columns: &[String]) -> Result<(), SqliteError> {
    match w {
        WhereExpr::Cmp { column, value, .. } => {
            if !table_columns.iter().any(|c| c == column) {
                return Err(SqliteError::ERROR
                    .with_msg(format!("no such column: {column} in WHERE")));
            }
            if matches!(value, Value::Identifier(_)) {
                return Err(SqliteError::ERROR
                    .with_msg("WHERE with identifier RHS not supported in slim subset".into()));
            }
            Ok(())
        }
        WhereExpr::And(a, b) | WhereExpr::Or(a, b) => {
            validate_where_tree(a, table_columns)?;
            validate_where_tree(b, table_columns)
        }
    }
}

/// A fully resolved SELECT — columns are bound by index, table
/// reference is verified.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedSelect {
    pub table: String,
    /// Each entry is `(column_name, source_index_in_table)`. Empty
    /// means "all columns in order" (SELECT *).
    pub columns: Vec<ResolvedColumn>,
    /// Validated WHERE clause preserving op + AND/OR structure. None
    /// when the original SELECT has no WHERE.
    pub where_clause: Option<ResolvedWhereExpr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedColumn {
    pub name: String,
    pub source_index: usize,
}

/// WHERE expression with column names validated against the table.
/// Same shape as `parse::WhereExpr`; we re-export it under a new name
/// so resolve-layer callers don't need to import parse directly.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedWhereExpr {
    Cmp {
        op: WhereOp,
        column: String,
        value: Value,
    },
    And(Box<ResolvedWhereExpr>, Box<ResolvedWhereExpr>),
    Or(Box<ResolvedWhereExpr>, Box<ResolvedWhereExpr>),
}

fn resolve_where(
    expr: &WhereExpr,
    table_columns: &[String],
) -> Result<ResolvedWhereExpr, SqliteError> {
    match expr {
        WhereExpr::Cmp { op, column, value } => {
            if !table_columns.iter().any(|c| c == column) {
                return Err(SqliteError::ERROR.with_msg(format!(
                    "no such column: {column} in WHERE"
                )));
            }
            // slim subset: reject Identifier RHS (column-to-column compares
            // would need an EvalEnv, not yet wired into resolve).
            if matches!(value, Value::Identifier(_)) {
                return Err(SqliteError::ERROR.with_msg(
                    "WHERE with identifier RHS not supported in slim subset".into(),
                ));
            }
            Ok(ResolvedWhereExpr::Cmp {
                op: *op,
                column: column.clone(),
                value: value.clone(),
            })
        }
        WhereExpr::And(a, b) => Ok(ResolvedWhereExpr::And(
            Box::new(resolve_where(a, table_columns)?),
            Box::new(resolve_where(b, table_columns)?),
        )),
        WhereExpr::Or(a, b) => Ok(ResolvedWhereExpr::Or(
            Box::new(resolve_where(a, table_columns)?),
            Box::new(resolve_where(b, table_columns)?),
        )),
    }
}

/// Resolve a single SELECT against the schema.
pub fn resolve_select(stmt: &SelectStmt, schema: &Schema) -> Result<ResolvedSelect, SqliteError> {
    // Verify table exists
    let table = schema
        .tables
        .get(&stmt.from)
        .ok_or_else(|| SqliteError::ERROR.with_msg(format!("no such table: {}", stmt.from)))?;

    // Resolve column list
    let columns = if stmt.all {
        // SELECT *: all columns in order
        table
            .columns
            .iter()
            .enumerate()
            .map(|(i, name)| ResolvedColumn {
                name: name.clone(),
                source_index: i,
            })
            .collect()
    } else {
        let mut out = Vec::new();
        for col_name in &stmt.columns {
            let idx = table
                .columns
                .iter()
                .position(|c| c == col_name)
                .ok_or_else(|| {
                    SqliteError::ERROR
                        .with_msg(format!("no such column: {col_name} in table {}", stmt.from))
                })?;
            out.push(ResolvedColumn {
                name: col_name.clone(),
                source_index: idx,
            });
        }
        out
    };

    // Validate WHERE (column refs + slim subset restrictions)
    let where_clause = match &stmt.where_clause {
        Some(w) => Some(resolve_where(w, &table.columns)?),
        None => None,
    };

    Ok(ResolvedSelect {
        table: stmt.from.clone(),
        columns,
        where_clause,
    })
}

/// Resolve a CREATE TABLE statement. Returns the schema entry to add.
pub fn resolve_create(
    table: &str,
    columns: &[ColumnDef],
) -> Result<(String, Table), SqliteError> {
    if table.is_empty() {
        return Err(SqliteError::ERROR.with_msg("CREATE TABLE: empty name".into()));
    }
    let names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
    // Duplicate-column check
    let mut seen: HashMap<&str, ()> = HashMap::new();
    for n in &names {
        if seen.insert(n.as_str(), ()).is_some() {
            return Err(SqliteError::ERROR.with_msg(format!("duplicate column: {n}")));
        }
    }
    Ok((
        table.to_string(),
        Table {
            columns: names,
            rows: Vec::new(),
        },
    ))
}

/// Top-level dispatch.
pub fn resolve(stmt: &Stmt, schema: &Schema) -> Result<ResolvedStmt, SqliteError> {
    match stmt {
        Stmt::Select(s) => Ok(ResolvedStmt::Select(resolve_select(s, schema)?)),
        Stmt::CreateTable(ct) => {
            let (name, table) = resolve_create(&ct.table, &ct.columns)?;
            Ok(ResolvedStmt::CreateTable(name, table))
        }
        Stmt::Insert(i) => {
            // Verify table exists, count columns
            let table = schema.tables.get(&i.table).ok_or_else(|| {
                SqliteError::ERROR.with_msg(format!("no such table: {}", i.table))
            })?;
            if i.values.len() != table.columns.len() {
                return Err(SqliteError::ERROR.with_msg(format!(
                    "INSERT: {} values for {} columns",
                    i.values.len(),
                    table.columns.len()
                )));
            }
            Ok(ResolvedStmt::Insert(ResolvedInsert {
                table: i.table.clone(),
                values: i.values.clone(),
            }))
        }
        Stmt::DropTable(dt) => Ok(ResolvedStmt::DropTable(dt.table.clone())),
        Stmt::Empty => Ok(ResolvedStmt::Empty),
    }
}

/// Top-level resolution result.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedStmt {
    Select(ResolvedSelect),
    CreateTable(String, Table),
    Insert(ResolvedInsert),
    DropTable(String),
    Empty,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedInsert {
    pub table: String,
    pub values: Vec<Value>,
}

// Helper trait on Table to enumerate its column info (used by Vm).
impl Table {
    pub fn column_info(&self) -> Vec<ColumnInfo> {
        self.columns
            .iter()
            .enumerate()
            .map(|(i, n)| ColumnInfo {
                index: i as u32,
                name: n.clone(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{parse_sql, Stmt, Value};
    use crate::vdbe::Schema;

    fn make_schema() -> Schema {
        let mut s = Schema::default();
        s.tables.insert(
            "t".into(),
            Table {
                columns: vec!["a".into(), "b".into()],
                rows: vec![],
            },
        );
        s
    }

    #[test]
    fn resolve_select_star() {
        let stmts = parse_sql("SELECT * FROM t").unwrap();
        let s = make_schema();
        let r = resolve(&stmts[0], &s).unwrap();
        match r {
            ResolvedStmt::Select(rs) => {
                assert_eq!(rs.table, "t");
                assert_eq!(rs.columns.len(), 2);
                assert_eq!(rs.columns[0].name, "a");
                assert_eq!(rs.columns[0].source_index, 0);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn resolve_select_subset() {
        let stmts = parse_sql("SELECT b FROM t").unwrap();
        let s = make_schema();
        let r = resolve(&stmts[0], &s).unwrap();
        match r {
            ResolvedStmt::Select(rs) => {
                assert_eq!(rs.columns.len(), 1);
                assert_eq!(rs.columns[0].name, "b");
                assert_eq!(rs.columns[0].source_index, 1);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn resolve_unknown_table_errors() {
        let stmts = parse_sql("SELECT * FROM nosuch").unwrap();
        let s = make_schema();
        assert!(resolve(&stmts[0], &s).is_err());
    }

    #[test]
    fn resolve_unknown_column_errors() {
        let stmts = parse_sql("SELECT z FROM t").unwrap();
        let s = make_schema();
        assert!(resolve(&stmts[0], &s).is_err());
    }

    #[test]
    fn resolve_create_duplicate_column_errors() {
        let stmts = parse_sql("CREATE TABLE foo (a INTEGER, a TEXT)").unwrap();
        let s = make_schema();
        let r = resolve(&stmts[0], &s);
        assert!(r.is_err());
    }

    #[test]
    fn resolve_insert_wrong_arity() {
        let stmts =
            parse_sql("CREATE TABLE t (a, b); INSERT INTO t VALUES (1)").unwrap();
        let s = make_schema();
        // First statement CREATE → ok, second is INSERT (value count mismatch)
        let r1 = resolve(&stmts[0], &s).unwrap();
        let r2 = resolve(&stmts[1], &s);
        assert!(matches!(r1, ResolvedStmt::CreateTable(_, _)));
        assert!(r2.is_err());
    }

    #[test]
    fn resolve_drop_table() {
        let stmts = parse_sql("DROP TABLE t").unwrap();
        let s = make_schema();
        let r = resolve(&stmts[0], &s).unwrap();
        assert!(matches!(r, ResolvedStmt::DropTable(ref n) if n == "t"));
    }
}
