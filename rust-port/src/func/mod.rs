//! T-0018 — Built-in scalar function registry.
//!
//! Partial port of `sqlite-source/src/func.c`. Slim subset of ~20 scalar
//! functions. Aggregate functions (count/sum/avg) and date/time are
//! out of scope (covered by separate tasks in L2).
//!
//! # C source correspondence
//!
//! | Rust item             | C source                              |
//! |-----------------------|---------------------------------------|
//! | `BuiltinRegistry`     | `sqlite3BuiltinFunc.def[]`            |
//! | `abs`                 | `absFunc`                             |
//! | `typeof`              | `typeofFunc`                          |
//! | `length`              | `lengthFunc`                          |
//! | `upper`/`lower`       | `upperFunc`/`lowerFunc`               |
//! | `substr`              | `substrFunc`                          |
//! | `trim`/`ltrim`/`rtrim`| `trimFunc` + variants                 |
//! | `replace`             | `replaceFunc`                         |
//! | `hex`                 | `hexFunc`                             |
//! | `quote`               | `quoteFunc`                           |
//! | `coalesce`/`ifnull`   | `coalesceFunc`/`ifnullFunc`           |
//! | `nullif`              | `nullifFunc`                          |
//! | `round`               | `roundFunc`                           |
//! | `unicode`             | `unicodeFunc`                         |
//! | `char`                | `charFunc`                            |
//! | `zeroblob`            | `zeroblobFunc`                        |

#![allow(dead_code)]

use crate::error::SqliteError;
use crate::expr::{FunctionRegistry, SqliteValue};

/// A registry that returns `Err(SqliteError::ERROR)` for any call.
///
/// Used by expression tests that don't need functions.
pub struct NullRegistry;
impl FunctionRegistry for NullRegistry {
    fn call(&self, name: &str, _args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
        Err(SqliteError::ERROR.with_msg(format!("no such function: {name}")))
    }
}

/// The default registry, implementing the slim subset of builtins.
pub struct BuiltinRegistry;
impl FunctionRegistry for BuiltinRegistry {
    fn call(&self, name: &str, args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
        match name.to_ascii_lowercase().as_str() {
            "abs" => abs(args),
            "typeof" => typeof_fn(args),
            "length" => length(args),
            "upper" => upper(args),
            "lower" => lower(args),
            "substr" => substr(args),
            "trim" => trim_fn(args, TrimKind::Both),
            "ltrim" => trim_fn(args, TrimKind::Left),
            "rtrim" => trim_fn(args, TrimKind::Right),
            "replace" => replace(args),
            "hex" => hex(args),
            "quote" => quote_fn(args),
            "coalesce" => coalesce(args),
            "ifnull" => ifnull(args),
            "nullif" => nullif(args),
            "round" => round(args),
            "unicode" => unicode(args),
            "char" => char_fn(args),
            "zeroblob" => zeroblob(args),
            _ => Err(SqliteError::ERROR),
        }
    }
}

enum TrimKind {
    Both,
    Left,
    Right,
}

// ─── SQL null propagation helper ─────────────────────────────────────────

/// SQL equality with NULL propagation: `NULL == anything` is false.
fn eq_sql(a: &SqliteValue, b: &SqliteValue) -> bool {
    if matches!(a, SqliteValue::Null) || matches!(b, SqliteValue::Null) {
        return false;
    }
    match (a, b) {
        (SqliteValue::Integer(x), SqliteValue::Integer(y)) => x == y,
        (SqliteValue::Real(x), SqliteValue::Real(y)) => x == y,
        (SqliteValue::Integer(x), SqliteValue::Real(y)) => (*x as f64) == *y,
        (SqliteValue::Real(x), SqliteValue::Integer(y)) => *x == (*y as f64),
        (SqliteValue::Text(x), SqliteValue::Text(y)) => x == y,
        _ => false,
    }
}

fn null_if_null(v: &SqliteValue) -> Result<&SqliteValue, SqliteError> {
    if matches!(v, SqliteValue::Null) {
        Err(SqliteError::ERROR) // wrong arity / null arg where not allowed
    } else {
        Ok(v)
    }
}

