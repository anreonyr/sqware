# memory — 帧、页表与审计（物理侧）

> 路径约定：`文件:行` 相对 `kernel/src/`。簿记侧（Space / Map / 窗口）见 [space.md](space.md)。

## 1 · 语义定位

`memory` 是**物理侧**，三块互不重叠的职责：

| 子模块 | 管 |
|---|---|
| `allocator` | 帧、块、后备仓三个后端 + 唯一的计数权威 `statistics` |
| `manager` | 页表原语（walk / 装叶 / 回收）、satp 模式探测、ASID 与清退、缺页判定 |
| `allocator/fence` | 审计：帧种类、活块账本、O(1) 核对点、关机终值 |

分界：`space` 决定「这段 VA 该不该有页」，`memory` 决定「页从哪来、页表怎么写、清退怎么到齐」。
**大页未实现**（只走 4 KiB 叶）。

## 2 · 结构

| 文件 | 职责 |
|---|---|
| `allocator/portal.rs` | 后端判别位（`Backend:24` / `switch:79`）——紧急路径可无锁取用后备仓 |
| `allocator/bump.rs` | 启动期一次性推进 |
| `allocator/hybrid.rs` | 分流：`≤ 2 KiB` 走 block，`> 2 KiB` 走 frame（`hybrid.rs:38`） |
| `allocator/block.rs` | 小对象池 + 池泵 + 簿记表 `Tally`；`page_clear:555` |
| `allocator/frame.rs` | buddy 帧分配器 + `pagemeta`（每页「在不在手」的唯一一份账，`frame.rs:18/68`） |
| `allocator/spare.rs` | 有序合并的 free-list 后备仓（`HEADER=32`、`MAX_ALIGN=16`） |
| `allocator/statistics.rs` | **唯一计数权威**——所有对外读数走它 |
| `allocator/bitmap.rs` | 位图工具 |
| `manager/{addr,entry,mode}.rs` | 地址类型、页表项（512 × 8 B，`entry.rs:14`）、satp 模式探测（57→48→39，`mode.rs:52`） |
| `manager/table.rs` | `TableNode:113`、`walk_mut:188`、`walk_raw:224`、`recycle:390` |
| `manager/asid.rs` | ASID 分配与 `shootdown:129` |
| `manager/fault.rs` | 缺页判定与处置（`handle_page_fault:105`） |
| `allocator/fence/{kind,checker,ledger,audit}.rs` | 审计四件套（见 §5） |

## 3 · 三个后端

| 后端 | 服务对象 | 关键事实 |
|---|---|---|
| `block` | 小对象（≤ 2 KiB，经 hybrid 分流） | 池 + 泵；`Tally` 簿记每一档 |
| `frame` | 页与页块（buddy） | `pagemeta` 是「这页在不在手」的**唯一**答案 |
| `spare` | 两者耗尽时的后备 | 有序合并 free-list；预算即契约（见 §10） |

## 4 · 页表与清退

- 页表项 512 × 8 B（`entry.rs:14`），satp 模式在启动期探测（`mode.rs:52`）。
- `table::walk_mut` 是**写路径唯一入口**（`table.rs:188`）；`walk_raw:224` 供只读采样
  （诊断的栈回溯靠它，见 [diagnose.md](diagnose.md)）；`recycle:390` 回收整棵子树。
- **ASID 顺序不可换**：`shootdown` 到齐之后才还位图、才销账（`asid.rs:83`、
  `space/adapter.rs:375-383`）——否则位图复用后同一键会换主。

## 5 · 审计（fence）

- **帧种类 `Kind`**（`fence/kind.rs:27`）：15 种，按 `End{Zero, Walk, Held, Report, Retire}`
  分组，每组的关机期望终值不同——判据因此是「逐种类」的，不是一个大总数。
- **活块账本 `Ledger`**：`Ledger::init(512 * 1024)`，soft cap = 448 K 条（`block.rs:324`、
  `ledger.rs:51-55`），**满即 `LedgerOom` panic**。
- **`checker`**：O(1) 核对点（`check_frame_held` / `check_frame_free`，`checker.rs:39/54`），
  在 `debug_assertions` **与** `audit` 两档都在场（`checker.rs:4-7`）。
- **账键单射**：`(asid << 44) | (va >> 12)`（`fence/mod.rs:391`）；**默认档 `fence::key`
  恒返回 0**（`:391-400`）——用户堆账只在 audit 档存在。
- 曾经删掉的第二份账：`banker`（页金库位图）已删（`fence/mod.rs:22-24`）。

## 6 · 不变量

