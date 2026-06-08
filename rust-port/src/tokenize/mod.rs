//! SQL Tokenizer — partial port of `sqlite-source/src/tokenize.c`.
//!
//! Implements a slim tokenizer that lexes SQL into tokens. Supports:
//! - Identifiers (TK_ID)
//! - String literals (TK_STRING) — single-quoted with `''` escape
//! - Blob literals (TK_BLOB) — `X'...'` form
//! - Integer literals (TK_INTEGER)
//! - Float literals (TK_FLOAT)
//! - Punctuation: `( ) , ; . + - * / % = < > <= >= != ! ~ & | ^`
//! - Keywords (recognized case-insensitively): SELECT, FROM, WHERE,
//!   INSERT, INTO, VALUES, UPDATE, SET, DELETE, CREATE, TABLE,
//!   DROP, INDEX, VIEW, BEGIN, COMMIT, ROLLBACK, AND, OR, NOT,
//!   NULL, IS, IN, LIKE, GLOB, BETWEEN, AS, ON, USING, JOIN,
//!   LEFT, RIGHT, INNER, OUTER, CROSS, NATURAL, ON, ALL, DISTINCT,
//!   GROUP, BY, HAVING, ORDER, LIMIT, OFFSET, ASC, DESC
//!
//! # C source correspondence
//!
//! | Rust item       | C source                          |
//! |-----------------|-----------------------------------|
//! | `TokenKind`     | `enum TokenType` (parse.h)        |
//! | `Token`         | (pToken, pParse) in getToken       |
//! | `tokenize`      | `sqlite3GetToken` (tokenize.c:354) |
//!
//! # Behavior contract
//!
//! - Whitespace is skipped between tokens.
//! - Line comments (`-- ...`) are skipped until end of line.
//! - Block comments (`/* ... */`) are skipped (no nesting).
//! - String literals use single quotes; doubled `''` is an escape
//!   for a single `'` inside the string.
//! - Blob literals: `X'...'` or `x'...'` (case-insensitive prefix).
//! - Keywords are matched case-insensitively (the canonical form is
//!   uppercase; the C source does the same).
//! - Numeric literals: integer if no `.` or `e`/`E`; otherwise float.

use crate::error::SqliteError;

/// A token kind — the equivalent of the C source's `enum TokenType`.
///
/// We use a Rust enum to keep the public surface type-safe. The
/// discriminants are arbitrary; the C source uses sequential integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    /// End of input.
    Eof,
    /// White space (skipped during tokenization).
    Space,
    /// Comment (skipped during tokenization).
    Comment,
    /// Identifier (column / table / function name, possibly a
    /// keyword that the parser doesn't recognize).
    Id,
    /// String literal: `'...'`.
    String,
    /// Blob literal: `X'...'`.
    Blob,
    /// Integer literal.
    Integer,
    /// Float literal.
    Float,
    /// Variable: `?`, `:name`, `@name`, or `$name`.
    Variable,
    /// Punctuation.
    Punct(Punct),
    /// Keyword.
    Keyword(Keyword),
}

/// Punctuation kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Punct {
    LParen,    // (
    RParen,    // )
    Comma,     // ,
    Semi,      // ;
    Dot,       // .
    Plus,      // +
    Minus,     // -
    Star,      // *
    Slash,     // /
    Percent,   // %
    Eq,        // =
    Lt,        // <
    Gt,        // >
    Le,        // <=
    Ge,        // >=
    Ne,        // != or <>
    Concat,    // ||
    BitAnd,    // &
    BitOr,     // |
    BitNot,    // ~
    LShift,    // <<
    RShift,    // >>
}

