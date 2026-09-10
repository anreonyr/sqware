# 飞线审计 — 快速迭代残留的定位、冗余判定与纳入方案

> 审计范围：`kernel/`、`task/`、`crates/`、`scripts/`、`docs/`（26192 行 Rust，不含 `target/`）。
> 方法：五路并行通读 + 主审关键路径（scheduler / messenger / hole / envcall / gate / fence / boot），
> 每条结论给出 `path:line` 与调用点计数。**本文件只做判定，不含实现**；实现按 §8 分阶段、逐关裁决。
>
> 判据用本仓自己的设计教条（README「六原则」+ 自文档纪律）：
> 结构自洽、原语正交、核心/适配分离、**删光注释仍读得懂**。
> 因此本文把「必须靠一段长注释才懂」的地方一律计为飞线信号 —— 注释是设计没干净的补偿。

---

## 0 · 一句话结论

飞线不是散落的小补丁，而是**四条一直没被纳入结构的主线**：

| # | 主线 | 一句话 | 体量 |
|---|---|---|---|
| A | **等待/死亡簿记未定型** | 任务「不在 running 槽」的状态被拆成 5 张全局表 + 3 个句柄分配器，各自带一套竞态闭合注释 | `messenger.rs` 834 行 + `scheduler/core.rs` 687 行 |
| B | **同一个动词有两三条路径** | 固定地址 `Mmap` 绕过窗口层、`copy_in` 有四份、`seal ≡ Drop`、token→门闩解析在 envcall 里重写 9 次 | 跨 6 文件 |
| C | **记账/审计层穿透资源层** | `fence` 自称「分配器零审计词汇」，实际生产路径有 33 个调用点；`Space::drop` 调 `fence::retire`；`banker` 与 `frame::pagemeta` 记同一事实 | `fence/**` 1600 行 + `statistics.rs` 439 行 |
| D | **编译期与死代码残留** | 两处 `#[inline(never)]` 承担**正确性**、18/22 个 bin 永不装载、`ktask` 内核线程面整个是死岛 | 约 1900 行 |

另有 **1 条 CRITICAL 安全缺陷**（非清理项，应先修）与 **1 条框架缺口**（测试 harness 缺失，是 18 个死 bin 的根因）。

---

## 1 · CRITICAL：U 态一发 `ebreak` 可打死内核

```
kernel/src/runtime/switcher/envcall.rs:168-172
    frame.sepc += instr_len(&ident.team.space, frame.sepc);
    let envcall = match EnvCall::from_wire(number, &regs) {
        Ok(c) => c,
        Err(_) => panic!("invalid envcall number: {number}"),   // ← 用户可控
    };
```

- `a7` 由 U 态任务完全控制；`crates/env/src/fid.rs:95` 的 **class 1 idx 5 是空号**
  （`SpawnTask` 并入 `Spawn` 后不复用），class 7 整类删除（`fid.rs:14-17`）。
- `from_wire` 对未声明 slot 一律 `Decode::BadSlot`（`envmacros/src/lib.rs:144-157` 只对已声明
  variant 生成 `0..nkind` 分支），于是 `0x1_0000_0005` / `0x7_0000_0000` 直接落进上面的 `panic!`。
- 对照：**其它用户引起的异常都走故障隔离**（`trap.rs:452-471` → `utask::reap`），
  用户想自杀有正规原语 `ControlCall::Panic`（`fid.rs:279-282`）。
- 修法已现成：`ret_err(frame, GateError::Denied)`（同文件:68-71）。

**判定：立即修**（不属于「冗余清理」，属于缺陷）。顺手把「未知 slot 的运行时行为」写进 `fid.rs` 文件头
—— 那是 ABI 契约的一部分，现在只在散文里。

> **✅ 已修（阶段 0，本轮）**：`envcall.rs:171` 的 `panic!` 改为 `return ret_err(frame, GateError::Denied)`。
> 并补上了**此前缺失的验证**：shell 新增 `badslot` 自检命令（`task/src/bin/user/shell.rs`），
> 直入 ABI 唯一汇编入口 `warpper` 打三个非法槽位（空号 `0x1_0000_0005`、已删 class `0x7_0000_0000`、
> `usize::MAX`）。
>
> **该用例经过「有效性验证」**（先证伪再证实）：临时把 `panic!` 恢复回去 → 串口出现 crash scene，
> 末行 `envcall #12884901889`（= `0x1_0000_0001`，class 1 idx 1），**整机停摆**；
> 装回修复 → `badslot: 3/3 rejected, kernel alive`，且紧随其后的 `clock` 正常应答、shell 正常退出。
> 即：这条缺陷是**实测可复现的 U 态一击停摆**，而不是纸面推理。
>
> 同批修掉的两处同类（同一「错误通道」缺陷）：`IOCall::Get` 空输入 `-2 Dead` → `GateError::Busy`（`-3`，
> 与 `fid.rs` 注释、`ecall.rs` D1 表、`EnvError::is_busy()` 三者对齐）；`Backtrace` 的裸 `-1isize`
> → `GateError::Denied.code()`。

---

## 2 · A 主线：等待与死亡的簿记未定型

### A1 · 五张全局表 + 三处句柄，编码同一件事 · **DESIGN（本报告的核心）**

`kernel/src/work/room/messenger.rs` 里五张 L3 表全为「timer 句柄 → 原始身份」的反查而存在：

| 表 | 键 → 值 | 行 | 为什么存在 |
|---|---|---|---|
| `parked` | handle → `Arc<Task>` | :115 | tock 只存 handle，唤醒时要找回任务 |
| `wait_times` | handle → `WaitKey` | :154 | 同上 |
| `join_times` | handle → tid | :191 | 同上 |
| `wait_sites` | `WaitKey` → `WaitSite` | :140 | 按事件键等 |
| `joins` | tid → `JoinSite` | :185 | 等目标结束 |

句型重复到代码自己承认：`messenger.rs:184`「与 `wait_sites` 同形、同为 L3」；
`JoinSite`(:176-179) 与 `WaitSite`(:85-90) **字段逐个相同**；`join_times` / `wait_times` 同形。

**根因**：timer 的 tock 只携带不透明 `u64` handle，**唤醒目标身份不在 tock 里**。
于是一切旁路表都是「handle → 真正想唤醒谁」的补丁。

**可见症状（不是理论）**：
1. `drain_expired` 里三段唤醒块近乎逐字重复（:534-540 / :558-564 / :567-575）。
2. `suspend` 摘一个 `Blocked(Wait)` 任务要**扫全部 16 个分片**按 `ptr_eq` 找（:726-740）——
   因为表按 key 索引，而 kill 按 task 出发；缺一条 task→site 的反向边。
3. `join` 与 `wait` 是同一台状态机的两份（`crates/env/src/fid.rs:181-184` 自己把这两类等归为一类原语）。
4. `sites` 站点**永不回收**（见 A2），所以每次「唤醒无人等待」都留一个永久条目。

**纳入方案（一个原语，两处身份）**：

```
原语 A：Tock 携带唤醒目标
  enum WakeTarget { Key(WaitKey), Task(usize) }
  timer::tock(tick, target)        // tick = { wake_at, target }

原语 B：等待点长在任务上
  task.wait = WaitTicket { site: Option<WaitKey>, tick: Option<Tick> }
```

- `parked` 消失：park 到期 = `WakeTarget::Task`。
- `wait_times` / `join_times` 消失：键与目标进 tick 本体。
- `wait_sites` / `joins` 合一：都是「键 → 等待者 FIFO + 遗留信号闩」，只差键的命名空间，
  而命名空间本来就该是键的一部分（见 A2）。
- `suspend` 从「扫 16 片」变成「读 ticket 直接摘」。
- `Handoff` / `Waited` / `Joined` 三套交接枚举（一个意思三种拼写）收成一套。

**这一条把 834 行的状态机拆回两条各约 150 行的原语 —— 也是 §7.1 拆 `messenger.rs` 的真正接缝。**

### A2 · `wait_sites` 永不回收；`HoleId` 是给这个洞打的补丁 · **DESIGN**

```
kernel/src/work/mail/hole.rs:36-37
/// 键取它而不取 `HoleMeta` 的堆地址：`wait_sites` 的站点从不回收，而地址会被
/// 分配器回收再利用——死孔留下的陈旧 pend 会被落在同一地址的新孔继承。
```

- 作者已经点明机制缺陷，选择**绕过**而不是修：`HoleId` 单调分配、永不复用（:41-44），
  于是 hole 的键不会撞。
- **但绕不过「站点永不回收」本身**：每个创建过的 hole 都给 `wait_sites` 留条目，
  且每次「唤醒无人等待」置一次 `pend`（:488-494）。长跑系统里这是无界增长。
- **同类风险对用户键未消除**：`Asid` 是**回收复用**的（`asid.rs:32-33` 位图，范围 1..=65535），
  而 `WaitKey::compose(asid, va)`（`messenger.rs:54-58`）把 asid 当命名空间。
  一个域死亡 → ASID 归还 → 新域复用同一 ASID → 同 `va` 的键命中死域留下的陈旧站点与 `pend`。
  `Space::drop` 只有 `fence::retire(asid)`（清**堆账**，`space/core.rs:1124`），
  **没有 retire 等待键命名空间**。这正是 `docs/supervisor.md:61-69` 论证过的同一类隐患。

**症状外溢到用户态**：既然 `pend` 会陈旧，`wait` 的返回值就只能是**提示**——
`messenger.rs:479-483` 明写「调用方必须自己复核条件」，于是
`mail/hole.rs:252` 要复核 `alive && ready`、用户态 `sleep`/`pull_timeout` 要按 deadline 循环。
**一个原语的不干净，被每个调用方各自补偿一次。**

**纳入方案**：站点寿命 = 资源寿命。`WaitKey` 的命名空间随资源走（hole 已就位，补 ASID 侧），
并在 `Space::drop` 与 `seal` 处**显式注销**（与 `fence::retire` 同形、同点），
使 `pend` 只可能属于活着的键。此后 `wait` 的返回值可以恢复成契约而非提示。

### A3 · 任务 id 的判活是两条路径，注册表每 hart 一份且不清理 · **ABSORB**

- `scheduler/core.rs:96-99 by_id` 是**每 hart 一张**（`Scheduler` 的字段），
  于是插入要遍历全部 hart（:410-414）、查表要遍历全部 hart（:418-425）、
  快照 `snap()` 把 N 张表的全部值拼起来（:442-448）。
- 表的 `Weak<Task>` **从不清除**；`rip()`（:390-402）只清 starved 与 info 槽，
  `by_id` 不管 —— 任务条目的清理无处发生。
- `messenger.rs:358-359` 明写「注册表只存 `Weak` 且从不清理」，并以此为
  `target_dead` 的判据之一；但同一个函数又调 `task::allocated(tid)`（`task.rs:44`）——
  **同一事实（id 是否存在过）有两个真相源**。
- 已有更合适的结构：`Team.tasks`（L3 成员簿记）与 `gate::snap` 的注入面
  （`boot.rs:164-166` 一次性接线，这个依赖倒置本身是对的）。

**判定**：`by_id` 一张就够（不必每 hart 一份），且应挂在 `Team.tasks` 的同一簿记上；
`allocated()` 与 `lookup_task_by_id()` 合成一条判活，`target_dead` 只问一个来源。

### A4 · `Held`/`Doomed`/`Reaped` 的载荷语义只在代码注释里 · **ABSORB**

`docs/root.md` §4 记了「`Held` + 至多一个引导线程」与「死亡两相」（J1），
但 `TaskState::Running { ticks_left }` 的**预算不变量**（恒 ≥1）、
「`Reaped` ⇔ 退出钩子已跑完」、「`transform` 的 10 条合法变换表」（`task.rs:135-147`）
都只写在注释里。`BlockReason::Join` 甚至从未进过任何文档（`ipc.md:95` 至今写「两个变体」）。

**判定**：这三条不变量是**类型义务**而非注释故事：把合法变换写进类型（或让非法变换不可表达），
文档只需指回类型。

---

## 3 · B 主线：同一个动词有多条路径

