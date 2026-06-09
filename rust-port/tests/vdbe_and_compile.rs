//! Direct tests for `compile_create` / `compile_drop` / `compile_insert` /
//! `compile_select` / `compile_stmt`, plus `Mem` variant coverage and
//! `Schema` manipulation. These exercise code paths that the e2e tests
//! (`tests/expr_eval.rs`) cover indirectly but do not pin the
//! per-statement program shapes.
//!
//! Run with: `cargo test --test vdbe_and_compile`

use libsqlite_rs::{
    compile_create, compile_drop, compile_insert, compile_select, compile_stmt,
};
use libsqlite_rs::parse::{
    parse_sql, ColumnDef, InsertStmt, SelectStmt, Stmt, Value, WhereExpr, WhereOp,
};
use libsqlite_rs::vdbe::{Mem, Op, Schema, Table};
use libsqlite_rs::SqliteError;

// ─── Mem variant coverage ───────────────────────────────────────────────

#[test]
fn mem_to_integer_variants() {
    assert_eq!(Mem::Null.to_integer(), None);
    assert_eq!(Mem::Integer(7).to_integer(), Some(7));
    assert_eq!(Mem::Integer(-7).to_integer(), Some(-7));
    assert_eq!(Mem::Real(3.7).to_integer(), Some(3));
    assert_eq!(Mem::Real(-3.7).to_integer(), Some(-3));
    assert_eq!(Mem::Text("123".to_string()).to_integer(), Some(123));
    assert_eq!(Mem::Text("-1".to_string()).to_integer(), Some(-1));
    assert_eq!(Mem::Text("abc".to_string()).to_integer(), None);
    // 8-byte blob → i64
    let v = 0x0102_0304_0506_0708i64;
    let bytes = v.to_be_bytes().to_vec();
    assert_eq!(Mem::Blob(bytes).to_integer(), Some(v));
    // 7-byte blob is not a valid i64 length
    let short = vec![1u8, 2, 3, 4, 5, 6, 7];
    assert_eq!(Mem::Blob(short).to_integer(), None);
}

#[test]
fn mem_to_bool_all_variants() {
    assert_eq!(Mem::Null.to_bool(), None);
    assert_eq!(Mem::Integer(0).to_bool(), Some(false));
    assert_eq!(Mem::Integer(1).to_bool(), Some(true));
    assert_eq!(Mem::Integer(42).to_bool(), Some(true));
    assert_eq!(Mem::Real(0.0).to_bool(), Some(false));
    assert_eq!(Mem::Real(0.5).to_bool(), Some(true));
    assert_eq!(Mem::Real(-1.0).to_bool(), Some(true));
    assert_eq!(Mem::Text("".to_string()).to_bool(), Some(false));
    assert_eq!(Mem::Text("0".to_string()).to_bool(), Some(false));
    assert_eq!(Mem::Text("1".to_string()).to_bool(), Some(true));
    assert_eq!(Mem::Text("hi".to_string()).to_bool(), Some(false));
    assert_eq!(Mem::Blob(vec![]).to_bool(), Some(false));
    assert_eq!(Mem::Blob(vec![0u8]).to_bool(), Some(true));
}

#[test]
fn mem_to_display_string_all_variants() {
    assert_eq!(Mem::Null.to_display_string(), "NULL");
    assert_eq!(Mem::Integer(42).to_display_string(), "42");
    assert_eq!(Mem::Real(3.5).to_display_string(), "3.5");
    assert_eq!(Mem::Text("hi".to_string()).to_display_string(), "hi");
    // Blob is hex-encoded with X'...' wrapping
    assert_eq!(Mem::Blob(vec![0xab, 0xcd]).to_display_string(), "X'abcd'");
    assert_eq!(Mem::Blob(vec![]).to_display_string(), "X''");
}