/// SQL keywords (subset). The C source has 100+ keywords; we
/// implement the most common ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Keyword {
    Select,
    From,
    Where,
    Insert,
    Into,
    Values,
    Update,
    Set,
    Delete,
    Create,
    Table,
    Drop,
    Index,
    View,
    Trigger,
    Begin,
    Commit,
    Rollback,
    Savepoint,
    Release,
    And,
    Or,
    Not,
    Null,
    Is,
    In,
    Like,
    Glob,
    Between,
    As,
    On,
    Using,
    Join,
    Left,
    Right,
    Inner,
    Outer,
    Cross,
    Natural,
    All,
    Distinct,
    Group,
    By,
    Having,
    Order,
    Limit,
    Offset,
    Asc,
    Desc,
    If,
    Else,
    Case,
    When,
    Then,
    End,
    Primary,
    Key,
    Unique,
    Check,
    Default,
    References,
    Foreign,
    /// Unknown identifier (not a keyword).
    NotKeyword,
}

/// A token: kind + the source text that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    /// What kind of token.
    pub kind: TokenKind,
    /// The source text (slice of the original input).
    pub text: String,
    /// Line number (1-based) where the token starts.
    pub line: u32,
    /// Column number (0-based) within the line.
    pub col: u32,
}

impl Token {
    /// Construct a token.
    pub fn new(kind: TokenKind, text: impl Into<String>, line: u32, col: u32) -> Self {
        Token {
            kind,
            text: text.into(),
            line,
            col,
        }
    }
}

/// Tokenize an entire SQL string into a `Vec<Token>`. Returns an
/// `Err` with the line/col of the first syntax error.
///
/// Mirrors the C source's `sqlite3GetToken` (tokenize.c:354) called
/// in a loop. Whitespace and comments are filtered out.
pub fn tokenize(input: &str) -> Result<Vec<Token>, SqliteError> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    let mut line: u32 = 1;
    let mut col: u32 = 0;
    while i < bytes.len() {
        // Skip whitespace.
        let c = bytes[i];
        if c == b'\n' {
            line += 1;
            col = 0;
            i += 1;
            continue;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            col += 1;
            continue;
        }
        // Line comment: -- ... \n
        if c == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment: /* ... */
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            col += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                if bytes[i] == b'\n' {
                    line += 1;
                    col = 0;
                } else {
                    col += 1;
                }
                i += 1;
            }
            if i + 1 < bytes.len() {
                i += 2; // skip */
                col += 2;
            }
            continue;
        }
        // String literal
        if c == b'\'' {
            let start_line = line;
            let start_col = col;
            i += 1;
            col += 1;
            let text_start = i;
            loop {
                if i >= bytes.len() {
                    return Err(SqliteError(1)); // unterminated
                }
                let bc = bytes[i];
                if bc == b'\n' {
                    return Err(SqliteError(1)); // SQLITE_ERROR
                }
                if bc == b'\''
                    && i + 1 < bytes.len()
                    && bytes[i + 1] == b'\''
                {
                    // Doubled '' is an escape for ' inside the string.
                    i += 2;
                    col += 2;
                    continue;
                }
                if bc == b'\'' {
                    // Closing quote.
                    break;
                }
                i += 1;
                col += 1;
            }
            let body = &input[text_start..i];
            i += 1; // skip closing quote
            col += (i - text_start) as u32 + 1;
            tokens.push(Token::new(TokenKind::String, body, start_line, start_col));
            continue;
        }
        // Blob literal
        if (c == b'X' || c == b'x')
            && i + 1 < bytes.len()
            && bytes[i + 1] == b'\''
        {
            let start_line = line;
            let start_col = col;
            i += 2;
            col += 2;
            let text_start = i;
            while i < bytes.len() && bytes[i] != b'\'' {
                i += 1;
            }
            if i >= bytes.len() {
                return Err(SqliteError(1));
            }
            let body = &input[text_start..i];
            i += 1;
            col += (i - text_start) as u32 + 1;
            tokens.push(Token::new(TokenKind::Blob, body, start_line, start_col));
            continue;
        }
        // Identifier or keyword
        if c == b'_' || c.is_ascii_alphabetic() {
            let start_line = line;
            let start_col = col;
            let start = i;
            while i < bytes.len()
                && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric())
            {
                i += 1;
                col += 1;
            }
            let text = &input[start..i];
            let upper = text.to_ascii_uppercase();
            let kw = match_keyword(&upper);
            let kind = if kw == Keyword::NotKeyword {
                TokenKind::Id
            } else {
                TokenKind::Keyword(kw)
            };
            tokens.push(Token::new(kind, text, start_line, start_col));
            continue;
        }
        // Number
        if c.is_ascii_digit() {
            let start_line = line;
            let start_col = col;
            let start = i;
            let mut is_float = false;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
                col += 1;
            }
            if i < bytes.len() && bytes[i] == b'.' {
                is_float = true;
                i += 1;
                col += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                    col += 1;
                }
            }
            if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                is_float = true;
                i += 1;
                col += 1;
                if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                    i += 1;
                    col += 1;
                }
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                    col += 1;
                }
            }
            let text = &input[start..i];
            let kind = if is_float {
                TokenKind::Float
            } else {
                TokenKind::Integer
            };
            tokens.push(Token::new(kind, text, start_line, start_col));
            continue;
        }
        // Variable
        if c == b'?' || c == b'$' || c == b':' || c == b'@' {
            let start_line = line;
            let start_col = col;
            let start = i;
            i += 1;
            col += 1;
            // For ?, $: optional name (number, alpha, or _). For
            // :name, @name: must start with alpha or _.
            if c == b'?' || c == b'$' {
                if i < bytes.len()
                    && (bytes[i].is_ascii_alphabetic()
                        || bytes[i].is_ascii_digit()
                        || bytes[i] == b'_')
                {
                    while i < bytes.len()
                        && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
                    {
                        i += 1;
                        col += 1;
                    }
                }
            } else {
                // :name, @name
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
                {
                    i += 1;
                    col += 1;
                }
            }
            let text = &input[start..i];
            tokens.push(Token::new(TokenKind::Variable, text, start_line, start_col));
            continue;
        }
        // Punctuation (multi-character operators first).
        let start_line = line;
        let start_col = col;
        let text_start = i;
        let punct = match_punct(bytes, &mut i, &mut col);
        match punct {
            Some(p) => {
                let text = input.get(text_start..i).unwrap_or("").to_string();
                tokens.push(Token::new(TokenKind::Punct(p), text, start_line, start_col));
            }
            None => {
                return Err(SqliteError(1));
            }
        }
    }
    Ok(tokens)
}