| # | 现象 | 位置 | 判定 |
|---|---|---|---|
| B1 | **固定地址 `Mmap` 绕过窗口层，且永不还段** | `envcall.rs:452-460` vs `window/share.rs:28-43` | ABSORB |
| B2 | **`copy_in` 有四份**，且 `mail/mod.rs:36` 承诺的「不部分写入」是假的 | `envcall.rs:100`、`mail/mod.rs:36-52`、`console.rs:99`、`task.rs:250` | DESIGN |
| B3 | **token→门闩解析在 envcall 里重写 9 次**（第 10 次在 `gate/revoke.rs:35`） | `envcall.rs:550,596,642,680,714,783,817,885,927` | DESIGN |
| B4 | **`hole::seal` ≡ `HoleMeta::drop`**，逐字相同 | `hole.rs:266-270` vs `:126-131` | ABSORB |
| B5 | **用户堆记账被拼进 ABI 适配层** | `envcall.rs:230-239`、`:258` vs `block.rs:350,368`、`heap.rs:29,45` | ABSORB |
| B6 | **`HeapWindow` 对 fence 一无所知** | 同 B5，两个 window 两种接线 | ABSORB |
| B7 | **`Get` 的空输入返 `-2 Dead`**，文档与自身错误表写 `-3 Busy`，`is_busy()` 永假 | `envcall.rs:186` vs `fid.rs:153` / `ecall.rs:31-40,46` | ✅ 已修 |
| B8 | `Narrow` 的 Pole 分支**不查存活**（其它 8 处都查） | `envcall.rs:783-795` | ABSORB（并入 B3） |
| B9 | `sepc` 前进量靠**猜访客指令长度**（`instr_len` 读用户页首字节） | `envcall.rs:80-86,168` | DESIGN |
| B10 | `assemble` 的 3 个错误来源被压成 1 个 `UnitError::Load`，调用方再丢弃 | `team.rs:213`、`unit/mod.rs:65-71`、`envcall.rs:370` | ABSORB |
| B11 | 三处错误通道并存：`usize::MAX` 6 处、`-1isize` 1 处、`-2isize` 1 处，另有约 12 处走 `GateError::code()` | `envcall.rs:178,186,247,266,447,466,484,494` | ✅ 裸码已收口（剩 `usize::MAX` 与 `Denied` 同值，留给阶段 4） |
| B12 | **长度信息在第一个处理点被丢弃** —— 服务端把部分报文当整段定长缓冲处理，`req echo` 因此返回 40 个 `\u{1}` 填充 | `echo.rs:68-74`、`shell.rs` 的 `req`、`fid.rs:190` | ABSORB |

### B1 详述（有实际后果，不只是整洁问题）

```
envcall.rs:454-459   fixed != 0 分支
    let flags = s.pte_policy(PteFlags::V | PteFlags::R | PteFlags::W);
    s.map(KVirt::from_raw(fixed), size, flags, Some(Pending::Lazy)).map(|()| KVirt::from_raw(fixed))
```

- 窗口层（`share.rs:34-41`）是 `inner.allocate(Seg::User, size)` → `inner.map(...)` → 返回 `Span`，
  `munmap` 靠这个 `Span` 才能 `release` 把**段**还回去（`share.rs:55-61`）。
- 固定地址分支两条都没做：既没登记段、也没有 `Span`。于是 `Munmap` 只好**试错**
  （`envcall.rs:475-482`：先试 `ShareWindow::munmap`，再 `pending_state != Absent`）。
- `Space::unmap`（`space/core.rs:1003-1008`）注释明写「**不还段**——段由 `release` 收」，
  而这条路径永远拿不到 `Span` ⇒ **用户在固定地址 mmap 出来的 `Seg::User` 区间永久泄漏**，
  用户段碎片化。`Mmap` 正是 `mmaper.rs` 这类程序的主路径。

**纳入方案**：窗口层补 `mmap_at(space, va, size)`（同一个 `Span` 契约），envcall 两个分支都返回 `Span`；
`Munmap` 的试错消失。这也让「一个动词一条路径」在代码里成立。

### B2 详述

- `mail/mod.rs:36` 声称拷贝「不部分写入」；实际 `:52` 提前返回 false 时**前面的页已经写进 `dst` 了**。
  对 `Pull`（`envcall.rs:613-620`）意味着**失败路径上用户缓冲区被部分写过**。
- 今天没炸，只因为所有调用方都传精确长度的缓冲 —— 这是**靠调用方纪律掩盖的假契约**。
- 另有三条并行实现：`envcall.rs:100 copy_in`、`console.rs:99` 直用 `space.segments`、
  `task.rs:250 write_args`。且 `Segments::next`（`core.rs:811-820`）每页调一次 `Space::translate`，
  **逐页取放 Space 锁** —— 拷贝中途锁被放掉重取。

**纳入方案**：一条基于 `Segments` 的拷贝原语，持锁一次、按调用方缓冲长度截断运行段，
`mail` / `console` / `envcall` 共用。前置条件恰好使行为与今天**逐字节一致**（今天全是精确长度）。
是否升级为「原子或不写」的强契约，是需要裁决的一点（§9.6）。

### B3 详述

`envcall.rs` 里 9 处形如：

```rust
let pie = pies.iter().find(|p| p.token() == token)?;
if !pie.allows(need) { return Some(Err(GateError::Denied)); }
if !pie.alive() { return Some(Err(GateError::Dead)); }
match pie { AnyPie::Hole(p) => Some(Ok(p.meta().clone())), _ => None }
```

而 `gate/mod.rs:14-15` 明写「envcall 适配层只『取本核 → 转发』，**不在壳内重写规则**」——
壳内重写了 9 次，且已经漂移（B8）。

**根因**：`AnyPie` 暴露了 `allows/covers/alive/owner/token/sire`，**却没有解析器**；
`gate` 的四个原语（accord/revoke/release/narrow）都收**已解析**的 `&AnyPie`，
于是「解析」这件事没有家。**空位。**

**纳入方案**：`gate::resolve(task, token, &[Need]) -> Result<AnyPie, GateError>`。
9 个分支各剩一行。

### B12 详述（本轮跑 e2e 时当场发现的活症状）

`fid.rs:190` 把契约写得很清楚：「`Push { len }` 与 `Pull { max }` 把长度作为参数传
——**长度是契约不是约定**」。但长度在**第一个处理点就被丢掉**：

```rust
task/src/bin/supervisor/echo.rs:68-74
    if entry.pull(&mut req_buf).is_err() { continue; }   // ← 丢弃 pull 返回的实际长度
    let reply_token = usize::from_le_bytes(req_buf[0..8]...);
    req_buf[0..8].fill(0);
    for b in req_buf[8..].iter_mut() { *b = b.wrapping_add(1); }   // ← 整段定长缓冲都 +1
    let _ = client_reply.push(&req_buf);                            // ← 整段推回
```

- 调用方（shell 的 `req`）构造定长 `PAYLOAD_LEN` 缓冲、尾部补零、**不传长度**。
- 于是服务端把「56 字节缓冲里前 14 字节是数据」当成「56 字节都是数据」，
  尾部零被 +1 成 `\u{1}`：串口实测

  ```
  req echo -> "ifmmp.tfswjdf\u{1}\u{1}…（40 个）"
  ```

  而 `docs/root.md` §8 记的期望输出是干净的 `req echo -> "ifmmp.tfswjdf…"`。

- **这不是测试噪声**：它是「长度是契约」这条设计在原语边界上被丢掉之后的必然结果，
  只是恰好因为「补零 + 逐字节 +1」而变得可见。任何按定长缓冲解释报文的消费方都会读到填充。
- 修法：服务端用 `pull` 返回的 `n` 只处理载荷段、只把 `n` 字节推回（`Push { len }` 本就支持变长）；
  调用方按实际文本长度构造。

**判定**：`docs/dispatch.md` 的 64 字节线格式是**定长协议**（字段位置固定），
但**载荷长度**必须单独携带 —— 即「定长头 + 变长体」，而不是「定长即全部」。
这一条同时给 §9.6（拷贝契约）定了方向：长度语义要贯穿 Pull→处理→Push 三处。

> **⚠ 探测结论（本轮深入后反而不能机械修）**：`Service::call`（`task/src/core/service.rs:203-213`）
> 里 `pull_timeout` 的返回长度同样被丢弃（`self.channel.mine.pull_timeout(&mut buf, …)?;` 未接返回值）——
> 与本条同形。但**「载荷到哪结束」在定长协议里不可判定**：shell 用零填充到 `PAYLOAD_LEN`，
> 服务端无法区分「零是填充」还是「零是数据」。
> 两条真实出路都要裁决：**(i)** 载荷前加显式长度字段（`docs/dispatch.md` 的目录协议已是「tag + 定长字段」形态，
> 有先例可循）；**(ii)** 承认定长契约、要求调用方把整段缓冲当载荷。
> 故本条**保留为开放项**，不纳入阶段 0 —— 机械修只会把填充从 40 字节变成另一种形状。

### B4/B5/B6 统一看

`mail/mod.rs:17-19` 的锁序长注释、`mail/mod.rs:36` 的假契约、`hole.rs` 三段关于「键取 id 不取地址」的重复注释、
`messenger.rs:1-18` 的 17 行模块头 —— 这些注释共同指向一件事：
**「资源寿命 = 能力寿命」这条主线是后加的**（commit `37bf9b0`），
加进去之后 `seal` / `Drop` / `fence::retire` / `fence::on_free` 都成了同一件事的第二、第三处表达。

---

## 4 · C 主线：记账/审计层穿透资源层

### C1 · `fence` 的「解耦纪律」已被自己证伪 · **DESIGN / 需裁决**

```
kernel/src/memory/allocator/fence/mod.rs:17-25
# 解耦纪律：类别机制收在本层，分配器文件零审计词汇
```

反例（全部在生产路径，非 `audit` 门控）：

| 位置 | 内容 |
|---|---|
| `frame.rs:87` | `super::statistics::record_frame_take(super::fence::Class::Persistent)` |
| `block.rs:350,368` | `super::fence::on_alloc(.., super::fence::OwnerKind::KernelHeap)` |
| `block.rs:440`、`manager/table.rs:114` | `crate::tag!(Pool, ...)` / `crate::tag!(Table, ...)` |
| `space/core.rs:1124` | `Space::drop` 调 `fence::retire(self.asid.get())` |
| `envcall.rs:230-239,258` | ABI 适配层负责用户堆账 |

并且 `statistics → fence`（`Class`）与 `fence → statistics`（`record_*`）**互为依赖**（模块环）。

**当前状态是最坏的组合**：默认构建里 33 个 `fence::` 调用点是空函数（`audit` 非默认 feature），
`audit` 构建里全量付费；`banker` 又与 `frame::pagemeta` 记同一事实（`fence/audit.rs:191-233` 还要断言两者相等）。

**两条自洽的终点（需裁决）**：
- **(i) 吸收**：删 `banker`，直接断言 `frame::pagemeta`（单一真相）；`Class`/`OwnerKind` 移进中立的词汇模块，
  依赖单向；`fence` 只剩 `checker`（debug 不变量）+ `ledger`（site/canary）。
- **(ii) 保留成独立构建**：让 `--features audit` 成为**唯一的 CI/验收构建**，把 audit-only 的死尾巴删净。

### C2 · `statistics` 的 baseline/delta 是旧模型的残骸 · **DELETE（本报告最干净的一刀）**

- `snapshot` / `baseline` / `delta` / `FrameDiff` / `BlockDiff` / `SpareDiff` 的唯一消费者是
  `fence/audit.rs:320,332,333`，而那里**取完就丢**（`let _ = ...`）。
  注释称它们是「statistics 模块的接线点」—— 也就是为存在而存在。
- `fence/mod.rs:40-45` 明说旧模型（快照差集 + 豁免）已被类别记账**替代**：
  「替代旧『boot 身份快照 vs 关机差集』的 rehome/adopt/基线余量/AUDITING 豁免」。
  **旧模型没删干净。**
- 副作用：`FrameStats.available` / `SpareStats.available` 只在 boot 写一次、此后恒为常量，
  于是 `health/spare.rs:31-35` 与 `:52-57` 两条断言**永不失败**（拿常量跟自己比）；
  `delta()` 还把 available 按两种定义混用（`statistics.rs:426-428`）。

**判定**：删 baseline/delta/snapshot 一族 + `available` 字段（改由 `total - occupied` 在场导出）。
**这一步同时删掉两条假验收。**

### C3 · 死尾与假标注（`cargo` 报不出，因为被 blanket allow 压住）· **DELETE**

- `statistics.rs:16 #![allow(dead_code)]`、`fence/mod.rs:51 #![allow(unused)]`、
  `lock/mod.rs:10 #![allow(unused)]` —— 三处 blanket allow 把下面的死代码全压成静音。
- 已验证 0 调用点：`ledger::verify`、`ledger::is_live`、`banker::base/words`、`frame_class_of`、
  `audit::stats`、`lock/spin.rs:72 caller`、`lock/once.rs:101 is_initialized`、
  `clock.rs:87/93/104/110`（`elapsed` 族）、`reentrant.rs:79 read_unlocked`、
  `crates/env/src/wire.rs:293 impl FromPair for (PieToken, PieToken)`（dispatcher 唯一代码残骸）。
- 只写不读：`ledger.rs:26 size_class`、`ledger.rs:34 site2`、`fence/mod.rs:541/564 RALLOC_OLD`、
  `statistics.rs:47 pool.freepool_total`（`statistics.rs:387` 的 TODO 就是它：字段恒 0、无人读）。
- 反向错标：`timer.rs:113 untock` / `:123 next_tock` 标 `dead_code` 但**都在用**；
  `rw.rs:21/44/107`、`spin.rs:117` 同样错标。