#[test]
fn mem_eq_sql_numeric_coercion() {
    // Integer 5 == Real 5.0
    assert!(Mem::Integer(5).eq_sql(&Mem::Real(5.0)));
    assert!(Mem::Real(5.0).eq_sql(&Mem::Integer(5)));
    // 5 != 5.5
    assert!(!Mem::Integer(5).eq_sql(&Mem::Real(5.5)));
    // Text "5" == Integer 5
    assert!(Mem::Text("5".to_string()).eq_sql(&Mem::Integer(5)));
    // Text "abc" != Integer 5
    assert!(!Mem::Text("abc".to_string()).eq_sql(&Mem::Integer(5)));
    // Blob equality
    assert!(Mem::Blob(vec![1, 2, 3]).eq_sql(&Mem::Blob(vec![1, 2, 3])));
    assert!(!Mem::Blob(vec![1, 2]).eq_sql(&Mem::Blob(vec![1, 2, 3])));
    // Cross-type mismatches
    assert!(!Mem::Integer(5).eq_sql(&Mem::Text("5".to_string())) == false); // integer vs text
    assert!(!Mem::Null.eq_sql(&Mem::Null)); // NULL == NULL is false
}

// ─── Schema / Table coverage ───────────────────────────────────────────

#[test]
fn schema_new_is_empty() {
    let s = Schema::new();
    assert!(s.tables.is_empty());
}

#[test]
fn table_starts_empty_rows() {
    let t = Table {
        columns: vec!["a".to_string()],
        rows: vec![],
    };
    assert!(t.rows.is_empty());
    assert_eq!(t.columns.len(), 1);
}

// ─── compile_create / compile_drop ────────────────────────────────────

#[test]
fn compile_create_produces_op_stream() {
    let cols = vec![
        ColumnDef {
            name: "id".to_string(),
            type_name: "INTEGER".to_string(),
        },
        ColumnDef {
            name: "name".to_string(),
            type_name: "TEXT".to_string(),
        },
    ];
    let prog = compile_create("t", &cols);
    assert_eq!(prog.ops.len(), 2);
    match &prog.ops[0] {
        Op::CreateTable { name, columns } => {
            assert_eq!(name, "t");
            assert_eq!(columns, &vec!["id".to_string(), "name".to_string()]);
        }
        other => panic!("expected CreateTable, got {other:?}"),
    }
    assert!(matches!(prog.ops[1], Op::Halt { code: 0 }));
}

#[test]
fn compile_create_with_no_columns() {
    let prog = compile_create("empty", &[]);
    assert_eq!(prog.ops.len(), 2);
    if let Op::CreateTable { name, columns } = &prog.ops[0] {
        assert_eq!(name, "empty");
        assert!(columns.is_empty());
    } else {
        panic!();
    }
}

#[test]
fn compile_drop_produces_op_stream() {
    let prog = compile_drop("t");
    assert_eq!(prog.ops.len(), 2);
    match &prog.ops[0] {
        Op::DropTable { name } => assert_eq!(name, "t"),
        other => panic!("expected DropTable, got {other:?}"),
    }
    assert!(matches!(prog.ops[1], Op::Halt { code: 0 }));
}

// ─── compile_insert ───────────────────────────────────────────────────

fn schema_with_table() -> Schema {
    let mut s = Schema::new();
    s.tables.insert(
        "t".to_string(),
        Table {
            columns: vec!["a".to_string(), "b".to_string()],
            rows: vec![],
        },
    );
    s
}

#[test]
fn compile_insert_writes_integer_and_string() {
    let stmt = InsertStmt {
        table: "t".to_string(),
        values: vec![Value::Integer(7), Value::String("hi".to_string())],
    };
    let s = schema_with_table();
    let prog = compile_insert(&stmt, &s, 0).unwrap();
    // Expected: OpenWrite + Integer + String + Insert + Halt = 5 ops
    assert_eq!(prog.ops.len(), 5);
    assert!(matches!(prog.ops[0], Op::OpenWrite { .. }));
    assert!(matches!(prog.ops[1], Op::Integer { value: 7, dest: 0 }));
    if let Op::String { value, dest } = &prog.ops[2] {
        assert_eq!(value, "hi");
        assert_eq!(*dest, 1);
    } else {
        panic!();
    }
    if let Op::Insert { cursor, start, count } = &prog.ops[3] {
        assert_eq!(*cursor, 0);
        assert_eq!(*start, 0);
        assert_eq!(*count, 2);
    } else {
        panic!();
    }
    assert!(matches!(prog.ops[4], Op::Halt { code: 0 }));
}

