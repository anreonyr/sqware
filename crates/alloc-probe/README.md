# alloc-probe — 分配器的宿主侧压测台

**目的（解耦）**：内核里"分配器的账坏了"和"某个内核对象没析构"读数曾经一个样（都是
关机审计里一块内存还在册；那道审计已按用户裁决整体删除，见 `docs/memory.md` §5）。
本 crate 是**测试侧**的同一刀：

* 把内核**逐字未改**的分配器源码 `include!` 进来 —— `kernel/src/memory/allocator/`
  的 `bump.rs` / `frame.rs` / `block.rs`；
* 平台垫片（`lock` / `machine` / `statistics` / `PAGE_SIZE` / `InitError`）把内核依赖顶掉，
  **分配器自己的簿记分配落在宿主堆上**，于是泄漏检测器能直接看见"分配器有没有漏掉自己的元数据"；
* 内核对象的生命周期不在场 ⇒ 任何失败都只能归因于分配器。

> 垫片是**垫片**：内核改了 `statistics` 的**写侧**签名，这里就得跟着改（`lib.rs` 的
> `memory::allocator::statistics`）。`include!` 进来的源码一个字都不改 —— 那是本 crate
> 全部结论的前提。

---

## 第一层：宿主单元测试

三条判据通道，**互不顶替**（一条缺陷只归一个通道，这是本 crate 的纪律）：

| 通道 | 谁 | 生成什么 | 判什么 |
|---|---|---|---|
| **甲 · 配对** | `mockalloc` + `proptest`（`tests/pairing.rs`） | 随机**分配/释放序列** | 幕内每一笔宿主分配都配对：泄漏 / 重复释放 / 错指针 / 错尺寸 / 错对齐逐类点名 |
| **乙 · 用量** | `dhat` + `proptest`（`tests/heap.rs`） | 随机**工作负载**（同一批序列） | 宿主堆用量：装台预算 + 每一幕 `Δtotal_bytes = Δcurr_bytes = Δmax_bytes = 0` |
| **丙 · 影子账** | 本 crate 自己（`src/harness.rs` + `src/tests.rs`）：被压后端 = `bump`/`frame`/`block` + **`hybrid`（分流）/ `spare`（后备仓）** | 同一批序列（**LCG 确定性语料**，不装 proptest 也跑） | 区间不重叠 / 指针满足 `layout.align()` / 交付区写读一致 / `block.rs:310` 的请求门必须拒 / 预算内的合法请求必须成 |

甲、乙各自**自带凭证**（"检测器没响"与"检测器没插上"必须分得开）：
mockalloc 那条腿拿"一次 `Box` 往返 = 1 笔分配"当凭证、并另有一条**反向对照**（`mem::forget`
必须被判 `Leak`）；dhat 那条腿拿"一次已知分配 ⇒ `Δtotal_bytes` 恰为 4096 B"当凭证。

### 序列的定义只有一处

`harness::plan(&[u8]) -> Vec<Op>`：**每两个字节一条动作**（取块 1/2、还单块 3/8、还全部 1/8；
`req` 字节选请求档位）。proptest 生成的是字节，序列的定义在那里 —— 于是

* 换随机源不改判据（`plain` 档用 LCG 跑同一批定义，一件 proptest 都不装）；
* proptest 失败时打出的**最小输入可以直接贴回复现**，不依赖 `proptest-regressions/` 文件
  （那类文件在本 crate 里是**关掉**的：`Config::default()`（带 `std` 的那份）会挂
  `FileFailurePersistence::SourceParallel`，测试不该把仓库写脏）。

### 判据要能在幕内读账，所以模型自己不许分配

`harness::run` 全程**零宿主分配**（状态全在栈上的固定容量数组：48 槽影子账 + 几个 `usize`，
没有 `Vec`/`HashMap`/`String`/`format!`）。两条腿都把 `run` 放进各自的"幕"里读账 ——
幕内只要有**任何**无关分配就会污染读数（mockalloc 会把它当配对对象、dhat 会把它算进字节增量）。
计划本身（`Vec<Op>`）与影子账的构造都在**幕外**。

---

