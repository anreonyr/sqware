# task — 执行单元与调度（Team · Task · 血缘 · 站点）

> 路径约定：本文的 `文件:行` 相对 `kernel/src/`，行号对应写下时的仓库状态。

## 1 · 语义定位

```text
Team = 进程    持唯一 Space + 成员簿记 + 血缘（sire / heir）
Task = 线程    共享 Team 的空间，独持自己的 trap 帧
```

分工按**「谁负责哪一类事实」**切，不按对象切：

| 组件 | 只管 | 明确不管 |
|---|---|---|
| `scheduler` | 谁在本核跑、量化轮转、跨核偷取 | 事件等待、死亡、授权 |
| `messenger` | 事件等待与唤醒、死亡两相、回收 | 谁被调度（它只投递与摘除） |
| `conductor` | 任务计数、全退出停机、休眠核唤醒 | 单个任务的状态 |
| `unit`（Task/Team） | 类型与状态机、血缘、装载 | 调度策略、等待语义 |

授权不在这一层：`Build` / `Spawn` / `Hatch` / `Join` 是 envcall，过不过门闩由 `work/unit/gate`
决定（见 [pie.md](pie.md)）。`room/` 与外界只有**注入面**耦合（退出钩子、存活任务快照），
不反向依赖 gate/mail——这条单向边是 lockdep 能立住的前提（`messenger/mod.rs:15-19`）。
`Task::release` 明说「不做授权」（`work/unit/task.rs:253-256`）；scheduler 也承认
park/wait/reap 已全部移出（`core/hart.rs:331-333`）。

## 2 · 结构（文件 → 职责）

```text
work/mod.rs:5        unit / room / mail 三分
work/room/mod.rs:3   scheduler / messenger / conductor
```

| 文件 | 职责 |
|---|---|
| `work/unit/task.rs` | `Task` / `TaskIdent` / `TaskBuilder` / `TaskState` / `TaskTag`；合法迁移表 |
| `work/unit/team.rs` | `Team` / `TeamBuilder` / 内核单例；血缘与成员簿记（`team.rs:33-53`） |
| `work/unit/life.rs` | `Life` / `TaskLife`——「键的存活单元」，站点判死的唯一依据 |
| `work/unit/loader.rs`、`parser.rs` | ELF → Space durable；装载是**单次 `with_flush` 原子事务** |
| `work/room/scheduler/core/hart.rs` | `Scheduler`：`inner`(L1) + `badge` + `starved_len` 锁外镜像 + `steal_cursor` |
| `work/room/scheduler/core/ident.rs` | `Badge` / `Identity{Live, Last}`——标识的两态 |
| `work/room/scheduler/core/table.rs` | `SCHEDULERS`、`ROSTER` 名册、全机扫描、`rip` |
| `work/room/scheduler/core/fetch.rs` | `fetch` / `steal` / WFI 睡眠协议 |
| `work/room/scheduler/{boot,trap}.rs` | **入口面**：多核 panic 卧倒 + 二选一转发，仅此 |
| `work/room/messenger/mod.rs` | `EXIT_REASON` 逐核暂存槽 + `SiteStats` 观测面 |
| `work/room/messenger/{doom,reap,handoff}.rs` | 扑杀两相、回收与躯壳队列、落点 `Handoff{Resume,Switch}` |
| `work/room/messenger/wait/{mod,site,holder}.rs` | 站点表、等待者、`WakeKey`、票根 `Ticket` |
| `work/room/conductor.rs` | 停机判据与屏障、休眠核位图 / IPI、钩子注入面 |

## 3 · 关键类型与状态机

**标识两态**：`Identity{Live, Last}`（`core/ident.rs`）。任务退场后 `Badge` 降级为 `Last`，
只留诊断身份——「已回收」与「从未分配」因此不糊在一起（后者查名册直接落空）。

**状态机**（合法表 `work/unit/task.rs:181-203`，**非法迁移 panic**）：

