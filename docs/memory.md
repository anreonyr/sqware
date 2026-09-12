# memory — 帧、页表与审计（物理侧）

> 路径约定：`文件:行` 相对 `kernel/src/`。簿记侧（Space / Map / 窗口）见 [space.md](space.md)。

## 1 · 语义定位

`memory` 是**物理侧**，三块互不重叠的职责：

| 子模块 | 管 |
|---|---|
| `allocator` | 帧、块、后备仓三个后端 + 唯一的计数权威 `statistics` |
| `manager` | 页表原语（walk / 装叶 / 回收）、satp 模式探测、ASID 与清退、缺页判定 |

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

## 5 · 审计面：**已整体撤除**

`allocator/fence/`（帧种类 `Kind` / 活块账本 `Ledger` / O(1) 核对点 `checker` / 关机终值
`audit`）连同它在 `boot` 里的三个关机钩子（`probe_messenger` / `probe_teams` /
`check_baseline`）一起删掉了。裁决与理由：

- **判据的唯一通道是 `framework` 档的用例**（`health/` 三条）。审计层是**第二套**记账：
  它自己维护一份「谁在册、谁该归零」的账，与分配器的 `pagemeta` / `Tally` 互为解释
  ——两份账一旦漂移，两边都不再有牙（`banker` 的删除就是这条纪律的第一次执行）。
- 它要判的东西仍有人判，只是换了地方：帧池「净漏帧」由 `health/pagetable.rs` 的
  `frame.occupied − block.occupied` 每轮核（§10），分配器闭环由 `stress` / `spare`
  两条用例演练。
- **代价（如实记下）**：关机期的逐对象终值判词（`[audit] leak: <kind> N`）、弱引用收支
  普查、站点表孤儿/墓碑读数**都没有了**；`audit` 档不再比默认档多出运行时判据，只多
  `space.audit()` 与挂起自检（见 §6）。想要回哪条读数，就在 `health/` 里加一条真读它
  的用例——而不是恢复一层「只在关机时说话」的账。
- 留下的是**唯一性纪律**本身：帧「在不在手」只问 `frame::pagemeta`（`frame.rs:18/68`），
  块池在册页只问 `Tally`（`block.rs:99`），帧池水位只问 `statistics`（`statistics.rs`）
  ——一个事实一份账。

## 6 · 不变量

| 不变量 | 违反会怎样 | 谁守着 |
|---|---|---|
| 帧释放必「仍在手」，弹出必 free | 双释放 / 重叠分配 | `frame::pagemeta` 一份账 + `health/stress.rs` 的持有/反还演练 |
| 簿记 ⇔ PTE 双向一致 | 悬垂 PTE | `SpaceInner::audit`（audit 档，见 space.md §3） |
| ASID 先清退再还位图/销账 | 位图复用后键换主 | `asid.rs:83`、`adapter.rs:375-383` |
| 清退到齐前帧与段不得易主 | 远核旧条目污染新映射 | `salvage.rs:88` |
| 跨挂起不得持强/弱引用 | 任务外壳归还不掉（弃帧不析构） | `weak::check_block_heldout`（挂起前当场断言，`work/unit/weak.rs`） |
| 一个事实只有一份账 | 两份账互相解释，判据失效 | `frame::pagemeta` / `Tally` / `statistics` 各自唯一（见 §5） |

## 7 · 裁决账

| 裁决 | 定论 | 理由要点 |
|---|---|---|
| 删 `banker` | 只留 `pagemeta` 一份账 | 两份账记同一件事 |
| 删整个 `fence` 审计层 | 判据只走 `framework` 用例 | 第二套账会与分配器互释；观测面没有读者就是负债（见 §5） |
| 已物化页的写缺页不恢复 | 判 `false` | 「等于把 `Mprotect` 从边界降级成建议」（`fault.rs:131-141`） |
| 三种锁退出档 | 不刷 / 本核刷 / 跨核清退 | 放宽无远核义务、收紧必须就地清退（`adapter.rs:224-240`） |
| 后备仓预算即契约 | 常驻 + 1 MiB dump 预算不得被吃穿 | 报告与诊断要在分配失败时仍能跑（见 [diagnose.md](diagnose.md)） |
| `statistics` 唯一计数权威 | 所有读数只此一处 | 计数分家会让关机账失去意义 |

## 8 · 时序：一次缺页物化的物理侧动作

1. `resolve_anonymous`（`manager/fault.rs:63`）→ `Space::materialize`（`space/adapter.rs:318`）
   拿到写事务。
2. 逐页 `SpaceInner::frame()`（`space/core.rs:291`）领帧——**此时还没有种类**：领帧的这层
   不认识种类（`core.rs:286-290`）。
3. 窗口用 `tag!(Lazy, …)` 在**造对象那一层**标注来源（种类记账随审计层删了，
   `tag!` 现在只剩「读代码时看得见这一层造的是什么」这一件事）。
4. `install`（`core.rs:586`）装叶并 `inject` 登记；任何一步失败由 `InstallGuard`
   （`core.rs:515/551-575`）按 `MapMode::Materialize` 清叶 + 摘 frames 键，回到原状。
5. 整段区间在 `with_flush` 出口本核刷 TLB；跨核收紧另走 `with_shootdown`。

## 9 · 已知边界

1. **用户堆泄漏现在没有任何判据**：审计层的用户堆账（键 `(asid, 页索引)`）随
   `fence` 一起删了，`Mmap`/`Munmap` 在用户态也没有调用方。
2. **health 的「净在途帧」是差集估计**：`frame.occupied − block.occupied`
   （`health/pagetable.rs:33/79`），只在无并发分配活动时准。
3. **`MapError::DramOverlap` 只在内核空间 init 用**（`work/unit/mod.rs:112`），名字说的是
   「DRAM 恒等映射越过用户栈窗口」。
4. **`page_clear` 的时机**（`block.rs:555`）：整页无账时才清，属于顺手做的，不是判据。

## 10 · 判据与验证

- **三个 health 用例**（gate 一律 `#[cfg(any(debug_assertions, feature = "framework"))]`
  ——与它们唯一的消费者同在）：
  - `spare::accept`：ring 常驻 + `DUMP_BUDGET = 1 MiB` 未被吃穿 + 1 KiB 逐块拉到
    `AllocError`（**失败返 `Err` 不 panic**）+ 全归还后余量逐字节还原。
  - `pagetable`：32 轮 map/unmap，表数 `base + levels` ↔ 回落 `base`、`translate` 命中与
    落空、「在途帧 − 块池持页」回轮前，**每轮 `space.audit()`**。
  - `stress::accept`：五幕分配器演练（block 混合/持有、frame 多档/持有、frame 耗尽—反还）；
    耗尽后 `split_block` 必须返 `None` 而不是挂死。
  - 运行时机：`boot::init` 里 `init_depend` → 用例入口 → `spawn_root`（`boot.rs`）：
    framework 档走 `framework::run`（`[case] ok/FAIL` 逐例打点 + 末行汇总），debug 档走
    `health::run`（逐项 `[health] … ok`）。**其余档一个都不跑，用例体也不编进去。**
- **关机序列**：`boot.rs` 的关机钩子只剩 `scheduler::core::rip` 与 `block::flush` 两条
  ——停机不看账（见 §5）。
- **门**：`scripts/examine.nu`（默认 / audit / harden / framework 四档）。allocator 的判据
  落在 framework 档的用例汇总行 `[case] cases 3 ok 3 fail 0`。