## 实测读数（标定值写死在 `harness::INIT_*`，谁漂了两条腿一起响）

| 读数 | 值 | 出处 |
|---|---|---|
| 装台期（`bump`→`block`→`frame` 的 `init`）宿主分配 | **10 笔 / 11348 B** | 甲、乙两个后端**逐项相等**（`num_allocs↔Δtotal_blocks`、`mem_allocated↔Δtotal_bytes`、`mem_leaked↔Δcurr_bytes`） |
| 其中**未配对**（终身表） | **10 笔（全部）** —— 连一笔"取了又还"的临时分配都没有 | 清单见 `harness::INIT_LIFETIME_LEAKS`：`frame.rs:558/165/169`、`block.rs:283/296/590`、`spare.rs:311` |
| 随机序列幕内的宿主分配 | **0**（64/64 幕判词 `NoData`） | **热路径不上全局堆**：`frame`/`block` 的元数据全在装台期分配好，热路径只有原子计数 |
| 台面容量（影子账通道） | 取尽得 **3572 个单页块**（13.95 MiB；差的是 `bump` 簿记与后备仓仓区 —— 后者 `ring_bytes(4)+32+1 MiB` ≈ 1.03 MiB，按 buddy 取整成 **2 MiB**）；**两轮取尽页数一致**，还尽后 **4 MiB 大块可取回** | `tests.rs::buddy_capacity_is_conserved_and_recovers` |
| 确定性语料 | 256 种子 × 96 B = **12288 条动作**：交付 5474 块 / 契约拒 675 / 影子账满跳过 1842 | `tests.rs::deterministic_corpus`（不装 proptest 也跑） |

**为什么装台是甲腿的主战场**：热路径既然一笔都不分配，随机序列的幕里就无事可判（判词
`NoData`）—— 那条断言本身就是判据："热路径不上全局堆"被固化了，谁往热路径塞一笔宿主分配，
这里当场响（见下面的逆向验证）。**逐笔账**（10 笔 / 11348 B）则把"分配器自己留了什么"钉死到
最后一笔：多一笔就是有人没还，字节漂了就是某张表改了尺寸。

### 逆向验证（判据有没有牙）

往热路径里塞一笔故意的宿主分配（`statistics::record_block_take` 里 `Box::leak`），两条腿
**各自当场响**，验完撤掉：

```
甲：幕内出现配对错误 Leak：分配 1 笔 / 释放 0 笔 / 分配 1 B / 释放 0 B（计划 [120, 0]）
乙：工作负载在宿主堆上分配了 1 字节
```

两条腿都把计划缩到**最小输入**（`[120, 0]`），可直接贴回语料复现。

---

---

## 多线程（扩展）：并发压力下的判据

**支点：线程即 hart。** 内核按 hart 选 per-hart 池（`block.rs` 的 `blocks[hart_id]`），于是宿主
shim 的 `machine::hart_id()` 改成按线程分配 id（`hart_bind` 给确定性绑定）。这一改让并发层压到
三条平时压不到的路径：**per-hart 池分家**、**跨 hart 归还**（`home != me` ⇒ `feed` 进泵，本池
下次 `pull` 先 `suck` 抽回）、**同池并发**（线程数 > hart 数时两线程共用一池）。

| 测试目标 | 通道 | 判据（失败长什么样） |
|---|---|---|
| 分配/释放配对 | `mockalloc`（单线程）+ **原子计数器**（并发） | `statistics` 写侧原子账：**块级严格** `take == give`；帧级/池级只差池迟滞（≤ 池数 × 9 档） |
| 内存使用量/泄漏 | `dhat`（单线程）+ **LeakSanitizer / `dropcount`** | LSan 覆盖单线程 + 并发两层；`dropcount` 判**本层自己的**在途记账对象一个不漏（读数归因的前提） |
| 随机操作序列 | `proptest` + **原子计数器** | 随机（线程数 × 计划长度）的并发轮次，判据与守恒用例同一批 |
| 并发压力/吞吐量 | **`malloc-bench-rs`** | Larson / mstress × `GlobalAlloc` 适配器（分流点同 `hybrid.rs:38`），与 `System` 同条件对照 |
| 数据竞争 | **ThreadSanitizer / Miri** | 见下面两条裁决（TSan 必须 `-Zbuild-std`；Miri 按"调用次数"计费） |
| 并发交错正确性 | **Loom / Shuttle** | **挂不上** —— 裁决见下 |

