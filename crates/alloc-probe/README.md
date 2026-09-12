# alloc-probe — 分配器的宿主侧压测台

**目的（解耦）**：内核里"分配器的账坏了"和"某个内核对象没析构"读数曾经一个样（都是
关机审计里一块内存还在册；那道审计已按用户裁决整体删除，见 `docs/memory.md` §5）。
本 crate 是**测试侧**的同一刀：

* 把内核**逐字未改**的分配器源码按 `#[path]` 编进来 —— `kernel/src/memory/allocator/`
  的 `bump.rs` / `frame.rs` / `block.rs`；
* 平台垫片（`lock` / `machine` / `PAGE_SIZE` / `InitError`）把内核依赖顶掉，**分配器
  自己的簿记分配落在宿主堆上**，于是泄漏检测器能直接看见"分配器有没有漏掉自己的元数据"；
* 内核对象的生命周期不在场 ⇒ 任何失败都只能归因于分配器。

## 两个泄漏检测后端（互斥，二选一）

1. **LeakSanitizer（默认路径，零额外依赖）**

   ```sh
   ./run.sh lsan
   # 等价于：RUSTFLAGS="-Zsanitizer=leak" cargo test --target x86_64-unknown-linux-gnu
   ```

   LSan 在进程退出时报告"分配了但没释放"的块（含分配器自己的元数据），失败即是
   **分配器侧的漏**。

2. **smartalloc（`--features smartalloc`）**

   ```sh
   ./run.sh smartalloc
   # 等价于：cargo test --target x86_64-unknown-linux-gnu --features smartalloc
   ```

   `smartalloc` 提供宿主分配路径 + 孤儿缓冲转储（**不能**当 `#[global_allocator]`：
   实测挂上后进程启动阶段 SIGSEGV，理由见 `src/lib.rs` 的 `mod smart` 头注）。
   注意：它需要能拉 crates.io（本仓 registry 缓存只读时，把 `CARGO_HOME` 指到工作区内，
   或先 vendor 再 `[patch.crates-io]`）。

**两者互斥**：LSan 靠拦截 `malloc/free`，而 smartalloc 把 malloc 换掉了 —— 一起上会
互相打架，故分两次跑。

## 为什么内核里跑不了这两个东西

内核是 `no_std` + `riscv64gc-unknown-none-elf`：没有 host 的 `malloc` 可拦截，sanitizer
运行时要 std；`smartalloc` 是个 C 库绑定，同样按 host ABI 工作。故它们是**宿主侧**工具。
内核侧对应的判据是**框架档的用例**（`health/{pagetable,spare,stress}.rs`，跑在启动后、
逐例打点），不是这里的影子账——两边不共享代码，只共享同一批不变量。

## 判据从哪来

* [x] `src/lib.rs` 的 [`fault::allocator_fault`]：分配器**自身**违例的当场炸通道
      （与"对象泄漏"分开归因）；
* [x] `tests.rs` 的**影子账**：同一帧不得交付两次、块池区间不得重叠；
* [x] 收尾对账：借出去的都还回去之后，影子账与分配器自有簿记都应回到起点；
* [x] 泄漏检测器本身（LSan / smartalloc）：分配器元数据有没有漏，由它们答。
