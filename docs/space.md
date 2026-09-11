# space — 地址空间（Space · Map · Window · Seg）

> 路径约定：`文件:行` 相对 `kernel/src/`；行号对应写下时的仓库状态。
> 物理侧（帧、页表、审计）见 [memory.md](memory.md)，权柄见 [pie.md](pie.md)。

## 1 · 语义定位

`space` 是**簿记**：一张 VA→PA 的 Map 表 + 两段 + 一棵随模式而定的页表树，全收在
`SpaceInner`（`work/unit/space/core.rs:46`）。它回答三个问题：这段 VA 在不在？谁在用？
拿掉它要等谁？

| 管 | 不管 |
|---|---|
| 帧与页表的分配、权限、翻译、TLB 一致性、关机记账 | 清单格式（在 root 域）、任务/域语义（team/task）、COW（**无代码**）、大页（**未实现**） |

`SpaceKind`（`space/mod.rs:42`）只表 S/U 页表，**不表达 ASID**——ASID 是
`memory/manager/asid.rs` 的事。空间种类与地址空间标识因此不会互相冒充：
「这是不是内核空间」与「这是哪个空间」是两个问题。

## 2 · 结构

| 文件 | 职责 |
|---|---|
| `space/mod.rs` | `SpaceKind`（S/U 页表；不含 ASID） |
| `space/core.rs` | `SpaceInner` + 映射原语：`map:140` `claim:167` `attach:192` `borrow:217` `unmap:241` `frame:291` `protect:348` `install:586`；`InstallGuard:515`、`MapMode:502` |
| `space/adapter.rs` | 门与刷：`with:217`（不刷）/ `with_flush:227`（本核刷）/ `with_shootdown:242`（跨核清退）、`pte_policy:185`、`Drop:373` |
| `space/map.rs` | `Map:54` / `Pending:30` / `runs:122` / `carve:168` / `is_borrowed:112` |
| `space/seg.rs` | `Seg:22` / `Segment:34`——段表，**lowest first-fit** |
| `space/salvage.rs` | `Span:25` / `Salvage::reclaim:88`——拆下来的帧的**料箱** |
| `space/window/*` | 零状态窗口策略：`FrameWindow::claim` `frame.rs:27`、`HeapWindow` `heap.rs:30/48`、`StackWindow::claim` `stack.rs:37`、`ShareWindow` `share.rs:28/51` |

三张小状态机：

| 类型 | 取值 | 含义 |
|---|---|---|
| `Pending`（`map.rs:30`） | `None` / `Lazy` / `Guard` | 全物化 / 触页物化 / **永不物化** |
| `MapMode`（`core.rs:502`） | `Materialize` / `Claim` | 失败时**回滚方式**不同 |
| 帧种类 `Kind`（`memory/allocator/fence/kind.rs:27`，15 种） | 按 `End` 分组 | 关机终值判据的键（见 [memory.md](memory.md)） |

## 3 · 不变量

| 不变量 | 违反会怎样 | 谁守着 |
|---|---|---|
| 簿记 ⇔ PTE 双向一致 | 悬垂 PTE 指向已归还帧 | `SpaceInner::audit` `core.rs:461`（audit 档） |
| 段表并入 Space 锁；`allocate`/`deallocate` 只在事务内 | 死锁 / 竞争 | `core.rs:18-20`、`adapter.rs:217` |
| 清退到齐前帧与段不得易主 | 远核旧条目污染新映射 | `salvage.rs:88`、`:106` |
| 借入映射只能收紧（新 flags ⊆ 当前叶 PTE） | 按 VA 单方面扩他人权限 | `map.rs:112` + `protect` 闸 `core.rs:375-401` |
| U 位单一出口 ＝ 空间种类 | S 态 SUM=0 访问 U 页缺页 | `adapter.rs:185`；例外：`frame.rs:30`、`borrow` |
| ASID 先 shootdown 再还位图/销账 | 复用后同一键换主 | `asid.rs:83`、`adapter.rs:375-383` |
| 帧释放必「仍在手」、弹出必 free | 双释放 / 重叠分配 | `fence/checker.rs:39/54`（debug + audit 档） |
| 账键单射 `(asid<<44)\|(va>>12)` | 碰撞误销账 | `fence/mod.rs:391` |

