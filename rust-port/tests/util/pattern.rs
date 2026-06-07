//! `tests/util/pattern.rs` — sqlite3_strglob / sqlite3_strlike严格对齐 C行为测试。
//!
//! 本文件用 `oracle/sqlite3`同样的输入跑 Rust 实现,
//! 每个测试在文档注释里标出 C端 `func.c` 行号 +期望返回值。
//!
//! 注意:C端定义在 `func.c:712-714`:
//! ```c
//! #define SQLITE_MATCH0
//! #define SQLITE_NOMATCH1
//! #define SQLITE_NOWILDCARDMATCH2
//! ```
//! 这与 spec 中提到的"SQLITE_NOMATCH(27)"不同 — spec 把 SQLITE_NOTICE(27)误标了;
//!实际 pattern 比较的失败返回1 或2,不是27。我们以 C源码为准。

use libsqlite_rs::{
 sqlite3_strglob, sqlite3_strlike, SQLITE_MATCH, SQLITE_NOMATCH, SQLITE_NOWILDCARDMATCH,
};

// ============================================================================
//1. sqlite3_strglob: `a*`匹配 `abc` → MATCH (func.c:887-895)
// ============================================================================
#[test]
fn strglob_star_matches_abc() {
 let r = sqlite3_strglob(Some(b"a*"), Some(b"abc"));
 assert_eq!(r, SQLITE_MATCH, "a* must match abc");
}

// ============================================================================
//2. sqlite3_strglob: `a*` 不匹配 `bc` → NOMATCH
//    C 行为验证:实际 sqlite3_strglob("a*", "bc") == 1 (SQLITE_NOMATCH)
//    说明:虽然 pattern 含 '*',但 C 版在找不到首字符 'a' 后 break,
//    外层 fallthrough 返回 SQLITE_NOWILDCARDMATCH(2)... 但实际编译调用
//    sqlite3_strglob("a*", "bc") 返回 1。修正:期望 SQLITE_NOMATCH(1)。
// ============================================================================
#[test]
fn strglob_star_no_match_returns_nowildcard() {
 let r = sqlite3_strglob(Some(b"a*"), Some(b"bc"));
 assert_eq!(
 r, SQLITE_NOMATCH,
 "a* must not match bc — verified against C sqlite3_strglob (returns 1)"
 );
}

// ============================================================================
//3. sqlite3_strglob: 空 pattern匹配空串 → MATCH
// ============================================================================
#[test]
fn strglob_empty_pattern_matches_empty_string() {
 let r = sqlite3_strglob(Some(b""), Some(b""));
 assert_eq!(r, SQLITE_MATCH);
}

// ============================================================================
//4. sqlite3_strglob: `?`匹配单字符 `a`
// ============================================================================
#[test]
fn strglob_question_matches_single_char() {
 assert_eq!(sqlite3_strglob(Some(b"?"), Some(b"a")), SQLITE_MATCH);
}

// ============================================================================
//5. sqlite3_strglob: `?` 不匹配空串 → NOMATCH
// ============================================================================
#[test]
fn strglob_question_no_match_empty() {
 assert_eq!(sqlite3_strglob(Some(b"?"), Some(b"")), SQLITE_NOMATCH);
}

// ============================================================================
//6. sqlite3_strglob: `abc` 字面量匹配 `abc`
// ============================================================================
#[test]
fn strglob_literal_match() {
 assert_eq!(sqlite3_strglob(Some(b"abc"), Some(b"abc")), SQLITE_MATCH);
}

// ============================================================================
//7. sqlite3_strglob: 大小写敏感 — `ABC` 不匹配 `abc`
// ============================================================================
#[test]
fn strglob_is_case_sensitive() {
 assert_eq!(
 sqlite3_strglob(Some(b"ABC"), Some(b"abc")),
 SQLITE_NOMATCH,
 "GLOB is case-sensitive"
 );
}

// ============================================================================
//8. sqlite3_strglob: `.` 是字面字符,必须精确匹配 — `a.c` 不匹配 `abc`
//    C 行为验证:实际 sqlite3_strglob("a.c","abc")=1, strglob("a.c","axc")=1,
//    strglob("a.c","ac")=1。说明:.在 GLOB 中是普通字面字符,不是通配符;
//    它只匹配字面意义的 '.'(0x2E),不匹配 'b'/'x'/'c'。要匹配任意单字符用 '?'。
// ============================================================================
#[test]
fn strglob_dot_in_pattern_is_literal() {
 assert_eq!(
 sqlite3_strglob(Some(b"a.c"), Some(b"abc")),
 SQLITE_NOMATCH,
 "`.` is a literal in GLOB, must match `.` exactly (C verified: returns 1)"
 );
 assert_eq!(
 sqlite3_strglob(Some(b"a.c"), Some(b"axc")),
 SQLITE_NOMATCH,
 "`.` is a literal in GLOB, must match `.` exactly (C verified: returns 1)"
 );
 assert_eq!(sqlite3_strglob(Some(b"a.c"), Some(b"ac")), SQLITE_NOMATCH);
}

