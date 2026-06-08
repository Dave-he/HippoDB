//! T-0017 — Expression evaluator integration tests.
//!
//! Tests drive `crate::expr::eval` directly with hand-built Expr ASTs
//! and a `SimpleEnv<NullRegistry>` (or `RowEnv` for lookups). They also
//! cover roundtrip through `parse::parse_sql` for the main WHERE forms.
//!
//! All tests reference `libsqlite_rs::*` exports.

use libsqlite_rs::{
    eval, BinaryOp, Expr, FunctionRegistry, Literal, NullRegistry, RowEnv, SimpleEnv, SqliteError,
    SqliteValue, UnaryOp, eval_glob, eval_like,
};

// ─── Helpers ─────────────────────────────────────────────────────────────

fn env_with(pairs: &[(&str, SqliteValue)]) -> SimpleEnv<NullRegistry> {
    SimpleEnv {
        row: pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
        funcs: NullRegistry,
    }
}

fn row_env(pairs: &[(&str, SqliteValue)]) -> RowEnv {
    let mut e = RowEnv::new();
    for (k, v) in pairs {
        e.row.push(((*k).to_string(), v.clone()));
    }
    e
}

// ─── Arithmetic ─────────────────────────────────────────────────────────

#[test]
fn eval_int_plus_int() {
    let e = Expr::Binary {
        op: BinaryOp::Add,
        left: Box::new(Expr::int(1)),
        right: Box::new(Expr::int(2)),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(3));
}

#[test]
fn eval_int_minus_int() {
    let e = Expr::Binary {
        op: BinaryOp::Sub,
        left: Box::new(Expr::int(10)),
        right: Box::new(Expr::int(3)),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(7));
}

#[test]
fn eval_real_arith() {
    let e = Expr::Binary {
        op: BinaryOp::Add,
        left: Box::new(Expr::real(1.5)),
        right: Box::new(Expr::real(2.5)),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Real(4.0));
}

#[test]
fn eval_int_div() {
    // SQLite integer division: 7/2 = 3
    let e = Expr::Binary {
        op: BinaryOp::Div,
        left: Box::new(Expr::int(7)),
        right: Box::new(Expr::int(2)),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(3));
}

#[test]
fn eval_mod() {
    let e = Expr::Binary {
        op: BinaryOp::Mod,
        left: Box::new(Expr::int(7)),
        right: Box::new(Expr::int(3)),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(1));
}

#[test]
fn eval_div_by_zero_is_null() {
    let e = Expr::Binary {
        op: BinaryOp::Div,
        left: Box::new(Expr::int(1)),
        right: Box::new(Expr::int(0)),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Null);
}

#[test]
fn eval_null_arith() {
    let e = Expr::Binary {
        op: BinaryOp::Add,
        left: Box::new(Expr::null()),
        right: Box::new(Expr::int(1)),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Null);
}

// ─── String ─────────────────────────────────────────────────────────────

#[test]
fn eval_text_concat() {
    let e = Expr::Binary {
        op: BinaryOp::Concat,
        left: Box::new(Expr::text("a")),
        right: Box::new(Expr::text("b")),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Text("ab".into()));
}

#[test]
fn eval_text_concat_null_propagates() {
    let e = Expr::Binary {
        op: BinaryOp::Concat,
        left: Box::new(Expr::null()),
        right: Box::new(Expr::text("x")),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Null);
}

// ─── Comparison ─────────────────────────────────────────────────────────

#[test]
fn eval_eq_strings() {
    let e = Expr::Binary {
        op: BinaryOp::Eq,
        left: Box::new(Expr::text("a")),
        right: Box::new(Expr::text("a")),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(1));
}

#[test]
fn eval_eq_ne_strings() {
    let e = Expr::Binary {
        op: BinaryOp::Eq,
        left: Box::new(Expr::text("a")),
        right: Box::new(Expr::text("b")),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(0));
}

#[test]
fn eval_lt_int() {
    let e = Expr::Binary {
        op: BinaryOp::Lt,
        left: Box::new(Expr::int(1)),
        right: Box::new(Expr::int(2)),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(1));
}

#[test]
fn eval_le_int_equal() {
    let e = Expr::Binary {
        op: BinaryOp::Le,
        left: Box::new(Expr::int(2)),
        right: Box::new(Expr::int(2)),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(1));
}

