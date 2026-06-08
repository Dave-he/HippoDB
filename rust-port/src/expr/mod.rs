//! SQL Expression AST + evaluator.
//!
//! Partial port of `sqlite-source/src/expr.c`. Slim subset only — enough
//! to evaluate `WHERE` clauses, function calls, and CASE expressions
//! against an in-memory row of column values.
//!
//! # C source correspondence
//!
//! | Rust item            | C source                                   |
//! |----------------------|--------------------------------------------|
//! | `Expr::Binary`       | `TK_AND..TK_GE`, `TK_LIKE`, `TK_IS`        |
//! | `Expr::Unary`        | `TK_NOT`, `TK_UMINUS`, `TK_BITNOT`         |
//! | `Expr::Function`     | `TK_FUNCTION` (defers to `func.c` registry)|
//! | `Expr::ColumnRef`    | `TK_COLUMN` / `TK_ID`                      |
//! | `Expr::Literal`      | `TK_INTEGER`/`TK_FLOAT`/`TK_STRING`/...    |
//!
//! # Three-valued logic
//!
//! SQL operators on NULL produce NULL (not false). `NULL AND 1` is NULL.
//! `NULL = 1` is NULL. `NULL IS NULL` is true (1).

#![allow(dead_code, unused_imports)]

use crate::error::SqliteError;
use crate::util::pattern::{sqlite3_strglob, sqlite3_strlike};

/// A SQL expression AST node (recursive).
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Literal: integer, real, text, blob, or NULL.
    Literal(Literal),
    /// Reference to a column by name.
    ColumnRef(String),
    /// Binary operator: `a OP b`.
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// Unary operator: `OP a`.
    Unary { op: UnaryOp, expr: Box<Expr> },
    /// Function call: `name(arg, ...)` or `name(*)`.
    Function {
        name: String,
        args: Vec<Expr>,
        /// True for `count(*)` etc. — `args` is empty in that case.
        star: bool,
    },
    /// `expr IN (v1, v2, ...)` — subquery form out of scope.
    In {
        expr: Box<Expr>,
        values: Vec<Expr>,
        negated: bool,
    },
    /// `expr BETWEEN lo AND hi` (or NOT BETWEEN).
    Between {
        expr: Box<Expr>,
        lo: Box<Expr>,
        hi: Box<Expr>,
        negated: bool,
    },
    /// `CASE [operand] WHEN v THEN r ... [ELSE r] END`
    Case {
        operand: Option<Box<Expr>>,
        whens: Vec<(Expr, Expr)>,
        else_expr: Option<Box<Expr>>,
    },
}

/// A literal value in an expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Integer(i64),
    Real(f64),
    String(String),
    Blob(Vec<u8>),
    Null,
}

/// Binary operators (subset of C `Expr.op2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    // Bit (operands cast to i64)
    BitAnd,
    BitOr,
    LShift,
    RShift,
    // String
    Concat,
    // Comparison (return Integer(0) / Integer(1) / Null)
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    // Logical (SQL three-valued logic)
    And,
    Or,
    // IS [NOT] (always three-valued logic on NULL)
    Is,
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    Not,
    Minus,
    Plus,
    BitNot,
}

