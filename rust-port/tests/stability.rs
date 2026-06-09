//! Stability / fuzz tests — assert that the engine survives bad or
//! unusual input without panicking. Errors are expected and OK; panics
//! are not.
//!
//! Run with: `cargo test --test stability`
//!
//! The fuzz test is `#[ignore]`'d by default because it runs 10k random
//! SQL strings; pass it explicitly with:
//!
//!     cargo test --test stability -- --ignored --nocapture

use libsqlite_rs::{run_sql, Schema};

use libsqlite_rs::parse::parse_sql;
use libsqlite_rs::tokenize::{tokenize, TokenKind};

/// Helper: assert that `body` either runs to completion (Ok) or
/// returns a `SqliteError`. A panic is a test failure. The closure
/// must return `()`; if you need to discard a `Result`, do it inside
/// (e.g. `let _ = foo();`).
fn assert_no_panic(label: &str, body: impl FnOnce() + std::panic::UnwindSafe) {
    let result = std::panic::catch_unwind(body);
    match result {
        Ok(()) => {}
        Err(_) => panic!("[stability:{label}] PANICKED — that's a stability bug"),
    }
}

// ── 1. Empty / degenerate inputs ─────────────────────────────────────

#[test]
fn empty_string() {
    assert_no_panic("empty-string", || {
        let _ = parse_sql("");
    });
}

#[test]
fn just_semicolons() {
    assert_no_panic("just-semis", || {
        let _ = parse_sql(";;;;;;");
    });
}

#[test]
fn single_space() {
    assert_no_panic("single-space", || {
        let _ = parse_sql(" ");
    });
}

#[test]
fn whitespace_only() {
    assert_no_panic("ws-only", || {
        let _ = parse_sql(" \t\n\r  ");
    });
}

#[test]
fn nonsense_keywords() {
    for s in &[
        "WIBBLE", "FOO BAR BAZ", "SELECTOR", "ATABLE", "X'Y'Z'",
        "SELECT WHERE", "FROM TO", "CREATE DROP",
    ] {
        let s = *s;
        assert_no_panic(s, || {
            let _ = parse_sql(s);
        });
    }
}

#[test]
fn truncated_statements() {
    for s in &[
        "SELECT", "SELECT *", "SELECT * FROM", "INSERT", "INSERT INTO",
        "INSERT INTO t", "INSERT INTO t VALUES", "INSERT INTO t VALUES (",
        "CREATE", "CREATE TABLE", "CREATE TABLE t", "CREATE TABLE t (",
        "DROP", "DROP TABLE", "WHERE x", "WHERE x =", "WHERE x =;",
    ] {
        let s = *s;
        assert_no_panic(s, || {
            let _ = parse_sql(s);
        });
    }
}

// ── 2. Unicode handling ──────────────────────────────────────────────

#[test]
fn unicode_in_string_literal() {
    // 中文 / emoji / mixed scripts — tokenizer should preserve bytes
    let sql = "SELECT * FROM t WHERE name = '你好世界 🌍 hello'";
    let toks = tokenize(sql).expect("tokenize");
    let text_token = toks
        .iter()
        .find(|t| matches!(t.kind, TokenKind::String))
        .expect("should have a string token");
    if let TokenKind::String = text_token.kind {
        // text_token.text contains the raw content; we just want to
        // assert the round-trip didn't lose the unicode bytes.
        assert!(text_token.text.contains("你好世界"));
        assert!(text_token.text.contains("🌍"));
    }
}

#[test]
fn unicode_identifier_is_error_not_panic() {
    // Identifiers with non-ASCII — our slim parser requires the id to
    // be an `Id` token. If the tokenizer doesn't emit Id for unicode
    // (depends on its rules), parse will error cleanly. Either way,
    // no panic.
    let _ = parse_sql("SELECT 你好 FROM t");
    let _ = parse_sql("SELECT café FROM t");
}

// ── 3. Very long inputs ─────────────────────────────────────────────

#[test]
fn very_long_identifier() {
    let long_name: String = "a".repeat(10_000);
    let sql = format!("SELECT {long_name} FROM t");
    let _ = parse_sql(&sql);
}