// ============================================================================
//9. sqlite3_strglob: NULL pointer 处理 (func.c:888-891)
// ============================================================================
#[test]
fn strglob_null_string_handling() {
 // zString == NULL → return zGlobPattern != NULL
 assert_eq!(sqlite3_strglob(Some(b"x*"), None), SQLITE_NOMATCH);
 // zString == NULL 且 zGlobPattern == NULL → return0 (MATCH)
 assert_eq!(sqlite3_strglob(None, None), SQLITE_MATCH);
 // zGlobPattern == NULL 但 zString != NULL → return1 (NOMATCH)
 assert_eq!(sqlite3_strglob(None, Some(b"abc")), SQLITE_NOMATCH);
}

// ============================================================================
//10. sqlite3_strlike: `%`匹配零或多字符 — `a%`匹配 `abc`
// ============================================================================
#[test]
fn strlike_percent_matches_any() {
 assert_eq!(
 sqlite3_strlike(Some(b"a%"), Some(b"abc"),0),
 SQLITE_MATCH,
 "LIKE %% matches zero or more chars (func.c:704, no_case=1)"
 );
 assert_eq!(sqlite3_strlike(Some(b"a%"), Some(b"a"),0), SQLITE_MATCH);
 // 对齐 C:strlike("a%","bc",0) 返回 SQLITE_NOMATCH(1) — 见 func.c:878
 // (pat='a' vs str='b',noCase 仍不等,不会进 matchAll 分支)
 assert_eq!(sqlite3_strlike(Some(b"a%"), Some(b"bc"),0), SQLITE_NOMATCH);
}

// ============================================================================
//11. sqlite3_strlike: `_`匹配单字符 — `a_`匹配 `ab` 但不匹配 `abc`
// ============================================================================
#[test]
fn strlike_underscore_matches_single() {
 assert_eq!(sqlite3_strlike(Some(b"a_"), Some(b"ab"),0), SQLITE_MATCH);
 assert_eq!(
 sqlite3_strlike(Some(b"a_"), Some(b"abc"),0),
 SQLITE_NOMATCH,
 "_ matches exactly one char"
 );
 assert_eq!(sqlite3_strlike(Some(b"a_"), Some(b"a"),0), SQLITE_NOMATCH);
}

// ============================================================================
//12. sqlite3_strlike: 大小写不敏感 — `ABC`匹配 `abc`
// ============================================================================
#[test]
fn strlike_case_insensitive_default() {
 assert_eq!(sqlite3_strlike(Some(b"ABC"), Some(b"abc"),0), SQLITE_MATCH);
 assert_eq!(sqlite3_strlike(Some(b"AbC"), Some(b"aBc"),0), SQLITE_MATCH);
}

// ============================================================================
//13. sqlite3_strlike: ESCAPE字符转义 `*`(LIKE 中实际是 `%`/`_`)
// ============================================================================
#[test]
fn strlike_escape_backslash() {
 // ESCAPE '\\' 转义 '%' → 字面量匹配
 assert_eq!(
 sqlite3_strlike(Some(b"a\\%b"), Some(b"a%b"), b'\\' as u32),
 SQLITE_MATCH,
 "\\ must escape %% as literal"
 );
 assert_eq!(
 sqlite3_strlike(Some(b"a\\%b"), Some(b"axb"), b'\\' as u32),
 SQLITE_NOMATCH
 );
 // 转义 `_`
 assert_eq!(
 sqlite3_strlike(Some(b"a\\_b"), Some(b"a_b"), b'\\' as u32),
 SQLITE_MATCH
 );
 assert_eq!(
 sqlite3_strlike(Some(b"a\\_b"), Some(b"axb"), b'\\' as u32),
 SQLITE_NOMATCH
 );
}

// ============================================================================
//14. sqlite3_strlike: ESCAPE0行为 — esc=0 时任何字符都是字面量,
// 转义失效,`%`始终是通配符 (func.c:901-909;LIKE 默认 no_case=1)
// ============================================================================
#[test]
fn strlike_escape_zero_disables_escape() {
 // esc=0 → '%'仍然是通配符(即使前一个字符看起来像 escape)
 //实际 SQLite行为:esc=0 表示不启用 ESCAPE 处理,完全字面化 %/_
 // 测试:模式中裸 '%'匹配0+字符
 assert_eq!(
 sqlite3_strlike(Some(b"a%b"), Some(b"axb"),0),
 SQLITE_MATCH,
 "with esc=0, %% is still wildcard"
 );
 //模式中带 '\\' 但 esc=0 → '\\' 是字面字符(不是 escape),匹配需要 '\\'
 assert_eq!(
 sqlite3_strlike(Some(b"a\\%b"), Some(b"a\\xb"),0),
 SQLITE_MATCH,
 "with esc=0, \\\\ is literal backslash, %% is wildcard"
 );
}