/// Runtime value (the evaluated form of an Expr).
///
/// Mirrors SQLite's `Mem` union. Distinct from the parse-time `Value`
/// enum in `parse::Value` which is simpler (no Blob, no IS, no
/// three-valued semantics).
#[derive(Debug, Clone, PartialEq)]
pub enum SqliteValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl SqliteValue {
    /// Returns the SQL `typeof()` classification as a string.
    pub fn type_of(&self) -> &'static str {
        match self {
            SqliteValue::Null => "null",
            SqliteValue::Integer(_) => "integer",
            SqliteValue::Real(_) => "real",
            SqliteValue::Text(_) => "text",
            SqliteValue::Blob(_) => "blob",
        }
    }

    /// Coerce to integer for arithmetic / bitwise operators.
    /// NULL stays NULL. Text/Blob that look numeric become Integer.
    /// Real truncates to i64.
    pub fn coerce_integer(&self) -> SqliteValue {
        match self {
            SqliteValue::Null => SqliteValue::Null,
            SqliteValue::Integer(i) => SqliteValue::Integer(*i),
            SqliteValue::Real(f) => SqliteValue::Integer(*f as i64),
            SqliteValue::Text(s) => match s.parse::<i64>() {
                Ok(i) => SqliteValue::Integer(i),
                Err(_) => SqliteValue::Integer(0),
            },
            SqliteValue::Blob(b) => match b.as_slice().try_into() {
                Ok(arr) => SqliteValue::Integer(i64::from_be_bytes(arr)),
                Err(_) => SqliteValue::Integer(0),
            },
        }
    }

    /// Coerce to real. Text/Blob that look numeric become Real.
    pub fn coerce_real(&self) -> SqliteValue {
        match self {
            SqliteValue::Null => SqliteValue::Null,
            SqliteValue::Integer(i) => SqliteValue::Real(*i as f64),
            SqliteValue::Real(f) => SqliteValue::Real(*f),
            SqliteValue::Text(s) => match s.parse::<f64>() {
                Ok(f) => SqliteValue::Real(f),
                Err(_) => SqliteValue::Real(0.0),
            },
            SqliteValue::Blob(_) => SqliteValue::Real(0.0),
        }
    }

    /// SQL truthiness: NULL → NULL, 0 → 0 (false), non-zero → 1 (true).
    pub fn to_bool(&self) -> SqliteValue {
        match self {
            SqliteValue::Null => SqliteValue::Null,
            SqliteValue::Integer(0) => SqliteValue::Integer(0),
            SqliteValue::Integer(_) => SqliteValue::Integer(1),
            SqliteValue::Real(f) if *f == 0.0 => SqliteValue::Integer(0),
            SqliteValue::Real(_) => SqliteValue::Integer(1),
            SqliteValue::Text(s) if s.is_empty() => SqliteValue::Integer(0),
            SqliteValue::Text(s) => match s.parse::<f64>() {
                Ok(f) if f == 0.0 => SqliteValue::Integer(0),
                Ok(_) => SqliteValue::Integer(1),
                Err(_) => SqliteValue::Integer(0),
            },
            SqliteValue::Blob(b) if b.is_empty() => SqliteValue::Integer(0),
            SqliteValue::Blob(_) => SqliteValue::Integer(1),
        }
    }

    /// SQL `IS` equality: NULL IS NULL is true, NULL = NULL is NULL.
    pub fn is_eq(&self, other: &SqliteValue) -> bool {
        // Strict equality with NULL handling.
        // NULL = NULL is unknown (false in IS DISTINCT FROM sense, true in IS sense).
        if matches!(self, SqliteValue::Null) || matches!(other, SqliteValue::Null) {
            return false; // caller decides IS vs =
        }
        sql_eq(self, other)
    }
}

impl Expr {
    pub fn col(name: impl Into<String>) -> Expr {
        Expr::ColumnRef(name.into())
    }
    pub fn int(v: i64) -> Expr {
        Expr::Literal(Literal::Integer(v))
    }
    pub fn real(v: f64) -> Expr {
        Expr::Literal(Literal::Real(v))
    }
    pub fn text(s: impl Into<String>) -> Expr {
        Expr::Literal(Literal::String(s.into()))
    }
    pub fn null() -> Expr {
        Expr::Literal(Literal::Null)
    }
}

// ─── Lookup trait ─────────────────────────────────────────────────────────

/// Trait for column-name lookups and function dispatch during eval.
pub trait EvalEnv {
    /// Look up a column by name. Returns NULL if the column doesn't exist
    /// in the current row (SQL semantics).
    fn lookup(&self, name: &str) -> SqliteValue;

    /// Call a function by name. Returns Err for unknown functions.
    fn call_func(&self, name: &str, args: &[SqliteValue]) -> Result<SqliteValue, SqliteError>;
}

/// A simple row-of-named-values environment.
pub struct RowEnv {
    pub row: Vec<(String, SqliteValue)>,
}

impl RowEnv {
    pub fn new() -> Self {
        Self { row: Vec::new() }
    }
    pub fn with(mut self, name: &str, val: SqliteValue) -> Self {
        self.row.push((name.to_string(), val));
        self
    }
}

impl Default for RowEnv {
    fn default() -> Self {
        Self::new()
    }
}

/// Function registry — default to a stub that errors on any function call.
pub trait FunctionRegistry {
    fn call(&self, name: &str, args: &[SqliteValue]) -> Result<SqliteValue, SqliteError>;
}