| 迁移 | 触发 | 代码 |
|---|---|---|
| `Held` → `Starved` | `Hatch` → `Task::release`（放行入队 + `launch` 踢一核） | `task.rs:257-273` |
| `Starved` → `Running` | `prepare`：满额 `TIME_SLICE=8` + 写 kernel_sp + 武装定时器 | `core/hart.rs:178-198` |
| `Running` → `Starved` | 预算尽 `advance` / 主动让出 `starve` / 被唤醒 `rise` | `core/hart.rs:312-329`、`:277-294` |
| `Running` → `Blocked` | `block`（**唯一写点**） | `wait/mod.rs:65` |
| `Blocked` → `Starved` | `rise`（四路唤醒同形收尾） | `wait/mod.rs:113-126` |
| `{Held,Starved,Blocked,Running}` → `Doomed` | 扑杀 `suspend`；自退路径由 `reap` 就地补停摆 | `messenger/doom.rs:94`、`reap.rs:43` |
| `Doomed` → `Reaped` | `reap`：钩子跑完 → 入 `HUSKS`（**唯一置位点**） | `messenger/reap.rs:46` |

四条状态名**只覆盖一条进入路径**：放行、让出、唤醒都落到 `Starved`——名字是"登记处"，
不是"经历"（`task.rs:51-56` 自认，名字经用户裁决保留）。

## 4 · 不变量

| 不变量 | 违反会怎样 | 谁守着 |
|---|---|---|
| 容器 ⇔ 状态（starved 只收 Starved；装槽前 running 必空） | 队列与状态分家，装错帧 | `core/hart.rs:154-162,211-226`（`debug_assert`） |
| 状态变换穷尽于合法表 | 静默进入不可达态 | `task.rs:181-202`（panic） |
| `Running{ticks_left} ≥ 1` | 落盘 `Running{0}`，永不再让出 | `task.rs:220` + `core/hart.rs:312`（先判后减） |
| `Reaped` ⇔ 退出钩子已跑完 | `Join` 判据出现竞态 | `reap.rs:39-47`（唯一置位 / 唯一入壳） |
| 队列非空 ⇒ 键还活着 | 站点钉住死资源 | `block` 锁内判死 `wait/mod.rs:72-78`；`prune` `site.rs:224-231`；载体 `life.rs:41-57` |
| 站点只有活 / 墓碑 / 孤儿三形态，其余即删 | 每次 park 留一个空壳，表随运行增长 | `site.rs:184-197`（消费信标后当场 `prune`） |
| 计数挂**产生处** | `Held` 被扑杀时配不平 ⇒ `done()` 恒假、永不停机 | `task.rs:390-492` + `conductor.rs:46-52` |
| L1(`Scheduler`) 与 L3 任何方向不嵌套；不持 L3 去 drop `Arc<Task>` | lockdep 违规 / drop 链里取 L2 | `messenger/mod.rs:15-19`；实录见 `reap.rs:101-110` |
| 跨挂起不得持强引用 | 被扑杀时引用永不回落，任务永久钉住 | `wait/mod.rs:94-99` |
| 名册只增不删（`delist` 保留未实现） | 「已回收」与「从未分配」重新糊在一起 | `core/table.rs:89-95` |
| 观察者只读 `tag`；键票只能问容器 | 读到被独占写的 payload | `task.rs:74-91`、`doom.rs:106-130`、`holder.rs:16-24` |

## 5 · 时序：一次 `Wait` 的站点路径

选它而不是 `Spawn`：**站点表是挂起任务的唯一容器**，这条路上把「键 / 票 / 信标 / 强引用」
四样东西的分工一次讲清。`block(key, life, dur)`（`wait/mod.rs:45-106`）：

1. **信标先探** `take_beacon`——缺键即无信标，**不 `or_insert`**；命中即 `Resume`，
   且消费后当场 `prune`（`site.rs:184-197`）。
2. **离核**：`current().swap()` 取走 running、装下一个 starved；无后继则 `Badge::shed`
   降级为 `LastIdent`（`core/hart.rs:239-254`）。