fn to_text(v: &SqliteValue) -> String {
    match v {
        SqliteValue::Null => String::new(),
        SqliteValue::Integer(i) => i.to_string(),
        SqliteValue::Real(f) => format!("{f}"),
        SqliteValue::Text(s) => s.clone(),
        SqliteValue::Blob(b) => String::from_utf8_lossy(b).into_owned(),
    }
}

// ─── Implementations ─────────────────────────────────────────────────────

fn abs(args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
    if args.len() != 1 {
        return Err(SqliteError::ERROR);
    }
    Ok(match &args[0] {
        SqliteValue::Null => SqliteValue::Null,
        SqliteValue::Integer(i) => SqliteValue::Integer(i.wrapping_abs()),
        SqliteValue::Real(f) => SqliteValue::Real(f.abs()),
        other => {
            // Try numeric coercion
            let s = to_text(other);
            if let Ok(i) = s.parse::<i64>() {
                SqliteValue::Integer(i.wrapping_abs())
            } else if let Ok(f) = s.parse::<f64>() {
                SqliteValue::Real(f.abs())
            } else {
                SqliteValue::Null
            }
        }
    })
}

fn typeof_fn(args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
    if args.len() != 1 {
        return Err(SqliteError::ERROR);
    }
    Ok(SqliteValue::Text(args[0].type_of().into()))
}

fn length(args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
    if args.len() != 1 {
        return Err(SqliteError::ERROR);
    }
    Ok(match &args[0] {
        SqliteValue::Null => SqliteValue::Null,
        SqliteValue::Text(s) => SqliteValue::Integer(s.chars().count() as i64),
        SqliteValue::Blob(b) => SqliteValue::Integer(b.len() as i64),
        SqliteValue::Integer(i) => SqliteValue::Integer(i.to_string().len() as i64),
        SqliteValue::Real(f) => SqliteValue::Integer(format!("{f}").len() as i64),
    })
}

fn upper(args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
    if args.len() != 1 {
        return Err(SqliteError::ERROR);
    }
    Ok(match &args[0] {
        SqliteValue::Null => SqliteValue::Null,
        SqliteValue::Text(s) => SqliteValue::Text(s.to_ascii_uppercase()),
        other => SqliteValue::Text(to_text(other).to_ascii_uppercase()),
    })
}

fn lower(args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
    if args.len() != 1 {
        return Err(SqliteError::ERROR);
    }
    Ok(match &args[0] {
        SqliteValue::Null => SqliteValue::Null,
        SqliteValue::Text(s) => SqliteValue::Text(s.to_ascii_lowercase()),
        other => SqliteValue::Text(to_text(other).to_ascii_lowercase()),
    })
}

fn substr(args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
    if args.len() < 2 || args.len() > 3 {
        return Err(SqliteError::ERROR);
    }
    let s = match &args[0] {
        SqliteValue::Null => return Ok(SqliteValue::Null),
        other => to_text(other),
    };
    let start = match &args[1] {
        SqliteValue::Integer(i) => *i,
        SqliteValue::Null => return Ok(SqliteValue::Null),
        other => {
            let t = to_text(other);
            t.parse::<i64>().unwrap_or(1)
        }
    };
    let len: Option<i64> = if args.len() == 3 {
        match &args[2] {
            SqliteValue::Integer(i) => Some(*i),
            SqliteValue::Null => return Ok(SqliteValue::Null),
            other => Some(to_text(other).parse::<i64>().unwrap_or(0)),
        }
    } else {
        None
    };

    // SQLite: start is 1-indexed. Negative starts count from end.
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len() as i64;
    let s_idx = if start > 0 {
        (start - 1) as usize
    } else if start < 0 {
        (n + start).max(0) as usize
    } else {
        0
    };
    if s_idx >= chars.len() {
        return Ok(SqliteValue::Text(String::new()));
    }
    let end_idx = match len {
        Some(l) if l >= 0 => (s_idx as i64 + l).min(n) as usize,
        Some(l) => (n + l).max(s_idx as i64) as usize,
        None => chars.len(),
    };
    let end_idx = end_idx.max(s_idx);
    let out: String = chars[s_idx..end_idx].iter().collect();
    Ok(SqliteValue::Text(out))
}