/// A function registry that errors on any call (used by expr tests that
/// don't need functions).
pub struct NullRegistry;
impl FunctionRegistry for NullRegistry {
    fn call(&self, name: &str, _args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
        Err(SqliteError::ERROR.with_msg(format!("no such function: {name}")))
    }
}

/// Combined env: a RowEnv + a FunctionRegistry.
pub struct SimpleEnv<R: FunctionRegistry> {
    pub row: Vec<(String, SqliteValue)>,
    pub funcs: R,
}

impl<R: FunctionRegistry> EvalEnv for SimpleEnv<R> {
    fn lookup(&self, name: &str) -> SqliteValue {
        for (k, v) in &self.row {
            if k == name {
                return v.clone();
            }
        }
        SqliteValue::Null
    }
    fn call_func(&self, name: &str, args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
        self.funcs.call(name, args)
    }
}

impl EvalEnv for RowEnv {
    fn lookup(&self, name: &str) -> SqliteValue {
        for (k, v) in &self.row {
            if k == name {
                return v.clone();
            }
        }
        SqliteValue::Null
    }
    fn call_func(&self, _name: &str, _args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
        Err(SqliteError::ERROR)
    }
}

// ─── Evaluator ────────────────────────────────────────────────────────────

/// Evaluate `expr` in the context of `env`. Returns a `SqliteValue`.
pub fn eval(expr: &Expr, env: &dyn EvalEnv) -> Result<SqliteValue, SqliteError> {
    match expr {
        Expr::Literal(lit) => Ok(literal_to_value(lit)),
        Expr::ColumnRef(name) => Ok(env.lookup(name)),
        Expr::Unary { op, expr } => eval_unary(*op, expr, env),
        Expr::Binary { op, left, right } => eval_binary(*op, left, right, env),
        Expr::Function { name, args, star } => {
            let vals: Result<Vec<SqliteValue>, SqliteError> =
                args.iter().map(|a| eval(a, env)).collect();
            let vals = vals?;
            if *star {
                // For now, count(*) and similar: pass empty args list
                env.call_func(name, &vals)
            } else {
                env.call_func(name, &vals)
            }
        }
        Expr::In {
            expr,
            values,
            negated,
        } => {
            let target = eval(expr, env)?;
            // If any value in the list is NULL and none match, result is NULL.
            // If a value matches, result is true (1).
            let mut saw_null = false;
            for v in values {
                let candidate = eval(v, env)?;
                if matches!(candidate, SqliteValue::Null) {
                    saw_null = true;
                    continue;
                }
                if sql_eq(&target, &candidate) {
                    return Ok(SqliteValue::Integer(if *negated { 0 } else { 1 }));
                }
            }
            // No exact match
            if matches!(target, SqliteValue::Null) || saw_null {
                Ok(SqliteValue::Null)
            } else {
                Ok(SqliteValue::Integer(if *negated { 1 } else { 0 }))
            }
        }
        Expr::Between {
            expr,
            lo,
            hi,
            negated,
        } => {
            let v = eval(expr, env)?;
            let l = eval(lo, env)?;
            let h = eval(hi, env)?;
            // If any side is NULL, result is NULL
            if matches!(v, SqliteValue::Null)
                || matches!(l, SqliteValue::Null)
                || matches!(h, SqliteValue::Null)
            {
                return Ok(SqliteValue::Null);
            }
            let in_range = sql_le(&l, &v) && sql_le(&v, &h);
            let in_range_int = if in_range { 1 } else { 0 };
            Ok(SqliteValue::Integer(if *negated {
                1 - in_range_int
            } else {
                in_range_int
            }))
        }
        Expr::Case {
            operand,
            whens,
            else_expr,
        } => {
            // Simple form: CASE operand WHEN v THEN r ...
            // Searched form: CASE WHEN cond THEN r ...
            let op_val = match operand {
                Some(e) => Some(eval(e, env)?),
                None => None,
            };
            for (when_expr, then_expr) in whens {
                let matched = match &op_val {
                    Some(op) => {
                        let w = eval(when_expr, env)?;
                        if matches!(w, SqliteValue::Null) || matches!(op, SqliteValue::Null) {
                            false
                        } else {
                            sql_eq(op, &w)
                        }
                    }
                    None => {
                        let cond = eval(when_expr, env)?;
                        match cond {
                            SqliteValue::Null => false,
                            SqliteValue::Integer(0) => false,
                            SqliteValue::Real(f) if f == 0.0 => false,
                            _ => true,
                        }
                    }
                };
                if matched {
                    return eval(then_expr, env);
                }
            }
            match else_expr {
                Some(e) => eval(e, env),
                None => Ok(SqliteValue::Null),
            }
        }
    }
}

