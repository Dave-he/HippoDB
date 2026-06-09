//! Micro-benchmarks for libsqlite_rs.
//!
//! Uses `std::time::Instant` to avoid adding a new dependency. These
//! tests are `#[ignore]`'d by default so `cargo test` stays fast; run
//! them explicitly with:
//!
//!     cargo test --test bench -- --ignored --nocapture
//!
//! Each test prints its throughput (ops/sec) and a per-op latency so
//! the numbers are interpretable without a full benchmark harness.

use std::time::Instant;

use libsqlite_rs::{run_sql, Schema};

/// Measure `body` running `iters` times and report ops/sec + ns/op.
fn bench<F: FnMut()>(label: &str, iters: u64, mut body: F) {
    let start = Instant::now();
    for _ in 0..iters {
        body();
    }
    let elapsed = start.elapsed();
    let per_op_ns = elapsed.as_nanos() / iters as u128;
    let ops_per_sec = (iters as f64) / elapsed.as_secs_f64();
    println!(
        "[bench:{label:>32}] {iters:>8} iters in {elapsed:>12?} \
         = {ops_per_sec:>12.0} ops/sec  ({per_op_ns:>6} ns/op)"
    );
}

// ── 1. Tokenize + parse throughput ───────────────────────────────────

#[test]
#[ignore]
fn bench_parse_simple_select() {
    let sql = "SELECT x, y FROM users WHERE x = 1";
    bench("parse.simple-select", 10_000, || {
        let _ = libsqlite_rs::parse::parse_sql(sql).unwrap();
    });
}

#[test]
#[ignore]
fn bench_parse_complex_select() {
    // Wider parse tree: more columns, longer WHERE, AND/OR.
    let sql = "SELECT a, b, c, d, e FROM big WHERE a > 1 AND b < 100 OR c = 5";
    bench("parse.complex-select", 10_000, || {
        let _ = libsqlite_rs::parse::parse_sql(sql).unwrap();
    });
}

#[test]
#[ignore]
fn bench_tokenize_only() {
    let sql = "SELECT x, y FROM users WHERE x > 1";
    bench("tokenize.simple", 10_000, || {
        let _ = libsqlite_rs::tokenize::tokenize(sql).unwrap();
    });
}

// ── 2. Expression eval throughput ────────────────────────────────────

struct NullReg;
impl libsqlite_rs::FunctionRegistry for NullReg {
    fn call(
        &self,
        _name: &str,
        _args: &[libsqlite_rs::SqliteValue],
    ) -> Result<libsqlite_rs::SqliteValue, libsqlite_rs::SqliteError> {
        Err(libsqlite_rs::SqliteError::ERROR)
    }
}

#[test]
#[ignore]
fn bench_eval_arith() {
    use libsqlite_rs::{eval, BinaryOp, Expr, Literal, SimpleEnv};
    let env: SimpleEnv<NullReg> = SimpleEnv {
        row: vec![("x".to_string(), libsqlite_rs::SqliteValue::Integer(42))],
        funcs: NullReg,
    };
    let expr = Expr::Binary {
        op: BinaryOp::Add,
        left: Box::new(Expr::Binary {
            op: BinaryOp::Mul,
            left: Box::new(Expr::ColumnRef("x".to_string())),
            right: Box::new(Expr::Literal(Literal::Integer(2))),
        }),
        right: Box::new(Expr::Literal(Literal::Integer(8))),
    };
    bench("eval.arith", 50_000, || {
        let _ = eval(&expr, &env).unwrap();
    });
}

// ── 3. run_select throughput vs table size ───────────────────────────

fn seed_table(schema: &mut Schema, n: usize) {
    let sql = "CREATE TABLE t (x INTEGER, y INTEGER)";
    run_sql(sql, schema).unwrap();
    for i in 0..n {
        let sql = format!("INSERT INTO t VALUES ({i}, {})", i * 2);
        run_sql(&sql, schema).unwrap();
    }
}

#[test]
#[ignore]
fn bench_run_select_100_rows() {
    let mut schema = Schema::default();
    seed_table(&mut schema, 100);
    bench("run_select.100", 1_000, || {
        let _ = run_sql("SELECT x, y FROM t WHERE x > 50", &mut schema).unwrap();
    });
}