实测读数（`./run.sh mt -- --nocapture`）：

```text
[mt] 跨 hart 归还：hart0 池 0 / hart1 池 1 / 交回后归属本池 8/8
[mt] 4 线程 × 160 B：交付 150 块 / 块级原子账 62/62 平 / 帧级差 26 / 池级差 26（上界 36）
[mt] dropcount：96 个在途记账对象全部恰好析构一次
bench Larson 2.32 Mops/s（System 100.54 ⇒ 0.02×）/ Mstress 2.42（System 167.59 ⇒ 0.01×）
```

`bench` 的形状要交代清楚（否则读数会被当成内核性能）：宿主 `SpinLock` 是**阻塞 `Mutex`** 不是
自旋锁、台面只有 16 MiB（池频繁借还页）、`Tally` 是一把**全局**锁 —— 三条都让宿主读数偏悲观。
它只回答"分流在真并发下是什么量级"，**不是**内核性能结论，也不是真机多 hart 的数据（那要在
QEMU 里由内核框架档量）。

### 裁决一：Loom / Shuttle **现在挂不上**（不是不想挂）

两者的模型都是"同一个闭包跑很多遍、每遍换一种交错"，**要求被测状态在闭包内构造**。本 crate 的
被测对象是 `bump` / `frame` / `block` 三个 `OnceLock` 单例（内核就是这么写的），状态**跨遍保留**：
第 2 遍起 freelist / 页表 / 池已经不是初始状态，模型检查随之失去意义。要挂上得先在**内核侧**
开一个口子（二选一）：① 给三个后端各加"实例可构造"的入口；② 把单例换成可注入实例。两条都在
内核里，超出本 crate 的范围。在此之前，交错覆盖由**对抗线程 + 栅栏**（跨 hart 回路的 `feed→suck`、
同一起跑线的 churn）与 **TSan / Miri** 承担。

### 裁决二：TSan 必须 `-Zbuild-std`，否则它报的"竞争"全是假的

第一次只用 `-Zsanitizer=thread -Cunsafe-allow-abi-mismatch=sanitizer` 跑，报了 **42 条竞争，全是
假阳性**：两份栈都落在 `FrameAllocator::allocate` 同一把锁里，争用地址是 `frame.rs:165` 分配的
freelist 元素数组。成因：Rust 的 `std::sync::Mutex` 走 **futex** 而不是 pthread，它给 TSan 的同步
注解写在 std 源码里（`#[cfg(sanitize = "thread")]`）—— **预编译 std 没插桩，那些注解被编掉**，
TSan 于是看不见这把锁。改 `-Zbuild-std`（连 std 一起插桩）后：**4/4 通过、零竞争报告**。

### 裁决三：Miri 按"分配器调用次数"计费，规模要分级

Miri 的耗时随**分配器调用次数**线性涨（实测：round-trip 两条秒级；百次级调用的 churn / 语料
是分钟级，全量跑 20 分钟未完成）。故分工写死在代码里：

* `cfg(miri)` 把台面缩到 **128 页**（512 KiB）、churn 缩到 **8 轮**；
* `buddy_capacity…`（4000+ 次拆分）与 `deterministic_corpus` 在 Miri 下 **`ignore`**（理由写在断言旁）；
* 结果：`./run.sh miri` **2 分钟跑完、4 通过 2 忽略**，审计的是 UB（裸指针来去、交付区写读）；
* **并发在 Miri 下不跑**（`tests/mt.rs` 整文件 `not(miri)`）—— 竞争那件事交给 TSan，各管一段。

## 十条路（互斥，见 `run.sh`）

