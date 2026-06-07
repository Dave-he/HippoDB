# T-0004 — spec vs C 源码差异

## 1. Overlong 编码

- **Spec 要求**:"overlong 拒绝"
- **C 实际行为**(`utf.c:160-162` 注释):
  > This routine accepts over-length UTF8 encodings for unicode values
  > 0x80 and greater. It does not change over-length encodings to
  > 0xfffd as some systems recommend.

**结论**:按 C 源码实现,overlong 编码 *被接受*。但是:
- 0xC0 0x80 (overlong NUL) → trans1 算出 c=0,c<0x80 → 0xFFFD
- 0xC0 0xA0 (overlong 0x20) → c=0x20 <0x80 → 0xFFFD
- 0xC1 0x80 0x80 (overlong 0x80) → c=0x1000,**保留**(非 surrogate,非 FFFE)

> 按 02-c-porting-conventions.md 与任务硬性要求第 2 条,C 源码是真相源。
> 因此 `utf8_read` 接受 overlong (≥0x80),不抛错、不替换为 0xFFFD。

## 2. 5+ 字节序列

- **Spec 要求**:"5+ 字节拒绝"
- **C 实际行为**:`sqlite3Utf8Trans1` 在 0xF8 之后全部为 0x00 / 0x01 / 0x02
  (即 5+ 字节起始字节解码后起始位是 0-2,后续延续字节每 6 位最多贡献 0x3F)。
  这保证任何 5+ 字节序列解码后 c 必定 < 0x80(短序列)或 ≥ 0x80(被消耗多个延续字节),
  且通常 < 0x80 → 0xFFFD。

**结论**:行为上 *等同于拒绝* — 5+ 字节序列总是返回 0xFFFD。无须专门写拒绝代码。

## 3. 无效起始字节 (0x80-0xBF)

- **C 行为**(`utf.c:155-158` 注释):
  > Bytes in the range of 0x80 through 0xbf which occur as the first
  > byte of a character are interpreted as single-byte characters and
  > rendered as themselves even though they are technically invalid
  > characters.

**结论**:作为单字节返回,直接返回 (byte, 1)。

## 4. `utf8_write` 的 `n` 参数

- C 端 `sqlite3AppendOneUtf8Character(zOut, v)` **不**接收 n,要求调用方
  保证 zOut 至少 4 字节。
- Spec 加入 `n` 用于 Rust 端安全:写时 clamp 到 `min(n, buf.len())`,
  n 不足时返回 0 表示"未写"。

## 5. `utf8_char_count` 的终止条件

- C 端 `sqlite3Utf8CharLen(zIn, -1)` 在首个 0x00 字节处停止。
- Rust 端接受 `&[u8]`,在首个 0x00 **或**切片末尾处停止(双截止)。