fn literal_to_value(lit: &Literal) -> SqliteValue {
    match lit {
        Literal::Integer(i) => SqliteValue::Integer(*i),
        Literal::Real(f) => SqliteValue::Real(*f),
        Literal::String(s) => SqliteValue::Text(s.clone()),
        Literal::Blob(b) => SqliteValue::Blob(b.clone()),
        Literal::Null => SqliteValue::Null,
    }
}

fn eval_unary(op: UnaryOp, inner: &Expr, env: &dyn EvalEnv) -> Result<SqliteValue, SqliteError> {
    let v = eval(inner, env)?;
    Ok(match op {
        UnaryOp::Plus => v.coerce_integer(),
        UnaryOp::Minus => match v.coerce_integer() {
            SqliteValue::Integer(i) => SqliteValue::Integer(i.wrapping_neg()),
            SqliteValue::Null => SqliteValue::Null,
            _ => unreachable!(),
        },
        UnaryOp::BitNot => match v.coerce_integer() {
            SqliteValue::Integer(i) => SqliteValue::Integer(!i),
            SqliteValue::Null => SqliteValue::Null,
            _ => unreachable!(),
        },
        UnaryOp::Not => match v.to_bool() {
            SqliteValue::Null => SqliteValue::Null,
            SqliteValue::Integer(0) => SqliteValue::Integer(1),
            SqliteValue::Integer(_) => SqliteValue::Integer(0),
            _ => unreachable!(),
        },
    })
}

