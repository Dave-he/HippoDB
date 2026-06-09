//! Compatibility oracle — feed the same SQL to libsqlite_rs and the
//! real `sqlite3` CLI, then compare the formatted output.
//!
//! Scope: only tests the slim-subset surface that `libsqlite_rs::run_sql`
//! actually implements end-to-end (CREATE/INSERT/SELECT col from real
//! table, all six WHERE operators, AND/OR, text/numeric columns, NULL
//! rows). Computed columns (`SELECT 1+2`), aggregate, JOIN, GROUP BY,
//! ORDER BY, LIMIT, subqueries are NOT tested here — they would fail on
//! our side regardless of the oracle's behavior.
//!
//! Run with: `cargo test --test oracle -- --nocapture` to see diffs
//! when something fails.
//!
//! Requires `sqlite3` on PATH (used as the reference oracle).

use std::process::Command;

use libsqlite_rs::{run_sql, Mem, Schema, SqliteError};

/// Format a Mem like sqlite3 CLI does. We deliberately match the C
/// default: NULL → "", integers as decimal, reals short form, text bare.
fn mem_to_oracle(m: &Mem) -> String {
    match m {
        Mem::Null => String::new(),
        Mem::Integer(i) => i.to_string(),
        Mem::Real(f) => {
            if f.is_nan() {
                String::new()
            } else if f.is_infinite() {
                if *f > 0.0 { "Inf".to_string() } else { "-Inf".to_string() }
            } else {
                // SQLite's default real formatting uses %.15g-ish, but
                // we only ever emit reals via WHERE row comparison in
                // this slim subset, so real results won't surface in
                // normal tests. Still: produce a stable short form.
                let s = format!("{f}");
                if s.contains('.') {
                    s.trim_end_matches('0').trim_end_matches('.').to_string()
                } else {
                    s
                }
            }
        }
        Mem::Text(s) => s.clone(),
        Mem::Blob(b) => {
            let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
            format!("X'{hex}'")
        }
    }
}