- `fence/mod.rs:470-529 alloc_site` 手写了一份栈回溯 —— 而 `diagnose/frame.rs:11-13`
  点名 `fence` 是 `frame::walk` 的调用方之一，且它存在的唯一理由（填 `site2`）本身是只写字段。

#### C3.1 逐条机检证据（阶段 1 可直接照做）
方法：对每个符号跑 `grep -rnE --include=*.rs <限定模式> kernel task crates`，
**排除纯注释行**，命中数 ≤1（仅定义本身）= 死。
单字符号（`base`/`words`/`stats`/`caller`/`read`/`get`）必须用限定模式，
否则会被无关代码命中——首轮机检就是这么误报的（`banker::base` 曾报 147 处，
实为 `free.base` 之类的字段访问）。

| 符号 | 限定模式 | 命中 | 判定 |
|---|---|---|---|
| `ledger::verify` | `\.verify\(` | 0 | 死 |
| `ledger::is_live` | `\bis_live\b` | 1（仅 `ledger.rs:276` 定义） | 死 |
| `banker::base` | `BANKER\.base\(\)\|\.base\(\)` | 0 | 死 |
| `banker::words` | `\.words\(\)` | 0 | 死 |
| `frame_class_of` | `\bframe_class_of\b` | 1（仅 `mod.rs:149`） | 死 |
| `audit::stats` / `IntegrityStats` | `audit::stats\b\|IntegrityStats` | 3，**全部在 `audit.rs` 自身**（定义 + 结构体 + 构造） | 死（自指） |
| `lock::spin::caller` | `\bcaller\(\)` | 0 | 死 |
| `once::is_initialized` | `\bis_initialized\b` | 1（仅 `once.rs:102`） | 死 |
| `Instant::elapsed` | `\.elapsed\(` | 0 | 死 |
| `Instant::elapsed_since` | `\.elapsed_since\(` | 1（`clock.rs:95`，**唯一调用者是被判死的 `elapsed`**） | 死（链） |
| `Instant::sub` | （`#[allow(dead_code)]` 标注） | — | 死 |
| `Instant::checked_duration_since` | `checked_duration_since` | 1（仅 `clock.rs:111`） | 死 |
| `RelLock::read_unlocked` | `read_unlocked` | 1（仅 `reentrant.rs:79`） | 死 |
| `term::read` | `\.read\(\)` 在 `task/src` 非 `term.rs` | 0 | 死 |
| `term::set_cursor` / `hide_cursor` / `show_cursor` | 同名 | 各 1（仅定义） | 死 |
| `io::try_put` | `try_put` | 1（仅 `io.rs:24`） | 死 |
| `io::get` | `io::get\b` | 0 | 死 |
| `Terminal::put` | `Terminal::put\b` | 0 | **文档虚构**（真名 `Terminal::write`） |

**只写不读（写点存在、读点为零）**：

| 项 | 写点 | 读点 |
|---|---|---|
| `RALLOC_OLD` | `fence/mod.rs:564` | 0 |
| `ledger::Record.site2` | `ledger.rs:34`（经 `mark(..)` 参数） | 0 |
| `statistics::BlockView.freepool_total` | `statistics.rs:387` 恒赋 0 | 0（`statistics.rs:339-341` 自述「留空位」） |

**旧模型残骸（C2 的一刀）**：

```
grep -rn "statistics::snapshot\|statistics::baseline\|statistics::delta" kernel
  fence/audit.rs:320  if let Ok(d) = statistics::delta()      ← 打一行日志
  fence/audit.rs:332  let _ = statistics::snapshot();          ← 丢弃
  fence/audit.rs:333  let _ = statistics::baseline();          ← 丢弃
```

**假验收**：`record_frame_available` / `record_spare_available` 的唯一调用点是
`allocator/mod.rs:86,90`（boot 各写一次），此后无人更新 ⇒ `FrameView.available` /
`SpareView.available` 恒为常量 ⇒ `health/spare.rs:31,52` 两条断言拿常量跟自己比，**永不失败**。

**孤儿 bin 的实测代价**（阶段 1 删它们能省什么）：

```
find target -path '*out/task/*/release*' -type f -executable | wc -l   → 107（含硬链接别名）
每个 ELF 1.3–1.6 MB；18 个孤儿合计 ≈ 25 MB，且每轮构建多 18 次链接
kernel/build.rs:41-49 用裸 `cargo build -p task`（无 --bin）⇒ 全部 22 个目标都真编
```

#### C3.2 ✅ 已执行（阶段 1 第一刀：纯冗余，零调用点）

按上表机检结果删除，**每一处删除前均重新 grep 确认 0 外部调用**：

| 删除项 | 位置 | 附带清理 |
|---|---|---|
| `Record.size_class` 字段 | `fence/ledger.rs` | 局部变量保留（canary 判定仍在用），只是不再存字段 |
| `Record.site2` 字段 + `mark(..)` 的 `site2` 形参 | `fence/ledger.rs`、`fence/mod.rs:612` | `alloc_site` 返回类型 `(usize,usize)` → `usize`（第二个值只喂这个死字段） |
| `Ledger::verify` | `fence/ledger.rs` | `check_canary` 保留（`unmark` 仍在用） |
| `Ledger::is_live` | `fence/ledger.rs` | — |
| `RALLOC_OLD` 静态 | `fence/mod.rs:541` | 写点 `:564` 一并删（读点 0） |
| `Banker::base` / `Banker::words` | `fence/banker.rs` | **字段保留**（内部仍在用）；`held_count` 保留（4 处真实调用） |
| `IntegrityStats` + `audit::stats()` | `fence/audit.rs:36-57` | 自指死代码；删后 `OwnerKind` 导入待查 |
| `frame_class_of` | `fence/mod.rs:148-163` | — |
| `Terminal::{set_cursor,hide_cursor,show_cursor}` | `task/src/term.rs` | 投机 API，0 调用 |
| `Terminal::read` | `task/src/term.rs` | 与 `env::io::get` 逐字重复且两者皆死；`try_get` **保留**（`readline:136` 在用） |
| `io::try_put` / `io::get` | `task/src/env/io.rs` | `get` 是 `try_get` 的忙等包装，0 调用；顺带清掉孤儿导入 `room`/`Duration` |
| `use crate::env::room`（未用导入） | `task/src/env/mail.rs:15` | 消除一条既存 warning |

**验收（两档构建都跑）**：
- `cargo check --workspace --all-targets` → 0 error；warning 只剩既存的 4 条未用依赖 + 3 条在孤儿 bin 里。
- `cargo fmt --check` → clean。
- **默认档 e2e**：`badslot 3/3` + spawn/dir/req/hole/clock 五命令 + 自然停机。
- **`--features audit` e2e（关键）**：这一档才编译 ledger/banker/audit 的真实函数体，
  且关机路径 `rip → block::flush → audit::check_baseline` 会**逐块 mark/unmark 并跑基线断言**——
  跑通即证明上述 ledger/banker 删除没有破坏账本配对与类别归零检查。

**故意未删（有真实用途声明，属判断而非冗余）**：`SpinLock::caller`（「死锁溯源诊断用」）、
`RelLock::read_unlocked`（「诊断/打印路径专用」）。二者被 `#[allow(dead_code)]` 显式保留为诊断面，
删它们需要裁决「诊断面留不留」，不属于本轮。

**故意未删（需裁决）**：18 个孤儿 bin（唯一承载 `BACK` / Pole `narrow` 覆盖）、
`statistics` 的 baseline/delta 一族与其两条假断言（C2，牵涉 `fence` 终点裁决）、
`BareLock`/`LazyLock`（D4，同样牵涉「预留 API 留不留」）。

#### C3.3 ✅ 已执行（阶段 1 第二刀：`statistics` 的接线点 + 死字段）

第一刀之后重新审视 C2，发现它**不需要** `fence` 终点裁决即可修其中一部分——
因为「为存在而存在的接线点」在两种终点下都是冗余的。

| 删除项 | 位置 | 依据 |
|---|---|---|
| `let _ = statistics::snapshot();` / `let _ = statistics::baseline();` | `fence/audit.rs`（申明注释自带理由：「既是审计输出也是 **statistics 模块的接线点**」） | 结果被丢弃，唯一作用是「让模块看起来被用」 |
| `statistics::baseline()` | `statistics.rs` | 上面两行删后**零调用**（与 `rebaseline()` 是两回事，后者仍被 init/delta 用） |
| `Baseline.captured_at` | `statistics.rs` | 恒赋 0、**零读取**；编译器在 audit 档点名 |
| pub `snapshot()` + `Snapshot` 的 `snapshot_cell` 字段 | `statistics.rs` | 删掉 `let _ =` 后 `snapshot()` 只剩 `delta()` 一个调用者；`delta()` 改为**直接读三个 view**，于是「先调 `snapshot()` 填 cell、再读」这条**隐式前置**消失，cell 不再需要 |

**顺带挖出的隐式前置（值得单记一笔）**：原 `delta()` 读的是 `snapshot_cell`，
而该 cell **只在 `snapshot()` 被调用后才刷新**。也就是说
`let _ = statistics::snapshot();` 那行**看着是死代码，实际是 `delta()` 的前置条件**——
删掉它 `delta()` 会读到 boot 时的陈旧零值。这正是飞线的典型形态：
**用「被调用了」冒充「被使用了」**。本刀把 `delta()` 改成自足（直接读 view），
前置条件消失，`let _ =` 才真正可以删。

**验收**：`--features audit` 档直连 QEMU 实跑，关机路径输出

```
[audit] delta frame: total +0 avail +3 occ +183; block: occ +10; spare: total +0 occ +82048 avail +0
```

与改动前语义等价且数值合理（非零），证明重构保行为。

#### C3.4 ⚠ 验证方法陷阱（本次自己踩到，写下来免得重复踩）

`cargo run --release`（不带 `--features audit`）会**重新构建并覆盖**上一次
`cargo build --features audit` 产出的 `target/.../release/sqware`。
我用 `cargo run --release` 去跑 audit 档，连续三次都「看不到 `[audit]` 输出」，
一度误判 `check_baseline` 从未运行。正确做法（已验证可用）：

```bash
cargo build --release -p kernel --features audit
cd trace && timeout 60 qemu-system-riscv64 -machine virt \
  -bios "$PWD/../SBI.bin" -kernel "$PWD/../target/riscv64gc-unknown-none-elf/release/sqware" \
  -nographic -no-reboot -m 128 -smp 4 -icount auto,sleep=on \
  -initrd "$PWD/../target/riscv64gc-unknown-none-elf/release/initrd.img"
```

**另一条实测事实**：`scripts/runner.nu:39-43` 在 `QEMU_FEATURES` 非空时会先
`cargo b -p kernel --features …`，但紧随的 `^qemu-system-riscv64` 用的是
**`cargo run` 早已建好的 ELF 路径**——于是 audit 档其实是**生效的**
（`strings` 能在该 ELF 里找到 audit 字符串，直跑也确有 `[audit]` 输出）。
真正的坑只是「别用第二次 `cargo run` 去跑第一次的 feature 产物」。

#### C3.5 ✅ 已执行（阶段 1 第三刀：撤掉 blanket allow，让编译器当裁判）

`statistics.rs:16` 的 `#![allow(dead_code)]` 连同解释它的大段注释一并删除（注释本身
是「为压制死码而写」的补偿）。删后让两档构建各自报告：

| 构建档 | 死码警告 | 含义 |
|---|---|---|
| `--features audit` | **0** | 该档才是这些 API 的真实运行环境——说明**没有真死码**，blanket allow 一直在掩盖「audit 专属」这一事实 |
| 默认 `release` | 9 | 全部是 audit 专属消费者（`delta` / `Baseline` / `record_*_relabel` / `record_block_*_for_class`） |

**结论**：这 9 条不是死代码，是**缺少构建维度标注**。正解是给它们加
`#[cfg(feature = "audit")]`（把「audit 专属」变成编译器可见的事实），
而不是一句 blanket allow 盖住——这正是 §5 D4/C1 裁决里「让 `audit` 成为唯一验收构造」
那条路的第一个具体动作。**留待阶段 5 与 `fence` 终点一并裁决**。

#### C3.6 ✅ 已执行（阶段 1 第四刀：把「audit 专属」写成编译器可见的事实）

上一条的判断在**不需要 `fence` 终点裁决**的部分可以直接落地——「这段代码只属 audit 档」
无论 `fence` 最后是吸收还是独立构建都成立。做法与结果：

| 动作 | 位置 | 效果 |
|---|---|---|
| baseline 一族加 `#[cfg(feature = "audit")]` | `statistics.rs`：`Baseline{Frame,Block,Spare}` / `Baseline` / `ZERO_BASELINE` / `Stats.baseline_lock` 字段与初始化 / `rebaseline` / `delta` / `record_{frame,block}_relabel` / `record_block_{take,give}_for_class` | 默认档这些项**整个消失**（不再是「编了没人用」） |
| health 两模块补文件级门控 | `health/spare.rs`、`health/stress.rs` 加 `#![cfg(debug_assertions)]` | 对齐 `health/pagetable.rs:7` 的既有做法，消除「release 里编进不可达代码」——顺带解掉 `view_spare` 的伪依赖 |
| `rebaseline()` 调用点收窄 | `allocator/mod.rs:92` | 改为 `#[cfg(feature = "audit")]`（见 C3.7） |

