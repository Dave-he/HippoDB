# T-0002 — 决策与发现记录(由 Claude 第一轮自动写)

> 这是 Claude 在第一轮跑 T-0002 时主动写下的发现笔记(归功 stream-json 检测):
> Claude 识别出 backlog 里的 spec 写错了。

## 与任务描述的差异

任务描述说 "`malloc(0) 返回非 null 可 free`",但 C 源码
(`sqlite-source/src/malloc.c`) 明确是 NULL:

```c
void *sqlite3_malloc(int n){
  ...
  return n<=0 ? 0 : sqlite3Malloc(n);
}

void *sqlite3Malloc(u64 n){
  void *p;
  if( n==0 || n>SQLITE_MAX_ALLOCATION_SIZE ){
    return 0;
  }
  ...
}
```

## 决策

按 C 源码实现 — `malloc(0) == NULL`, `malloc64(0) == NULL`,
`free(NULL)` 是 no-op。任务描述(backlog 里的)错了。

## 同时发现 T-0001 的桩代码也错了

T-0001 的 `src/api.rs::sqlite3_malloc` 写的是:

```rust
if n <= 0 { return 1usize as *mut c_void; }  // 假非 null
```

这是**错误**的(为了"先让它能编译"留的桩)。T-0002 要替换它。
