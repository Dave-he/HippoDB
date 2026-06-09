//! Coverage tests for the public helper functions in `libsqlite_rs` that
//! were exposed but not directly tested in the per-module inline tests.
//!
//! Run with: `cargo test --test resolve_helpers`

use libsqlite_rs::SqliteError;
use libsqlite_rs::parse::{WhereExpr, WhereOp, Value};
use libsqlite_rs::resolve::validate_where_tree;

#[test]
fn validate_where_tree_accepts_known_column() {
    let cols = vec!["x".to_string(), "y".to_string()];
    let w = WhereExpr::Cmp {
        op: WhereOp::Eq,
        column: "x".to_string(),
        value: Value::Integer(1),
    };
    assert!(validate_where_tree(&w, &cols).is_ok());
}

#[test]
fn validate_where_tree_rejects_unknown_column() {
    let cols = vec!["x".to_string()];
    let w = WhereExpr::Cmp {
        op: WhereOp::Eq,
        column: "z".to_string(),
        value: Value::Integer(1),
    };
    let r = validate_where_tree(&w, &cols);
    assert!(r.is_err());
    assert_eq!(r.unwrap_err().code(), 1); // SQLITE_ERROR
}

#[test]
fn validate_where_tree_rejects_identifier_rhs() {
    let cols = vec!["x".to_string(), "y".to_string()];
    let w = WhereExpr::Cmp {
        op: WhereOp::Eq,
        column: "x".to_string(),
        value: Value::Identifier("y".to_string()),
    };
    let r = validate_where_tree(&w, &cols);
    assert!(r.is_err());
    // slim subset message: "WHERE with identifier RHS not supported"
    assert_eq!(r.unwrap_err().code(), 1);
}

#[test]
fn validate_where_tree_walks_and() {
    let cols = vec!["a".to_string(), "b".to_string()];
    let w = WhereExpr::And(
        Box::new(WhereExpr::Cmp {
            op: WhereOp::Eq,
            column: "a".to_string(),
            value: Value::Integer(1),
        }),
        Box::new(WhereExpr::Cmp {
            op: WhereOp::Gt,
            column: "b".to_string(),
            value: Value::Integer(2),
        }),
    );
    assert!(validate_where_tree(&w, &cols).is_ok());
}

#[test]
fn validate_where_tree_walks_or() {
    let cols = vec!["a".to_string()];
    let w = WhereExpr::Or(
        Box::new(WhereExpr::Cmp {
            op: WhereOp::Eq,
            column: "a".to_string(),
            value: Value::Integer(1),
        }),
        Box::new(WhereExpr::Cmp {
            op: WhereOp::Eq,
            column: "missing".to_string(), // typo on the OR branch
            value: Value::Integer(2),
        }),
    );
    assert!(validate_where_tree(&w, &cols).is_err());
}

#[test]
fn validate_where_tree_deeply_nested() {
    // (a=1 AND (b=2 OR unknown=3)) AND c=4
    let cols = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    let inner_or = WhereExpr::Or(
        Box::new(WhereExpr::Cmp {
            op: WhereOp::Eq,
            column: "b".to_string(),
            value: Value::Integer(2),
        }),
        Box::new(WhereExpr::Cmp {
            op: WhereOp::Eq,
            column: "unknown".to_string(),
            value: Value::Integer(3),
        }),
    );
    let and1 = WhereExpr::And(
        Box::new(WhereExpr::Cmp {
            op: WhereOp::Eq,
            column: "a".to_string(),
            value: Value::Integer(1),
        }),
        Box::new(inner_or),
    );
    let top = WhereExpr::And(
        Box::new(and1),
        Box::new(WhereExpr::Cmp {
            op: WhereOp::Eq,
            column: "c".to_string(),
            value: Value::Integer(4),
        }),
    );
    assert!(validate_where_tree(&top, &cols).is_err());
}

#[test]
fn validate_where_tree_handles_all_cmp_ops() {
    let cols = vec!["x".to_string()];
    for op in [WhereOp::Eq, WhereOp::Ne, WhereOp::Lt, WhereOp::Le, WhereOp::Gt, WhereOp::Ge] {
        let w = WhereExpr::Cmp {
            op,
            column: "x".to_string(),
            value: Value::Integer(0),
        };
        assert!(validate_where_tree(&w, &cols).is_ok(), "op {op:?} should be valid");
    }
}

#[test]
fn validate_where_tree_handles_string_and_null_rhs() {
    let cols = vec!["name".to_string()];
    let w_str = WhereExpr::Cmp {
        op: WhereOp::Eq,
        column: "name".to_string(),
        value: Value::String("alice".to_string()),
    };
    assert!(validate_where_tree(&w_str, &cols).is_ok());

    let w_null = WhereExpr::Cmp {
        op: WhereOp::Eq,
        column: "name".to_string(),
        value: Value::Null,
    };
    assert!(validate_where_tree(&w_null, &cols).is_ok());
}

#[test]
fn sqlite_error_with_msg_preserves_code() {
    // with_msg is currently a no-op that returns self; this test pins
    // the contract: it must preserve the original error code.
    let e = SqliteError::ERROR.with_msg("context".to_string());
    assert_eq!(e.code(), 1);

    let nomem = SqliteError::NOMEM.with_msg("OOM".to_string());
    assert_eq!(nomem.code(), 7);

    let ok = SqliteError::OK.with_msg("ignored".to_string());
    assert_eq!(ok.code(), 0);
    assert!(ok.is_ok());
}