## 4 · 裁决账

| 裁决 | 定论 | 理由要点 |
|---|---|---|
| `Segment` 取代 interval | 无 `Arc`、无锁、无注册 | 「段内互不重叠……段再大也零 up-front 成本」（`seg.rs:4-12`） |
| 帧种类不在 `frame()` 标 | 在造对象那一层标（`tag!`） | 「帧分配器与装配核心都不携带种类参数」（`core.rs:286-290`） |
| 拆除统一走料箱 | 摘下的帧一律交 `Salvage` | 「清退到齐后才归还（远核可能仍持旧条目）」（`core.rs:238-240`） |
| 借入页不许加宽 | `WidenDenied`，**先校验后落改** | 「叶 PTE 是权限的权威……`narrow` 的 cap ⊆ 页表契约正是靠『加宽无路可走』成立」（`core.rs:329-338`） |
| 三档锁退出 | 不刷 / 本核刷 / 跨核清退 | 「新增放宽无远核义务……收紧必须就地跨核清退」（`adapter.rs:224-240`） |
| 已物化页的写缺页不恢复 | 判 `false`（fault isolation） | 「等于把 `Mprotect` 从边界降级成建议」（`fault.rs:131-141`） |
| 借入即「空帧表的 Map」 | `pending:None ∧ frames 空` | 与 `ShareWindow` 的懒区在同一段表里共存（`map.rs:104-115`） |

## 5 · 时序：`Mmap` → 触碰 → 缺页 → `Munmap`

1. 用户 `Mmap{size, at}`（`runtime/switcher/envcall.rs:525`）；`at == 0` 走
   `ShareWindow::mmap`（`share.rs:28`）：`with`（不刷）→ `Segment::allocate(Seg::User)`
   （`seg.rs:60`）→ `map(flags, Some(Lazy))`（`core.rs:140`）→ 返回 `Span`。
2. 首访触发 `PageFault::capture`（`manager/fault.rs:45`）→ `handle_page_fault:105`：
   先 `translate` 重走一遍，`satisfies` 命中即算 resolved。
3. `pending_state == Lazy` → `resolve_anonymous:63` → `Space::materialize`
   （`adapter.rs:318`）→ `with_flush` → `install` 逐页 `frame()` + `tag!(Lazy, …)` + 装叶 +
   `inject`；任何一步失败由 `InstallGuard`（`core.rs:551-575`）按 `Materialize` 清叶并摘
   frames 键。`Guard` / `Absent` / 已物化不满足 ⇒ `false`（用户任务被判故障）。
4. `Munmap`（`envcall.rs:546`）→ `ShareWindow::munmap`（`share.rs:51`）：`holds` 校验 →
   `inner.unmap`（清叶 + 摘或裂 Map，帧入 `Salvage`）→ `take_span` → `reclaim`：
   `asid::shootdown` → 还段 → 丢 maps。
5. 其余入口：`Allocate`/`Deallocate` → `HeapWindow`（Eager，`envcall.rs:271/297`）；
   `Spawn` → `StackWindow::claim` + `FrameWindow::claim`（`unit/task.rs:402/408`）；
   `Build` → `SpaceBuilder::{kernel,supervisor,user}` + `loader`（`loader.rs:49`）。

## 6 · 与其它机制的关系

- **锁序**（`lock/depend.rs:45-66`）：`Space=2 < Asid=5 < Frame=6 < Block=7 < Ledger=8 <
  Tally=9 < Spare=10`。`Space::with` 的闭包内**禁再调 `Space`**（`adapter.rs:215`），故
  上层（gate / mail / pie）只能经窗口与 `with` **单向进入**，不可能反向回调。
- **space → life**：空间持 `Arc<Life>`，`WakeKey::Space{space: asid}` 靠它判死
  （`adapter.rs:70-72,198-204`）——空间不依赖 room。
- **space → fence 单向**：`SpaceInner::frame()` 只领帧，种类由窗口 `tag!` 标注；
  fence 的 audit 反向只读 `frame::is_held`（`fence/audit.rs:109`），分配器文件里零种类词汇。
- **借入映射**服务 machine / `DockMeta` / mail ring：帧归外部所有（`core.rs:215-216`）。