3. **登记**：`Ticket::alloc`（单调、永不复用）→ `hold`（票根**只存 `Weak`**，防出现第二强持有者）
   → `timer::tock`。**先票根后 tock**：堆可见 ⇒ 票根必在，否则到期路径命中空。
4. **入队**：写 `Blocked{key, ticket}` → 站点锁内 `or_insert` 建站、重写 `site.life`、
   **判键死活 + 再查一次信标**；两支撤销，一支入队。
5. **收尾**：撤销支 `void(ticket)` + `rise`；入队支 `drop(task)`（跨挂起不复持强引用）。
   落点恒为 `Switch(next_pa.unwrap_or_else(run))`。

**三条恢复路径**：

| 路径 | 谁用 | 做法 |
|---|---|---|
| `wake(key)` | 普通事件 | 摘队首；无人在等则**置信标**；键已死则删站且不建站（`wait/mod.rs:287-310`） |
| `redeem(ticket)` | 定时器到期 | 票 → 票根取回键与持票人 → 按票号摘队；陈旧登记每步自然落空，**无需取消记账**（`:322-349`） |
| `wipe` / `wipe_space` | 资源 / 空间退役 | 整键 / 整空间 `void` + `rise`（`:219-270`） |

收尾统一走 `rise`：置 `Starved` → 记事件 → 投本核队列 → **批量之后踢一次** `kick`
（IPI 从 O(N) 降到 O(1)，`:113-126`）。

## 6 · 死亡两相（为什么必须分两相）

`suspend`（`doom.rs:61-131`）与 `reap`（`reap.rs:39-48`）不能合并：退出钩子会**摘门闩**，
摘门闩会**唤醒等待者**；只要还有受害者留在任何容器里，它就可能被别的核偷走并在"注定要死"
的状态下运行，从而看到一个**已经死掉的资源**（`doom.rs:4-7`）。

```text
第 1 相 suspend —— 只摘容器，统一置 Doomed
  Held     → take_held
  Starved  → remove_from_starved
  Blocked  → 扫分片问容器拿键与票
  Running  → 记 doomed + SSIP nudge（目标核自己 trap 时自查自退）
第 2 相 cull → 收齐后才逐个 reap：跑钩子 → Reaped → 入 HUSKS
```

- `tag` 只作提示，**容器返回值才是结论**；不一致就重来（RETRY=4），耗尽按 `Running` 兜底
  （`doom.rs:61-131`）。
- 他核 `Running` **不同步拉走**：那会破坏「`Reaped` 不在 running 槽」这条不变量，故只发
  SSIP 让它自己退（`doom.rs:133-141` + `trap.rs:218-234`）。
- 延迟的是**回收**不是收尾：栈 / 帧 / 空间的归还留到 `bury`——**不能在自己正在用的栈上回收
  自己**（`reap.rs:6-11,50-58`）。
- 阶段边界即安全边界（`doom.rs:150-166`）。

### 6.1 两个入口：级联 与 他杀

同一套两相有**两个触发者**：**级联**（父域退出 ⇒ 沿 `heir` 扑杀子树，挂在那条任务的
退出钩子上）与**他杀**（`RoomCall::Doom` ⇒ 目标**一个域**，`envcall.rs` 里的一支）。

- 判据 = **血缘、传递、按域比较**（`messenger::descends`）：目标域沿 `Team.sire` 上溯，
  命中发起者的**域**即放行。按域而非按 task——`sire` 链上记的是**建域那一枚 task**，
  按 task 比会让同域的另一线程杀不了自己的子域。**严格祖先**：自己不算自己的后代，
  否则「杀我域里的一枚线程」会退化成「杀掉我整个域连同我自己」。
- 语义 = **域粒度**：`task` 只是"指认域"的手柄，执行直接复用 `cull(&[team], reason)`
  （同域的线程一并走，不会剩半个域）——与 Linux `kill <pid>` 同款。