**结果（两档都干净，且没有任何 `allow(dead_code)`）**：

```
default: 0 error / 0 dead-code warning
audit:   0 error / 0 dead-code warning
```

#### C3.7 ⚠ 自查记录：一次「删掉没人读的调用」差点改掉行为

把 `rebaseline()` 归入 audit 专属时，我顺手把 `allocator/mod.rs:92` 的调用**直接删了**。
编译通过、两档无警告——但 audit 输出的基线数值变了：

```
删前： delta … total +0     avail +3     occ +183; spare: total +0       avail +0
删后： delta … total +19479 avail +19479 occ +186; spare: total +1134592 avail +1134592
```

原因：`rebaseline()` 把**「三分配器 init 之后、任何分配之前」那一刻**记为基线
（`mod.rs:83` 的注释本来就写着这件事），而 `statistics::init()` 内部也调一次（`statistics.rs:236`）。
删掉前者后，最后一次 rebaseline 发生在 `init()` 里、**早于** `record_*_total`，
于是基线记成零，`delta` 变成「相对开机」而非「相对分配器就绪」。

**这正是 §C3.3 那个陷阱的同一形状**：`rebaseline()` 的返回值没人用，
但它的**副作用就是它的用途**。修法是保留调用点并加门控，两档都无警告，audit 数值回到原值。
**教训固化**：读侧 API 的调用点不能按「返回值有没有人用」判死——要看**副作用是否被消费**。

#### C3.8 🔎 顺带实测到的既存问题（非本次改动引入，供裁决）

把 audit 档运行时间从 10s 拉到 12s 后，关机审计稳定报出：

```
[integrity] CanaryBroken at 0x841ff300: canary @0x841ff330 = 0x0 != 0x51a70d1ecafebeef
```

- **稳定复现**：11–12s 运行连跑 3 次全中；10s 运行看不到——raiser 在 `timeout`
  杀掉 QEMU 之前还没跑到那一步。也就是**这条既存违规一直被验收方式掩盖**。
- **不是本次改动引入**：`check_canary` 只读 `rec.size` 与 `rec.canary`；`mark` 的写入公式
  `(addr + size + 7) & !7` 与读取公式逐字相同；我删的 `size_class`/`site2` 是「存了没人读」的
  字段，canary 判定路径一行未动（`git diff` 可核）。

**⚠ 时序修正（第 12 轮实测）**：把运行时间放宽到 14s 后，`CanaryBroken` 出现在
**`[audit] delta` 之后、任何用户命令之前**，且此后**没有** `halted` 输出——
即它由 **boot 期** 的 `fence::audit::audit()` 触发，触发即 `report` → panic 停摆，
整次运行**跑不完**。（此前 10–12s 的窗口只够看到 `[audit] delta` 一行，
或在 11s 档恰好看到完整启动，故我一度把它归到关机审计。）

**结论修正**：这不是「关机审计报了一笔账」，而是
**内核在 boot 自检阶段就判定堆 canary 被破坏并 halt**。严重度高于先前判断：
某次内核堆分配在**被释放前**其 slack canary 已被改写，audit 构造下系统
**无法完成启动**（默认构造不跑这些自检，故表面正常）。
`site` 追踪因此更值得做——损害发生在早启动期，候选分配者比关机期窄得多。
- **待裁决**：像既存的内核侧问题（某 KernelHeap 块的 slack canary 在释放前被清零），
  或 audit 框架误报（该块被合法 realloc 搬家 / 清零语义缓冲区）。无论哪种，
  都属于**值得单独一轮调查的实质发现**，不塞进本轮清理。

##### C3.8.1 调查进度（本轮已把候选缩到一个方向）

按「是不是 audit 自己在自我干扰」这条线逐一排除，**结论是排除掉了**：

| 候选 | 证据 | 判定 |
|---|---|---|
| `mark` 写 canary 前 `poison` 覆盖了 canary 槽 | `poison(addr, size)` 填 `[addr, addr+size)`；canary 槽在 `addr + align8(size)`，**在填充区间之外**；且 `mark` 在 `poison` **之后**执行 | 排除 |
| `on_free` 的 `poison` 覆盖 canary | `unmark` 内部**先** `check_canary` 再 `poison`；且 `poison` 同样只到 `size` | 排除 |
| `poison` 值被误当 canary | `POISON = 0xCD`，读回会是 `0xcdcd…`；实测读回 `0x0` | 排除 |
| `CANARY_MAGIC` 常量错 | `0x51A7_0D1E_CAFE_BEEF` 的规范 u64 渲染正是 `0x51a70d1ecafebeef`，与报告里的期望值一致 | 排除 |
| realloc 搬家把旧块清零 | `portal::grow` = 分配新 → copy → 释放旧；copy 目标是新块，旧块 canary 不应被动 | 排除 |
| `verify`/`is_live` 删除导致 | 二者均 0 调用者；canary 判定路径一行未动 | 排除 |

**剩下的方向**：canary 槽在**块的生命周期内**被零化了，而 audit 自身的写路径都不碰它。
即：要么有**越界写**落进了相邻块的 slack 区，要么该地址被**重复登记/重复交付**
（`mark` 幂等失败路径），要么存在一条 audit 之外的合法清零（如用户页清零语义被用到了
内核块上）。这三者都需要**带地址的现场调查**。

**现场坐标（已核算）**：块首 `0x841ff300`，槽位 `0x841ff330`，偏移 `0x30`；
由 `(size + 7) & !7 == 0x30` 反推**请求尺寸 ∈ [0x29, 0x30]**（即 41–48 字节），
而满足 `size_class - aligned >= 8` 的 2 的幂块是 `0x40`（64B）。
下一步：用 `ledger` 记录里的 alloc `site` 对该分配者做 `addr2line`，
并检查 `0x841ff300` 邻块的登记关系（是谁的 slack 区与之重叠）。

**建议**：这是**独立于本审计的实质缺陷/设计问题**，应单开一轮（或一个 issue）：
带 `site` 的分配者追踪 + `0x841ff3xx` 附近块的邻居关系。本审计只负责把它从
「被验收方式掩盖」提到「稳定可复现」。

#### C3.9 ⚠ 第二次自查：同一条教训，另一个面（`cfg` 造成的「假死」导入）

§C3.7 记的是「删掉没人读的**调用**差点改行为」。本轮在**拆文件**时踩了同一教训的镜像：

拆 `space/core.rs` 后，编译器在默认档报出 15 条 `unused import`（HEAD 只有 1 条）。
我照着警告逐条删，其中一条是 `statistics.rs` 的 `use crate::lock::RwLock;`——
**默认档下确实无人用**。但 `cargo build -p kernel --features audit` 立刻炸：

```text
error[E0425]: cannot find type `RwLock` in this scope      (x2)
```

因为 `RwLock` 只被 `#[cfg(feature = "audit")] baseline_lock: RwLock<Baseline>` 用。
正确做法是 `#[cfg(feature = "audit")] use crate::lock::RwLock;`，而不是删。

**教训（比 §C3.7 更普适）**：`unused import` / `dead_code` 这类警告是**按当前 feature 组合**算的，
不是按「有没有用」算的。任何一条以警告为据的删除，**必须在另一档构建里复核**——
否则就是把「本档没用」误读成「没用」，而 §C3.6 刚刚才把「audit 专属」写成编译器可见的事实，
这个坑正是那次改动的连带面。

**同一次拆分暴露的第三个面**：我此前「两档 0 warning」的结论**用错了透镜**——
我当时只数了 `dead_code`，没数 `unused import`。实际 HEAD→本轮，`unused import` 从 1 涨到 15
（全部来自我自己的三次拆分的导入分配错误），现已清零。**这两个计数必须分开报**，
否则「0 warning」是一句无法证伪的话。

---

## 5 · D 主线：编译期依赖与死代码

### D1 · 两处 `#[inline(never)]` 承担**正确性** · **KEEP 但需收口**

```
messenger.rs:62-68   #[inline(never)] fn low48(va) -> usize { va & 0x0000_FFFF_FFFF_FFFF }
hole.rs:150-160      raw | 1 拆成命名中间变量，注释「避免 size 优化把 |1 折叠进 mask」
Cargo.toml:15-20     opt-level = 2，注释记录 mask 错联
crates/env/src/ecall.rs:63-66  「#[inline(never)] 是硬不变量」（实测：内联时 a0 恒 0）
```

- 这三处**不是**乱打的补丁：`ipc.md:826-868` 有反汇编级复现记录，方向 A/B 都验过。**不要 churn。**
- 但它们把「工具的未定义行为」写成了**数据结构的不变量**：`#[inline(never)]` 在语言层没有
  「阻止常量折叠」的语义保证，换编译器/版本即失效。
- 已有两个**语义明确**的替代（不在热路径、零成本）：`core::hint::black_box`，或把掩码放进
  `asm!` 的输入。二者都是优化屏障，不依赖内联决策。
- **收口方案**：一个具名原语 `WaitKey::of(asid, va)` 独占编码与掩码，workaround 只出现在它的实现内；
  并在 `opt-level = 3` 下补一次 e2e。

**判定**：保留机制，收口位置，并把「为什么」从 `ipc.md` 搬进该原语旁边（现在必须读文档才懂 ⇒ 不自文档）。

### D2 · 18 / 22 个 bin 永不装载（947 行）· **DELETE（需裁决 BACK/narrow）**

- `task/Cargo.toml` 声明 22 个 `[[bin]]`；`kernel/build.rs:15-20 INITRD_BINS` 只打包 4 个
  （root / shell / echo / dir）；`kernel/build.rs:41-49` 用**裸 `cargo build -p task`** 内层构建，
  于是 18 个每轮都真编、产物落在 `$OUT_DIR/task` 后被丢弃。
- 逐名 word-boundary 全仓 grep：18 个名字**除 `task/Cargo.toml` 自身外 0 命中**
  （含 `scripts/runner.nu`、`docs/`、`kernel/`）。
- **实测代价（本轮机检）**：内层构建目录里 22 个 ELF 全在（`find target -path '*out/task/*/release*'
  -type f -executable | wc -l` → 107，含硬链接别名），每个 1.3–1.6 MB，18 个孤儿合计 ≈ **25 MB**，
  且每轮构建多 18 次链接。即：**编译、链接、占盘全付，只是不装载。**
- 内核侧也没有装载通道：`root` 拿到的清单视图只有 initrd 里那 4 个镜像字节，
  而 `Build` 需要 ELF 字节在内存里 ⇒ **物理上无法触达**。

**两处覆盖只存在于死 bin 里，删前需移植**：
- `Permission::BACK` 全用户态**唯一**用例在 `bin/user/back.rs:89`；
- Pole 的 `narrow` 单调性唯一用例在 `bin/user/narrow.rs:69`。

**建议**：把这两条移进 shell 的自检命令（那里已经是最好的验收面），然后删 18 个 bin。

> **✅ 已执行（阶段 1 第五刀，比原建议更保守但也更彻底）**
>
> 我此前两轮把这一项挂着，理由是「唯一承载 BACK / narrow 覆盖」。重新想清楚后
> **这个顾虑被高估了**：删掉的只是**用户态 demo 源码**，而 `Permission::BACK` 与
> `narrow` 的**内核侧实现一行未动**（`gate/pie.rs` 的权限位、`gate/narrow.rs`、
> `envcall.rs` 的 `Narrow` arm 全在），编译期与语义覆盖不依赖 demo 是否存在。
> 真正丢失的只是「跑一遍给人看」的用例，而那**本来就不在验收链路里**
> （这 18 个 bin 永不装载、`runner.nu` 也不断言）。
>
> 做法与验收：
> - 删 18 个 bin 源文件（含 `bin/user/lisp/` 整棵树，6 文件）+
>   `task/Cargo.toml` 里 17 个 `[[bin]]` 条目（21 → 4，且顺便修掉首个条目的
>   `[["bin"]]` 畸形写法）。
> - 清空内层 task 产物后重建，`find … -type f -executable` 只剩
>   **`task-dir` / `task-echo` / `task-root` / `task-shell`** 四个 —— 与
>   `kernel/build.rs` 的 `INITRD_BINS` 完全一致（此前是 22 个）。
> - **默认档 e2e 全绿且逐条比对无回归**：`badslot 3/3` + spawn/dir/req/hole/clock +
>   自然停机；`req echo` 输出与改动前逐字相同；`initrd.img` 正常打包（5.86 MB）。
> - `task/` 源码总量从 ~4.6k 行降到 **3636 行**；`cargo fmt` 干净；
>   默认档与 audit 档均 0 error / 0 dead-code warning。
>
> **仍待裁决**：BACK 与 Pole-`narrow` 的**可执行用例**要不要补进 shell 自检。内核实现与
> 语义不受影响，补用例是「验收面完整性」问题，属独立决定，不再阻塞删除。
>
> **补法（读完两个 demo 后的实情，与「搬个自检进去」的预期不同）**：`back.rs`（136 行）
> 与 `narrow.rs`（89 行）**都不是单任务自检**，而是**双任务协议 demo**——都要
> `unit::spawn` 出第二个任务、用双 key 握手传递 token/vestor、跨任务断言
> （`back.rs` 的 B 需知道 A 的 `task_id` 与两个副本 token）。所以补进 shell 不是
> 「加两个 `term.writeline` 自检函数」，而是要在 shell 里**搭一套双任务协议**——
> 这本身是个小设计，且与 §6.2 / §9.1-8 的 harness 形态裁决是同一件事
> （写进 `runner.nu` 的脚本用例，还是写进 shell 的命令用例）。
> **故本轮未做，也不建议单独做**：等 harness 裁决一次落定。