// ============================================================================
//15. sqlite3_strglob: `[...]`字符集 — C端支持,本任务 spec 未要求,
// 但既然要"byte-for-byte 一致",保留即可。`[` 作为 match_other 参数传入。
// ============================================================================
#[test]
fn strglob_character_class() {
 assert_eq!(sqlite3_strglob(Some(b"[abc]"), Some(b"a")), SQLITE_MATCH);
 assert_eq!(sqlite3_strglob(Some(b"[abc]"), Some(b"d")), SQLITE_NOMATCH);
 assert_eq!(
 sqlite3_strglob(Some(b"[^abc]"), Some(b"d")),
 SQLITE_MATCH,
 "^ inverts class"
 );
 assert_eq!(
 sqlite3_strglob(Some(b"[a-z]"), Some(b"m")),
 SQLITE_MATCH,
 "range a-z"
 );
}

// ============================================================================
//16. sqlite3_strglob: `*` 在末尾匹配任意后缀
// ============================================================================
#[test]
fn strglob_trailing_star() {
 assert_eq!(sqlite3_strglob(Some(b"*.txt"), Some(b"a.txt")), SQLITE_MATCH);
 assert_eq!(
 sqlite3_strglob(Some(b"*.txt"), Some(b"a.csv")),
 SQLITE_NOWILDCARDMATCH
 );
}

// ============================================================================
//17. sqlite3_strlike: NULL pointer 处理 (func.c:902-905)
// ============================================================================
#[test]
fn strlike_null_string_handling() {
 assert_eq!(sqlite3_strlike(Some(b"a%"), None,0), SQLITE_NOMATCH);
 assert_eq!(sqlite3_strlike(None, None,0), SQLITE_MATCH);
 assert_eq!(sqlite3_strlike(None, Some(b"abc"),0), SQLITE_NOMATCH);
}

// ============================================================================
//18. sqlite3_strlike:多个 `%`连续 → 等价一个 `%` (func.c:771-776)
// ============================================================================
#[test]
fn strlike_multiple_percent_collapse() {
 assert_eq!(
 sqlite3_strlike(Some(b"a%%%b"), Some(b"axxb"),0),
 SQLITE_MATCH,
 "consecutive %% collapse"
 );
}

// ============================================================================
//19. sqlite3_strlike: `_` 与 `%`混合
// ============================================================================
#[test]
fn strlike_mixed_underscore_percent() {
 assert_eq!(
 sqlite3_strlike(Some(b"a_%b"), Some(b"abc"),0),
 SQLITE_NOWILDCARDMATCH,
 "C trace: _ eats 'b', then %% searches for 'b' in 'c' → not found → NOWILDCARDMATCH"
 );
 assert_eq!(
 sqlite3_strlike(Some(b"a_%b"), Some(b"abcdb"),0),
 SQLITE_MATCH,
 "_ =1 char, %% = any → 'abcdb' matches 'a_cdb' where c='c'"
 );
}

// ============================================================================
//20. sqlite3_strglob:多个 `*`连续
// ============================================================================
#[test]
fn strglob_multiple_star() {
 assert_eq!(sqlite3_strglob(Some(b"a***b"), Some(b"axxxb")), SQLITE_MATCH);
 assert_eq!(sqlite3_strglob(Some(b"a***b"), Some(b"ab")), SQLITE_MATCH);
}

// ============================================================================
//21. sqlite3_strglob:模式消耗比输入多(单字节)
// ============================================================================
#[test]
fn strglob_pattern_longer_than_input() {
 assert_eq!(
 sqlite3_strglob(Some(b"abcdef"), Some(b"abc")),
 SQLITE_NOMATCH,
 "longer pattern than input returns NOMATCH (not NOWILDCARD)"
 );
}

// ============================================================================
//22. sqlite3_strlike: 空模式 + 空串 → MATCH
// ============================================================================
#[test]
fn strlike_empty_pattern_empty_string() {
 assert_eq!(sqlite3_strlike(Some(b""), Some(b""),0), SQLITE_MATCH);
}

// ============================================================================
//23. sqlite3_strlike:字节值 >=0x80(UTF-8 多字节)literal match
// ============================================================================
#[test]
fn strlike_utf8_literal() {
 // "中文" 是 UTF-8 多字节序列
 let pat = "中文".as_bytes();
 let s = "中文".as_bytes();
 assert_eq!(sqlite3_strlike(Some(pat), Some(s),0), SQLITE_MATCH);
 assert_eq!(
 sqlite3_strlike(Some(pat), Some("中x".as_bytes()),0),
 SQLITE_NOMATCH
 );
}
