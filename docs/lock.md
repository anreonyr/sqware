# lock — 锁与锁序检查（lockdep）

> 路径约定：`文件:行` 相对 `kernel/src/`。层级表与 space/mail 的锁序有关，见 [space.md](space.md) §6。

## 1 · 语义定位

`lock/` 是**中断安全的同步原语集合**：关 `SIE` 的互斥、不关中断的互斥、无锁读三种形态
（`lock/mod.rs:1-8`）。`depend.rs` 在同一批原语上加**两层强制**：跨锁的层级单调，以及
不可重入锁的同锁重入检测。

它的生效范围是**编译期决定的**：

```text
debug 构建           → lockdep 在
--profile harden     → lockdep 在（release + debug_assertions）
--release（默认档）  → lockdep **编掉**，只剩 Level 这个值类型
```

理由见 §3 末：门的默认产物只有 `--release`，所以要另设 harden 档才看得见它。

## 2 · 结构

| 文件 | 职责 |
|---|---|
| `lock/mod.rs` | 门面 + `depend` 宏族：`depend_enter!`（读 `ra`，`:31-38`）、`depend_check!`（取前校验 `:43-52`）、`depend_acquire!`（记入 `:58-67`）、`depend_release!`（移除 `:70-75`）、`init_depend:89-93` |
| `lock/spin.rs` | `SpinLock`：先 `TrapGuard::save` 关 `SIE` 再自旋；guard 带 `PhantomData<*const ()>` **禁 `Send`**（必须本 hart 释放，`:40-46`）；`caller` 记调用点供溯源（`:70-76`） |
| `lock/rw.rs` | `RwLock`：单个 `AtomicUsize`（`WRITER_BIT \| 读者数`，`:19-30`），写者优先；三条同 hart 死锁自检（降级 `:79-91`、写重入 `:119-129`、升级 `:142-151`）→ `depend::report`，release 下退化为 `panic!`。**无 level、不 `new_level`、不记 held set** |
| `lock/reentrant.rs` | `RelLock`：`owner = hart_id + 1` + `count`，同 hart 重入合法、跨 hart 互斥（`:95-156`）；`Level::Space` 的唯一用户（`space/adapter.rs:138`） |
| `lock/once.rs` | `OnceLock`：一次写、多次读（读路径一次 Acquire load，`:42-51`） |
| `lock/lazy.rs` | `LazyLock`：`fn() -> T` + `OnceLock`；**当前无用户**（`:14`） |
| `lock/bare.rs` | `BareLock`：不关 `SIE`，`lock()` 是 `unsafe fn`，仅任务上下文；**当前无用户**（`:37,66`） |
| `lock/trap.rs` | `TrapGuard`：`SIE` 的 save/restore，锁内部复用，不对外 |
| `lock/depend.rs` | `Level` 定义、`HeldSet`、`check`、`report`、`init` |

`Level` 是 `#[repr(u8)]` 枚举 + derive `Ord`（`depend.rs:35-66`）：

```text
1 Scheduler   2 Space   4 L3(帧/槽)   5 Asid   6 Frame
7 Block       9 Tally       10 Spare
```

**空槽不是笔误**：3 与 8 是删掉的旧槽位（8 当年是审计账本 `Ledger` 的层级，随
`allocator/fence` 一起删了），`Block=7` 当前全仓无调用点。声明方式是在构造处
调 `new_level`（`spin.rs:59`、`reentrant.rs:65`、`bare.rs:49`），比较规则在 `depend::check`：
先 `contains` 判重入，再 `Some(lv) 且 lv <= max(held)` 即违规——**严格递增**（`depend.rs:250-257`）；
`max_level` 只数 `Some` 的（exempt 不参与，`:150-152`）。

## 3 · lockdep 细节

- **每 hart 一份 `HeldSet`**（`MAX_HELD = 8`，真实嵌套 ≤ 4，`depend.rs:115-134`），由 `init`
  在分配器就绪之后装配（`boot.rs:112`）。
- **取前校验**：`check` 在**自旋之前**跑（此时 `SIE` 已关）——「死锁发生在自旋之后，
  自旋之前必先暴露」（`depend.rs:9-10`）。`try_lock` 不查（非阻塞没有 ABBA 边界，
  `spin.rs:114-116`）。
- **双侧平衡**：`acquire` 取到后记入（含调用点），满 → `held set overflow`；`release` 移除，
  缺 → `release of unheld lock`（`:264-282`）。**exempt（`level = None`）也记入**，只为与
  release 平衡并让 `contains` 生效（`:21-24`）；`RelLock` 末次递减才 release（`reentrant.rs:184-187`）。
- **跨核不查**：held set 只读本核（`:186-187`）——跨 hart 争用是真自旋。
- **报文体**（`report`，`:80-113`）：第一件事是把门户切到后备仓（spare）**再** `format!`——
  违规时那把锁常仍被本核持着，走主堆分配会重入自旋。随后拼
  `[depend] {what}: {lock:#x} ({lock:#x})` + 可选 `caller:` 行 + 逐条 `held:`（含
  `acquired at`、`<-- max held`）+ `rule: new level must exceed max(held); violation`，
  最后 `panic!` → halt（他核由 `halt::alarm` 停）。
  `what` 取值：`recursive acquisition` / `lock-order level violation` / `held set overflow` /
  `release of unheld lock`；rw 另有三条（`rw.rs:83,122,144`）。