| 不变量 | 违反会怎样 | 谁守着 |
|---|---|---|
| 帧释放必「仍在手」，弹出必 free | 双释放 / 重叠分配 | `checker.rs:39/54`（debug + audit） |
| 簿记 ⇔ PTE 双向一致 | 悬垂 PTE | `SpaceInner::audit`（audit 档，见 space.md §3） |
| ASID 先清退再还位图/销账 | 位图复用后键换主 | `asid.rs:83`、`adapter.rs:375-383` |
| 清退到齐前帧与段不得易主 | 远核旧条目污染新映射 | `salvage.rs:88` |
| 账键单射 | 碰撞误销账 | `fence/mod.rs:391` |
| 一个事实只有一份账 | 两份账互相解释，判据失效 | `fence/mod.rs:22-24`（banker 删除的由来） |

## 7 · 裁决账

| 裁决 | 定论 | 理由要点 |
|---|---|---|
| 删 `banker` | 只留 `pagemeta` 一份账 | 两份账记同一件事（`fence/mod.rs:22-24`、`audit.rs:108-111`） |
| 无「赦免」机制 | 按种类自带期望终值分组判 | 曾靠「快照物化」豁免，结果 prime 自扰（`fence/mod.rs:52-54`） |
| 已物化页的写缺页不恢复 | 判 `false` | 「等于把 `Mprotect` 从边界降级成建议」（`fault.rs:131-141`） |
| 三种锁退出档 | 不刷 / 本核刷 / 跨核清退 | 放宽无远核义务、收紧必须就地清退（`adapter.rs:224-240`） |
| 后备仓预算即契约 | 常驻 + 1 MiB dump 预算不得被吃穿 | 报告与诊断要在分配失败时仍能跑（见 [diagnose.md](diagnose.md)） |
| `statistics` 唯一计数权威 | 所有读数只此一处 | 计数分家会让关机账失去意义 |

## 8 · 时序：一次缺页物化的物理侧动作

1. `resolve_anonymous`（`manager/fault.rs:63`）→ `Space::materialize`（`space/adapter.rs:318`）
   拿到写事务。
2. 逐页 `SpaceInner::frame()`（`space/core.rs:291`）领帧——**此时还没有种类**：领帧的这层
   不认识种类（`core.rs:286-290`）。
3. 窗口用 `tag!(Lazy, …)` 在**造对象那一层**标注种类（审计据此分组）。
4. `install`（`core.rs:586`）装叶并 `inject` 登记；任何一步失败由 `InstallGuard`
   （`core.rs:515/551-575`）按 `MapMode::Materialize` 清叶 + 摘 frames 键，回到原状。
5. 整段区间在 `with_flush` 出口本核刷 TLB；跨核收紧另走 `with_shootdown`。

## 9 · 已知边界

1. **`Ledger` 满即 panic**：soft cap 448 K 条（`block.rs:324`、`ledger.rs:51-55`），没有
   「丢最旧」或降级路径。
2. **默认档 `fence::key` 恒 0**（`fence/mod.rs:391-400`）：用户堆账只在 audit 档存在，
   故 default 档的堆泄漏看不见。
3. **health 的「净在途帧」是差集估计**：`frame.occupied − block.occupied`
   （`health/pagetable.rs:33/79`），只在无并发分配活动时准。
4. **`MapError::DramOverlap` 只在内核空间 init 用**（`work/unit/mod.rs:112`），名字说的是
   「DRAM 恒等映射越过用户栈窗口」。
5. **`page_clear` 的时机**（`block.rs:555`）：整页无账时才清，属于顺手做的，不是判据。

## 10 · 判据与验证

- **三个 health 探针**（**全部 `#[cfg(debug_assertions)]`**，`health/mod.rs:35-40`）：
  - `spare::accept`（`health/spare.rs:23-68`）：ring 常驻 + `DUMP_BUDGET = 1 MiB` 未被吃穿 +
    1 KiB 逐块拉到 `AllocError`（**失败返 `Err` 不 panic**）+ 全归还后余量还原。
  - `pagetable`（`health/pagetable.rs:18-92`）：32 轮 map/unmap，表数 `base + levels` ↔ 回落
    `base`、`translate` 命中与落空、「在途帧 − 块池持页」回轮前，**每轮 `space.audit()`**。
  - `stress::accept`（`health/stress.rs:44-136`）：五幕分配器演练（block 混合/持有、frame 多档/
    持有、frame 耗尽—反还）；耗尽后 `split_block` 必须返 `None` 而不是挂死。
  - 运行时机：`boot::init` 里 `init_depend` → `health::run()` → `spawn_root`（`boot.rs:112-118`）。
    **release 档一个都不跑**。
- **audit 档关机序列**：`boot.rs:170-182` 的钩子链 `probe_messenger → scheduler::rip →
  block::flush → check_baseline`；门断言关机账逐种类配平（`[audit] shutdown checks ok:
  zero 7/7 held 6/6 tables 141/141 …`）。
- **门**：`scripts/examine.nu` 的 audit 轮抓 `[audit] leak: <kind> N` 与
  `table frames != kernel-walk count`（`:285-297`）。