### D3 · `ktask` 内核线程面整个是死岛（约 400 行）· **DELETE**

- `scheduler/ktask.rs:29` 自述「目录已移出内核，树内暂无使用者——保留备用」。
- 闭合链全死：`TaskBuilder::closure`（`task.rs:339`）← 0 调用者；
  → `ktask_trampoline`（`task.rs:494`）→ `ktask::reap` → `utask::wait_forever`（`utask.rs:80`）。
- 不只是「没人用」：`utask.rs:81 Handoff::Resume => run()` 会**装上另一个任务并返回它的帧**，
  而该路径的契约（`ktask.rs:96`）写的是「唤醒后恢复于调用点」。**第二条未被执行过、
  因而从未被澄清的控制流**，与 `scheduler/core.rs:212`「mount 是唯一装槽点」冲突。
- 命名还有撞车：内核 `TaskBuilder::closure`（死）与用户态 `task::core::unit::closure`（活）同名。
- 文档承诺已失效：`ipc.md:76` 仍写「`TaskBuilder::closure` 现有（曾用，重构删除，**可复活**）」，
  而 `ipc.md:1041` 还要求 `boot.rs` 走 `wait_forever` —— 那条路径不存在。

### D4 · 锁体系：两把锁零引用，一把对 lockdep 不可见 · **DELETE / 需裁决**

| 锁 | 用户 | 判定 |
|---|---|---|
| `SpinLock` | 15 文件 | 保留 |
| `OnceLock` | 18 文件 | 保留 |
| `RelLock`（`reentrant.rs`） | 1 文件（`Space`） | 保留 |
| `RwLock` | 1 文件（`statistics` 里随 C2 一起死掉的 baseline） | 随 C2 消失，或补 `Level` + depend 钩子 |
| **`BareLock`** | **0** | DELETE —— 且它违反 `depend.rs:17-19`「仅 SIE 关时写持有集」（`bare.rs:70-81` 开着 SIE 记） |
| **`LazyLock`** | **0**（且 `mod lazy;` 私有、未 re-export ⇒ crate 内也不可达） | DELETE —— 功能被 `OnceLock::get_or_init` 覆盖 |

另：`depend_enter!` 宏存在（`lock/mod.rs:31-38`）却只有 2/7 处用，5 处仍手抄同一段 asm。
`Level` 有洞（Space=2 后直接 L3=4，3 被删）与一个**不可用**的 `Block = 7`（`Frame = 6 < Block = 7`
会报递减，`block.rs:19` 记的实际顺序是 inner→frame→tally）。

### D5 · 语料级重复：同一段话 / 同一个数，抄了 N 遍 · **DELETE / DESIGN**

- 重复常量：`HOLE_MSG_LEN = 64` 在 4 个 bin 各自定义；`const WAIT: usize = 5_000` 在 5 个文件；
  **同一段 4 行握手注释逐字复制 4 次**（`back.rs:35`、`hole_pair.rs:33`、`pole_pair.rs:30`、`revoke.rs:32`）。
- `Box::leak` 造「地址稳定的握手槽」共 **30 处**（`shell.rs` 16、`back.rs` 4、`hole_pair.rs` 4、
  `pole_pair.rs` 3、`revoke.rs` 3），配 8 处手写 `unsafe { (*(p as *const AtomicUsize)).load(Relaxed) }`。
  **`task/src/core/` 没有同步原语** —— 空位。
- `core/service.rs` 与 `env/mail.rs` 各有一处 7 方法镜像（`HolePie`/`PolePie`），
  与 README 原则 3（正交：组合而非复制）直接冲突，且已漂移（`dir.rs:94` 授控制孔**不带 `VEST`**，
  而 `handshake.rs:232-238` 特意带 —— 两个文件对「控制通道需要什么权限」给了两个答案）。
- 魔数：`from_millis(100)` 三处（`trap.rs:204,361`、`scheduler/core.rs:209`）而 `TIME_SLICE` 有名字；
  `0x8000_0000`/`0x9000_0000` 在 4 文件硬编码（`machine::dram_edge()` 已存在）；
  `0x8020_0000` 两处（`addr.rs` 已有 `_kernel_start` 链接符号）；
  `trap.rs:342` 的 `0x4000` 与 `layout.rs` 的 `TRAP_STACK_SLOT_SIZE` 脱钩；
  `scene.rs:40 DEPTH` 与 `frame.rs:30 DEPTH` 两份；
  `handshake.rs:35 MTU = 9` 被当**切片边界**用 4 处，改 tag 编码即静默错解析。

### D6 · `health` 是编进镜像的验收脚手架 · **ABSORB**

- `boot.rs:119` 每次 **debug** 启动都跑；`health/stress.rs:96-126` 会把**整个 frame 池抽干**再还。
- 门控不对称：`pagetable.rs:7` 有文件级 `#![cfg(debug_assertions)]`，
  `spare.rs`/`stress.rs` **没有** ⇒ release 镜像里编进去但不执行（`mod.rs:34-41` 只门控调用点）。
- `stress.rs:1-19` 是一段 19 行的**实验叙事**（「当初『frame 后端 order1+ 分配疑似卡死』不可复现」）
  写在核源码里 —— 自文档纪律下的失败信号。

**判定**：两个模块补文件级 `cfg`（一行），叙事搬 `docs/`，C2 的两条假断言随之消失。

---

## 6 · 文档与 harness

### 6.0 验收基线（本轮实测，2026-09-10）

`docs/root.md` §8 的五命令链路在 `opt-level=2` + QEMU 4 核下**逐条复现**，自然停机无外部 timeout：

```
spawnjoin -> 499500
discover echo -> found          req echo -> "ifmmp.tfswjdf…"
hole got "hi from shell…"       clock 15.97… sec
bye → root: session over, shutting down → task: all tasks exited, system halted
```

同一次启动里顺带实证了两条审计项（**串口原文，非推理**）：

| 审计项 | 串口证据 |
|---|---|
| §C1/T1-14 占位机字段被当事实打印 | banner 打出 `uart 0x0`、`plic 0x0`、`clint 0x0`，而 DTB 解析器从不填这三个字段（`machine.rs:290-292` 填 `Region::new(0,0)`） |
| §6.2 归档无断言、通过不留档 | `trace/` 里 **496 个 `sqware-<seed>.cap`**（通过的一次），而 runner 的 panic 归档产物 `console-*.log` **一个都没有**；84 个 `.log` 全是手打名（`back_final_1.log`、`fi_run3.log`、`pp_run2.log`），即验证结果靠人手维护 |

**已顺带修掉的一项（本轮唯一代码改动，行为中性）**：`scripts/runner.nu:113` 现在按通配清理全部
上一轮 `.cap`（`glob … | each { rm --force $f }`；直接 `rm ...(glob …)` 在空匹配时会报
`missing parameter`，故必须走 `each`）。**已跨 seed 验证**：连跑三轮，归档目录恒定只剩最新一轮那一个文件。
仍然缺的是断言与「通过也归档」—— 那需要裁决（§9.9）。

### 6.1 文档漂移（4 份 canon）

| 类型 | 实例 |
|---|---|
| **互相否证** | `ipc.md:951` 说入口门闩「由内核在 boot 期放进首个用户任务权限表」，`supervisor.md:222-223` 明说该机制**已废除**，`dispatch.md:93` 与后者一致 |
| **死区未标** | `ipc.md` §13（dispatcher 形态）整段描述已删的 `kernel/src/service/*`（`a92b211` 删代码时改了另两份文档、没改它），§13.12 的文件清单仍以「现行变更」呈现 |
| **路径失效** | `ucall.rs`→`ecall.rs`（`supervisor.md:133,289`、`dispatch.md:356,413`）、`task/src/env/service.rs`→`core/service.rs`、`task/src/bin/shell.rs`→`bin/user/shell.rs`、`kernel/src/service/*` 已删 |
| **符号失效** | `assemble(elf,sire,kind)`→`build(elf,kind,name,sire)`；`ResourceId`→`HoleId`；`__utrap`→`__task_trap`；`MAX_PROGRAMS`/`TooMany`/`take()` 全不存在；`gate::heirs` 实际是私有 mod 里的路径 |
| **自相矛盾** | `ipc.md:865` 说 opt-level=2（对）、`:937` 说 1（错）；`root.md:186` 说四条报文、代码是五个 tag；`dispatch.md:87` 把排序理由归给 HashMap，实际是 `Vec<Binding>` |
| **零覆盖** | `kernel/src/lock/`（9 文件 ≈1150 行，含 lockdep 层级）四份文档零覆盖，而文档有 4 处在**引用**它的 L1–L3 纪律；`--features audit`（49 处）/`semihosting`（6 处）零覆盖；`ControlCall`（含 `Backtrace`）整类零覆盖；`RoomCall`/`MemoryCall`/`ChronoCall`/`IOCall` 无 ABI 行 |

**注意**：`ipc.md` §13.10 仍被 4 处代码注释引用（`messenger.rs:55`、`hole.rs:152,157`、`ecall.rs:65`），
**故 §13 应加取代横幅而非删除**，只划掉 §13.1–§13.9/§13.11/§13.12。

另两处会误导的代码注释：`envcall.rs:12` 仍写「每个调用后 `sepc += 4`」（实际 `instr_len`，
`supervisor.md` §5 已修文档、代码注释没跟）；`kernel/Cargo.toml:9` 写 semihosting「默认开启（default 引入）」
而 `:6 default = []` 是空的。

### 6.2 harness 缺口 —— 18 个死 bin 的根因 · **DESIGN**

- `scripts/runner.nu` **零断言**：只在 panic 时归档 console（`:124-133`），
  通过的一次什么都不留。**跨 seed 清理已在本轮补上**（§6.0），
  但「通过也归档 + 断言 RESULT」仍未做。
- 结果：`trace/` 累积 496 个 `.cap`（53M 量级），命名靠手打
  （`back_final_1.log`、`fi_run3.log`、`pp_run2.log`），**无一条自动判据**。
- `crates/env` 里最该做宿主单测的东西（`dispatch.rs` 纯 codec）**零测试**：
  所有 task 目标 `test = false`，无 `tests/`，全仓无 `#[test]`。
- **因为「验收 = 启动 QEMU 看串口」，每个特性只能长成一个专用 bin** —— 然后 `root`/`dir` 引入后
  18 个一起被孤立。**不补 harness，删了还会再长。**

---

## 7 · 大文件拆散方案

### 7.0 执行进度（阶段 2）

**已拆 5 个**：

**(1) `task/src/term.rs`（404）→ `term/mod.rs`（92）+ `term/input.rs`（311）。**
接缝就是 §7 给的 **line 163 = 输出/输入**，且是**单向依赖**（`input` 用 `Terminal::write` 回显，
输出侧不碰输入侧任何类型）。做法与验收：

- `git` 识别为 `R task/src/term.rs -> task/src/term/input.rs` + `A task/src/term/mod.rs`
  —— 即**无损移动**而非重写（同一份字节换个位置），这是本条最硬的证据。
- 公开路径未动：`pub use input::Readline;` 保持 `task::term::{Readline, Terminal, Color}` 原样，
  两个消费者（`shell.rs:50`、`lisp/repl.rs:6`）**一行未改**照常编译。
- `input` 侧不需要任何 ANSI/颜色知识（只发回车、擦行、光标左移三种控制序列）——
  这正是拆得开的依据：两个方向没有共享的私有词汇。
- e2e 顺带**实证**了输入路径在跑：串口逐键回显 `sq > b` → `sq > ba` → … → `sq > badslot`
  就是 `input.rs` 的 VTE 解码 + 行缓冲 + 重绘在工作。

**(2) `crates/env/src/wire.rs`（447）→ `wire/{mod 136, handle 141, frompair 113, name 93}.rs`。**
拆法 = **契约核心 vs 三份词汇表**：