```sh
# 单线程（宿主单元测试）
./run.sh smartalloc   # 默认：孤儿缓冲转储（总量面）
./run.sh mockalloc    # 第一层·甲：配对判词（逐笔账）
./run.sh dhat         # 第一层·乙：宿主堆用量（气压计）
# 多线程（扩展）
./run.sh mt           # 并发层：守恒 / 模式写读 / per-hart 池 / 跨 hart 回路 + dropcount
./run.sh tsan         # 数据竞争：ThreadSanitizer（-Zbuild-std，见裁决二）
./run.sh miri         # UB：Miri（规模分级，2 分钟，见裁决三）
./run.sh bench        # 并发压力/吞吐量：malloc-bench-rs
# 零后端 / 其它
./run.sh plain        # 不接管：只看用例过不过（--no-default-features）
./run.sh lsan         # LeakSanitizer（必须不接管；覆盖单线程 + 并发两层）
./run.sh orphan       # 演示：一次普通 Rust 分配漏掉 ⇒ 收尾转储点名
```

* **三个后端三选一**：rustc 只允许一个 `#[global_allocator]`，而三个后端读数口径不同
  （孤儿转储 / 配对判词 / 用量读数）。同时开两个由 `lib.rs` 的 `compile_error!` 挡在编译期。
* **`plain`**：`--no-default-features`。影子账那批判据**不依赖后端**，这条路跑的就是它们
  （确定性 LCG 语料 12288 条动作 + 容量守恒 + 契约门 + 耗尽—恢复）。
* **`lsan`**：`RUSTFLAGS=-Zsanitizer=leak` + `--no-default-features`。**必须不接管**：
  接管之后每个活块都被接管层的队列指着，LSan 会一律判 "still reachable"（等于没测）。
  这条路是干净的：分配器留着的东西（台面、终身表）都从 `'static` 可达，LSan 不报。
* **`orphan`**：`--example orphan`，走全局分配器漏一块，看转储。

## 接管：debug 档的宿主分配器

三行，各一条腿（`src/lib.rs`）：

```text
smartalloc   static HOST_ALLOCATOR: smartalloc::SmartAlloc            = smartalloc::SmartAlloc;
mockalloc    static HOST_ALLOCATOR: Mockalloc<System>                 = Mockalloc(System);
dhat         static HOST_ALLOCATOR: dhat::Alloc                       = dhat::Alloc;
```

**两个前提**（来源都写在代码里，也都在配置里落实）：

1. **指针对齐**（`smartalloc` 那条腿）：上游 `sizeof(struct abufhead) == 40`（x86-64），
   用户指针 = malloc 基址 + 40 ⇒ 只有 **8 字节对齐**，违反 Rust `GlobalAlloc` 的契约。
   直接挂上去的实测症状：进程起始阶段 SIGSEGV，`movdqa (%r14)`（hashbrown 表扩容里第一条
   16 字节 SIMD 载入），一个用例都跑不到。修法：`Cargo.toml` 的 `[patch.crates-io]` 指向本地
   vendored 的 `smartalloc-sys`，在 C 里引入 `SM_ALIGN = 64`。**Rust 侧仍是 crate 原样**。
   （`mockalloc` 包的是 `System`，超 16 字节对齐走 std 自己的 `posix_memalign`，不需要补丁。）
2. **单线程**：`smartalloc` 的 C 层是一张无锁全局链表 + `assert`，多线程并发会写坏它；
   `dhat` 的读数是**进程级**的，并发用例会把别人的分配算进我们的增量。故
   `.cargo/config.toml` 钉 `RUST_TEST_THREADS = "1"`。

**已知边界（后端自己的）**：接管模式下孤儿报告的 `FILE:LINE` 恒是**全局分配器声明处**
（Rust 的 `__rust_alloc` shim 不透传 caller），不是泄漏点 —— 上游 README 也这么写。实测
`smartalloc` 档的转储还是**总量面**：它把 std 自己的长命分配（主线程名 `main`、测试框架的
结构……）与分配器的终身表混在一张清单里，读不出"分配器漏没漏"。**逐笔账要看甲腿**。

---

## 已知边界（本 crate 自己的，都要交代）

