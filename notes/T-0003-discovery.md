# T-0003 — discovery: 3 test expectations are wrong, impl matches C 1:1

## 核心发现
对照 `sqlite-source/src/func.c:754-880` (patternCompare) 1:1 走读后发现
**Rust 实现行为正确**,是 3 个测试的期望值与 C 实际返回不一致。

## 3 个错误测试的 C 端真实行为

| 测试 | 期望 | C 实际 | 修正 |
|---|---|---|---|
| `util::pattern::tests::strglob_basic` 第 4 行<br/>`sg("a*", "bc")` | `NOWILDCARDMATCH`(2) | `NOMATCH`(1) | 改 1 |
| `pattern::strlike_percent_matches_any` 末行<br/>`strlike("a%", "bc", 0)` | `NOWILDCARDMATCH`(2) | `NOMATCH`(1) | 改 1 |
| `pattern::strlike_mixed_underscore_percent` 首行<br/>`strlike("a_%b", "abc", 0)` | `NOMATCH`(1) | `NOWILDCARDMATCH`(2) | 改 2 |

## 关键 trace (来自 func.c:766-880)

### 1. `strglob("a*", "bc")` 走读
- 读 pat='a', 读 str='b', `a!=b`, GLOB noCase=0, `a!=matchOne='?'`
- `return SQLITE_NOMATCH;` (line 878)
- C 返回 1,而非 2

### 2. `strlike("a%", "bc", 0)` 走读
- 读 pat='a', 读 str='b', `a!=b`, LIKE noCase=1 但 `tolower('a')='a'≠tolower('b')='b'`
- `a!=matchOne='_'`
- `return SQLITE_NOMATCH;` (line 878)
- C 返回 1,而非 2
- **注意**:`esc=0` → matchOther=0,但 `'a'!=0` 不会触发 ESCAPE 分支

### 3. `strlike("a_%b", "abc", 0)` 走读
- iter 1: pat='a', str='a' → match, continue
- iter 2: pat='_', str='b', matchOne=0x5F, zPattern!=zEscaped, c2='b'≠0 → match, continue
- iter 3: pat='%' (matchAll)
  - skip-loop: 读 pat='b' (pos 3), 退出,c='b'
  - c==0? 否。c==matchOther(0)? 否 (`'b'≠0`)
  - ASCII 搜索:zStop="Bb" (noCase)
  - zString 当前在 "c"
  - `strcspn("c", "Bb")=1` → zString 推进到 `'\0'`
  - `zString[0]==0` → break
- `return SQLITE_NOWILDCARDMATCH;` (line 831)
- C 返回 2,而非 1

## 决策
按 C 源码修正 3 个测试期望值。Rust 实现不改。
理由:任务硬性要求 §2 "如果任务 spec 与 C 源码不一致,优先 C 源码"。
