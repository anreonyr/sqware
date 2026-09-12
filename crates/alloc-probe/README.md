# alloc-probe — 分配器的宿主侧压测台

**目的（解耦）**：内核里"分配器的账坏了"和"某个内核对象没析构"读数曾经一个样（都是
关机审计里一块内存还在册；那道审计已按用户裁决整体删除，见 `docs/memory.md` §5）。
本 crate 是**测试侧**的同一刀：

* 把内核**逐字未改**的分配器源码 `include!` 进来 —— `kernel/src/memory/allocator/`
  的 `bump.rs` / `frame.rs` / `block.rs`；
* 平台垫片（`lock` / `machine` / `PAGE_SIZE` / `InitError`）把内核依赖顶掉，**分配器
  自己的簿记分配落在宿主堆上**，于是泄漏检测器能直接看见"分配器有没有漏掉自己的元数据"；
* 内核对象的生命周期不在场 ⇒ 任何失败都只能归因于分配器。

## 接管：debug 档的宿主分配器 = smartalloc

```rust
#[global_allocator]
static HOST_ALLOCATOR: smartalloc::SmartAlloc = smartalloc::SmartAlloc;
```

就这一行（`src/lib.rs`）——**crate 原样使用**：本 crate 的每一次宿主分配（含被压测的
内核分配器自己去要的元数据）都记在 smartalloc 的账上；收尾 `dump_orphans()` 把还在账上
的块打出来。`smartalloc` 自己写着 `#![cfg(debug_assertions)]`，故"接管"只可能发生在
debug 档 —— 测试跑的就是 debug 档。

**两个前提**（来源都写在代码里，也都在配置里落实）：

1. **指针对齐**：上游 `sizeof(struct abufhead) == 40`（x86-64），用户指针 = malloc 基址
   + 40 ⇒ 只有 **8 字节对齐**，违反 Rust `GlobalAlloc` 的契约（返回指针须满足
   `layout.align()`）。直接挂上去的实测症状：进程起始阶段 SIGSEGV，`movdqa (%r14)`
   —— hashbrown 表扩容里第一条 16 字节 SIMD 载入，一个用例都跑不到。
   修法：`Cargo.toml` 的 `[patch.crates-io]` 指向本地 vendored 的 `smartalloc-sys`，在 C
   里引入 `SM_ALIGN = 64`（`smalloc` 抬高基址、把真基址记进头、`sm_free` 归还它）。
   **Rust 侧仍是 crate 原样**，补丁只在 C 源。
2. **单线程**：C 层是一张无锁全局链表 + `assert`，多线程并发 alloc/free 会把它写坏。
   故 `.cargo/config.toml` 钉 `RUST_TEST_THREADS = "1"`。

**已知边界（crate 自己的）**：接管模式下孤儿报告的 `FILE:LINE` 恒是**全局分配器声明处**
（Rust 的 `__rust_alloc` shim 不透传 caller），不是泄漏点 —— 上游 README 也这么写
（"refers to the `#[global_allocator]` itself and can be ignored"）。要真实分配点，得像
crate 原本的用法那样**显式**调 `SmartAlloc`。

## 四条路（互斥，见 `run.sh`）

```sh
./run.sh smartalloc   # 默认：接管 + 跑用例（含"接管生效"的对齐判据）
./run.sh plain        # 不接管：只看用例过不过（--no-default-features）
./run.sh lsan         # LeakSanitizer（必须不接管：见下）
./run.sh orphan       # 演示：一次普通 Rust 分配漏掉 ⇒ 收尾转储点名
```

* **`smartalloc`（默认）**：debug 档宿主堆由它接管。判据 = 用例 + 收尾转储。
* **`plain`**：`--no-default-features`，全局分配器回到系统那份 —— 把"用例失败"与
  "接管层的报告"分开看。
* **`lsan`**：`RUSTFLAGS=-Zsanitizer=leak` + `--no-default-features`。**必须不接管**：
  接管之后每个活块都被 smartalloc 的队列指着，LSan 会一律判成 "still reachable"（等于
  没测）。
* **`orphan`**：`--example orphan`，走全局分配器漏一块，看转储。

## 为什么内核里跑不了这些东西

内核是 `no_std` + `riscv64gc-unknown-none-elf`：没有 host 的 `malloc` 可拦截，sanitizer
运行时要 std；`smartalloc` 是个 C 库绑定，同样按 host ABI 工作。故它们是**宿主侧**工具。
内核侧对应的判据是**框架档的用例**（`health/{pagetable,spare,stress}.rs`，跑在启动后、
逐例打点），不是这里的账 —— 两边不共享代码，只共享同一批不变量。

## 判据从哪来

* [x] 接管点 + 对齐判据（`tests::host_allocator_took_over`）：64 字节对齐的分配必须真拿到
      64 对齐的指针 —— 它同时钉住"全局分配器是接管层"与"vendored 补丁编进去了"；
* [x] `tests.rs` 的**影子账**：同一帧不得交付两次、块池区间不得重叠；
* [x] 收尾对账：借出去的都还回去之后，影子账与分配器自有簿记都应回到起点；
* [x] `fault::allocator_fault`：分配器**自身**违例的当场炸通道（与"对象泄漏"分开归因）。