1. **类目表不进宿主账**：内核 debug/framework 档的 `statistics::install_frame_kinds` /
   `install_block_kinds` 会分配逐帧、逐页的类目表（§5.1 那套标注账）。宿主 shim 里它们是
   **空壳**（`Ok(())`，见 `lib.rs` 的 shim 注①）—— 在宿主上再实现一遍表尺寸必然与内核漂移
   （"一个事实只有一份账"）。**代价**：本 crate 的宿主用量读数**不含**那份类目表，
   要看它得在内核侧量。
2. **甲腿的判词只报第一类**：`mockalloc` 的 `AllocInfo::finish()` 先判笔数
   （`num_allocs > num_frees ⇒ Leak`），于是装台幕里"重复释放/错尺寸/错对齐"会被 `Leak`
   **盖住**。补法是**双对账**：多释放一笔会挪 `num_leaks()`、错尺寸/错对齐会挪
   `mem_leaked()`，两条都对到常量上（`tests/pairing.rs` 里有逐条注）。
3. **影账只认"我取的那几块"**：池有几页**迟滞**（每 size class 留 1 个空闲页不还，
   `block.rs` 的 arena 迟滞是设计）。故判据是"我取的全部还回去了"，**不是**"内存池归零"。
   台面级的两件事另开通道判：`tests.rs` 的**容量守恒**（取尽—还尽—再取尽页数不变）与
   **还尽后大块必可取回**（合并没漏）。
4. **`bump` 不进随机序列**：它的 `deallocate` 是空操作（一次性推进），没有"释放配对"这回事。
5. **proptest 不用 `fork`/`timeout`**：默认 feature 会经 `rusty-fork` 起子进程，与"全局分配器
   + 进程内单例分配器（`bump`/`block`/`frame` 都是 `OnceLock`）+ 台面是一块进程内宿主缓冲"
   这套构造直接冲突（子进程里另有一份台面）。故 `proptest` 关了默认 feature。
6. **影子账的容量是 48 块**（在册字节预算 8 MiB）：每步要扫在册区间，容量压小是为了让每步
   便宜；满了 `Take` 不落到分配器上（报告里的 `skipped` 如实记着，不假装跑了）。

## 为什么内核里跑不了这些东西

内核是 `no_std` + `riscv64gc-unknown-none-elf`：没有 host 的 `malloc` 可拦截，sanitizer
运行时要 std；`smartalloc` / `mockalloc` / `dhat` 都按 host ABI 工作。故它们是**宿主侧**工具。
内核侧对应的判据是**框架档的用例**（`health/{pagetable,spare,stress}.rs`，跑在启动后、逐例
打点），不是这里的账 —— 两边不共享代码，只共享同一批不变量。

## 判据从哪来

* [x] 接管凭证三条腿各一条：`tests.rs::host_allocator_took_over`（64 字节对齐）、
      `tests/pairing.rs::mockalloc_is_watching`（一取一还 = 1 笔）、
      `tests/heap.rs` 的 ①（已知分配 ⇒ 已知字节）；
* [x] 反向对照：`mockalloc_names_a_deliberate_leak`（`mem::forget` ⇒ 必报 `Leak`）；
* [x] `harness` 的**影子账**：区间不重叠、对齐、交付区写读一致、`block.rs:310` 的请求门、
      预算内合法请求必成；
* [x] 收尾对账：借出去的都还回去（`Report.given == Report.taken`），且在册清零；
* [x] 台面级：容量守恒 + 还尽后大块可取回 + 取尽返 `Err` 不 panic；
* [x] `tests/pairing.rs` 的**装台逐笔账**（10 笔 / 11348 B）与 `tests/heap.rs` 的**装台预算**
      （总量/常驻/幂等）；
* [x] 逆向验证：热路径塞一笔宿主分配 ⇒ 甲、乙两条腿各自当场响（见上）；
* [x] 并发层（`tests/mt.rs`）：per-hart 池分家 + 跨 hart 归还回本池（`own(pa)` 读数）、块级原子账严格平、
      帧级/池级只差迟滞上界、`registry_live() == 0`、`dropcount` 句柄全析构；
* [x] `fault::allocator_fault`：分配器**自身**违例的当场炸通道（与"对象泄漏"分开归因）。