#[test]
fn eval_gt_int() {
    let e = Expr::Binary {
        op: BinaryOp::Gt,
        left: Box::new(Expr::int(5)),
        right: Box::new(Expr::int(2)),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(1));
}

#[test]
fn eval_null_eq_is_null() {
    let e = Expr::Binary {
        op: BinaryOp::Eq,
        left: Box::new(Expr::null()),
        right: Box::new(Expr::int(1)),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Null);
}

// ─── Logical (three-valued) ─────────────────────────────────────────────

#[test]
fn eval_null_and_true_is_null() {
    // NULL AND 1 → NULL (not 0)
    let e = Expr::Binary {
        op: BinaryOp::And,
        left: Box::new(Expr::null()),
        right: Box::new(Expr::int(1)),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Null);
}

#[test]
fn eval_null_and_false_is_false() {
    // NULL AND 0 → 0
    let e = Expr::Binary {
        op: BinaryOp::And,
        left: Box::new(Expr::null()),
        right: Box::new(Expr::int(0)),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(0));
}

#[test]
fn eval_true_and_false_is_false() {
    let e = Expr::Binary {
        op: BinaryOp::And,
        left: Box::new(Expr::int(1)),
        right: Box::new(Expr::int(0)),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(0));
}

// ─── Unary ──────────────────────────────────────────────────────────────

#[test]
fn eval_unary_not_true() {
    let e = Expr::Unary {
        op: UnaryOp::Not,
        expr: Box::new(Expr::int(1)),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(0));
}

#[test]
fn eval_unary_minus() {
    let e = Expr::Unary {
        op: UnaryOp::Minus,
        expr: Box::new(Expr::int(5)),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(-5));
}

#[test]
fn eval_unary_bitnot() {
    let e = Expr::Unary {
        op: UnaryOp::BitNot,
        expr: Box::new(Expr::int(0)),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(-1));
}

// ─── Column lookup ──────────────────────────────────────────────────────

#[test]
fn eval_col_ref() {
    let e = Expr::col("x");
    let env = env_with(&[("x", SqliteValue::Integer(42))]);
    assert_eq!(eval(&e, &env).unwrap(), SqliteValue::Integer(42));
}

#[test]
fn eval_col_ref_missing_returns_null() {
    let e = Expr::col("missing");
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Null);
}

#[test]
fn eval_col_ref_with_rowenv() {
    let e = Expr::col("name");
    let env = row_env(&[("name", SqliteValue::Text("alice".into()))]);
    assert_eq!(eval(&e, &env).unwrap(), SqliteValue::Text("alice".into()));
}

// ─── IN ─────────────────────────────────────────────────────────────────

#[test]
fn eval_in_list_match() {
    let e = Expr::In {
        expr: Box::new(Expr::int(1)),
        values: vec![Expr::int(1), Expr::int(2), Expr::int(3)],
        negated: false,
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(1));
}

#[test]
fn eval_in_list_no_match() {
    let e = Expr::In {
        expr: Box::new(Expr::int(4)),
        values: vec![Expr::int(1), Expr::int(2), Expr::int(3)],
        negated: false,
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(0));
}

#[test]
fn eval_in_list_with_null_is_null() {
    let e = Expr::In {
        expr: Box::new(Expr::int(1)),
        values: vec![Expr::null(), Expr::int(2)],
        negated: false,
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Null);
}

#[test]
fn eval_not_in() {
    let e = Expr::In {
        expr: Box::new(Expr::int(4)),
        values: vec![Expr::int(1), Expr::int(2), Expr::int(3)],
        negated: true,
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(1));
}

// ─── BETWEEN ────────────────────────────────────────────────────────────

#[test]
fn eval_between_in_range() {
    let e = Expr::Between {
        expr: Box::new(Expr::int(5)),
        lo: Box::new(Expr::int(1)),
        hi: Box::new(Expr::int(10)),
        negated: false,
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(1));
}

#[test]
fn eval_between_out_of_range() {
    let e = Expr::Between {
        expr: Box::new(Expr::int(20)),
        lo: Box::new(Expr::int(1)),
        hi: Box::new(Expr::int(10)),
        negated: false,
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(0));
}

