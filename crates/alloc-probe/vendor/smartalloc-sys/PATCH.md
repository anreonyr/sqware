# 本地补丁：`smartalloc-sys` 的 `SM_ALIGN`

这份是 **crats.io 上 `smartalloc-sys 0.2.0` 的原样副本**，只改了 `csrc/smartall.c`
一处（见下）。用 `Cargo.toml` 的 `[patch.crates-io]` 接进来；上游升级时按这份说明重打。

## 为什么必须打这个补丁

上游 `smalloc()` 的布局是 `[malloc 基址][struct abufhead][用户区][尾部哨兵]`，返回的是
`malloc 基址 + sizeof(struct abufhead)`。在 x86-64 上：

```text
struct queue { void *qnext, *qprev; }              // 16
struct abufhead { queue abq; unsigned ablen;       // 20
                  /* 4 字节填充 */                  // 24
                  char *abfname;                    // 32
                  unsigned short ablineno; }        // 34 → sizeof = 40
```

`malloc` 只保证 16 字节对齐 ⇒ 用户指针 = 16 对齐基址 + 40 ⇒ **只有 8 字节对齐**。
Rust 的 `GlobalAlloc` 契约要求返回指针满足 `layout.align()`，所以把它挂成
`#[global_allocator]` 之后，任何 `align > 8` 的分配都拿到违约指针。实测症状（
`crates/alloc-probe` 的接管档）：

```text
Program received signal SIGSEGV
=> movdqa (%r14),%xmm0          ← hashbrown::raw::RawTableInner::resize_inner
   test::term::terminfo::parser::compiled::parse
```

—— 测试 harness 启动阶段的第一条 16 字节 SIMD 载入就炸，一个用例都跑不到。
（上游 README 把这类崩溃归给"围着 libc malloc 的一层账"，**不是**真正原因。）

## 改了什么

```diff
+#define SM_ALIGN 64
+
 struct abufhead
 {
 	struct queue abq;		 /* Links on allocated queue */
 	unsigned ablen;			 /* Buffer length in bytes */
 	char *abfname;			 /* File name pointer */
 	unsigned short ablineno; /* Line number of allocation */
+	void *abraw;			 /* Real malloc() base (the lifted base is not) */
+	char abpad[16];			 /* Pad sizeof(struct abufhead) to 64 = SM_ALIGN */
 };
```

`smalloc()`：多要 `SM_ALIGN` 字节，把基址抬高到 `SM_ALIGN` 边界（`base`），头仍紧贴用户
指针之下（`sm_free` 依旧用 `ptr - sizeof(abufhead)` 找得到），真基址记进 `abraw`；
尾部哨兵与 `ablen` 的算法不变（仍以 `base` 为基准）。

`sm_free()`：先从头上读出 `raw`（下面那次 `memset(0xAA)` 会把头擦掉），最后
`free(raw)` 而不是 `free(被抬高的 base)`。

`sizeof(struct abufhead)` 因此变成 **64**（对齐 8，且是 `SM_ALIGN` 的整数倍），
用户指针 = `64 对齐基址 + 64` ⇒ **64 字节对齐**，覆盖 Rust 侧现实中的全部
`layout.align()`（std 结构最多要 16，`u128`/SIMD 要 16，`#[repr(align(64))]` 要 64）。

> 仍然不是通用的：要求 `align > 64` 的 `Layout`（比如按页对齐的请求）拿不到保证。
> 本 crate 的被测对象（帧/块分配器）只在自己的 arena 里做页对齐，宿主堆上不出现这种
> 请求；真要支持，得让 C 侧的 `smalloc` 接受对齐参数，而那会改 FFI 签名（= 改 crate），
> 与"Rust 侧照用 crate 原样"这条相冲突。

其余文件与上游 0.2.0 逐字一致（`build.rs` / `src/lib.rs` / `Cargo.toml` / `smartall.h`）。