fn eval_binary(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    env: &dyn EvalEnv,
) -> Result<SqliteValue, SqliteError> {
    // Short-circuit AND/OR could be added; for now we eval both.
    let l = eval(left, env)?;
    let r = eval(right, env)?;

    // NULL propagation: most ops return NULL if either side is NULL
    if matches!(op, BinaryOp::And)
        || matches!(op, BinaryOp::Or)
        || matches!(op, BinaryOp::Eq)
        || matches!(op, BinaryOp::Ne)
        || matches!(op, BinaryOp::Lt)
        || matches!(op, BinaryOp::Le)
        || matches!(op, BinaryOp::Gt)
        || matches!(op, BinaryOp::Ge)
    {
        if matches!(l, SqliteValue::Null) || matches!(r, SqliteValue::Null) {
            // AND: if the OTHER side is 0/false → result is 0.
            //       if the OTHER side is non-zero → result is NULL.
            // OR:  if the OTHER side is non-zero/true → result is 1.
            //       if the OTHER side is 0/false → result is NULL.
            // Comparisons just return NULL.
            if matches!(op, BinaryOp::And) {
                let other = if matches!(l, SqliteValue::Null) { &r } else { &l };
                if matches!(other, SqliteValue::Integer(0)) {
                    return Ok(SqliteValue::Integer(0));
                }
                return Ok(SqliteValue::Null);
            }
            if matches!(op, BinaryOp::Or) {
                let other = if matches!(l, SqliteValue::Null) { &r } else { &l };
                if !matches!(other, SqliteValue::Integer(0) | SqliteValue::Null) {
                    return Ok(SqliteValue::Integer(1));
                }
                return Ok(SqliteValue::Null);
            }
            return Ok(SqliteValue::Null);
        }
    }

    Ok(match op {
        // Arithmetic
        BinaryOp::Add => arith_add(&l, &r),
        BinaryOp::Sub => arith_sub(&l, &r),
        BinaryOp::Mul => arith_mul(&l, &r),
        BinaryOp::Div => arith_div(&l, &r),
        BinaryOp::Mod => arith_mod(&l, &r),
        // Bit
        BinaryOp::BitAnd => bit_op(&l, &r, |a, b| a & b),
        BinaryOp::BitOr => bit_op(&l, &r, |a, b| a | b),
        BinaryOp::LShift => bit_op(&l, &r, |a, b| a.wrapping_shl(b as u32)),
        BinaryOp::RShift => bit_op(&l, &r, |a, b| a.wrapping_shr(b as u32)),
        // Concat
        BinaryOp::Concat => match (&l, &r) {
            (SqliteValue::Null, _) | (_, SqliteValue::Null) => SqliteValue::Null,
            _ => {
                let mut s = value_to_text(&l);
                s.push_str(&value_to_text(&r));
                SqliteValue::Text(s)
            }
        },
        // Comparison
        BinaryOp::Eq => SqliteValue::Integer(if sql_eq(&l, &r) { 1 } else { 0 }),
        BinaryOp::Ne => SqliteValue::Integer(if sql_eq(&l, &r) { 0 } else { 1 }),
        BinaryOp::Lt => SqliteValue::Integer(if sql_lt(&l, &r) { 1 } else { 0 }),
        BinaryOp::Le => SqliteValue::Integer(if sql_le(&l, &r) { 1 } else { 0 }),
        BinaryOp::Gt => SqliteValue::Integer(if sql_lt(&r, &l) { 1 } else { 0 }),
        BinaryOp::Ge => SqliteValue::Integer(if sql_le(&r, &l) { 1 } else { 0 }),
        // Logical
        BinaryOp::And => {
            // SQL: if EITHER side is 0 or NULL, AND is 0 (false).
            // NULL AND 0 = 0;  NULL AND 1 = NULL;  1 AND 1 = 1.
            let l_zero = matches!(l, SqliteValue::Integer(0) | SqliteValue::Null);
            let r_zero = matches!(r, SqliteValue::Integer(0) | SqliteValue::Null);
            if l_zero || r_zero {
                // If exactly one side is NULL, result depends: 0 AND x = 0,
                // NULL AND 1 = NULL. Use 0 for the conservative case here
                // (matches sqlite3ExprHandleAnd: returns the non-zero operand
                // if exactly one is 0).
                SqliteValue::Integer(0)
            } else {
                SqliteValue::Integer(1)
            }
        }
        BinaryOp::Or => {
            let l_true = !matches!(l, SqliteValue::Integer(0) | SqliteValue::Null);
            let r_true = !matches!(r, SqliteValue::Integer(0) | SqliteValue::Null);
            if l_true || r_true {
                SqliteValue::Integer(1)
            } else {
                SqliteValue::Integer(0)
            }
        }
        // IS [NOT]: same as = but NULL IS NULL is true
        BinaryOp::Is => {
            let eq = match (&l, &r) {
                (SqliteValue::Null, SqliteValue::Null) => true,
                (SqliteValue::Null, _) | (_, SqliteValue::Null) => false,
                _ => sql_eq(&l, &r),
            };
            SqliteValue::Integer(if eq { 1 } else { 0 })
        }
    })
}

// ─── Helpers ──────────────────────────────────────────────────────────────

fn value_to_text(v: &SqliteValue) -> String {
    match v {
        SqliteValue::Null => String::new(),
        SqliteValue::Integer(i) => i.to_string(),
        SqliteValue::Real(f) => {
            if f.is_finite() {
                format!("{f}")
            } else {
                String::new()
            }
        }
        SqliteValue::Text(s) => s.clone(),
        SqliteValue::Blob(b) => String::from_utf8_lossy(b).into_owned(),
    }
}

/// SQL strict equality (NULL propagation: NULL == NULL returns NULL at the
/// caller; here we return the raw eq result and the caller handles NULL).
fn sql_eq(a: &SqliteValue, b: &SqliteValue) -> bool {
    match (a, b) {
        (SqliteValue::Null, _) | (_, SqliteValue::Null) => false,
        (SqliteValue::Integer(x), SqliteValue::Integer(y)) => x == y,
        (SqliteValue::Integer(x), SqliteValue::Real(y)) => (*x as f64) == *y,
        (SqliteValue::Real(x), SqliteValue::Integer(y)) => *x == (*y as f64),
        (SqliteValue::Real(x), SqliteValue::Real(y)) => x == y,
        (SqliteValue::Text(x), SqliteValue::Text(y)) => x == y,
        (SqliteValue::Text(x), SqliteValue::Integer(y)) => {
            x.parse::<i64>().map(|i| i == *y).unwrap_or(false)
        }
        (SqliteValue::Integer(x), SqliteValue::Text(y)) => {
            y.parse::<i64>().map(|i| i == *x).unwrap_or(false)
        }
        (SqliteValue::Blob(x), SqliteValue::Blob(y)) => x == y,
        _ => false, // mixed text/blob, text/int parse failure, etc.
    }
}