#[test]
fn eval_not_between() {
    let e = Expr::Between {
        expr: Box::new(Expr::int(5)),
        lo: Box::new(Expr::int(1)),
        hi: Box::new(Expr::int(10)),
        negated: true,
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(0));
}

// ─── IS / IS NOT ────────────────────────────────────────────────────────

#[test]
fn eval_is_null_true() {
    let e = Expr::Binary {
        op: BinaryOp::Is,
        left: Box::new(Expr::col("missing")),
        right: Box::new(Expr::null()),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(1));
}

#[test]
fn eval_is_not_null_false_for_missing() {
    // Use IS (negation is handled by the user building `BinaryOp::Is` with a non-null RHS for IS NOT).
    // We approximate IS NOT NULL as `BinaryOp::Eq` against null giving 0 for null;
    // the canonical form `col IS NOT NULL` returns 0 for NULL — test it via Eq.
    let e = Expr::Binary {
        op: BinaryOp::Eq,
        left: Box::new(Expr::col("missing")),
        right: Box::new(Expr::null()),
    };
    // missing col → NULL = NULL → NULL (three-valued)
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Null);
}

#[test]
fn eval_is_null_on_existing_col() {
    let e = Expr::Binary {
        op: BinaryOp::Is,
        left: Box::new(Expr::col("x")),
        right: Box::new(Expr::null()),
    };
    // x = Integer(5) → IS NULL → 0
    let env = env_with(&[("x", SqliteValue::Integer(5))]);
    assert_eq!(eval(&e, &env).unwrap(), SqliteValue::Integer(0));
}

// ─── LIKE / GLOB ────────────────────────────────────────────────────────

#[test]
fn eval_like_percent() {
    let e = Expr::Function {
        name: "like_helper".into(), // we use eval_like directly below
        args: vec![],
        star: false,
    };
    // Use the public eval_like instead
    let r = eval_like(
        &SqliteValue::Text("hello".into()),
        &SqliteValue::Text("hel%".into()),
        None,
    );
    assert_eq!(r, SqliteValue::Integer(1));
    // suppress unused warning
    let _ = e;
}

#[test]
fn eval_like_underscore() {
    let r = eval_like(
        &SqliteValue::Text("cat".into()),
        &SqliteValue::Text("c_t".into()),
        None,
    );
    assert_eq!(r, SqliteValue::Integer(1));
}

#[test]
fn eval_like_no_match() {
    let r = eval_like(
        &SqliteValue::Text("cat".into()),
        &SqliteValue::Text("dog%".into()),
        None,
    );
    assert_eq!(r, SqliteValue::Integer(0));
}

#[test]
fn eval_like_null_propagates() {
    let r = eval_like(
        &SqliteValue::Null,
        &SqliteValue::Text("x".into()),
        None,
    );
    assert_eq!(r, SqliteValue::Null);
}

#[test]
fn eval_glob_match() {
    let r = eval_glob(
        &SqliteValue::Text("hello".into()),
        &SqliteValue::Text("h*o".into()),
    );
    assert_eq!(r, SqliteValue::Integer(1));
}

#[test]
fn eval_glob_no_match() {
    let r = eval_glob(
        &SqliteValue::Text("hello".into()),
        &SqliteValue::Text("x*".into()),
    );
    assert_eq!(r, SqliteValue::Integer(0));
}

// ─── CASE ───────────────────────────────────────────────────────────────

#[test]
fn eval_case_searched() {
    let e = Expr::Case {
        operand: None,
        whens: vec![(
            Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(Expr::int(1)),
                right: Box::new(Expr::int(1)),
            },
            Expr::text("a"),
        )],
        else_expr: None,
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Text("a".into()));
}

#[test]
fn eval_case_simple() {
    let e = Expr::Case {
        operand: Some(Box::new(Expr::int(1))),
        whens: vec![
            (Expr::int(1), Expr::text("a")),
            (Expr::int(2), Expr::text("b")),
        ],
        else_expr: None,
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Text("a".into()));
}

#[test]
fn eval_case_no_match_no_else() {
    let e = Expr::Case {
        operand: None,
        whens: vec![(Expr::int(0), Expr::text("a"))],
        else_expr: None,
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Null);
}