fn match_punct(bytes: &[u8], i: &mut usize, col: &mut u32) -> Option<Punct> {
    let c = bytes[*i];
    let next = bytes.get(*i + 1).copied();
    let p = match (c, next) {
        (b'(', _) => Punct::LParen,
        (b')', _) => Punct::RParen,
        (b',', _) => Punct::Comma,
        (b';', _) => Punct::Semi,
        (b'.', _) => Punct::Dot,
        (b'+', _) => Punct::Plus,
        (b'-', _) => Punct::Minus,
        (b'*', _) => Punct::Star,
        (b'/', _) => Punct::Slash,
        (b'%', _) => Punct::Percent,
        (b'=', _) => Punct::Eq,
        (b'<', Some(b'=')) => Punct::Le,
        (b'<', Some(b'>')) => Punct::Ne,
        (b'<', Some(b'<')) => Punct::LShift,
        (b'<', _) => Punct::Lt,
        (b'>', Some(b'=')) => Punct::Ge,
        (b'>', Some(b'>')) => Punct::RShift,
        (b'>', _) => Punct::Gt,
        (b'!', Some(b'=')) => Punct::Ne,
        (b'!', _) => return None,
        (b'~', _) => Punct::BitNot,
        (b'|', Some(b'|')) => Punct::Concat,
        (b'|', _) => Punct::BitOr,
        (b'&', _) => Punct::BitAnd,
        _ => return None,
    };
    *i += 1;
    *col += 1;
    if next.is_some() && matches!(p, Punct::Le | Punct::Ge | Punct::Ne | Punct::LShift | Punct::RShift | Punct::Concat) {
        *i += 1;
        *col += 1;
    }
    Some(p)
}