- **release 为什么编掉**：`POOL`/`HeldSet`/`check`/`acquire`/`release`/`report`/`init` 全体
  `#[cfg(debug_assertions)]`，只有 `Level` 作为值类型留下（锁的 `level` 字段与 `new_level`
  免 cfg）。**harden 档因此是必须的**：门的产物只有 `--release`，不另开一档，整条校验在门
  跑过的每个产物里都缺席（`Cargo.toml:22-30`）。

## 4 · 不变量

| 不变量 | 违反会怎样 | 谁守着 |
|---|---|---|
| held set 仅本 hart 访问，且只在 `SIE` 关时读写 | 数据竞争 / 记账错乱 | `check` 必须在 `TrapGuard::save` **之后**（`mod.rs:28-30`、`spin.rs:91-93`）；`BareLock` 例外，靠 unsafe 契约 |
| 记账双侧平衡（exempt 也记） | release 误报 unheld / 重入漏检 | 宏成对（`mod.rs:54-57,70-75`）、`RelLock` 末次递减才 release |
| 层级严格递增 | ABBA 死锁 | `depend.rs:250-257` |
| 锁的释放与获取同一 hart | guard 被搬到别的核释放 | `PhantomData<*const ()>` 禁 `Send`（`spin.rs:40-46`） |
| `RelLock` 同 hart 重入合法、跨 hart 互斥 | 自锁 / 互斥失效 | `reentrant.rs:95-156` |

## 5 · 裁决账

| 裁决 | 定论 | 理由 |
|---|---|---|
| lockdep 只在 `debug_assertions` | release 编掉（只留 `Level`） | 校验全在 held set；门的产物是 release ⇒ 另设 harden 档（`Cargo.toml:22-30`） |
| 校验时机 | **取前**（自旋前） | ABBA 发生在自旋之后（`depend.rs:9-10`） |
| 跨核锁序 | 不查 | held set per-hart（`:129,186-187`） |
| exempt 锁 | 记入但不校验层级 | 双侧平衡 + `contains` 生效（`:21-24`） |
| `RelLock` 重入 | 合法，跳过 check | `owner == me`（`reentrant.rs:105-113`） |
| `RwLock` | 只报警、不记账 | 无 level、不 `new_level`（`rw.rs:24-30`） |
| 报文体 | 有分配，但先切 spare | 门户的无锁判别位（`memory/allocator/portal.rs:1-9`） |

## 6 · 已知边界

1. **跨核 ABBA 不查**（`depend.rs:186-187`）。
2. **`RwLock` 完全脱离锁序**（`rw.rs:24-30`）；release 下三种自检退化为 `panic!`
   （`:90,128,150`）。
3. **层级表两个空槽**：`Level::Block = 7` 全仓无调用点，3 是删掉的旧槽位
   （`depend.rs:58-59`、`scheduler/core/hart.rs:21-22`）。
4. **符号化已移除但痕迹仍在**：报文体每个地址打两遍（`depend.rs:83,85,98-100`）；
   `symbol()` 退化成裸 hex（`diagnose/backtrace.rs:24-26`）；`FrameResolver::executable`
   恒 `false` 使 `classify` 的第三档不可达；`rustc-demangle` 在 `kernel/Cargo.toml:37-39`
   零调用点。
5. **两个原语无用户**：`LazyLock`（`lazy.rs:14`）、`BareLock`（`bare.rs:37,66`）。
6. **注释与代码不一致**：
   - ~~`boot.rs:114`「spare 预算验收恒跑」~~ —— **已修**（改为「三个探针全在 `debug_assertions`
     档，release 下本函数是空体」）。
   - ~~`depend.rs:77` 后备仓「层级 9」~~ —— **已修**（改为层级 10 `Spare`）。
   - ~~`once.rs` 的 Release 序声称~~ —— **已修（本轮）**，且**缺口一并修掉**：标记由
     `AtomicBool` 改为三态 `AtomicU8`（`EMPTY → WRITING → READY`），`set()` 抢到
     「写入中」后**写完 data 才** Release store「就绪」——发布晚于写入，
     「见 `READY` ⇒ 见数据」由 Acquire/Release 配对保证；`get()` 在「写入中」返回
     `None`；抢不到写入权的一方等它落到 `READY` 再报「已初始化」（写方被抢占时该循环
     可被中断，不死等）。消费者四处（`HERTZ` / `TRAP_STACK_PHYS` / `POOL` / `MACHINE`）。
   - `depend.rs:4-5` 说「机制关闭即零开销」，但 release 仍读 `ra` 并存 `caller`
     （`spin.rs:89,101`）。
   - ~~`rw.rs:21,44,107` 三处「未使用」~~ —— **已修**（三行 `allow` 删除；删后无新告警，
     说明确实都有消费者）。

## 7 · 判据与验证

- **panic 缺席** ＝ 捕获里没有 `[panic] at`（`scripts/examine.nu:447`）——报头由 `halt.rs`
  拼出，故任何 `debug_assert` 或 health 失败都必然暴露。
- **lockdep 缺席** ＝ harden 档捕获里没有 `[depend]`；命中即点名「lockdep 违规」判 FAIL
  （`:451-456`）。
- **harden 档的四串正向对照**（在 ELF 里 `grep -ac ≥ 1`，少一串当场 `exit 1`，`:222-227`）：
  `starved 容器只收 Starved 任务`、`unmark: no record`、`allocated non-free frame`、
  `lock-order level violation`——最后一条就是 lockdep 的报文体本身。
- **harden 轮的产物**：`--profile harden --features audit`，落点 `target/<triple>/harden/`
  （`:622-626`）；默认轮恒不带 feature，并有「默认档不该出现 audit 输出」的哨兵（`:517-521`）。