#[test]
fn very_long_column_list() {
    let cols: Vec<String> = (0..1000).map(|i| format!("c{i}")).collect();
    let sql = format!("SELECT {} FROM t", cols.join(", "));
    let _ = parse_sql(&sql);
}

#[test]
fn deeply_nested_and_or() {
    // 100 levels of AND — should not stack-overflow (we don't recurse
    // in the parser beyond a small depth, and `WhereExpr` is heap-allocated
    // so it scales fine, but verify).
    let mut sql = String::from("SELECT * FROM t WHERE x = 1");
    for _ in 0..100 {
        sql.push_str(" AND x = 1");
    }
    let _ = parse_sql(&sql);
}

// ── 4. Numeric edge cases ────────────────────────────────────────────

#[test]
fn large_integer_literal() {
    // i64::MAX = 9223372036854775807. Slim parser parses as i64.
    // Values larger overflow → parse error (not panic).
    let _ = parse_sql("SELECT 9223372036854775807 FROM t"); // i64::MAX
    let _ = parse_sql("SELECT 9223372036854775808 FROM t"); // i64::MAX + 1
    let _ = parse_sql("SELECT -9223372036854775808 FROM t"); // i64::MIN
    let _ = parse_sql("SELECT 99999999999999999999999 FROM t"); // way too big
}

#[test]
fn numeric_string_edge() {
    let _ = parse_sql("SELECT 0 FROM t");
    let _ = parse_sql("SELECT 00 FROM t"); // leading zero
    let _ = parse_sql("SELECT 0.0 FROM t");
    let _ = parse_sql("SELECT .5 FROM t");
    let _ = parse_sql("SELECT 1. FROM t"); // trailing dot — likely error
    let _ = parse_sql("SELECT 1e10 FROM t"); // scientific — likely error
}

// ── 5. Repeated run stability (idempotence) ─────────────────────────

#[test]
fn select_is_idempotent() {
    let mut s = Schema::default();
    run_sql("CREATE TABLE t (x INTEGER)", &mut s).unwrap();
    for i in 0..20 {
        run_sql(&format!("INSERT INTO t VALUES ({i})"), &mut s).unwrap();
    }
    let r1 = run_sql("SELECT x FROM t", &mut s).unwrap();
    let r2 = run_sql("SELECT x FROM t", &mut s).unwrap();
    let r3 = run_sql("SELECT x FROM t", &mut s).unwrap();
    assert_eq!(r1, r2);
    assert_eq!(r2, r3);
    assert_eq!(r1.len(), 20);
}

#[test]
fn where_is_idempotent() {
    let mut s = Schema::default();
    run_sql("CREATE TABLE t (x INTEGER)", &mut s).unwrap();
    for i in 0..50 {
        run_sql(&format!("INSERT INTO t VALUES ({i})"), &mut s).unwrap();
    }
    let r1 = run_sql("SELECT x FROM t WHERE x > 25", &mut s).unwrap();
    let r2 = run_sql("SELECT x FROM t WHERE x > 25", &mut s).unwrap();
    assert_eq!(r1, r2);
    assert_eq!(r1.len(), 24); // 26..49
}

#[test]
fn repeated_parse_is_idempotent() {
    let sql = "SELECT a, b FROM t WHERE x > 1 AND y < 100";
    let r1 = parse_sql(sql).unwrap();
    let r2 = parse_sql(sql).unwrap();
    assert_eq!(r1, r2);
}

#[test]
fn repeated_tokenize_is_idempotent() {
    let sql = "SELECT * FROM t WHERE x > 1";
    let r1 = tokenize(sql).unwrap();
    let r2 = tokenize(sql).unwrap();
    assert_eq!(r1, r2);
}

// ── 6. E2E: schema survives multiple operations ─────────────────────