## 7 · 已知边界

1. ~~**注释陈旧**~~ —— **已修（本轮）**（`window/mod.rs:5` 的原语清单改为 `map`/`claim`/`attach`/
   `borrow`/`unmap`）。原记录：`reserve(Lazy/Guard)` 是旧名，实体叫 `map`（`window/mod.rs:5`、
   `share.rs:3/20/37`、`stack.rs:4/29/46`、`core.rs:503/581`）。
2. ~~**注释陈旧**~~ —— **已修（本轮）**，随上一条一并改掉。原记录：`window/mod.rs:5` 提到的 `remove` 已不存在，现为 `unmap`（`core.rs:241`）
   与 `carve`（`map.rs:168`）。
3. ~~**文档链失效**~~ —— **已修（本轮）**（改为 `super::core::SpaceInner`）。原记录：`salvage.rs:7` 的 `[SpaceInner](super::inner::SpaceInner)` 指向不存在的
   `inner` 模块（实体在 `core`）。
4. ~~**注释与代码相反**~~ —— **已修（本轮）**（改为「懒映射：`ShareWindow::mmap` 的共享区；栈体
   相反，是立即物化」）。原记录：`map.rs:8` 把「mmap / declare / 栈体」并列——全仓无 `declare`；
   且栈体恰是 **Eager**（`stack.rs:50`），与同句并列的 mmap（Lazy）语义相反。
5. **注释与代码相反**：`stack.rs:3` 与 `unit/task.rs:399` 说「栈自窗口顶向下排」，而
   `Segment` 是 lowest first-fit **自低端起**（`seg.rs:53-77`）。
6. **名字与内容不符**：`Seg::Kernel`（`seg.rs:25`）名义是「内核帧段」，实际装的是每线程
   trap 帧 `[TEAM_FRAME_BASE, +64 MiB)`，且**进的是任务空间**（`task.rs:408`）。
7. **`Space::borrow` 绕过 `pte_policy`**（`adapter.rs:266`）：借入 U 空间时 U 位必须由
   调用者给对（`seed_trampoline` 自查的是 S 空间 flags，`adapter.rs:150`）。
8. **`asid::shootdown` 依赖 hartid 连续**：扫 `0..hart_count()` 后用 `hart % usize::BITS`
   分组掩码（`asid.rs:137-144`），注释自承「4 核均在首字」。
9. **`Segment::deallocate` 失败在 release 档静默**（`seg.rs:81-83`：只有 `debug_assert`）
   ⇒ 会静默段泄漏。
10. **双重清退**：`unmap`/`release` 出口先 `with_flush` 本核刷，`reclaim` 再跨核
    shootdown（`adapter.rs:291/313`）——第二次才是权威。
11. **`WidenDenied` 不出现在用户可见错误码**：`map_err` 把它并进 `Denied`
    （`envcall.rs:89-93`）。
12. **缺页「杀 task」只打日志**：三条 `error!` 之后统一 `return false`（`fault.rs:142-183`），
    路径名与注释称「杀 task」。

## 8 · 判据与验证

- **health 探针**（`health/pagetable.rs:18-92`，**debug-only**）：32 轮 4 MiB map/unmap，
  断言表数回到 `base`、`translate` 命中与落空都对、「在途帧 − 块池持页」回到轮前——
  每轮还调一次 `space.audit()`。
- **audit 档**：boot 后逐空间 `space.audit()`（`boot.rs:133/136`），核对簿记 ⇔ PTE；
  关机序列 `probe_messenger → scheduler::rip → block::flush → check_baseline`
  （`boot.rs:170-182`）。
- **验收门**：三档同一套判据；audit 档抓 `[audit] leak: <kind> N` 与
  `table frames != kernel-walk count`（`scripts/examine.nu:285-297`）；默认档哨兵
  「出现 audit 输出即挂」（`:520-524`）。
- **未覆盖**：用户侧只有 `alloc` 一条端到端命令（`programs/src/bin/user/shell.rs:971-973`
  → `MemoryCall::Allocate`）。**`Mmap`/`Munmap`/`Mprotect` 三条路径没有门覆盖**——
  `share.rs` 的懒区与借入所有权闸目前只有内核代码、注释与 health 探针在守。