fn trim_fn(args: &[SqliteValue], kind: TrimKind) -> Result<SqliteValue, SqliteError> {
    if args.is_empty() || args.len() > 2 {
        return Err(SqliteError::ERROR);
    }
    let s = match &args[0] {
        SqliteValue::Null => return Ok(SqliteValue::Null),
        other => to_text(other),
    };
    let chars: Vec<char> = if args.len() == 2 {
        match &args[1] {
            SqliteValue::Null => return Ok(SqliteValue::Null),
            other => to_text(other).chars().collect(),
        }
    } else {
        vec![' ']
    };

    let mut start = 0usize;
    let mut end = s.chars().count();
    let s_chars: Vec<char> = s.chars().collect();

    if matches!(kind, TrimKind::Both | TrimKind::Left) {
        while start < end && chars.contains(&s_chars[start]) {
            start += 1;
        }
    }
    if matches!(kind, TrimKind::Both | TrimKind::Right) {
        while end > start && chars.contains(&s_chars[end - 1]) {
            end -= 1;
        }
    }
    Ok(SqliteValue::Text(s_chars[start..end].iter().collect()))
}

fn replace(args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
    if args.len() != 3 {
        return Err(SqliteError::ERROR);
    }
    if matches!(&args[0], SqliteValue::Null)
        || matches!(&args[1], SqliteValue::Null)
        || matches!(&args[2], SqliteValue::Null)
    {
        return Ok(SqliteValue::Null);
    }
    let s = to_text(&args[0]);
    let from = to_text(&args[1]);
    let to = to_text(&args[2]);
    if from.is_empty() {
        // No-op (matches SQLite: "if from-pattern is empty, return original")
        return Ok(SqliteValue::Text(s));
    }
    let out = s.replace(&from, &to);
    Ok(SqliteValue::Text(out))
}

fn hex(args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
    if args.len() != 1 {
        return Err(SqliteError::ERROR);
    }
    Ok(match &args[0] {
        SqliteValue::Null => SqliteValue::Null,
        SqliteValue::Blob(b) => {
            let mut s = String::with_capacity(b.len() * 2);
            for byte in b {
                s.push_str(&format!("{byte:02X}"));
            }
            SqliteValue::Text(s)
        }
        SqliteValue::Integer(i) => {
            // Treat as two's-complement 64-bit and emit 16 hex digits
            SqliteValue::Text(format!("{:X}", *i as u64))
        }
        SqliteValue::Real(f) => {
            // C: doubles are converted to bytes then hex'd
            let bits = f.to_bits();
            let bytes = bits.to_be_bytes();
            let mut s = String::with_capacity(16);
            for byte in bytes {
                s.push_str(&format!("{byte:02X}"));
            }
            SqliteValue::Text(s)
        }
        other => {
            let s = to_text(other);
            SqliteValue::Text(s.bytes().map(|b| format!("{b:02X}")).collect())
        }
    })
}

fn quote_fn(args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
    if args.len() != 1 {
        return Err(SqliteError::ERROR);
    }
    Ok(match &args[0] {
        SqliteValue::Null => SqliteValue::Text("NULL".into()),
        SqliteValue::Integer(i) => SqliteValue::Text(i.to_string()),
        SqliteValue::Real(f) => SqliteValue::Text(format!("{f}")),
        SqliteValue::Text(s) => {
            let escaped = s.replace('\'', "''");
            SqliteValue::Text(format!("'{escaped}'"))
        }
        SqliteValue::Blob(b) => {
            let mut s = String::with_capacity(b.len() * 2 + 3);
            s.push_str("X'");
            for byte in b {
                s.push_str(&format!("{byte:02x}"));
            }
            s.push('\'');
            SqliteValue::Text(s)
        }
    })
}

