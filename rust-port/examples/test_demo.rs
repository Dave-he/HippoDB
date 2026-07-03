use libsqlite_rs::{run_sql, Schema};
fn main() {
    let mut s = Schema::new();
    println!("test 1: simple");
    let r = run_sql("CREATE TABLE users (id INTEGER, name TEXT); INSERT INTO users VALUES (1, 'alice'); INSERT INTO users VALUES (2, 'bob'); SELECT * FROM users;", &mut s);
    println!("rows: {:?}", r);
    let mut s2 = Schema::new();
    println!("test 2: with where");
    let r2 = run_sql("CREATE TABLE users (id INTEGER, name TEXT); INSERT INTO users VALUES (1, 'alice'); INSERT INTO users VALUES (2, 'bob'); SELECT name FROM users WHERE id > 1;", &mut s2);
    println!("rows: {:?}", r2);
}