#[test]
fn create_insert_select_drop_create_insert_select() {
    let mut s = Schema::default();
    run_sql("CREATE TABLE t (x)", &mut s).unwrap();
    run_sql("INSERT INTO t VALUES (1)", &mut s).unwrap();
    run_sql("INSERT INTO t VALUES (2)", &mut s).unwrap();
    let r1 = run_sql("SELECT x FROM t", &mut s).unwrap();
    assert_eq!(r1.len(), 2);

    run_sql("DROP TABLE t", &mut s).unwrap();

    // Re-create with different shape — verify fresh state.
    run_sql("CREATE TABLE t (a TEXT, b TEXT)", &mut s).unwrap();
    run_sql("INSERT INTO t VALUES ('hi', 'there')", &mut s).unwrap();
    let r2 = run_sql("SELECT a, b FROM t", &mut s).unwrap();
    assert_eq!(r2.len(), 1);
    // First element should be Text("hi"), Text("there") — but we don't
    // peek at the Mem enum; we just check len.
}

// ── 7. Property: WHERE over a column of all NULLs ────────────────────

#[test]
fn where_all_nulls_returns_nothing() {
    let mut s = Schema::default();
    run_sql("CREATE TABLE t (x INTEGER)", &mut s).unwrap();
    run_sql("INSERT INTO t VALUES (NULL)", &mut s).unwrap();
    run_sql("INSERT INTO t VALUES (NULL)", &mut s).unwrap();
    let r = run_sql("SELECT x FROM t WHERE x = 1", &mut s).unwrap();
    assert!(r.is_empty());
}

#[test]
fn where_all_nulls_or_with_match() {
    // Even with NULLs, the OR short-circuit should still let the
    // matching row through.
    let mut s = Schema::default();
    run_sql("CREATE TABLE t (x INTEGER)", &mut s).unwrap();
    run_sql("INSERT INTO t VALUES (NULL)", &mut s).unwrap();
    run_sql("INSERT INTO t VALUES (1)", &mut s).unwrap();
    let r = run_sql("SELECT x FROM t WHERE x = 99 OR x = 1", &mut s).unwrap();
    assert_eq!(r.len(), 1);
}

// ── 8. Tokenize/parse roundtrip stability ───────────────────────────

#[test]
fn parse_tokenize_parse_consistent() {
    let sql = "SELECT a, b FROM t WHERE x > 1 AND y < 100";
    let stmts1 = parse_sql(sql).unwrap();
    let toks = tokenize(sql).unwrap();
    let sql2 = toks
        .iter()
        .map(|t| t.text.clone())
        .collect::<Vec<_>>()
        .join(" ");
    let stmts2 = parse_sql(&sql2).unwrap();
    assert_eq!(stmts1, stmts2);
}

// ── 9. Random SQL fuzz (heavy) ─────────────────────────────────────

/// Tiny random-SQL generator. Produces statements from a small grammar.
/// The point is variety, not validity.
struct SqlFuzzer {
    rng: u64,
}

impl SqlFuzzer {
    fn new(seed: u64) -> Self {
        Self { rng: seed }
    }