fn coalesce(args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
    if args.is_empty() {
        return Err(SqliteError::ERROR);
    }
    for a in args {
        if !matches!(a, SqliteValue::Null) {
            return Ok(a.clone());
        }
    }
    Ok(SqliteValue::Null)
}

fn ifnull(args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
    if args.len() != 2 {
        return Err(SqliteError::ERROR);
    }
    Ok(if matches!(&args[0], SqliteValue::Null) {
        args[1].clone()
    } else {
        args[0].clone()
    })
}

fn nullif(args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
    if args.len() != 2 {
        return Err(SqliteError::ERROR);
    }
    if eq_sql(&args[0], &args[1]) {
        Ok(SqliteValue::Null)
    } else {
        Ok(args[0].clone())
    }
}

fn round(args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
    if args.is_empty() || args.len() > 2 {
        return Err(SqliteError::ERROR);
    }
    let x = match &args[0] {
        SqliteValue::Null => return Ok(SqliteValue::Null),
        other => other,
    };
    let digits: i64 = if args.len() == 2 {
        match &args[1] {
            SqliteValue::Null => return Ok(SqliteValue::Null),
            SqliteValue::Integer(i) => *i,
            other => to_text(other).parse::<i64>().unwrap_or(0),
        }
    } else {
        0
    };

    let factor = 10f64.powi(digits as i32);
    let r = match x {
        SqliteValue::Integer(i) => (*i as f64) / factor,
        SqliteValue::Real(f) => *f / factor,
        other => {
            let s = to_text(other);
            s.parse::<f64>().unwrap_or(0.0) / factor
        }
    };
    let rounded = round_half_away(r);
    if digits == 0 {
        // Return integer if digits == 0
        Ok(SqliteValue::Integer(rounded as i64))
    } else {
        let back = rounded * factor;
        Ok(SqliteValue::Real(back))
    }
}

fn round_half_away(x: f64) -> f64 {
    if x >= 0.0 {
        (x + 0.5).floor()
    } else {
        -(x.abs() + 0.5).floor()
    }
}

fn unicode(args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
    if args.len() != 1 {
        return Err(SqliteError::ERROR);
    }
    Ok(match &args[0] {
        SqliteValue::Null => SqliteValue::Null,
        other => {
            let s = to_text(other);
            match s.chars().next() {
                Some(c) => SqliteValue::Integer(c as i64),
                None => SqliteValue::Null,
            }
        }
    })
}

fn char_fn(args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
    if args.is_empty() {
        return Err(SqliteError::ERROR);
    }
    let mut s = String::new();
    for a in args {
        if matches!(a, SqliteValue::Null) {
            return Ok(SqliteValue::Null);
        }
        let n: i64 = match a {
            SqliteValue::Integer(i) => *i,
            SqliteValue::Real(f) => *f as i64,
            other => match to_text(other).parse::<i64>() {
                Ok(i) => i,
                Err(_) => return Ok(SqliteValue::Null),
            },
        };
        if let Some(c) = char::from_u32(n as u32) {
            s.push(c);
        } else {
            return Ok(SqliteValue::Null);
        }
    }
    Ok(SqliteValue::Text(s))
}