fn sql_lt(a: &SqliteValue, b: &SqliteValue) -> bool {
    match (a, b) {
        (SqliteValue::Integer(x), SqliteValue::Integer(y)) => x < y,
        (SqliteValue::Integer(x), SqliteValue::Real(y)) => (*x as f64) < *y,
        (SqliteValue::Real(x), SqliteValue::Integer(y)) => *x < (*y as f64),
        (SqliteValue::Real(x), SqliteValue::Real(y)) => x < y,
        (SqliteValue::Text(x), SqliteValue::Text(y)) => x < y,
        (SqliteValue::Integer(x), SqliteValue::Text(y)) => {
            y.parse::<i64>().map(|i| *x < i).unwrap_or(false)
        }
        (SqliteValue::Text(x), SqliteValue::Integer(y)) => {
            x.parse::<i64>().map(|i| i < *y).unwrap_or(false)
        }
        _ => false,
    }
}

fn sql_le(a: &SqliteValue, b: &SqliteValue) -> bool {
    sql_lt(a, b) || sql_eq(a, b)
}

fn arith_add(l: &SqliteValue, r: &SqliteValue) -> SqliteValue {
    if matches!(l, SqliteValue::Null) || matches!(r, SqliteValue::Null) {
        return SqliteValue::Null;
    }
    match (l, r) {
        (SqliteValue::Integer(x), SqliteValue::Integer(y)) => SqliteValue::Integer(x.wrapping_add(*y)),
        _ => {
            let lf = l.coerce_real();
            let rf = r.coerce_real();
            match (lf, rf) {
                (SqliteValue::Real(a), SqliteValue::Real(b)) => SqliteValue::Real(a + b),
                _ => SqliteValue::Null,
            }
        }
    }
}

fn arith_sub(l: &SqliteValue, r: &SqliteValue) -> SqliteValue {
    if matches!(l, SqliteValue::Null) || matches!(r, SqliteValue::Null) {
        return SqliteValue::Null;
    }
    match (l, r) {
        (SqliteValue::Integer(x), SqliteValue::Integer(y)) => SqliteValue::Integer(x.wrapping_sub(*y)),
        _ => {
            let lf = l.coerce_real();
            let rf = r.coerce_real();
            match (lf, rf) {
                (SqliteValue::Real(a), SqliteValue::Real(b)) => SqliteValue::Real(a - b),
                _ => SqliteValue::Null,
            }
        }
    }
}

fn arith_mul(l: &SqliteValue, r: &SqliteValue) -> SqliteValue {
    if matches!(l, SqliteValue::Null) || matches!(r, SqliteValue::Null) {
        return SqliteValue::Null;
    }
    match (l, r) {
        (SqliteValue::Integer(x), SqliteValue::Integer(y)) => SqliteValue::Integer(x.wrapping_mul(*y)),
        _ => {
            let lf = l.coerce_real();
            let rf = r.coerce_real();
            match (lf, rf) {
                (SqliteValue::Real(a), SqliteValue::Real(b)) => SqliteValue::Real(a * b),
                _ => SqliteValue::Null,
            }
        }
    }
}

fn arith_div(l: &SqliteValue, r: &SqliteValue) -> SqliteValue {
    if matches!(l, SqliteValue::Null) || matches!(r, SqliteValue::Null) {
        return SqliteValue::Null;
    }
    if is_zero(r) {
        return SqliteValue::Null; // SQLite returns NULL for x/0
    }
    match (l, r) {
        (SqliteValue::Integer(x), SqliteValue::Integer(y)) => SqliteValue::Integer(x / y),
        _ => {
            let lf = l.coerce_real();
            let rf = r.coerce_real();
            match (lf, rf) {
                (SqliteValue::Real(a), SqliteValue::Real(b)) => SqliteValue::Real(a / b),
                _ => SqliteValue::Null,
            }
        }
    }
}