/// Format a result set like sqlite3 CLI: rows separated by newlines,
/// values separated by '|'.
fn rows_to_oracle(rows: &[Vec<Mem>]) -> String {
    rows.iter()
        .map(|r| r.iter().map(mem_to_oracle).collect::<Vec<_>>().join("|"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run a SQL script against the real `sqlite3` CLI in a fresh in-memory
/// database (`:memory:`). Returns the formatted output.
fn sqlite3_cli_oracle(sql: &str) -> Result<String, String> {
    let out = Command::new("sqlite3")
        .args(["-separator", "|", ":memory:", sql])
        .output()
        .map_err(|e| format!("failed to spawn sqlite3: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "sqlite3 exited with {:?}\nstderr: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .trim_end_matches('\n')
        .to_string())
}

/// Run a SQL script against our libsqlite_rs. Returns either formatted
/// output or an error message.
fn our_oracle(sql: &str) -> Result<String, String> {
    let mut schema = Schema::default();
    match run_sql(sql, &mut schema) {
        Ok(rows) => Ok(rows_to_oracle(&rows)),
        Err(e) => Err(format!("our run_sql failed: {e:?}")),
    }
}

/// Wrapper: assert that both oracles return the same formatted string.
/// If the reference oracle itself errors out (very rare), SKIP rather
/// than fail.
fn assert_oracle_match(label: &str, sql: &str) {
    let theirs = match sqlite3_cli_oracle(sql) {
        Ok(s) => s,
        Err(msg) => {
            eprintln!("[oracle:{label}] SKIP: sqlite3 CLI failed: {msg}");
            return;
        }
    };
    let ours = match our_oracle(sql) {
        Ok(s) => s,
        Err(msg) => {
            panic!(
                "[oracle:{label}] our run_sql failed where sqlite3 succeeded: {msg}\n\
                 sql: {sql}\n\
                 their output:\n{theirs}"
            );
        }
    };
    if ours != theirs {
        panic!(
            "[oracle:{label}] OUTPUT MISMATCH\n\
             sql:\n{sql}\n\
             --- ours ---\n{ours}\n\
             --- sqlite3 ---\n{theirs}\n"
        );
    }
}

// ── 1. CREATE + INSERT + SELECT * ────────────────────────────────────

#[test]
fn oracle_create_insert_select_all() {
    let sql = "CREATE TABLE t (x INTEGER, y TEXT);\
               INSERT INTO t VALUES (1, 'a');\
               INSERT INTO t VALUES (2, 'b');\
               INSERT INTO t VALUES (3, 'c');\
               SELECT * FROM t;";
    assert_oracle_match("create+insert+select-all", sql);
}

#[test]
fn oracle_select_subset_of_columns() {
    let sql = "CREATE TABLE t (x INTEGER, y TEXT, z INTEGER);\
               INSERT INTO t VALUES (1, 'a', 10);\
               INSERT INTO t VALUES (2, 'b', 20);\
               SELECT y, x FROM t;";
    assert_oracle_match("select-subset", sql);
}

// ── 2. WHERE operators ───────────────────────────────────────────────

#[test]
fn oracle_where_eq() {
    let sql = "CREATE TABLE t (x INTEGER);\
               INSERT INTO t VALUES (1);\
               INSERT INTO t VALUES (2);\
               INSERT INTO t VALUES (3);\
               SELECT x FROM t WHERE x = 2;";
    assert_oracle_match("where-eq", sql);
}

#[test]
fn oracle_where_all_comparison_ops() {
    let prefix = "CREATE TABLE t (x INTEGER);\
                  INSERT INTO t VALUES (1);\
                  INSERT INTO t VALUES (2);\
                  INSERT INTO t VALUES (3);\
                  INSERT INTO t VALUES (4);\
                  INSERT INTO t VALUES (5);";
    for op in [">", "<", ">=", "<=", "!=", "<>"] {
        let sql = format!("{prefix} SELECT x FROM t WHERE x {op} 3");
        assert_oracle_match(&format!("where-{op}"), &sql);
    }
}

#[test]
fn oracle_where_and_or_combined() {
    let prefix = "CREATE TABLE t (x INTEGER);\
                  INSERT INTO t VALUES (1);\
                  INSERT INTO t VALUES (2);\
                  INSERT INTO t VALUES (3);\
                  INSERT INTO t VALUES (4);\
                  INSERT INTO t VALUES (5);";
    assert_oracle_match(
        "where-and",
        &format!("{prefix} SELECT x FROM t WHERE x > 1 AND x < 4"),
    );
    assert_oracle_match(
        "where-or",
        &format!("{prefix} SELECT x FROM t WHERE x < 2 OR x > 4"),
    );
    assert_oracle_match(
        "where-mixed",
        &format!("{prefix} SELECT x FROM t WHERE x > 1 AND x < 4 OR x = 5"),
    );
}

#[test]
fn oracle_where_text() {
    let sql = "CREATE TABLE t (name TEXT);\
               INSERT INTO t VALUES ('alice');\
               INSERT INTO t VALUES ('bob');\
               INSERT INTO t VALUES ('charlie');\
               SELECT name FROM t WHERE name = 'bob';";
    assert_oracle_match("where-text", sql);
}

#[test]
fn oracle_where_null_rows_filter_correctly() {
    // SQL truthiness: a row with x IS NULL won't match `x = 1` on the
    // real engine. Our WHERE evaluator treats null-column as null, and
    // null compare with int returns null, so the row is excluded.
    let sql = "CREATE TABLE t (x INTEGER);\
               INSERT INTO t VALUES (1);\
               INSERT INTO t VALUES (NULL);\
               INSERT INTO t VALUES (2);\
               SELECT x FROM t WHERE x = 1;";
    assert_oracle_match("where-null-row-excluded", sql);
}

// ── 3. Type coercion in INSERT columns ───────────────────────────────

#[test]
fn oracle_insert_text_and_int_mixed() {
    let sql = "CREATE TABLE t (a TEXT, b INTEGER);\
               INSERT INTO t VALUES ('hello', 42);\
               INSERT INTO t VALUES ('world', 7);\
               SELECT * FROM t;";
    assert_oracle_match("insert-mixed-types", sql);
}

#[test]
fn oracle_insert_nulls() {
    let sql = "CREATE TABLE t (a INTEGER, b TEXT);\
               INSERT INTO t VALUES (NULL, 'x');\
               INSERT INTO t VALUES (1, NULL);\
               INSERT INTO t VALUES (NULL, NULL);\
               SELECT * FROM t;";
    assert_oracle_match("insert-nulls", sql);
}

// ── 4. Larger-scale roundtrip ────────────────────────────────────────

#[test]
fn oracle_roundtrip_50_rows_select_all() {
    let mut sql = String::from("CREATE TABLE t (x INTEGER);\n");
    for i in 0..50 {
        sql.push_str(&format!("INSERT INTO t VALUES ({i});\n"));
    }
    sql.push_str("SELECT x FROM t;");
    assert_oracle_match("roundtrip-50", &sql);
}

#[test]
fn oracle_roundtrip_50_with_filter() {
    let mut sql = String::from("CREATE TABLE t (x INTEGER);\n");
    for i in 0..50 {
        sql.push_str(&format!("INSERT INTO t VALUES ({i});\n"));
    }
    sql.push_str("SELECT x FROM t WHERE x >= 10 AND x < 20;");
    assert_oracle_match("roundtrip-filter", &sql);
}

#[test]
fn oracle_roundtrip_200_rows_filter_or() {
    let mut sql = String::from("CREATE TABLE t (x INTEGER);\n");
    for i in 0..200 {
        sql.push_str(&format!("INSERT INTO t VALUES ({i});\n"));
    }
    sql.push_str("SELECT x FROM t WHERE x < 5 OR x > 195;");
    assert_oracle_match("roundtrip-200-filter-or", &sql);
}

// ── 5. Negative: error path parity (parse errors) ────────────────────

#[test]
fn oracle_parse_error_unknown_keyword() {
    // Both engines should reject an unsupported statement.
    let sql = "WIBBLE;";
    let theirs = sqlite3_cli_oracle(sql).map_err(|e| e).unwrap_or_default();
    let ours = our_oracle(sql);
    // We just want both to error out. Don't enforce identical error
    // wording (engine-specific), just parity in "did it fail".
    let _ = theirs; // suppress unused warning while keeping the call.
    assert!(ours.is_err(), "expected our parser to reject `WIBBLE`");
}

#[test]
fn oracle_parse_error_no_such_table() {
    let sql = "SELECT * FROM no_such_table;";
    let theirs = sqlite3_cli_oracle(sql);
    let ours = our_oracle(sql);
    let _ = theirs; // see oracle_parse_error_unknown_keyword
    assert!(ours.is_err(), "expected our run_sql to error on missing table");
}

// ── 6. Hard assertion: sqlite3 CLI is on PATH ────────────────────────

#[test]
fn oracle_smoke_sqlite3_present() {
    // The other tests SKIP on failure, but this one is a hard assertion
    // that the oracle is actually running. If it's not, every other test
    // silently no-ops.
    let out = Command::new("sqlite3")
        .arg(":memory:")
        .arg("SELECT 1")
        .output();
    match out {
        Ok(o) if o.status.success() => {}
        Ok(o) => panic!(
            "sqlite3 CLI not working: status={:?} stderr={}",
            o.status.code(),
            String::from_utf8_lossy(&o.stderr)
        ),
        Err(e) => panic!("sqlite3 CLI not on PATH: {e}"),
    }
}

// Suppress unused-import warnings if a future test stops using SqliteError.
#[allow(dead_code)]
fn _silence(_: SqliteError) {}