fn match_keyword(upper: &str) -> Keyword {
    match upper {
        "SELECT" => Keyword::Select,
        "FROM" => Keyword::From,
        "WHERE" => Keyword::Where,
        "INSERT" => Keyword::Insert,
        "INTO" => Keyword::Into,
        "VALUES" => Keyword::Values,
        "UPDATE" => Keyword::Update,
        "SET" => Keyword::Set,
        "DELETE" => Keyword::Delete,
        "CREATE" => Keyword::Create,
        "TABLE" => Keyword::Table,
        "DROP" => Keyword::Drop,
        "INDEX" => Keyword::Index,
        "VIEW" => Keyword::View,
        "TRIGGER" => Keyword::Trigger,
        "BEGIN" => Keyword::Begin,
        "COMMIT" => Keyword::Commit,
        "ROLLBACK" => Keyword::Rollback,
        "SAVEPOINT" => Keyword::Savepoint,
        "RELEASE" => Keyword::Release,
        "AND" => Keyword::And,
        "OR" => Keyword::Or,
        "NOT" => Keyword::Not,
        "NULL" => Keyword::Null,
        "IS" => Keyword::Is,
        "IN" => Keyword::In,
        "LIKE" => Keyword::Like,
        "GLOB" => Keyword::Glob,
        "BETWEEN" => Keyword::Between,
        "AS" => Keyword::As,
        "ON" => Keyword::On,
        "USING" => Keyword::Using,
        "JOIN" => Keyword::Join,
        "LEFT" => Keyword::Left,
        "RIGHT" => Keyword::Right,
        "INNER" => Keyword::Inner,
        "OUTER" => Keyword::Outer,
        "CROSS" => Keyword::Cross,
        "NATURAL" => Keyword::Natural,
        "ALL" => Keyword::All,
        "DISTINCT" => Keyword::Distinct,
        "GROUP" => Keyword::Group,
        "BY" => Keyword::By,
        "HAVING" => Keyword::Having,
        "ORDER" => Keyword::Order,
        "LIMIT" => Keyword::Limit,
        "OFFSET" => Keyword::Offset,
        "ASC" => Keyword::Asc,
        "DESC" => Keyword::Desc,
        "IF" => Keyword::If,
        "ELSE" => Keyword::Else,
        "CASE" => Keyword::Case,
        "WHEN" => Keyword::When,
        "THEN" => Keyword::Then,
        "END" => Keyword::End,
        "PRIMARY" => Keyword::Primary,
        "KEY" => Keyword::Key,
        "UNIQUE" => Keyword::Unique,
        "CHECK" => Keyword::Check,
        "DEFAULT" => Keyword::Default,
        "REFERENCES" => Keyword::References,
        "FOREIGN" => Keyword::Foreign,
        _ => Keyword::NotKeyword,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(tokens: &[Token]) -> Vec<TokenKind> {
        tokens.iter().map(|t| t.kind).collect()
    }

    #[test]
    fn empty() {
        assert_eq!(tokenize("").unwrap(), vec![]);
    }

    #[test]
    fn whitespace_only() {
        assert_eq!(tokenize("   \n\t").unwrap(), vec![]);
    }

    #[test]
    fn simple_select() {
        let toks = tokenize("SELECT * FROM t").unwrap();
        assert_eq!(
            kinds(&toks),
            vec![
                TokenKind::Keyword(Keyword::Select),
                TokenKind::Punct(Punct::Star),
                TokenKind::Keyword(Keyword::From),
                TokenKind::Id,
            ]
        );
    }

    #[test]
    fn identifier() {
        let toks = tokenize("my_table column1").unwrap();
        assert_eq!(kinds(&toks), vec![TokenKind::Id, TokenKind::Id]);
    }

    #[test]
    fn string_literal() {
        let toks = tokenize("'hello'").unwrap();
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].kind, TokenKind::String);
        assert_eq!(toks[0].text, "hello");
    }

    #[test]
    fn string_with_escaped_quote() {
        // Doubled '' is an escape for ' inside a string.
        let toks = tokenize("'it''s'").unwrap();
        assert_eq!(toks[0].kind, TokenKind::String);
        // The body has '' as the escaped ' sequence (5 chars).
        assert_eq!(toks[0].text, "it''s");
    }

    #[test]
    fn blob_literal() {
        let toks = tokenize("X'1234'").unwrap();
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].kind, TokenKind::Blob);
        assert_eq!(toks[0].text, "1234");
    }

    #[test]
    fn integer_literal() {
        let toks = tokenize("42").unwrap();
        assert_eq!(kinds(&toks), vec![TokenKind::Integer]);
    }

    #[test]
    fn float_literal() {
        let toks = tokenize("3.14").unwrap();
        assert_eq!(kinds(&toks), vec![TokenKind::Float]);
    }

    #[test]
    fn float_with_exponent() {
        let toks = tokenize("1.5e10").unwrap();
        assert_eq!(kinds(&toks), vec![TokenKind::Float]);
    }

    #[test]
    fn line_comment_skipped() {
        let toks = tokenize("SELECT -- a comment\n  *").unwrap();
        assert_eq!(
            kinds(&toks),
            vec![
                TokenKind::Keyword(Keyword::Select),
                TokenKind::Punct(Punct::Star),
            ]
        );
    }

    #[test]
    fn block_comment_skipped() {
        let toks = tokenize("SELECT /* hi */ *").unwrap();
        assert_eq!(
            kinds(&toks),
            vec![
                TokenKind::Keyword(Keyword::Select),
                TokenKind::Punct(Punct::Star),
            ]
        );
    }

    #[test]
    fn keywords_case_insensitive() {
        let toks = tokenize("select from Where").unwrap();
        assert_eq!(toks[0].kind, TokenKind::Keyword(Keyword::Select));
        assert_eq!(toks[1].kind, TokenKind::Keyword(Keyword::From));
        assert_eq!(toks[2].kind, TokenKind::Keyword(Keyword::Where));
    }

    #[test]
    fn multi_char_punct() {
        let toks = tokenize("a<=b>=c!=d<>e||f&g|h").unwrap();
        let ks = kinds(&toks);
        // 8 ids + 7 puncts
        assert_eq!(ks.len(), 15);
        // Spot-check the puncts
        let puncts: Vec<_> = ks.iter()
            .filter_map(|k| if let TokenKind::Punct(p) = k { Some(*p) } else { None })
            .collect();
        assert_eq!(puncts, vec![
            Punct::Le, Punct::Ge, Punct::Ne, Punct::Ne, Punct::Concat,
            Punct::BitAnd, Punct::BitOr,
        ]);
    }

    #[test]
    fn variable_placeholder() {
        let toks = tokenize("? :name @var $1").unwrap();
        assert_eq!(kinds(&toks), vec![
            TokenKind::Variable, TokenKind::Variable, TokenKind::Variable, TokenKind::Variable,
        ]);
    }

    #[test]
    fn complex_query() {
        let sql = "SELECT DISTINCT a.col, b.col + 1 AS c FROM t1 AS a JOIN t2 AS b ON a.id = b.id WHERE a.x > 0 AND b.y < 100 ORDER BY a.col LIMIT 10";
        let toks = tokenize(sql).unwrap();
        // Sanity: should produce many tokens without errors.
        assert!(toks.len() > 30);
    }

    #[test]
    fn line_number_tracking() {
        let toks = tokenize("SELECT\n  *\nFROM t").unwrap();
        assert_eq!(toks[0].line, 1); // SELECT
        assert_eq!(toks[1].line, 2); // *
        assert_eq!(toks[2].line, 3); // FROM
    }
}