- **原因码随杀令走**：退场原因码住的是**逐核暂存槽**，写它的必须是"在那颗核上调用
  `quit()` 的那段代码"；而他杀的受害者是在**别的核**上被 IPI 唤起、自己在 `trap.rs`
  里自退的 ⇒ 码只能随 `doomed` 一起过去（`doomed` 因此是 `task id → 原因码`，不再是一个
  集合成员）。"**谁杀的**"在下令那一刻记一笔（`RoomEvent::Doomed { tid, by }`）——
  一个事实一份账，不随杀令再抄一份。
- 内核给的三枚码：`EXIT_FAULT` / `EXIT_DOOM` / `EXIT_CASCADE`（与域自己的编号共用字段：
  域从 1 起、内核占高位段；**码与槽同住** `messenger/mod.rs`，谁写那格谁登记取值）。

## 7 · 多核与停机

- **hart 直达**：`tp → PerHart.scheduler`，表在即指针在（`core/table.rs:164-176`）。
- **空槽顺序**：本核队列 → `done()` → steal → WFI（`core/fetch.rs:28-44`，**顺序不可重排**）。
  偷取用 per-hart 游标随机起点 + 锁外 `backlog()` 预检 + `try_pull`，避免 cache 行乒乓。
- **睡眠协议**：「置位 → 复查 → 睡 → 醒后再取」，含哑睡壳（`fetch.rs:86-138`）。
- **停机判据**：`done() = ROOTED ∧ (PUSHED == 0 ∨ REAPED == PUSHED)`（`conductor.rs:66-72`）。
  `REAPED` 在 `bury` 归还完成之后才 +1——否则最后一个任务退出时别核抢先 `halt`，关机断言
  会误报泄漏（`reap.rs:140-142`）。
- **停机屏障**：胜出核 `yell()` 广播 + 等 `HALT_ARRIVED == hart_count`，之后跑注册钩子：
  messenger 观测 → `scheduler::core::rip` → block flush → audit 基线（`boot.rs:150-175`）。

## 8 · 裁决账

| 裁决 | 定论 | 理由要点 |
|---|---|---|
| 站点寿命 ＝ 资源寿命 | `wipe` 删站点、**不留墓碑**；判死靠 `Weak<Life>` 观察 | 不需要写路径 / 回调 / 第二张表；`Life` 零尺寸、不参与锁序（`life.rs:11-27`） |
| 扑杀分两相 | 先全停摆，再逐个跑钩子 | 见 §6 |
| 他核 `Running` 不自退 | 只记 `doomed` + SSIP | 同步拉走破「`Reaped` 不在 running 槽」 |
| 延迟回收 | 栈/帧/空间留到 `bury` | 不能在自己正在用的栈上回收自己 |
| `Held` 也计入 `PUSHED` | 计数挂产生处 | 否则 `Held` 被扑杀时 `done()` 恒假 |
| `Spawn` 恒产 `Held` | 授权后才 `Hatch` | 新线程权限表起步为空，**先授权后运行**（`task.rs:6-8`） |
| 名册一张而非每 hart 一张 | 全局 `ROSTER` | 每 hart 一份 = 全世界副本放大 H 倍（`table.rs:84-95`） |
| `WakeKey` 用枚举，不位打包 | 键是类型不是数字 | 掩码单射性与 mask helper 错联风险一起消失（`site.rs:23-28`） |
| `Handoff` 两态 | 本核无后继归 `run()` 收口 | 落点不是调用方的事（`handoff.rs:8-10`） |
| 信标只是提示 | 调用方必须复核条件 | 裸 `pull` 取走数据时信标不消费（`wait/mod.rs:281-286`） |
| 他杀只认血缘 | `RoomCall::Doom` 的判据 = `descends`（传递、按域、严格祖先） | 跨血缘的"该不该"是**政策**的活（root 的 `doom` 服务）；内核只答"能不能" |
| 杀令带原因码 | `doomed`: `task id → 原因码` | 受害者在**他核**自退，杀者写不进它的原因槽 |
| 级联不逐条记 `Doomed` | 父域自己那一笔 `Exit` 就是"谁杀的" | 一个事实一份账，不为子树里每个任务各记一条 |
| 他杀不等回收 | 下一句要等用 `Join` | Linux 的 `kill` 也是"送到即回" |

