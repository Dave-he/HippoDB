//! Standalone CLI demo: connect to an in-memory database, run a few
//! statements through `libsqlite_rs::run_sql`, and print the rows.
//!
//! Build & run (from rust-port/):
//!   cargo run --example run_sql_demo

use libsqlite_rs::{run_sql, Mem, Schema};

fn print_row(row: &[Mem]) {
    let parts: Vec<String> = row.iter().map(|m| m.to_display_string()).collect();
    println!("| {} |", parts.join(" | "));
}

fn main() {
    let mut schema = Schema::new();
    println!("libsqlite_rs run_sql demo");
    println!("========================");

    // Multi-statement script.
    let script = "\
        CREATE TABLE users (id INTEGER, name TEXT);\
        INSERT INTO users VALUES (1, 'alice');\
        INSERT INTO users VALUES (2, 'bob');\
        INSERT INTO users VALUES (3, 'charlie');\
        SELECT * FROM users;\
        SELECT name FROM users WHERE id > 1;\
    ";

    let rows = match run_sql(script, &mut schema) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("run_sql error: {e}");
            std::process::exit(1);
        }
    };

    println!("rows emitted by SELECTs: {}", rows.len());
    for row in &rows {
        print_row(row);
    }
    assert_eq!(rows.len(), 5, "3 user rows + 2 name rows");
    println!("OK");
}