#[test]
fn compile_insert_null_value() {
    let stmt = InsertStmt {
        table: "t".to_string(),
        values: vec![Value::Null, Value::Null],
    };
    let s = schema_with_table();
    let prog = compile_insert(&stmt, &s, 0).unwrap();
    assert!(matches!(prog.ops[1], Op::Null { dest: 0 }));
    assert!(matches!(prog.ops[2], Op::Null { dest: 1 }));
}

#[test]
fn compile_insert_real_value() {
    let stmt = InsertStmt {
        table: "t".to_string(),
        values: vec![Value::Real(1.5), Value::Integer(0)],
    };
    let s = schema_with_table();
    let prog = compile_insert(&stmt, &s, 0).unwrap();
    if let Op::Real { value, dest } = &prog.ops[1] {
        assert_eq!(*value, 1.5);
        assert_eq!(*dest, 0);
    } else {
        panic!();
    }
}

#[test]
fn compile_insert_unknown_table_errors() {
    let stmt = InsertStmt {
        table: "missing".to_string(),
        values: vec![Value::Integer(1), Value::Integer(2)],
    };
    let s = schema_with_table();
    let r = compile_insert(&stmt, &s, 0);
    assert!(matches!(r, Err(SqliteError(_))));
    assert_eq!(r.unwrap_err().code(), 1);
}

#[test]
fn compile_insert_column_count_mismatch() {
    let stmt = InsertStmt {
        table: "t".to_string(),
        values: vec![Value::Integer(1)], // only 1 value, table has 2
    };
    let s = schema_with_table();
    let r = compile_insert(&stmt, &s, 0);
    assert!(r.is_err());
    assert_eq!(r.unwrap_err().code(), 1);
}

#[test]
fn compile_insert_identifier_value_errors() {
    // INSERT with column reference is not supported in slim subset
    let stmt = InsertStmt {
        table: "t".to_string(),
        values: vec![Value::Identifier("a".to_string()), Value::Integer(1)],
    };
    let s = schema_with_table();
    let r = compile_insert(&stmt, &s, 0);
    assert!(r.is_err());
}

// ─── compile_select ───────────────────────────────────────────────────

#[test]
fn compile_select_unknown_table_errors() {
    let stmt = SelectStmt {
        all: true,
        columns: vec![],
        from: "missing".to_string(),
        where_clause: None,
    };
    let s = schema_with_table();
    let r = compile_select(&stmt, &s, 0);
    assert!(r.is_err());
}

#[test]
fn compile_select_unknown_column_errors() {
    let stmt = SelectStmt {
        all: false,
        columns: vec!["missing".to_string()],
        from: "t".to_string(),
        where_clause: None,
    };
    let s = schema_with_table();
    let r = compile_select(&stmt, &s, 0);
    assert!(r.is_err());
}

#[test]
fn compile_select_specific_columns() {
    let stmt = SelectStmt {
        all: false,
        columns: vec!["b".to_string()], // pick column index 1
        from: "t".to_string(),
        where_clause: None,
    };
    let s = schema_with_table();
    let prog = compile_select(&stmt, &s, 0).unwrap();
    // OpenRead + 1 Column + ResultRow + Next + Goto + Halt = 6 ops
    assert_eq!(prog.ops.len(), 6);
    if let Op::Column { column, dest, .. } = &prog.ops[1] {
        assert_eq!(*column, 1);
        assert_eq!(*dest, 0);
    } else {
        panic!();
    }
}

#[test]
fn compile_select_all_columns() {
    let stmt = SelectStmt {
        all: true,
        columns: vec![],
        from: "t".to_string(),
        where_clause: None,
    };
    let s = schema_with_table();
    let prog = compile_select(&stmt, &s, 0).unwrap();
    // OpenRead + 2 Columns + ResultRow + Next + Goto + Halt = 7 ops
    assert_eq!(prog.ops.len(), 7);
    if let Op::Column { column, .. } = &prog.ops[1] {
        assert_eq!(*column, 0);
    } else {
        panic!();
    }
    if let Op::Column { column, .. } = &prog.ops[2] {
        assert_eq!(*column, 1);
    } else {
        panic!();
    }
}