    fn next(&mut self) -> u64 {
        // xorshift64
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    fn pick<'a>(&mut self, items: &'a [&'a str]) -> &'a str {
        items[(self.next() as usize) % items.len()]
    }

    fn int(&mut self) -> String {
        ((self.next() % 200) as i64 - 100).to_string()
    }

    fn ident(&mut self) -> String {
        let n = 1 + (self.next() % 5) as usize;
        (0..n)
            .map(|i| {
                let c = (b'a' + ((self.next() >> (i * 4)) as u8 & 0xf) % 26) as char;
                c
            })
            .collect()
    }

    fn value(&mut self) -> String {
        match self.next() % 5 {
            0 => self.int(),
            1 => "NULL".to_string(),
            2 => format!("'{}'", self.ident()),
            3 => format!("'{} {}'", self.ident(), self.ident()),
            _ => self.int(),
        }
    }

    fn op(&mut self) -> &'static str {
        self.pick(&["=", "!=", "<>", "<", "<=", ">", ">="])
    }

    fn where_clause(&mut self) -> String {
        let depth = 1 + (self.next() % 3) as usize;
        let mut s = format!("{} {} {}", self.ident(), self.op(), self.value());
        for _ in 0..depth {
            let connector = self.pick(&["AND", "OR"]);
            s.push_str(&format!(" {connector} {} {} {}", self.ident(), self.op(), self.value()));
        }
        s
    }

    fn generate(&mut self) -> String {
        match self.next() % 5 {
            0 => {
                // bare statement
                self.pick(&[
                    "SELECT 1",
                    "SELECT * FROM t",
                    "SELECT x FROM t",
                    "SELECT x, y FROM t",
                    "INSERT INTO t VALUES (1)",
                    "INSERT INTO t VALUES (1, 2)",
                    "CREATE TABLE t (x)",
                    "CREATE TABLE t (x INTEGER)",
                    "DROP TABLE t",
                ])
                .to_string()
            }
            1 => {
                // SELECT with WHERE
                format!("SELECT * FROM t WHERE {}", self.where_clause())
            }
            2 => {
                // SELECT with literal
                let v = self.value();
                format!("SELECT {v} FROM t")
            }
            3 => {
                // Random keyword soup
                let s = self.pick(&[
                    "WIBBLE", "FOO", "BAR", "BAZ", "SELECTOR",
                    "FROMTO", "INSERTINTO", "WHEREEVER",
                ]);
                s.to_string()
            }
            _ => {
                // Empty / whitespace
                if self.next() % 2 == 0 {
                    String::new()
                } else {
                    "   \n\t  ".to_string()
                }
            }
        }
    }
}

#[test]
#[ignore]
fn fuzz_10k_random_sql() {
        let mut fuzzer = SqlFuzzer::new(0x1234_5678_DEAD_BEEF);
        let n = 10_000;
        let mut oks = 0;
        let mut errors = 0;
        let mut panicked = 0;
        for i in 0..n {
            let sql = fuzzer.generate();
            // panic catch is for stability: any panic is a bug, no matter
            // what the function returned. We swallow Result/panic into ().
            let result = std::panic::catch_unwind(|| {
                let _ = parse_sql(&sql);
            });
            match result {
                Ok(()) => {
                    // The body itself returned Ok(()); the parse_sql result
                    // was discarded. Distinguish parse_ok/parse_err by
                    // re-parsing (cheap) for the bookkeeping — won't panic.
                    match parse_sql(&sql) {
                        Ok(_) => oks += 1,
                        Err(_) => errors += 1,
                    }
                }
                Err(_) => {
                    panicked += 1;
                    eprintln!("[fuzz] PANIC at i={i} on sql=`{sql}`");
                    break;
                }
            }
        }
        eprintln!("[fuzz] {n} iters: {oks} ok, {errors} err, {panicked} panic");
        assert_eq!(panicked, 0, "fuzz found {panicked} panic(s) in {n} random SQLs");
    }

    #[test]
    #[ignore]
    fn fuzz_1k_random_sql_e2e() {
        // Run each generated SQL end-to-end through run_sql against a fresh
        // schema. Most should error (invalid column refs, etc.) but none
        // should panic.
        let mut fuzzer = SqlFuzzer::new(0xCAFE_F00D_BEEF_5678);
        let n = 1_000;
        let mut panicked = 0;
        for i in 0..n {
            let sql = fuzzer.generate();
            let result = std::panic::catch_unwind(|| {
                let mut s = Schema::default();
                // Pre-create a table to give the random SQL a chance to
                // do something useful, but most of it will error out.
                let _ = run_sql("CREATE TABLE t (x INTEGER, y INTEGER)", &mut s);
                let _ = run_sql("INSERT INTO t VALUES (1, 2)", &mut s);
                let _ = run_sql(&sql, &mut s);
            });
            if result.is_err() {
                panicked += 1;
                eprintln!("[fuzz-e2e] PANIC at i={i} on sql=`{sql}`");
                break;
            }
        }
        assert_eq!(panicked, 0, "e2e fuzz found {panicked} panic(s) in {n} SQLs");
        eprintln!("[fuzz-e2e] {n} iters: 0 panic (asserted above)");
    }