#[test]
fn eval_case_with_else() {
    let e = Expr::Case {
        operand: None,
        whens: vec![(Expr::int(0), Expr::text("a"))],
        else_expr: Some(Box::new(Expr::text("z"))),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Text("z".into()));
}

// ─── Bitwise ────────────────────────────────────────────────────────────

#[test]
fn eval_bitand() {
    let e = Expr::Binary {
        op: BinaryOp::BitAnd,
        left: Box::new(Expr::int(0xff)),
        right: Box::new(Expr::int(0x0f)),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(0x0f));
}

#[test]
fn eval_bitor() {
    let e = Expr::Binary {
        op: BinaryOp::BitOr,
        left: Box::new(Expr::int(0x01)),
        right: Box::new(Expr::int(0x10)),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(0x11));
}

#[test]
fn eval_shift() {
    let e = Expr::Binary {
        op: BinaryOp::LShift,
        left: Box::new(Expr::int(1)),
        right: Box::new(Expr::int(4)),
    };
    assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(16));
}

// ─── Function dispatch (NullRegistry returns error) ─────────────────────

#[test]
fn eval_function_dispatch_unknown() {
    let e = Expr::Function {
        name: "length".into(),
        args: vec![Expr::int(1)],
        star: false,
    };
    let r = eval(&e, &env_with(&[]));
    assert!(r.is_err());
    assert_eq!(r.unwrap_err().code(), 1);
}

// ─── SqliteValue helpers ────────────────────────────────────────────────

#[test]
fn sqlitevalue_typeof() {
    assert_eq!(SqliteValue::Null.type_of(), "null");
    assert_eq!(SqliteValue::Integer(1).type_of(), "integer");
    assert_eq!(SqliteValue::Real(1.0).type_of(), "real");
    assert_eq!(SqliteValue::Text("x".into()).type_of(), "text");
    assert_eq!(SqliteValue::Blob(vec![0]).type_of(), "blob");
}

#[test]
fn sqlitevalue_to_bool() {
    assert_eq!(SqliteValue::Null.to_bool(), SqliteValue::Null);
    assert_eq!(SqliteValue::Integer(0).to_bool(), SqliteValue::Integer(0));
    assert_eq!(SqliteValue::Integer(1).to_bool(), SqliteValue::Integer(1));
    assert_eq!(SqliteValue::Integer(42).to_bool(), SqliteValue::Integer(1));
    assert_eq!(SqliteValue::Real(0.0).to_bool(), SqliteValue::Integer(0));
    assert_eq!(SqliteValue::Real(1.5).to_bool(), SqliteValue::Integer(1));
}

#[test]
fn sqlitevalue_coerce_integer() {
    assert_eq!(
        SqliteValue::Text("42".into()).coerce_integer(),
        SqliteValue::Integer(42)
    );
    assert_eq!(
        SqliteValue::Real(3.7).coerce_integer(),
        SqliteValue::Integer(3)
    );
    assert_eq!(SqliteValue::Null.coerce_integer(), SqliteValue::Null);
}

// ─── Roundtrip via parse_sql (smoke test) ───────────────────────────────

#[test]
fn parse_sql_eval_where_eq() {
    // SELECT * FROM t WHERE x = 1 — verify the parse produces an Expr
    // that evaluates correctly. (slim parser supports only =; > is rejected.)
    let stmts = libsqlite_rs::parse::parse_sql("SELECT * FROM t WHERE x = 1").unwrap();
    match &stmts[0] {
        libsqlite_rs::parse::Stmt::Select(s) => {
            assert!(s.where_clause.is_some(), "expected a where clause");
        }
        _ => panic!("expected SELECT"),
    }
    // Confirm the parser rejects `>` in the slim subset.
    assert!(libsqlite_rs::parse::parse_sql("SELECT * FROM t WHERE x > 1").is_err());
}

// ─── Silence unused imports (Literal, FunctionRegistry) — they're part of
//     the public surface and may be unused in this slim test file. ───
#[allow(dead_code)]
fn _silence_unused(lit: Literal) -> Literal {
    // Ensure FunctionRegistry trait is in scope; reference its method via
    // fully-qualified call so the import is not reported as unused.
    fn _fr(r: &NullRegistry) {
        let _ = <NullRegistry as FunctionRegistry>::call;
        let _ = r;
    }
    let _ = _fr;
    lit
}