#[test]
#[ignore]
fn bench_run_select_1000_rows() {
    let mut schema = Schema::default();
    seed_table(&mut schema, 1000);
    bench("run_select.1000", 1_000, || {
        let _ = run_sql("SELECT x, y FROM t WHERE x > 500", &mut schema).unwrap();
    });
}

#[test]
#[ignore]
fn bench_run_select_10k_rows() {
    let mut schema = Schema::default();
    seed_table(&mut schema, 10_000);
    bench("run_select.10k", 200, || {
        let _ = run_sql("SELECT x, y FROM t WHERE x > 5000", &mut schema).unwrap();
    });
}

#[test]
#[ignore]
fn bench_run_select_no_filter() {
    let mut schema = Schema::default();
    seed_table(&mut schema, 1000);
    bench("run_select.1000.nofilter", 1_000, || {
        let _ = run_sql("SELECT x, y FROM t", &mut schema).unwrap();
    });
}

// ── 4. End-to-end: parse + run full SQL pipeline ─────────────────────

#[test]
#[ignore]
fn bench_run_sql_e2e() {
    let mut schema = Schema::default();
    run_sql("CREATE TABLE t (x INTEGER)", &mut schema).unwrap();
    for i in 0..100 {
        run_sql(&format!("INSERT INTO t VALUES ({i})"), &mut schema).unwrap();
    }
    bench("run_sql.e2e-where", 5_000, || {
        let _ = run_sql("SELECT x FROM t WHERE x > 50", &mut schema).unwrap();
    });
}

// ── 5. Underlying primitives (sanity check) ──────────────────────────

#[test]
#[ignore]
fn bench_hash_throughput() {
    use libsqlite_rs::str_hash;
    let s = "the quick brown fox jumps over the lazy dog";
    bench("hash.str_hash", 1_000_000, || {
        let _ = str_hash(s);
    });
}

#[test]
#[ignore]
fn bench_printf_int_throughput() {
    use libsqlite_rs::printf_int;
    bench("printf.int", 10_000, || {
        let _ = printf_int("%d", &[42i64]);
    });
}

// ── 6. Scaling: linear vs quadratic in table size ────────────────────

#[test]
#[ignore]
fn bench_scaling_full_scan() {
    // Print per-table-size latency so we can confirm it's linear in n.
    for &n in &[10, 100, 1_000, 5_000] {
        let mut schema = Schema::default();
        seed_table(&mut schema, n);
        let iters = if n <= 1_000 { 200 } else { 50 };
        let start = Instant::now();
        for _ in 0..iters {
            let _ = run_sql("SELECT x FROM t", &mut schema).unwrap();
        }
        let elapsed = start.elapsed();
        let per_op_us = elapsed.as_micros() as f64 / iters as f64;
        let per_row_us = per_op_us / n as f64;
        println!(
            "[bench:scaling.full_scan] n={n:>5}  \
             {iters:>4} iters  total {elapsed:>10?}  \
             per-op {per_op_us:>8.1} us  per-row {per_row_us:.3} us"
        );
    }
}

#[test]
#[ignore]
fn bench_scaling_filter() {
    // Filter that matches ~10% of rows: should be ~linear in n.
    for &n in &[10, 100, 1_000, 5_000] {
        let mut schema = Schema::default();
        seed_table(&mut schema, n);
        let iters = if n <= 1_000 { 200 } else { 50 };
        let start = Instant::now();
        for _ in 0..iters {
            let _ = run_sql("SELECT x FROM t WHERE x > 0", &mut schema).unwrap();
        }
        let elapsed = start.elapsed();
        let per_op_us = elapsed.as_micros() as f64 / iters as f64;
        let per_row_us = per_op_us / n as f64;
        println!(
            "[bench:scaling.filter]     n={n:>5}  \
             {iters:>4} iters  total {elapsed:>10?}  \
             per-op {per_op_us:>8.1} us  per-row {per_row_us:.3} us"
        );
    }
}