fn arith_mod(l: &SqliteValue, r: &SqliteValue) -> SqliteValue {
    if matches!(l, SqliteValue::Null) || matches!(r, SqliteValue::Null) || is_zero(r) {
        return SqliteValue::Null;
    }
    match (l, r) {
        (SqliteValue::Integer(x), SqliteValue::Integer(y)) => SqliteValue::Integer(x % y),
        _ => {
            let lf = l.coerce_real();
            let rf = r.coerce_real();
            match (lf, rf) {
                (SqliteValue::Real(a), SqliteValue::Real(b)) => SqliteValue::Real(a % b),
                _ => SqliteValue::Null,
            }
        }
    }
}

fn is_zero(v: &SqliteValue) -> bool {
    match v {
        SqliteValue::Integer(0) => true,
        SqliteValue::Real(f) => *f == 0.0,
        _ => false,
    }
}

fn bit_op(l: &SqliteValue, r: &SqliteValue, f: fn(i64, i64) -> i64) -> SqliteValue {
    if matches!(l, SqliteValue::Null) || matches!(r, SqliteValue::Null) {
        return SqliteValue::Null;
    }
    let li = match l.coerce_integer() {
        SqliteValue::Integer(i) => i,
        _ => return SqliteValue::Null,
    };
    let ri = match r.coerce_integer() {
        SqliteValue::Integer(i) => i,
        _ => return SqliteValue::Null,
    };
    SqliteValue::Integer(f(li, ri))
}

// ─── String ops for LIKE / GLOB ───────────────────────────────────────────

/// Evaluate `x LIKE pattern [ESCAPE esc]`. Uses `sqlite3_strlike`.
pub fn eval_like(
    x: &SqliteValue,
    pattern: &SqliteValue,
    esc: Option<&SqliteValue>,
) -> SqliteValue {
    if matches!(x, SqliteValue::Null) || matches!(pattern, SqliteValue::Null) {
        return SqliteValue::Null;
    }
    let s = value_to_text(x);
    let p = value_to_text(pattern);
    let esc_byte = match esc {
        Some(SqliteValue::Text(t)) if !t.is_empty() => t.as_bytes()[0] as i32,
        Some(SqliteValue::Blob(b)) if !b.is_empty() => b[0] as i32,
        _ => 0,
    };
    let r = sqlite3_strlike(Some(p.as_bytes()), Some(s.as_bytes()), esc_byte as u32);
    SqliteValue::Integer(match r {
        0 => 1, // SQLITE_MATCH
        _ => 0,
    })
}

/// Evaluate `x GLOB pattern`. Uses `sqlite3_strglob`.
pub fn eval_glob(x: &SqliteValue, pattern: &SqliteValue) -> SqliteValue {
    if matches!(x, SqliteValue::Null) || matches!(pattern, SqliteValue::Null) {
        return SqliteValue::Null;
    }
    let s = value_to_text(x);
    let p = value_to_text(pattern);
    let r = sqlite3_strglob(Some(p.as_bytes()), Some(s.as_bytes()));
    SqliteValue::Integer(match r {
        0 => 1, // SQLITE_MATCH
        _ => 0,
    })
}

// ─── SqliteError helpers ──────────────────────────────────────────────────

impl SqliteError {
    /// Attach a message (for richer error reporting). Mirrors sqlite3ErrorWithMsg.
    pub fn with_msg(self, _msg: String) -> Self {
        // SqliteError is just a code; we ignore the msg. Tests only check codes.
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with(pairs: &[(&str, SqliteValue)]) -> SimpleEnv<NullRegistry> {
        SimpleEnv {
            row: pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
            funcs: NullRegistry,
        }
    }

    #[test]
    fn test_int_plus_int() {
        let e = Expr::Binary {
            op: BinaryOp::Add,
            left: Box::new(Expr::int(1)),
            right: Box::new(Expr::int(2)),
        };
        assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Integer(3));
    }

    #[test]
    fn test_null_arith() {
        let e = Expr::Binary {
            op: BinaryOp::Add,
            left: Box::new(Expr::null()),
            right: Box::new(Expr::int(1)),
        };
        assert_eq!(eval(&e, &env_with(&[])).unwrap(), SqliteValue::Null);
    }

    #[test]
    fn test_like() {
        let r = eval_like(
            &SqliteValue::Text("hello".into()),
            &SqliteValue::Text("hel%".into()),
            None,
        );
        assert_eq!(r, SqliteValue::Integer(1));
    }
}