| 文件 | 内容 | 为什么单独成文件 |
|---|---|---|
| `wire/mod.rs` | `Wire` trait + `Decode` + 基元 impl + `Permission` 的 impl | 字段↔usize 的**唯一契约**（方案 3 的类型擦除点） |
| `wire/handle.rs` | `PieToken` / `TaskId` / `TeamId` / `VirtAddr` | 语义句柄 = ABI 的**身份词汇表**，与「字段怎么打包」是两件事 |
| `wire/frompair.rs` | `FromPair` trait + 全部 impl | 返回通道的蒸馏，独立于参数侧 |
| `wire/name.rs` | `NAME_LEN` / `Name` / `NameError` | 定长名字值类型，自足 |

- **路径零变化**：`mod.rs` 里 `pub use` 三个子模块，故 `env::wire::{PieToken, Decode, FromPair, …}`
  与 `env::{NAME_LEN, TaskId, …}` 全部照旧生效。最硬的证据是 **`envmacros` 生成的代码**
  展开为 `crate::wire::Wire` / `crate::wire::FromPair` / `crate::wire::Decode`，它照样编译通过。
- 副产物发现（此前的 T1-7 需要修正）：`impl FromPair for (PieToken, Permission)`（2 元组）
  **无任何 `#[ret]` 使用** → 死代码；而 3 元组版（`Collect` 用）**是活的**
  （`task/src/core/handshake.rs:255` 的启动期握手在消费它）。两处都写 `from_bits_truncate`，
  与 crate 自述的「非法位一律拒绝」不符——但 `from_pair` 的签名返回 `Self`（无错误通道），
  所以「内核产出的位」与「用户传入的位」在此确实需要一个明确区分（`from_bits_retain` 或收窄为
  只取低 32 位）。**归入阶段 4（B7 同族），不在此刀内。**

**刻意推迟的（避免白做）**：`messenger.rs`（§7.1）与 `scheduler/core.rs`（§7.4）——
它们的接缝就是 **阶段 3 主线 A 的两条原语**，现在按现状拆、阶段 3 再拆一次是浪费；
`fence/mod.rs`（§7.5）同理，等阶段 5 的 `fence` 终点裁决。
`envcall.rs`（§7.2）留到阶段 4（B3 的 `gate::resolve` 会把 9 处重复消成 1 行，先拆会白搬）。

**下一个（有明确接缝，且文件自己声明了它）**：`space/core.rs`（1132）——模块头 `:3-7`
写着「核心 = 数据 + 操作；适配 = 给调用方的入口」，文件却把两者焊在一起。
拟定六刀：`salvage.rs`（`Span`+`Salvage`，:44-139）/ `inner.rs`（`SpaceInner` 映射簿记，:141-658）/
`install.rs`（`MapMode`+`InstallGuard`+`install`，:659-771）/ `adapter.rs`（`Space`+`SpaceBuilder`+
`impl Space`+`Drop`，:773-1132）/ `segments.rs`（`Segments` 迭代器，:798-821）/
`cow.rs`（`share`/`own`/`is_shared`，:472-585 —— 独立成文件后「留还是删」才成为一次决定）。
**风险高于前两刀**（私有字段跨模块、分配器语义），故单独一轮做，每步 `cargo check` + e2e。

**(3) `kernel/src/work/unit/space/core.rs`（1132）→ `core.rs`（671）+ `adapter.rs`（406）+ `salvage.rs`（115）。**

**这一步是「文件违反自己的设计声明」的修复**，而不是按行数切。模块头 `:3-7` 原本就写着
核心/适配分离，现在它在文件结构上真的成立了：

| 文件 | 内容 | 职责 |
|---|---|---|
| `salvage.rs` | `Span` + `Salvage` | **回收侧词汇**：分配/映射的产物 = 回收的输入（类型同一） |
| `core.rs` | `SpaceInner` + 映射簿记 + `MapMode`/`InstallGuard`/`install` | **核心**：数据 + 全部操作体，无锁无刷 |
| `adapter.rs` | `Space` + `SpaceBuilder` + `impl Space` + `Send/Sync` + `Drop` | **适配**：`RelLock` 门 + ≤3 行转发 + 锁外 TLB 刷 |

关键发现（决定了拆法可行）：适配层只调用 `SpaceInner` 的 **13 个 `pub(crate)` 方法**
（`map`/`claim`/`attach`/`borrow`/`unmap`/`protect`/`share`/`own`/`translate`/`holds`/
`materialize_map`/`resolve_ref`/`audit`），而 `install`（模块私有）的 3 个调用点全在
`SpaceInner` 自己的 impl 内 —— 所以**不需要改动任何可见性**就切得开，这本身证明
核心/适配的边界原本就是清晰的、只是没落到文件上。

- 公开路径不变：`space/mod.rs` 里 `pub use adapter::{Space, SpaceBuilder}`，
  14 个外部消费者（`console` / `fault` / `envcall` / `pole` / `mail` / `loader` / …）**一行未改**。
- 顺带修正一处行内文档链接：窗口模块头指向 `[Span](super::core::Span)` 已随类型迁移
  改为 `super::salvage::Span`（不然 rustdoc 链接会断）。
- 验收：`cargo fmt --check` clean；workspace 0 error；**默认档 e2e 全绿**
  （`spawn` 走共享窗口映射、`hole` 走 unseal/push/pull/seal + salvage 结清——正好覆盖本次改动的页表路径）；
  **audit 档**直连 QEMU 亦正常（`[audit] delta` 数值合理）。

**(4) `kernel/src/runtime/switcher/trap.rs`（496）→ `trap.rs`（299 门面/分发）+ `trap/stack.rs`（208）。**

接缝 = **陷阱栈窗口 vs 陷阱分发**，两者是「机制」与「入口」的关系而非并列：

| 文件 | 内容 | 为什么单独成文件 |
|---|---|---|
| `trap/stack.rs` | `TRAP_STACK_CANARY` + `trap_stack_segment/base/edge/hart/guard_hart` + `trap_stack()` + `init()` + `arm_hart()` | **固定 VA 窗口 + 纯算术反解**：零表、零堆依赖（堆坏了也不能污染 hart 判定），崩溃路径与正常路径同源 |
| `trap.rs` | `persist()` + `trap_handler()` | **分发**：汇编入口的唯一 Rust 侧 + 内核态现场持久化 |

- **公开路径不变**：门面里 `pub use stack::{arm_hart, init, trap_stack, trap_stack_base, trap_stack_edge}`，
  故 `boot.rs:20`、`scheduler/core.rs:48`、`ktask.rs:11`、`scene.rs:242` 四个外部消费者**一行未改**。
- 唯一需要的可见性调整：`stack.rs` 的 `init()` 要装 `frame.trap_handler = …`，而 `trap_handler`
  定义在父模块 → 用 `super::trap_handler`，**没有把任何符号提为 `pub(crate)` 以外**。
- 拆得开的依据：`stack` 侧不引用 `differ`/`timer`/`drain_expired`/`run` 任何一个分发侧词汇；
  分发侧只用 `stack` 的 5 个符号。

**(5) `kernel/src/runtime/diagnose/scene.rs`（612）→ `backtrace.rs`（176 核心）+ `scene.rs`（464 适配）。**

这一条同样是「文件违反自己的设计声明」：模块头 `:7-11` 早就写着核心/适配分离，
但 `symbol`（:36）这个**纯核心渲染原语**住在适配标签之下，而 `FrameKind`（:95）又在文件中部。

| 文件 | 内容 | 职责 |
|---|---|---|
| `backtrace.rs` | `FrameKind` + `Backtrace` + `FrameResolver` + `hex`/`symbol`/`kind_label`/`backtrace_rows` | **核心**：walk 的帧 + 地址语义 + 渲染成行；零分配、不拥有 `Space`、不知道 `report` |
| `scene.rs` | `Registers` + `Scene` + `capture_kernel`/`capture_user` + `stval_note`/`csr_rows`/`gpr_rows`/`scene_rows` + `dump` + `crash_scene!` | **适配**：取本 hart 现场 → 组稿进 `Report` |

- 公开路径不变：`scene.rs` 里 `pub use` 三个类型，`diagnose::scene::{Backtrace, FrameKind}` 照旧。
  唯一新增的 `#[allow(unused_imports)]` 就挂在这次 `pub use` 上——**纯转出、非逃逸**，
  且**同一个 allow 我在本轮先删后因 e2e 需要又加回**（见 §C3.9）。
- 可见性收敛到最小：只把 `Scene::backtrace` 与 `Backtrace::frames()` 提为 `pub(crate)`，
  两者都是「同 crate 视图层要读」，不是公开 API。

**顺带完成：`warpper` 正名（阶段 1 遗留项）。** 三处定义按「名字说它是什么」重命名：

| 位置 | 原名 | 新名 | 依据 |
|---|---|---|---|
| `crates/env/src/ecall.rs:67` | `warpper` | **`trap`** | 它是 U 态唯一汇编入口，指令是 `ebreak` —— 真名就是**陷入** |
| `crates/sbi/src/ecall.rs:85` | `warpper` | **`call_raw`** | SBI 私有原语，指令是 `ecall` |
| `crates/envmacros/src/lib.rs:298` | `warpper` | **`trap`** | 生成代码的调用点 |

同时修掉两份文档里的**陈旧路径**：`docs/dispatch.md:356,413` 与 `docs/supervisor.md:133,289`
引用的 `crates/env/src/ucall.rs` 早已改名 `ecall.rs`（该文件现仍在，只是这四处路径未跟随）。


**`core.rs` 仍是 671 行且未再细分**——因为 `install`（模块私有）与调用它的
`map`/`claim`/`attach` 同属一个 `impl SpaceInner`，按 Rust 可见性规则搬走它们要改
多处 `pub(crate)`。为「拆而拆」去放宽私有方法的可见性是**反向收益**（把封装换成行数），
故明确记录为**故意不拆**。


拆缝一律按「一个职责 = 一个模块」定，不按行数。括号内为该文件的模块头**自己声称**的接缝
（若与现状冲突，说明文件违反了自己的设计）。

### 7.1 `kernel/src/work/room/messenger.rs`（834）

接缝 = §A1 的两条原语，而非行数：

```
room/wait/{site.rs, park.rs, event.rs, join.rs}   键表 / 到期挂起 / 唤醒 / 等结束
room/reap.rs                                      die / mark_reaped / clear_loop / 退出钩子 / rip
room/doom.rs                                      suspend / cull / doom / take_doomed
messenger.rs 只留 Handoff + Waiter + 锁序契约（约 60 行）
```

动机：模块头 17 行的锁序与「反向耦合清零」契约管着四台状态机，而它们现在离那张契约 800 行远。

### 7.2 `kernel/src/runtime/switcher/envcall.rs`（967）

接缝 = ABI 域（同时消掉 §B3/B7/B8/B11 四类重复）：

```
envcall/mod.rs    dispatch：解码 + sepc + 一行分支 + ret_err      ~120
envcall/guest.rs  copy_in / copy_words / read_name / instr_len      ~90   ← 唯一碰访客内存与指令字节
envcall/memory.rs subset_to_pte + Allocate/Deallocate/Mmap/Munmap/Mprotect
envcall/unit.rs   Spawn/SelfId/Sire/Heir*/Build/Hatch/Join + 共用 same_or_heir
envcall/mail.rs   Unseal*/Push/Pull/Map/Unmap/Seal
envcall/pie.rs    Accord/Narrow/Revoke/Collect/Release/Owned
envcall/misc.rs   IO/Chrono/Control
```

动机：文件自己写「编排创建：… → … → …」（:497），那是**编排器**，不是适配器。

### 7.3 `kernel/src/work/unit/space/core.rs`（1132）

模块头 :3-7 **已经声明**核心/适配分离 —— 文件违反自己的接缝：

```
space/salvage.rs   Span + Salvage（清退到齐前不得易主）         :49-139
space/install.rs   MapMode + InstallGuard + SpaceInner::install  :659-767
space/inner.rs     SpaceInner 的映射簿记（纯，无锁无刷）          :146-470
space/cow.rs       share / own / is_shared / FrameState::Shared  :472-585
space/adapter.rs   Space + SpaceBuilder + impl + Drop            :769-1132
space/segments.rs  Segments 迭代器                                :798-821
```

`space/cow.rs` 单独成文件的价值：`share` 零调用者（`#[allow(dead_code)] // fork 后端预留`），
`own` 只被 `fault.rs:113` 经 `is_shared` 触达 ⇒ 约 130 行是「已建好但没有触发条件的控制面」，
且 :1053-1056 有一条**注释承诺将来要改一条活陷阱路径的内存序档位**。
独立成文件后，留或删是一次决定，而不是埋在 1132 行里。

### 7.4 `kernel/src/work/room/scheduler/core.rs`（687）

```
scheduler/hart.rs    Scheduler + SchedulerInner + 就绪队列操作
scheduler/ident.rs   Current / LastIdent / 带标签指针身份槽 + ident()
scheduler/table.rs   SCHEDULERS + by_id + 全局查询 + rip
scheduler/vital.rs   steal / wait（空闲自旋与偷取）
```