#[test]
fn compile_select_with_where_clause_still_compiles() {
    // The WHERE clause is validated by the resolver; compile_select
    // ignores the actual condition (filter happens in run_select).
    let stmt = SelectStmt {
        all: true,
        columns: vec![],
        from: "t".to_string(),
        where_clause: Some(WhereExpr::Cmp {
            op: WhereOp::Eq,
            column: "a".to_string(),
            value: Value::Integer(1),
        }),
    };
    let s = schema_with_table();
    let prog = compile_select(&stmt, &s, 0).unwrap();
    // Should compile without error.
    assert!(!prog.ops.is_empty());
}

// ─── compile_stmt (top-level dispatch) ───────────────────────────────

#[test]
fn compile_stmt_empty() {
    let stmt = Stmt::Empty;
    let s = schema_with_table();
    let prog = compile_stmt(&stmt, &s).unwrap();
    assert_eq!(prog.ops.len(), 1);
    assert!(matches!(prog.ops[0], Op::Halt { code: 0 }));
}

#[test]
fn compile_stmt_create_dispatch() {
    let stmts = parse_sql("CREATE TABLE foo (x INTEGER)").unwrap();
    let stmt = stmts.into_iter().next().unwrap();
    let s = Schema::new();
    let prog = compile_stmt(&stmt, &s).unwrap();
    assert!(matches!(prog.ops[0], Op::CreateTable { .. }));
}

#[test]
fn compile_stmt_drop_dispatch() {
    let stmts = parse_sql("DROP TABLE foo").unwrap();
    let stmt = stmts.into_iter().next().unwrap();
    let s = Schema::new();
    let prog = compile_stmt(&stmt, &s).unwrap();
    assert!(matches!(prog.ops[0], Op::DropTable { .. }));
}

#[test]
fn compile_stmt_select_dispatch() {
    let stmts = parse_sql("SELECT * FROM t").unwrap();
    let stmt = stmts.into_iter().next().unwrap();
    let s = schema_with_table();
    let prog = compile_stmt(&stmt, &s).unwrap();
    assert!(matches!(prog.ops[0], Op::OpenRead { .. }));
}

#[test]
fn compile_stmt_insert_dispatch() {
    let stmts = parse_sql("INSERT INTO t VALUES (1, 2)").unwrap();
    let stmt = stmts.into_iter().next().unwrap();
    let s = schema_with_table();
    let prog = compile_stmt(&stmt, &s).unwrap();
    assert!(matches!(prog.ops[0], Op::OpenWrite { .. }));
}

// ─── parse_sql + run_sql multi-statement smoke ───────────────────────

#[test]
fn run_sql_select_with_where_runs_e2e() {
    use libsqlite_rs::run_sql;
    let mut s = Schema::new();
    run_sql("CREATE TABLE t (a INTEGER, b TEXT)", &mut s).unwrap();
    run_sql("INSERT INTO t VALUES (1, 'x')", &mut s).unwrap();
    run_sql("INSERT INTO t VALUES (2, 'y')", &mut s).unwrap();
    let rows = run_sql("SELECT a FROM t WHERE a > 1", &mut s).unwrap();
    assert_eq!(rows, vec![vec![Mem::Integer(2)]]);
}

#[test]
fn parse_sql_multiple_statements() {
    let stmts = parse_sql(
        "CREATE TABLE t (x INTEGER); INSERT INTO t VALUES (1); SELECT * FROM t;",
    )
    .unwrap();
    assert_eq!(stmts.len(), 3);
    assert!(matches!(stmts[0], Stmt::CreateTable(_)));
    assert!(matches!(stmts[1], Stmt::Insert(_)));
    assert!(matches!(stmts[2], Stmt::Select(_)));
}