## 9 · 已知边界

1. ~~**注释与代码不一致**~~ —— **已修（本轮）**（改称 `ROOTED`）。原记录：`conductor.rs:61-63` 写守门叫 `BOOT_DONE`，代码实为 `ROOTED`
   （`:27,67`）。
2. ~~**注释陈旧**~~ —— **已修（本轮）**（改为「调用方据此重来，重试耗尽按 `Running` 兜底」）。
   原记录：`core/table.rs:136-138` 说 `remove_from_starved` 返 false ⇒「本次 kill 丢失」，
   实际已被 RETRY + 按 `Running` 兜底取代（`doom.rs:61-131`）。
3. **待裁决**：`reap.rs:62-66` 仍走 `run()` 而非用 `swap` 给出的 `next_pa`，差别是后继多扣
   1 个量子（8→7），注释自记「本轮不动，待单独裁决」。
4. **信标可能陈旧**：`wait` 的返回只是提示（见 §8 末行）；有界等待方还须按 deadline 循环。
5. **`Level::L3` 里的 3 不是数值**：实际层级 L3 = 4，3 是删掉的旧槽位（`core/hart.rs:21-23`）。
6. **名册与分片表只增**：条目为 `Weak`，关机 `rip` 一次性放掉（`table.rs:94-95`）。
7. **已实证的假泄漏**：4 hart = 4 个 48B `LastIdent`，故关机前 `badge.clear()`
   （`core/ident.rs:87-90`）。
8. **`task_exit` 反向耦合只拆了一半**：dock/ring 那一侧仍直调过渡（`messenger/mod.rs:21-22`）。
9. **同域的兄弟线程不在血缘里**：`heir` 是 **task → 子 Team**（`UnitCall::Spawn{team:0}`
   把线程产进当前 Team，**不产生血缘边**）⇒ ① 杀一个任务 ≠ 杀整个域；② 级联**收不到**
   同域的兄弟线程。root 的他杀服务正因此必须由主线程显式收场（协议里的 `Quit`）。
10. **S 态域能把自己弄成不可中断**：清掉 `sstatus.SIE` 后 SSIP 送不进去 ⇒ `doomed` 记着
    但**永不执行**（U 态域做不到——它写不了 `sstatus`）。这是**既有**边界（级联同一条
    路），`Doom` 只是让它可被政策触发。
9. **轮转窗口**：`Switch` 事件特意落在 `seat` **之后**，防窗口内崩溃把已下台任务报成当前
   （`core/hart.rs:324-325`）。

## 10 · 判据与验证

- **验收门**（`scripts/examine.nu`）：默认档 9 条 marker，含 `spawnjoin -> 499500` 与
  `task: all tasks exited, system halted`（`:144-158`）；audit 档追加 `sleep 700ms`（票单调不
  复用 ⇒ 第二次仍要醒）、`stray: 3/3 illegal-id joins denied`、`cascade: ok` 与 `[audit] sites …`，
  并有 `AUDIT_ORDER` **顺序断言**（`:191-194`）；站点表判据是**无孤儿 / 无死键 / 无活站点**；
  harden 档以 `starved 容器只收 Starved 任务` 为正向对照探针，要求四串护栏在同一份产物里
  （`:208-227`）。
- **shell 自检**（`programs/src/bin/user/shell.rs`）：`spawn`（`Spawn`+`Join`，`:987-1001`）、
  `cascade`（三跳撤销 / 无关分支 / `release` 级联 / 任务消亡级联，`:326-471`）、`churn`
  （生灭压测，判据是「`churn 1` 与 `churn 2000` 的关机账逐字相同」，`:474-552`）、`stray`
  （野 id 去 `Join` 必 `Denied`，`:883-898`）、`badslot`（非法槽位拒掉且内核续跑）。
- **关机账**：`[audit] roster N alive 0`——名册活任务数是「哪个任务没走」的直接读数；
  帧/块账只说明"有东西没还"。