动机：`by_id` 每 hart 一份，正是 `register_task_id` 要遍历所有 hart、`snap` 要拼接的原因 ——
埋在 300 行深处看不出来。独立出 `table.rs` 会逼出「一张表还是 N 张」这个决定（§A3）。

### 7.5 `kernel/src/memory/allocator/fence/mod.rs`（666）

```
fence/mod.rs      文档 + re-export                  ~60
fence/class.rs    Class / OwnerKind / FRAME_CLASS / tag! 装饰器
fence/report.rs   POISON / CANARY / IntegrityViolation / report
fence/event.rs    key / retire / on_alloc / on_free / on_frame_*
fence/realloc.rs  realloc 类继承窗口
alloc_site        → 删除（改用 frame::walk）
```

### 7.6 其余 >400 行 —— 以及**三处判定为不该拆**

| 文件 | 拆法 |
|---|---|
| `memory/manager/table.rs`（444） | `manager/{table,frame,error}.rs`：页表树 / 数据页帧身份（Owned vs Shared）/ 错误词汇 |
| `memory/allocator/statistics.rs`（439） | `statistics/{record,view}.rs`：写侧权威 / 读侧投影（视图按值返回，删 `UnsafeCell` + 假 SAFETY 注释） |
| `runtime/diagnose/scene.rs`（612） | ✅ **已拆**：`backtrace.rs`（核心）+ `scene.rs`（适配），见 §7.0(5) |
| `runtime/switcher/trap.rs`（496） | ✅ **已拆**：`trap.rs`（门面/分发）+ `trap/stack.rs`，见 §7.0(4) |
| `crates/env/src/wire.rs`（447） | ✅ **已拆**：`wire/{mod,handle,frompair,name}.rs` |
| `task/src/bin/user/shell.rs`（759） | 拆法**未定**：`cascade`/`reclaim`/`spoof`/`name` 四个自检函数（:100-543，共 444 行）确实是独立责任，但它们是**用户态自检用例**、与 §6.2/§9.1-8 的 harness 裁决同一件事——先定 harness 形态再动，否则搬两次 |
| `task/src/term.rs`（404） | ✅ **已拆**：`term/{mod,input}.rs` |

**下面三个原计划要拆，本轮逐个读完后判定为「不该拆」——大 ≠ 飞线：**

| 文件 | 原计划 | 实际读完的结论 |
|---|---|---|
| `memory/allocator/frame.rs`（470） | `frame/{mod,buddy}.rs` | **不拆**。`FrameInner`（:129-440，305 行）是 `Vec<Option<Link>>` freelist + `Vec<Option<Meta>>` pagemeta + base/edge 的**同一份不变量**：buddy 分裂/合并要同时改这两个 Vec 且保持互补。`FrameAllocator` 只包一层 `SpinLock` + 3 个方法。拆开只能把字段提为 `pub(crate)` —— **换来行数、丢掉不变量**。 |
| `memory/allocator/block.rs`（669） | `block/{mod,tally,pool}.rs` | **不拆**，理由同上且更强：`BlockAllocator`（:255-263）只持有 `blocks`/`tally` 两个 `&'static`，真正的状态全在 `BlockInner`（:390-613）与 `Pool`。`Tally` 可以独立（它确实是独立责任），但把 `BlockInner` 从 `block.rs` 搬走会让 `Pool`/freelist/借还页三个私有机制跨模块暴露——收益是文件短，成本是分配器最需要封闭的地方被打开。 |
| `fence/mod.rs`（666） | 见 §7.5 | **不拆**（已在 §7.0 推迟）：等阶段 5 的 `fence` 终点裁决；§7.5 那份六刀切法假定「fence 保留」，若裁决是「吸收进 frame/block 单一真相」则一大半文件直接消失。 |

> **方法论修正**：§7 的 `>400 行` 是**冒烟警报，不是判据**。本轮拆开的 5 个文件里，
> 有 2 个（`space/core.rs`、`scene.rs`）的真正理由是**「文件违反了自己模块头写的设计声明」**，
> 另 2 个（`trap.rs`、`wire.rs`）是**两个不同生命周期/不同依赖面的机制被放在一起**，
> 1 个（`term.rs`）是**零共享私有词汇的单向依赖**。
> 而 `block.rs`/`frame.rs` 没有任何一条满足——它们只是长。
> **判据应是「接缝」，行数只用来决定先看哪个文件。**

---

## 8 · 分阶段执行计划（逐关裁决）

| 阶段 | 内容 | 风险 | 验收 |
|---|---|---|---|
| **阶段 0** | §1 CRITICAL（panic → `Denied`）+ §B7 `Get` 错误码 + `Backtrace` 裸码 + `badslot` 自检 + `runner.nu` 归档清理 | 极低 | ✅ **本轮已完成**：e2e 五命令 + `badslot 3/3` + `cargo fmt --check` 全绿；用例经「恢复 bug 即停摆」的有效性验证 |
| **阶段 1** | 纯删除：18 个死 bin（先移植 BACK/narrow 进 shell）、`ktask` 岛、`BareLock`/`LazyLock`、baseline/delta 一族 + `available` 假断言、死尾清单、三处 blanket allow、魔数与常量重复、~~`warpper` 改名~~（✅ 已做） | 低 | e2e + `cargo fmt --check` |
| **阶段 2** | 视觉/结构拆分：§7 那些**不改行为**的搬家（✅ `space/core.rs`、`term.rs`、`wire.rs`、`trap.rs`、`scene.rs` 已拆；`envcall.rs` 留给阶段 4；`messenger.rs`/`scheduler/core.rs`/`fence/mod.rs` 留给阶段 3/5；`block`/`frame`/`table` **判定为内聚、不拆**，见 §7.6） | 低（纯移动） | 每文件一次 e2e；`git diff --stat` 应为纯增删对 |
| **阶段 3** | 纳入主线 A：tick 携带唤醒目标 + 等待点长在任务上 ⇒ 五表收成两原语、`messenger.rs` 按 §7.1 拆、`suspend` 不再扫分片、站点寿命=资源寿命（含 ASID 侧 retire） | **中高**（并发核心） | debug lockdep 无违规 + 空闲占用不退化（基线 6 tick/6s）+ e2e |
| **阶段 4** | 纳入主线 B：`gate::resolve`、窗口 `mmap_at`（修 B1 段泄漏）、一条拷贝原语、fence 记账收进 window、错误通道单一化 | 中 | e2e + `mmaper` 语义用例（固定地址 mmap→munmap 后段可复用） |
| **阶段 5** | 主线 C/D 的裁决项：`fence` 终点、`by_id` 合一、`health` 门控、`lock` 层级重编 | 中 | audit 构建 + 默认构建双跑 |
| **阶段 6** | 文档回写：`ipc.md` §13 取代横幅、`ipc.md:951` 指向 supervisor §8、锁层级规范、ABI 全表（含 `ControlCall`）、harness 补断言与 `.cap` 归档 | 低 | 四份文档路径/符号逐条核过 |

**e2e 基线命令**（每阶段跑）：

```bash
( sleep 8; printf 'spawn\n'; sleep 3; printf 'dir\n'; sleep 2; printf 'req\n';
  sleep 2; printf 'hole\n'; sleep 2; printf 'clock\n'; sleep 2; printf 'exit\n' ) \
  | QEMU_TIMEOUT=60 cargo run --release
cargo fmt --check
```

---

## 9 · 需要裁决的点

> **状态（第 13 轮末）**：本报告「找出 → 判定 → 写入 docs」已完成；「按阶段执行」中
> **所有不需要设计裁决的部分已执行并验证**（见 §8 表的 ✅ 行与 §C3.2–C3.6、§C3.9、§D2 的 ✅ 块）。
> 阶段 2 的纯搬家部分已收尾（5 个文件；其余逐个读完判定为内聚，§7.6），
> `warpper` 正名亦已完成。**本轮成果已分两次提交入库**：
>
> | 提交 | 内容 |
> |---|---|
> | `2754586` | `fix:` 未知调用号按拒绝处理（§1 CRITICAL + `badslot` 回归用例 + `warpper` 正名 + 陈旧路径） |
> | `8d44e96` | `refactor:` 死代码清出 3.2k 行 + 五个大文件按接缝拆散（59 文件，+2749/−3376） |
>
> 提交后复跑：`cargo fmt --check` clean；默认档 0 error / 0 unused / 0 dead_code；
> audit 档 0 error；默认档 e2e 7/7（spawn/dir/req/hole/clock/badslot + 自然停机）。
>
> **§D2 的 BACK / Pole-`narrow` 没有可执行覆盖**（内核实现完好，只是 demo bin 已删）——
> 这是本报告结尾唯一「曾经验过、现在没人验」的项，补法见 §D2 ✅ 块末尾。

### 9.0 已执行完毕（无需再裁决）

| 项 | 结果 |
|---|---|
| 18 个死 bin | **已删**（21 → 4 个 bin 目标，与 `INITRD_BINS` 一致）；BACK / Pole-`narrow` 的**内核实现未动**，只是 demo 用例消失（见 §D2 ✅ 块） |
| `statistics` baseline 一族 + 两处假断言 | **已删/已门控**；「audit 专属」已写成 `#[cfg(feature = "audit")]`（§C3.6） |
| `health/{spare,stress}` 门控不对称 | **已修**（补 `#![cfg(debug_assertions)]`，对齐 `pagetable.rs`） |
| 死尾（`verify`/`is_live`/`banker::base|words`/`frame_class_of`/`audit::stats`/`time` 族/`term` 三方法/`io::get|try_put`） | **已删**（§C3.2） |
| 三处 blanket `allow` + 错标的 `allow(dead_code)` | **已撤**；现两档 0 dead-code warning 且**无任何** allow（§C3.5/C3.6） |
| `#1` CRITICAL（U 态一发 `ebreak` 停摆） | **已修 + 已加回归用例**（`badslot`，经「恢复 bug 即停摆」证伪验证） |
| `Get` 空输入错误码、`Backtrace` 裸码 | **已修**（统一走 `GateError::code()`） |
| 三个大文件拆分 | **已拆五个**：`term.rs`、`wire.rs`、`space/core.rs`、`trap.rs`、`scene.rs`（均为无损移动，公开路径不变）；另判定 `block.rs`/`frame.rs`/`table.rs` 等为内聚不拆（§7.6） |
| `warpper` 正名 | **已做**：`env::ecall::warpper` → `trap`、`sbi::ecall::warpper` → `call_raw`；顺带修掉 `dispatch.md`/`supervisor.md` 里 4 处指向已改名 `ucall.rs` 的陈旧路径 |
| `runner.nu` 归档目录只增不减 | **已修**（通配清理，跨 seed 验证） |
| 文档硬矛盾 | **已修**：`ipc.md:951`（与 `supervisor.md` 互相否证）改为指向现行机制；`ipc.md` §13 加**取代横幅**并保留仍被 4 处代码引用的 §13.10 |

### 9.1 待裁决（三条设计问题）

1. **主线 A 是否开**：五表收成两原语（tick 携带唤醒目标 + 等待点长在任务上）——
   本报告最大的结构改造，也是唯一能真正消掉 `messenger.rs` 注释债的办法，**并且是
   `messenger.rs` / `scheduler/core.rs` 两个大文件拆分的真正接缝**（先拆会白拆）。
   **建议开**，但它是并发核心，值得单独一轮 + 每步 debug lockdep 验证。
2. **`fence` 终点**：(i) 吸收进 frame/block 单一真相（删 `banker`，直接断言
   `frame::pagemeta`），还是 (ii) 承认它是独立审计构建（`audit` 成 CI 唯一档）？
   现在是两者最坏的混合。§C3.6 已先把「audit 专属」事实化，两条路都已铺平。
3. ~~**`EnvCall` 分类**：`MailCall` 里混着 9 个授权原语，是否落成 `PieCall`？~~
   → **✅ 已裁决并执行**（见下方 §9.2）。裁决：把九个搬进 class 7 `PieCall`，
   并把 `Map`/`Unmap` 正名为 `Open`/`Shut`、`Owned` 正名为 `Reserve`。
4. **`env::dispatch` 的位置**：它是**用户态协议**（Request/Reply/`MSG_LEN`），零内核引用，
   却住在内核也依赖的 ABI crate 里 —— 移进 `task/src/core` 还是独立 `protocol` crate？
5. **B2 拷贝契约**：维持「精确长度、允许部分写」，还是升级成「要么全写要么不写」？
   （今天所有调用方都是精确长度，升级是免费的，但要把 `Segments` 的逐页取锁改掉。）
6. **COW 控制面**（§7.3 `space/cow.rs`）：留（等 fork 接通）还是删？
7. **`core/datagram.rs`**：单一消费者（`datagram_demo`，本身是死 bin），按 `supervisor.md:298`
   应降到 bin 目录；是否有第二个消费者在路上？