fn zeroblob(args: &[SqliteValue]) -> Result<SqliteValue, SqliteError> {
    if args.len() != 1 {
        return Err(SqliteError::ERROR);
    }
    let n: i64 = match &args[0] {
        SqliteValue::Null => return Ok(SqliteValue::Null),
        SqliteValue::Integer(i) if *i >= 0 => *i,
        other => return Ok(SqliteValue::Null),
    };
    Ok(SqliteValue::Blob(vec![0u8; n as usize]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abs_int() {
        assert_eq!(abs(&[SqliteValue::Integer(-5)]).unwrap(), SqliteValue::Integer(5));
    }
    #[test]
    fn abs_null() {
        assert_eq!(abs(&[SqliteValue::Null]).unwrap(), SqliteValue::Null);
    }
    #[test]
    fn length_text() {
        assert_eq!(
            length(&[SqliteValue::Text("hello".into())]).unwrap(),
            SqliteValue::Integer(5)
        );
    }
    #[test]
    fn upper_lower() {
        assert_eq!(
            upper(&[SqliteValue::Text("Abc".into())]).unwrap(),
            SqliteValue::Text("ABC".into())
        );
        assert_eq!(
            lower(&[SqliteValue::Text("AbC".into())]).unwrap(),
            SqliteValue::Text("abc".into())
        );
    }
    #[test]
    fn substr_basic() {
        assert_eq!(
            substr(&[
                SqliteValue::Text("hello".into()),
                SqliteValue::Integer(2),
                SqliteValue::Integer(3)
            ])
            .unwrap(),
            SqliteValue::Text("ell".into())
        );
    }
    #[test]
    fn trim_default() {
        assert_eq!(
            trim_fn(
                &[SqliteValue::Text("  hi  ".into())],
                TrimKind::Both
            )
            .unwrap(),
            SqliteValue::Text("hi".into())
        );
    }
    #[test]
    fn replace_basic() {
        assert_eq!(
            replace(&[
                SqliteValue::Text("hello".into()),
                SqliteValue::Text("l".into()),
                SqliteValue::Text("L".into())
            ])
            .unwrap(),
            SqliteValue::Text("heLLo".into())
        );
    }
    #[test]
    fn hex_int() {
        assert_eq!(
            hex(&[SqliteValue::Integer(0xff)]).unwrap(),
            SqliteValue::Text("FF".into())
        );
    }
    #[test]
    fn quote_text() {
        assert_eq!(
            quote_fn(&[SqliteValue::Text("hi".into())]).unwrap(),
            SqliteValue::Text("'hi'".into())
        );
    }
    #[test]
    fn quote_null() {
        assert_eq!(
            quote_fn(&[SqliteValue::Null]).unwrap(),
            SqliteValue::Text("NULL".into())
        );
    }
    #[test]
    fn coalesce_skip_nulls() {
        assert_eq!(
            coalesce(&[SqliteValue::Null, SqliteValue::Integer(5)]).unwrap(),
            SqliteValue::Integer(5)
        );
    }
    #[test]
    fn ifnull_basic() {
        assert_eq!(
            ifnull(&[SqliteValue::Null, SqliteValue::Integer(7)]).unwrap(),
            SqliteValue::Integer(7)
        );
    }
    #[test]
    fn nullif_eq() {
        assert_eq!(
            nullif(&[SqliteValue::Integer(5), SqliteValue::Integer(5)]).unwrap(),
            SqliteValue::Null
        );
    }
    #[test]
    fn nullif_neq() {
        assert_eq!(
            nullif(&[SqliteValue::Integer(5), SqliteValue::Integer(7)]).unwrap(),
            SqliteValue::Integer(5)
        );
    }
    #[test]
    fn round_default() {
        assert_eq!(
            round(&[SqliteValue::Real(2.6)]).unwrap(),
            SqliteValue::Integer(3)
        );
    }
    #[test]
    fn unicode_a() {
        assert_eq!(
            unicode(&[SqliteValue::Text("A".into())]).unwrap(),
            SqliteValue::Integer(65)
        );
    }
    #[test]
    fn char_multi() {
        assert_eq!(
            char_fn(&[SqliteValue::Integer(72), SqliteValue::Integer(105)]).unwrap(),
            SqliteValue::Text("Hi".into())
        );
    }
    #[test]
    fn zeroblob_5() {
        let r = zeroblob(&[SqliteValue::Integer(5)]).unwrap();
        assert_eq!(r, SqliteValue::Blob(vec![0u8; 5]));
    }
    #[test]
    fn unknown_returns_err() {
        let r = BuiltinRegistry.call("nosuch", &[]);
        assert!(r.is_err());
    }
    #[test]
    fn typeof_int() {
        assert_eq!(
            typeof_fn(&[SqliteValue::Integer(1)]).unwrap(),
            SqliteValue::Text("integer".into())
        );
    }
    #[test]
    fn typeof_null() {
        assert_eq!(
            typeof_fn(&[SqliteValue::Null]).unwrap(),
            SqliteValue::Text("null".into())
        );
    }
}