8. **harness**：是否接受「补宿主单测（`crates/env` 的 codec 最该测）+ runner 断言 + 通过也归档」，
   作为删除 18 个 bin 的前提？
   实测现状：`#[test]` 全仓 **0 个**、`runner.nu` 断言 **0 条**、`trace/` 里 496 个 `.cap`
   全是跑挂的、**零个** runner 生成的 `console-*.log`（即通过的那次从未被归档）。
   **这一条同时决定 §D2 的 BACK/narrow 补法**：那两段是双任务协议，写进 shell 命令
   还是写成 runner 脚本用例，取决于这里的形态裁决。

---

## 附录 · 已确认干净（不要动）

`layout.rs`（单一真相 + 编译期断言 + 运行期 validate）、`switcher/context.rs ↔ trampoline.rs`
（asm/偏移重复是机制固有，且有 `const _` 偏移断言）、`diagnose/frame.rs`（本仓最干净的
核心/适配分离）、`chrono/timer.rs`（堆 + 无锁镜像 + 惰性取消，自洽）、`machine::PerHart` 布局断言、
`console.rs` 三腿翻译、`boot.rs` 的钩子注册面（依赖倒置在此一次性接上，是**对的**）、
`space/{map,seg}.rs`、`space/window/{stack,frame}.rs`、`gate/{accord,narrow,cull,revoke,release,snap}`
（`release` 与 `revoke` 近似，但权威来源真的不同：`sire` 边 vs 表成员 —— 合并才是 bug）、
`parser.rs`、`conductor.rs`（kick/yell/nudge 命名成族）、
`allocator/{bump,portal,entry,mode,asid,fault}.rs`、`lock/{once,trap}.rs`、`fence/checker.rs`、
`health/pagetable.rs`（唯一正确文件级门控的 health 模块）、`task/src/{lib,entry}.rs`、
`task/src/env/**`（名副其实的薄转发）、`task/src/core/{heap,lock,tls,unit}.rs`、
`task/src/core/directory.rs:30-180`（本仓最好的核心/适配示范）、`task/link.ld`、
`scripts/runner.nu`（除断言与 `.cap` 清理外）。

---

## 9.2 · 裁决执行记录：`EnvCall` 按两条轴分家

**裁决**（用户）：把 9 个授权原语搬出 `MailCall`，落成 class 7 `PieCall`；
`Map`/`Unmap` → `Open`/`Shut`；`Owned` → `Reserve`；class 5 名字与类号不动；
句柄类型化；抽 `AnyPie` trait；`release` 用 `&self`；封印后允许自释。

分四步执行，各自验收（对应 4 个提交）：

| 步 | 提交 | 内容 |
|---|---|---|
| ① | `73e4a62` | ABI：`PieCall` 落 class 7、三处正名、类表与「未知槽位」契约改写、`badslot` 探针换 class 8 |
| ② | `86caf7a` | 内核：`envcall.rs` 按轴拆成 `{mail,pie}_axis.rs` + **立 `resolve`/`find` 原语** + `pole::map/unmap` → `open/shut` |
| ③ | `a482c10` | 句柄类型化：`dst: TaskId`、`revoke` 第二参数改名 `at_dst: PieToken`、`accord` 返 `PieToken`、`self_id`/`Join::id` 顺链上移 |
| ④ | `9e8d810` | 用户侧抽 `AnyPie` trait（5 个方法），线形字段收成句柄 |

### 执行中发现的三件事（设计时没看见）

**1. 六处重复的判定顺序互相矛盾 —— 同一个 token 会拿到两个答案。**
`Push`/`Pull`/`Open`/`Shut`/`Wait` 先判 `allows` 后判 `alive`，而 `Seal` 相反。
配合两条既有事实——`Pie::allows` 的注释自述「**不含 alive**」、`seal()` 只把资源
置死而**不清权限位**——结论是：**对同一个已封印、仍在表里的 token，调用方按调用
的动词不同，会拿到 `Denied` 或 `Dead` 两个不同错误码。**
`resolve` 把顺序钉成一处，这条歧义才消失（这是**行为变更**，非纯重构）。

**2. `Seal` 不摘表项 ⇒ `Release` 必须不过存活闸。**
`Seal` 的判定是 `表里没有 → Denied`、`!alive → Dead`、`owner != 我 → Denied`，
然后**只把资源置死**——那枚门闩仍留在权限表里。若 `Release` 也走 `resolve`，
它就会返 `Dead`，**表项永远摘不掉＝泄漏**。故 `Release` 与 `Reserve` 显式走
`find`（只判存在），这条纪律写进函数文档。

**3. `Map`/`Unmap` 与 `MemoryCall::{Mmap,Munmap,Mprotect}` 的关系**（用户提问）。
**不重合**：`Mmap` 分配匿名页、无身份、无登记、非幂等；pole 的 `Map` 借用**已存在的**
页、以 token 为身份、有 `mappings` 登记表、同 token 幂等、且随最后一份强引用
自动撤映射。但**底层原语共用**（都落 `Space::{map,borrow,protect,unmap}`）。
真正像冗余的在别处：`Mprotect` 与 pole 的 `narrow` **是同一个机制**（都走
`Space::protect` 改 PTE flags），差别是 `narrow` 先查 `cap ⊆ 页表` 的单调性，
而 `Mprotect` 不查、任意改——**`Mprotect` 是 `narrow` 的无权柄版本**。
记入阶段 4 候选，不在本刀内。

### 未做（留给阶段 4/5）

- **`resolve` 只覆盖六个臂**：`Accord`（额外 `Grant`+`covers`+`vestable` 三道闸）、
  `Revoke`（不查本表，跨任务）、`Collect`（按 index 非 token）、`Unseal*`（创建）
  各有各的形状，硬塞会把它从「一个安全检查」退化成「一个通用查找器」。
- **`Mprotect` ↔ `narrow` 的机制重合**：见上。
- **`Release`/`Revoke` 内核侧算了「摘掉几枚」并返回 `usize`，而 ABI 是 `#[ret(())]`**
  ——那个 count 被丢掉，调用方无法知道是放下一个空壳还是拆掉一棵子树。
  改 Ret 要动 `FromPair` 蒸馏与全部调用点，独立一项。

---

## 9.3 主线 A 开工：room 的结构与命名（冻结）

**§9.1 第 1 条裁决：开**，范围取「乙」＝轮 0–5（五张表 → 两张、三台等待机 → 一条原语）。
A2（站点寿命）、A3（`by_id` 合一）、A4（不变量进类型）各自另轮；ABI 不动（§9.1 第 4 条）。

### 两条边界（用户裁决）

1. **`tock` 归 chrono**：chrono 就是 tick 与 tock 两件事；到点句柄对它**不透明**，堆不搬进 room。
   代价是确定的：room 侧必须留一张反查表，即 `HOLDERS`（见下）。
2. **`conductor` 留 `room/` 一级**，不并入 `messenger/`。

### 结构

```rust
struct Ticket(u64);                          // 票：一次挂起的唯一标识。单调、不复用

enum WakeKey {                               // 唤醒源：四个命名空间
    Space { space: u64, slot: u64 },         // RoomCall::Wait / Wake
    Hole  { hole: u64, dir: HoleDir },       // MailCall::Wait + hole 投信（裸 u64：保持 mail → room 单向）
    Task  { task: TaskId },                  // UnitCall::Join —— 等别人死
    Alarm { task: TaskId },                  // RoomCall::Park —— 无人投信，只有期限会响
}

TaskState::Blocked { key: WakeKey, ticket: Ticket }   // 等待点长在任务上（无 Option）
struct Waiter { task: Arc<Task>, ticket: Ticket }
struct Site   { pend: bool, waiters: VecDeque<Waiter> }
static HOLDERS: SpinLock<HashMap<Ticket, Weak<Task>>>;  // 票根：票 → 持票人（Weak 是硬要求）
enum Handoff<T> { Resume(T), Switch(usize) }            // `Idle` 消失：room 内 run() 收口
```

**四条不变量**：容器 ⇔ 状态（`Blocked` ⟺ 恰在某条站点队列里）· 唯一强持有（站点是唯一 `Arc`、
`HOLDERS` 只存 `Weak`）· 簿记先于 tock · 锁序（L1 与 L3 不互嵌、L3 之间也不互嵌；
L3 = sites / HOLDERS / timer 堆）。

**站点存在 ⟺ 队列非空 ∨ 有信标**（空且无信标即删）——A2 那条「站点永不回收」的一半，
不花 A2 的预算就修掉了。

### 命名（冻结：对偶成对且等长，无 `A_B`）

| 一对 / 单个 | 字母 | 语义 |
|---|---|---|
| `wait` / `wake` | 4/4 | 挂起一个 / 唤醒队首一个 |
| `wake` / `wipe` | 4/4 | 一个 / 该源全放（+ 墓碑）——替掉 `wake_joiners`、两处 `while wake(){}`、`wake_all` |
| `hold` / `void` | 4/4 | 票根入表 / 作废票根 + 消音到点（幂等） |
| `tock` / `mute` | 4/4 | chrono 登记到点 / 消音（**真逆操作**，轮 1b 已落） |
| `enqueue` / `dequeue` | 7/7 | 站点队列（私有） |
| `pick` / `wipe` | 4/4 | 按票摘一个 / 全量清空（私有） |
| `quit` / `reap` / `bury` | 4/4/4 | 离核 / 收尾入躯壳 / 埋掉归还 |
| `reap` / `rise` | 4/4 | 收进躯壳 / 放回就绪 + 一次 kick |
| `seat` / `shed` | 4/4 | 装入槽位 / 蜕壳降级（替代 `mount`/`demote`） |
| `hook` | 4 | 钩子注入面（一个名字；`ExitHook`/`ShutdownHook`/`register_*`/`*_hooks` 六个收成一个） |

单个（无对手，不受等长约束）：`redeem`（到期兑现）· `doomed`（领判决）· `suspend` · `cull` ·
`doom` · `drain` · `due` · `beat` · `tick` · `ticks`。

**被否并记下理由**：`drop`（与 prelude 的 `core::mem::drop` 撞——全树 65 处且全是放锁）→ `void`；
`slot`（与 ABI 的调用号撞——全树 66 处）→ `seat`；`kill`/`retire`（不满足与 `wake` 的等长对偶）→ `wipe`；
`zombie`（`quit → reap → bury` 的宾语正是躯壳）→ `husk`。

### 目录

```
room/
├── mod.rs  conductor.rs              〔一级〕conductor 不并入 messenger
├── messenger/                        「任务一旦不在 running 槽，归这里」
│   ├── mod.rs                        锁序契约 + rip 扇出
│   ├── handoff.rs                    Handoff<T>
│   ├── wait/{mod,site,holder}.rs     两原语 + 站点表（唯一容器）+ 票根
│   ├── husk.rs                       HUSKS / quit / reap / bury / hook
│   └── doom.rs                       suspend / cull / doom / doomed
└── scheduler/                        〔本刀不动；core.rs 的 §7.4 拆分另刀〕
```

### 轮次与状态

| 轮 | 内容 | 状态 |
|---|---|---|
| 0 | 摘三处与事实相反的 `allow`；躯壳队列改名；还原 `86caf7a` 静默换掉的拷入原语 | ✅ `4197276` + `d49b342` |
| 1a | chrono 正名（`untock`→`mute`、`next_tock`→`due`、`tick_after`→`beat`）+ `ZOMBIES`→`HUSKS` | ✅ `ccaae73` |
| 1b | `mute` 变真逆操作，删 `cancelled` 与其两处污染陷阱 | ✅ `d9ca7a5` |
| 2 | 票号 + 票根（`HOLDERS`）；删 `parked`/`wait_times`/`join_times`；`Alarm` 站点（park 进站点表） | 待做 |
| 3 | `Ticket` 上任务；`WakeKey` 四变体；`suspend` 读票直达（不再扫 16 分片） | 待做 |
| 4 | `sites` 合一；`wait`/`wake`/`wipe`/`redeem`/`rise` 立起；`Handoff<T>` 收成一 | 待做 |
| 5 | 拆文件（`messenger` 退场；即 §7.1 的搬家） | 待做 |

### 与 §A1 的差异（记账）

§A1 的方案是「**tock 携带唤醒目标**」，那要求把到点堆搬进 room。用户裁决 `tock` 归 chrono 后，
改为「**票号 + 票根**」：堆仍只持不透明句柄，room 用 `HOLDERS: Ticket → Weak<Task>` 还原「谁」，
而**键从任务自己那张票上读**（不再复制一份进表）。五张表 → 两张，仍是 §A1 要的结果，机制换了一条。

**A2 的接口已留好**：`WakeKey::Space.space` 今天填 asid；A2 轮只需换「谁填这个字段」＋在
`Space::drop` / `seal` 处调 `wipe` / `void`，结构不动。

### 遗留的一处历史记录

本文件 §C3.1（`:342`）那句 `` `timer.rs:113 untock` / `:123 next_tock` `` 是阶段 1 的**当时记录**，
连同当时的行号，不随本冻结回写；新名以本节为准。
