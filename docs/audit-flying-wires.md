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

### D3 · `ktask` 内核线程面整个是死岛（约 400 行）· **DELETE** — ✅ 已执行（见 §10.8）

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
space/cow.rs       ~~share / own / is_shared / FrameState::Shared~~ → **已删**（§10.20）
space/adapter.rs   Space + SpaceBuilder + impl + Drop            :769-1132
space/segments.rs  Segments 迭代器                                :798-821
```

`space/cow.rs` 单独成文件的价值：`share` 零调用者（`#[allow(dead_code)] // fork 后端预留`），
`own` 只被 `fault.rs:113` 经 `is_shared` 触达 ⇒ 约 130 行是「已建好但没有触发条件的控制面」，
且 :1053-1056 有一条**注释承诺将来要改一条活陷阱路径的内存序档位**。
独立成文件后，留或删是一次决定，而不是埋在 1132 行里。

> **✅ 已裁决（§10.20）**：整片删掉——`share` / `own` / `is_shared` / `FrameState::Shared` /
> `Kind::Cow` / `fault.rs` 的 COW 分支全部移除；`FrameState` 随之消失，`Map.frames` 直接持
> `Frame`。理由是"共享在这台机器上只有只读借用一个形态"，写共享是被放弃的特性。

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

**e2e 基线命令**（每阶段跑；**已换成脚本门，见 §9.3**）：

```bash
scripts/examine.sh          # 自退 + 无 panic + 九个 marker，连跑 3 次要求 3/3
cargo fmt --check
```

> 旧写法 `( sleep 8; printf 'spawn\n'; … ) | QEMU_TIMEOUT=60 cargo run --release` 已废：
> 它靠人眼比对 marker，超时被杀与正常结束在脚本层面同形，且**没有一条命令会触发 `redeem`**。
> 历史基线只到 `clock`，缺 `sleep 300` 探针与 `badslot`。

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
4. ~~**`env::dispatch` 的位置**：它是**用户态协议**（Request/Reply/`MSG_LEN`），零内核引用，
   却住在内核也依赖的 ABI crate 里 —— 移进 `task/src/core` 还是独立 `protocol` crate？~~
   → **✅ 已裁决并执行：独立 `crates/protocol`**（§10.21；用户裁决"新建 protocol 目录"）。
5. ~~**B2 拷贝契约**：维持「精确长度、允许部分写」，还是升级成「要么全写要么不写」？~~
   → **✅ 已裁决并执行：升级成"要么全写，要么不写"**（§10.21；提交 `c919ced`）。
6. ~~**COW 控制面**（§7.3 `space/cow.rs`）：留（等 fork 接通）还是删？~~
   → **✅ 已裁决并执行：删**（§10.20；用户裁决"删"）。
7. ~~**`core/datagram.rs`**：单一消费者（`datagram_demo`，本身是死 bin），按 `supervisor.md:298`
   应降到 bin 目录；是否有第二个消费者在路上？~~
   → **✅ 已裁决并执行：删**（§10.21；实测消费者数 = 0，提交 `7502806`）。
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

**站点存在 ⟺ 队列非空 ∨ 有信标**（空且无信标即删，`prune` 在出队点收口）——A2 那条「站点永不回收」的
一半，不花 A2 的预算就修掉了（轮 2b 已落）。

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
│   ├── reap.rs                       HUSKS / quit / reap / bury / hook〔计划名 husk.rs〕
│   └── doom.rs                       suspend / cull / doom / doomed
└── scheduler/                        ✅ 已拆（§10.15）：核心四文件 + 两个入口面
    ├── mod.rs                        薄壳 + 术语表
    ├── core/mod.rs                   薄壳 + 重导出（`scheduler::core::X` 外部路径不变）
    ├── core/hart.rs                  Scheduler / SchedulerInner / 容器四改点 / seat / swap / starve / advance
    ├── core/ident.rs                 身份槽 `Badge` / `Identity` / `LastIdent` / `ident()`
    ├── core/table.rs                 SCHEDULERS / current / rip / launch / 名册 / 全机扫描
    ├── core/fetch.rs                 steal / wait / fetch
    ├── boot.rs                       入口面：init / idle
    └── trap.rs                       入口面：run（续跑 / 轮转 / 取活）
```

### 轮次与状态

| 轮 | 内容 | 状态 |
|---|---|---|
| 0 | 摘三处与事实相反的 `allow`；躯壳队列改名；还原 `86caf7a` 静默换掉的拷入原语 | ✅ `4197276` + `d49b342` |
| 1a | chrono 正名（`untock`→`mute`、`next_tock`→`due`、`tick_after`→`beat`）+ `ZOMBIES`→`HUSKS` | ✅ `ccaae73` |
| 1b | `mute` 变真逆操作，删 `cancelled` 与其两处污染陷阱 | ✅ `d9ca7a5` |
| 2a | 唤醒键成枚举（`WakeKey`）+ 两张站点表合一（`WaitSite`/`JoinSite` → `Site`） | ✅ `1eb7ca1` |
| 2b | 票号 + 票根（`HOLDERS`）；等待点上任务（`Blocked { key, ticket }`）；`Alarm` 站点；删 `parked`/`wait_times`/`join_times`；`wipe` / `redeem` 立起；站点寿命 `prune` | ✅ `fd15ffd` |
| 3 | `Ticket` 上任务；`WakeKey` 四变体；`suspend` 读票直达 —— 已在 2b 内一并落掉 | ✅ `fd15ffd` |
| 3b | 落点收成一个 `Handoff<T>`（`Joined`/`JoinStep`/`Waited` 退场，`Idle` 消失）；`rise` 收掉四遍唤醒尾巴 | ✅ `c250398` |
| 4 | `sites` 合一（2a）；`wait`/`wake`/`wipe`/`redeem` 立起（2b）；`Handoff<T>` + `rise`（3b） | ✅ |
| 4a | 冻结表余下的正名：`quit` / `reap` / `bury` / `hook` / `seat` / `shed` | ✅ `c7a8d9d` |
| 5a | 拆出 `messenger/{reap,doom}.rs`（纯移动，外部引用不改，`pub(crate) use` 重导出） | ✅ `f1fb458` |
| 5b | 再拆 `messenger/{handoff.rs, wait/{mod,site,holder}.rs}` | ✅ 落地（见下「轮 5b 落地」）；上轮那次回退是误判 |

### 与 §A1 的差异（记账）

§A1 的方案是「**tock 携带唤醒目标**」，那要求把到点堆搬进 room。用户裁决 `tock` 归 chrono 后，
改为「**票号 + 票根**」：堆仍只持不透明句柄，room 用 `HOLDERS: Ticket → Weak<Task>` 还原「谁」，
而**键从任务自己那张票上读**（不再复制一份进表）。五张表 → 两张，仍是 §A1 要的结果，机制换了一条。

**A2 的接口已留好**：`WakeKey::Space.space` 今天填 asid；A2 轮只需换「谁填这个字段」＋在
`Space::drop` / `seal` 处调 `wipe` / `void`，结构不动。

### 验收基线的缺口（本轮实测发现）

原七条 e2e 命令（spawn / dir / req / hole / clock / badslot / exit）**没有一条**会触发
`redeem`（到点兑现）——轮 2b 新写的「票号 → 票根 → 持票人的键 → 站点」路径当时
**从未被跑过**，7/7 全绿并不覆盖它。基线已加 `sleep 300` 探针（shell 的 `sleep <ms>`
命令，输出 `sleep 300ms` → `woke`），marker 增到 **9 条**。

但「9/9」本身**不是确定性判据**：同一份产物连跑 3 次实测只有 **1/3** 通过，且失败原因是
**控制台输入会丢**（见下节），不是被测行为回归。故验收门改由 `scripts/examine.sh` 承担，
并明确要求「自退 + 无 panic + 9 marker」且**连跑 3 次 3/3**。

同类的可疑覆盖缺口（未验，留待 harness 补断言那一项）：`wipe` 的两个调用方
（`clear_loop` 的目标回收、hole 的 `seal`/`drop`）与 `prune` 的空站点删除，都没有
可观测的断言——它们只在内核内部生效。

### 轮 5b 失败，与验收基线的三处缺陷

**轮 5b 已回退**：拆 `messenger/{handoff.rs, wait/{mod,site,holder}.rs}` 后编译与 audit 档
均 0 error、零新增 warning，但**运行时在 `spawn` 之后卡死**（提示符出现后任何输入不再产出，
QEMU 跑满超时被杀）⇒ `git checkout` 回退，`f1fb458`（5a）为当前绿状态。

回退后同一份二进制连跑两次 e2e：**8/9**（缺 `task: all tasks exited, system halted`）
与 **9/9**。同一命令、同一产物、结果不同 ⇒ **「e2e 9/9」不是确定性判据**——此前多轮都把它
当作语义变更的唯一裁判，这个用法不成立。

顺着这条线查 harness，实测确认三处缺陷（证据均在 `trace/`）：

1. **panic 判定是一句永不成立的匹配**。`scripts/runner.nu:133` 以
   `str contains '"kind":"halt"'` 判 panic；而导出的真实形状是**外部标签嵌套**：
   `{"h":0,"when":29715467,"kind":{"room":{"spawn":{"tid":1}}}}`，停机记录实为
   `"kind":{"halt":"halt"}` / `"kind":{"halt":"panic"}`。实测：8868 行导出里
   `"kind":"halt"` 出现 **0** 次；整个 `trace/` 里 `console-*.log` **0** 个
   ⇒ 该分支从未触发过，panic 现场从未归档。
2. **唯一证据在下一轮开跑时被删**。`runner.nu:115` 每轮起 QEMU 前清掉全部
   `sqware-*.cap`，而 cap 只在「判定为 panic」时才 `mv` 成 `console-*.log`（因缺陷 1
   永不发生）。**故 8/9 那轮的终端输出已被 9/9 那轮删除**——「挂起发生在哪一步」的唯一
   直接证据是这么丢的，不是没抓到。
3. **没有停机断言**。pass/fail 靠人眼比对九个 marker；`timeout` 的 124 与正常结束在脚本
   层面同形。且导出文件只在 semihosting 开启时才存在——`semihosting` **不在** `default`
   （与 `kernel/Cargo.toml` 注释所述相反，§9.0 已记），默认档连结构化流都没有，判定无从下手。

**处置结果（本轮已落）**：

- 判定不再做子串侥幸：`scripts/runner.nu` 的判据改成三个可观测量 —— **qemu 是否自行退出**
  （退出码 0）/ **是否被 host 侧 `timeout` 杀**（124）/ **是否 panic**（捕获里的 `[panic] at`）。
  起跑前不再删上一轮捕获（发现残留即归档为 `…-stale.log`）；**导出恒归档**、捕获只在判定通过
  时才丢；判定失败 ⇒ 非零退出。
- 新增 `scripts/examine.sh`：三轮判据（**自退** + 无 panic + 九个 marker）叠加，**连跑 3 次要求 3/3**，
  每轮独立目录留档。判据不依赖 semihosting（原因见下条）。
- 修 harness 时又逮到两处**同类**缺陷（都是「永不成立/永不执行」的检查）：
  - `nu` 脚本在**外部命令非零退出时当场中止整个脚本**（实测：其后语句、以及调用方其后的
    语句都不执行）。旧 runner 的归档步骤因此**在每次超时被杀时都不执行**——恰是最需要它的
    失败路径；`archive` 里两个分支从未真正跑过。
  - qemu 那行 `terminating on signal 15 … (/usr/bin/timeout)` **走 stderr**，而捕获只接了
    stdout ⇒ 若照此写判定，就是又一个永不成立的检查（本轮先写错、随即被 `examine.sh` 的独立
    grep 抓住，已改为读退出码 + `o+e>|` 合并 stderr）。
  - 结论性写法（实测过三种）：**裸 `try { ^cmd | tee {…} } catch { }`**——保留终端流、
    保留真实退出码、脚本继续；`do -i` 会继续执行但把退出码清成 0；不包则当场中止。
- **结构化导出不能当 e2e 判据**：`semihosting` + `-icount auto` 下，诊断事件流（每个 room
  park/wake/envcall 都写一条）经 ebreak 落宿主文件，实测 guest 虚拟时间被拖慢约 **650×**
  （60 s 墙钟只推进 **92 ms**、写了 **5245** 条事件），e2e 在超时前走不完。故停机判据取
  「自行退出（code 0）」——停机走 srst，qemu 自己退出正是这一档；被超时杀则必然不是。

### 真正的问题：控制台输入会丢（不是挂起）

回退 5b 后连跑两次得 8/9 与 9/9，当时记为「关机路径偶发挂起」。**这个判断是错的**，本轮按新判据
连跑 3 次后查清了：失败那两轮的**逐键回显停在半路**——run1 最后一条回显是 `sq > badslot`
（此后输入的 `exit` 从未回显），run3 最后一条是 `sq > req`（此后的 `hole` 从未回显）。

即：**guest 活着**（每轮都在正常打提示符与结果），只是**之后的输入不再进入行编辑器**。所以
`task: all tasks exited` 缺失不是「屏障没释放」，而是 `exit` 根本没被 shell 收到；据此写下的
`conductor::halt` / `HALT_ARRIVED` 猜测一并作废。

复现率（同一份产物、同一命令、连跑 3 次）：**1/3 通过**，且丢输入的位置不固定（一次在 `hole`
前、一次在 `exit` 前）。这也解释了 5b 那次失败——它在 `spawn` 之后停住，与「输入丢失」同形，
**5b 的失败大概率不是 5b 引入的**，回退建立在误判之上，待本轮排查出结论后重新评估。

**A/B 排查结果（否掉了上面那个判断）**：从 2a 之前的 `d9ca7a5` 向 HEAD 逐点连跑 3 次 ——
`d9ca7a5`（1b）**3/3**、`1eb7ca1`（2a）**3/3**、`fd15ffd`（2b）**3/3**、`c250398`（3b）**3/3**，
四点全绿；随后在**另一批**里测 HEAD 却是 **4/6**——那两次失败连**第一条**命令都没生效：
guest 已经到了提示符（`SQware shell` / `type 'help'` / `sq > ` 都在），回显却只有初始那一条，
8–24 s 之间发出的八条命令**一个字节都没进去**。

差别不在提交而在**批次**，机制是：qemu 跑在 `-icount auto,sleep=on` 下——虚拟时钟跟着墙钟走，
但 guest 的**工作量**推进速度取决于宿主负载；而 e2e 的输入日程按墙钟排（`sleep 8; spawn;
sleep 3; dir; …`）。宿主一忙，命令就在 guest 还没走到那一步时到达，被 UART 丢（FIFO 16 字节，
早期输入不排队）。**所以 1/3、4/6 与「输入丢失」都是门自身的不可靠，不是内核回归**；同理，
5b 那次失败（现象同形）不该算在 5b 头上。

**已定位并修掉（本轮）**：把同样的八条命令、同样的 FIFO、同样的时序分别交给两条路：

| 路径 | 结果 |
|---|---|
| **直连 qemu**（门自己起，输入由门持有的 FIFO 直接给 qemu） | 3 轮 **24/24** 步全部生效 |
| **走 runner 链**（cargo → nu → timeout → qemu，stdin 穿过 nu） | **1/3** 通过，且在随机一步之后输入再也不生效 |

⇒ **内核没有回归；丢输入发生在 runner 链上**（nu 会吃掉 stdin 上的字节）。旁证：`drive | cargo`
那种写法里，写端在下游还没持有读端时会被 **SIGPIPE 杀掉**，于是日程后半段的命令一条都写不出去——
症状正是「guest 正常停在提示符、marker 全缺」。之前两批（1/3 与 4/6）与 5b 那次失败都归到这里。

**门的处置**：`scripts/examine.sh` 不再走 `cargo run`，改为自己 `cargo build` + 直接起 qemu，
输入用自己持有的 FIFO（`exec 3>` 阻塞到 qemu 打开读端 ⇒ 与「qemu 就绪」天然同步），
**逐步 expect**：每步等它的输出出现再发下一条，单步独立超时、失败指到具体命令。实测
连跑 3 次 **3/3**、连跑 6 次 **5/6**——比改前的 1/3 好得多，且每次失败都指名道姓。

**残留：已定位到 `-icount`（本轮）**。加了失败诊断后，四份现场完全一致：门持有 FIFO 写端 ✓、
**qemu `fd0` 仍指向该 FIFO** ✓、`write.err` 全空（写根本没报错）✓、qemu 进程活着 ✓，
而 guest 在此之后再无输出 ⇒ **字节到了 qemu，是 guest 停止取输入**。先前「qemu 自己不再持读
stdin」的推断**作废**（那是把 `-nographic` 的 monitor 复用与投递混为一谈）。失败步不固定
（req / hole / clock / badslot / dir 都出现过），故不是某条命令的问题。

对照实验（同一道门、同一批命令，只换这一个开关）：

| 配置 | 结果 |
|---|---|
| `-icount auto,sleep=on`（原样） | 8/8、6/10（合计 **14/18**） |
| **去掉 `-icount`**（`EXAMINE_ICOUNT=`） | **20/20**（两批各 10 轮） |

`-icount` 是按宿主时间给 guest 计时并节流的开关；去掉后失步不再出现 ⇒ 指向
**qemu / OpenSBI 的控制台读路径与 icount 节流之间的相互作用**，不是内核行为
（未改动的干净基线同样会卡，见上）。处置：**门不再用 `-icount`**——它本是 runner 为
「同 seed 可复现」加的，验收门不需要；runner 保持原样（交互式 `cargo run` 仍要可复现）。
本轮开头那次「8/9、缺自然停机」的历史现象，至此有了同一个解释。

已知债：门里的 QEMU 参数与 `scripts/runner.nu` 那份**重复**了，改一处要改两处（runner 仍服务
交互式 `cargo run`，本轮已证明它的 stdin 链不适合当验收门）。

**试过一条替代路，不通（记账）**：`-display none -serial pty`（本意是让 guest 控制台完全不
经过进程 stdin，从根上消掉 EPIPE 那类问题）。qemu 正常报出从设备路径
（`char device redirected to /dev/pts/0 (label serial0)`），但门把从设备读干也只有 **0 字节**
——同一个 `cat` 换回 stdio 直连就有 4 KB 引导输出。故这条路要先弄清 qemu 的 pty chardev
何时才把 guest 输出写到从设备上，或改走 `-serial pipe:`／socket chardev。
门保持现状：**FIFO 直连 + 逐步 expect**（3/3、5/6）。

另修 runner 自身一处：FAIL 分支原写作 `$"FAIL(seed …)"`，`(` 紧跟文本让 nu 把 `FAIL(...)`
当命令调用 ⇒ **失败路径自己崩掉、诊断丢失**（语义仍是退码 1）。改形后实测 FAIL 会正常打印
判定与 qemu 退出码。

- **5b 重来的纪律**：纯移动的判据是「导出的名字在两边指向同一样东西」；一旦为搬迁而放宽
  `pub(crate)` 或改动 import 形状，就必须按语义变更来验（先 3× e2e，再谈记账）。

### 遗留的一处历史记录

本文件 §C3.1（`:342`）那句 `` `timer.rs:113 untock` / `:123 next_tock` `` 是阶段 1 的**当时记录**，
连同当时的行号，不随本冻结回写；新名以本节为准。

### 轮 5b 落地

结构（行数）：`mod.rs` 518→**71**（锁序契约头 + `mod`/重导出 + `rip` + 扑杀面说明——因
`reap.rs`/`doom.rs` 本轮不动，它不是字面意义的「只有契约 + rip」）· `handoff.rs` **16** ·
`wait/mod.rs` **263**（`block`/`rise`/`park`/`wait`/`target_dead`/`join`/`wipe`/`wake`/`redeem`）·
`wait/site.rs` **156**（`WakeKey`/`Site`/`Waiter` + 分片 + `take_beacon`/`prune`）·
`wait/holder.rs` **57**（`Ticket`/`holders`/`hold`/`void`）。`reap.rs`/`doom.rs` **零改动**
（`git diff --quiet` 通过）：`mod.rs` 留了一组私有 `use`，连 `doom.rs` 那句
`use super::{prune, sites, void};` 都一行没改。

**外部路径一行未变**：`mod.rs` 用 `pub(crate) use` 重导出（照 5a 写法，比被导出条目窄，
不触发 E0364）。

**可见性账（唯一可能改变行为的地方）**：放宽 7 条（`Ticket.0` / `Ticket::alloc` / `Ticket::raw` /
`hold` / `WakeKey::fold` / `Site::new` 私有→`pub(super)`；`holders` 私有→`pub(in super::super)`
——**没有一条到 `pub(crate)`**）；规格显式化而有效范围不变 7 条（`Site.pend` / `waiters` /
`Waiter` 及其两字段 / `SITE_SHARDS` / `shard_at`）；收紧 8 条（`void` / `sites` / `prune` / `Site`、
`take_beacon`、`SITE_SHARDS_MASK` / `site_shard`、`block` / `rise` / `target_dead`——均无外部
使用者，编译器已证）。

**「不再是纯移动」的地方（记账）**：① 上面那张可见性表；② **产物非逐字节相同**——`.text`
274904 B vs 272656 B，模块结构变化改动了 codegen unit 划分与内联决策（连 `SpaceInner::unmap`、
`handle_page_fault` 这类不相干函数的大小都变了），故「产物相同」这条判据本轮不成立；
③ 两处 intra-doc 链接（`mod.rs` 头的 `block`、`wait/mod.rs` 的 `reap`）搬家后解析范围变了，
已改成纯代码串，不再假装是链接；④ `// ── 操作：唤醒 ──` 重复两遍的旧瑕疵原样带走。

**验收**：`cargo fmt --check` clean；`cargo check --workspace --all-targets` **13 条 warning，
与改前逐条相同（零新增）**，拆出的文件里一条没有；`cargo build --release -p kernel --features audit`
通过；`scripts/examine.sh` 多轮实跑为 2/3、3/3、2/3。

**逐轮结果本身不是判据**（门有已知残留噪声，见上节）：判据是**失败签名 + 基线对照**——
失败轮一律是「门写不进 FIFO / `只写出 N/8 条`」且 guest 侧回显为 0，失败步不固定
（req / hole / clock / sleep 各出现过），而**未改动的干净基线同一道门也只有 5/6**、签名完全相同
⇒ 与本轮无关。正面等价证据：基线 PASS 轮与改后 PASS 轮的全量控制台只差两行
（`free 0x8029…` 映像大 ~2 KB 导致空闲区起点挪一页、以及挂钟相关的 `clock` 值）。

### runner 瘦身：判定收归一处

`scripts/runner.nu` 245 → **177** 行（注释 91→62、非注释 136→100）。runner 的职责此刻只剩两条：
**cargo 集成点** + 交互式跑的**终端流与证据归档**；判据不再有第二份。

- **删**：`def verdict`（40 行，第二份判据）、`QEMU_EXPECT`（零消费者）、`const MARK_HALT`/`MARK_PANIC`、
  `main` 里的**二次构建**（建的是 debug、跑的是 release ELF，实测不影响被跑内核）、FAIL 分支与 `exit 1`。
  删后 `QEMU_FEATURES`/`QEMU_SEMI` 只剩「给 qemu 加 `-semihosting`」一个含义。
- **改为一行观察**（信息，不是判据）：`观察：qemu 退出码 N · <正常自退 | 检出内核 panic | 被 host
  超时杀 | 非零退出>`；**任何情况都不 `exit 1`**，`cargo run` 的退出码不再因内核行为而变。
- **证据策略原样保住**：导出恒归档；捕获只在「退出码 0 且无 panic」时才删；起跑前残留 cap 先归档不删。
- **采纳的两处判断**：① 观察行加第四档「非零退出」——否则 qemu 非 0 自退会被标成「正常自退」= 假话；
  ② **panic 先于 124**——panic 走 `halt_loop()` 兜底自旋必被超时杀，根因是 panic 而非超时。
- 头部四段（导出模型 / 判定 / 证据策略 / 约定）压成两行指向本节；四条 **nu 语义地雷**（外部命令非零
  退出当场中止脚本、`do -i` 清掉退出码、`<` 不支持、`o+e>|` 合并 stderr）留在脚本里。
- **记一个静默失效的风险**：panic 现在只认控制台那行 `[panic] at`（导出里的 `"halt":"panic"` 不再被看，
  因导出需 semihosting）。**将来若内核不再打这一行，panic 检测会无声失效**——改内核 panic 输出时
  必须同时改这里。另：`open $cap --raw` 遇非法 UTF-8 会报错，与旧版同等暴露，未改。
- 顺带修了旧注释的自相矛盾处（写 `sleep=off` 而实参是 `auto,sleep=on`，只改注释）；`.cargo/config.toml`
  注释里的旧名 `scripts/qemu-runner.nu` 同步改为 `scripts/runner.nu`（该文件仅此一行改动）。
- **未动**：runner 仍带 `-icount auto,sleep=on`（为「同 seed 可复现」）⇒ 交互式跑仍会吃到与 icount
  相关的那条偶发失步（约 1/5）；验收门已去掉 icount，故门不受影响。要不要给 runner 也换掉，是独立裁决。
- **验收（cargo 层，本轮实跑）**：正常轮 → `观察：qemu 退出码 0 · 正常自退`、cargo rc=0、归档目录空；
  超时轮（`QEMU_TIMEOUT=6`）→ `观察：… 124 · 被 host 超时杀`、捕获归档为 `console-*.log`、**rc 仍是 0**。
  残留风险（未验）：真实内核 panic 路径（只以 stub qemu 验过分支）。

### ③ QEMU 起法单一出处；runner 再收窄

- 新增 `scripts/boot.nu`（79 行）：**qemu 起法的唯一出处**——凑参数 + 外接 timeout + 起 qemu +
  原样返回退出码；不判定、不归档、不碰 stdin 的所有权。串口固定
  `-display none -serial stdio -monitor none`（不用 `-nographic` 的 monitor 复用：mux 会把 stdin
  静默改道）；`QEMU_ICOUNT=` 置空即关 icount（验收门关掉它，见上节）。
- `scripts/runner.nu` 177 → **127** 行：`config` 只留 cargo 集成要的四个字段（proj_root/elf/
  trapdir/seed），qemu 参数整段删除；`run_qemu` 变成「cd 归档目录 → rescue →
  `try { ^nu boot.nu $elf o+e>| tee { save } }` → 取退出码」。**qemu 命令行现在只有一处**。
- **实测**：正常轮 → `观察：qemu 退出码 0 · 正常自退`、cargo rc=0、归档目录空；
  超时轮（stdin 保持打开 + `QEMU_TIMEOUT=6`）→ `观察：… 124 · 被 host 超时杀` + 捕获归档。
- **一条新事实（记账）**：stdin 一旦 **EOF**，guest 会把它当 Ctrl-D ⇒ shell 自己退出 → 自然停机
  → qemu 退出码 0。所以「期望不停机」的存活探针**必须保持 stdin 打开**（`( sleep 30 )`），否则
  测到的是关机而不是超时；验收门自持 FIFO 写端不关，正因此不会踩到。

### 待做：验收门重写为 nu（设计已定）

- 门的输入需要**长驻写端**（既避 SIGPIPE，也避上面那条 EOF=Ctrl-D）。nu 没有长驻写句柄，
  故用 `^tail -f -n +1 <命令文件> > <FIFO>` 当喂食器（tail 持有写端、逐行吐出），门把命令
  `save --append` 进命令文件；逐步 expect 仍读控制台捕获。
- 判据与九步序列照旧：逐步 expect + 自退（非 124）+ 无 panic + 九个 marker 齐，icount 关闭。

**更正与教训（同一轮内抓到）**：`scripts/boot.nu` 最初把退出码当 `main` 的**返回值**——nu 会把
返回值**打印**出来、进程仍以 0 结束，于是消费者（runner）看到的永远是 0：**超时被杀那档被误报成
「正常自退」、捕获也不归档**（`9368be2` 的实测结论当时不成立，已由 `fix:` 更正为
`exit $env.LAST_EXIT_CODE`）。复验：超时轮（stdin 打开 +
`QEMU_TIMEOUT=6`）→ `观察：124 · 被 host 超时杀` + 捕获归档；正常轮 → `观察：0 · 正常自退` + 归档目录空。
**教训入地雷清单**：nu 里「返回一个整数」≠「以该码退出」，跨脚本传退出码必须显式 `exit`。

### 三条缺失断言的处置（裁决）

**前提更正**：`redeem` / `wipe` / `prune` **都已经被现有探针走到**，缺的是**断言**而不是覆盖——
`sleep <ms>` 走到 `redeem`（`woke` 就是断言，但只跑一个时长、一轮一次）；`hole` 的自测本就含
unseal / push / pull / **seal**，因而走到了 `wipe` 的 hole seal/drop 调用方，却**没有任何断言**
说「seal 释放了等待者」；`prune` 每次出队都在跑，**零观测量**。旁证：`reclaim`（unseal+release
反复，泄漏即耗尽帧池）就是本仓「有界增长探针」的先例。

裁决（本轮）：

1. **`redeem`**：门里再加一条**不同时长**（`sleep 300` 之后 `sleep 700`），断言两次 `woke` 都出现。
   票单调不复用 ⇒ 若「票 → 持票人 → 键 → 站点」这条路上有残留，第二次就会挂住。这是**结构性**
   断言，不是重复一次同样的检查。
2. **`wipe`**：让 `hole` 探针把「本次等待是被 **seal 唤醒**」与「等到超时」区分开打出来，门断言
   该句（同一轮内 seal 前后各一次等待，两种结局可辨）。
3. **`prune`**：**在 `audit` 档下加只读计数**（站点表规模）并打印，门增加一个 audit 档轮次。
   这是内核侧观测面，由用户裁决；**ABI 不动**——只加计数与打印，不加 envcall。
4. **顺序**：先把门从 `scripts/examine.sh` 重写成 `scripts/e2e.nu`（设计见上节），三条断言**一次**
   加进 nu 版，避免搬两遍。

### 命名：验收门改名 `examine`

按用户裁决，验收门 `scripts/e2e.sh` → **`scripts/examine.sh`**，环境变量前缀 `E2E_*` → `EXAMINE_*`，
文档中的活引用同步更新（历史叙述里的度量与结论不动）。**同时修掉一处代码与记载不符**：门里的
`icount` 开关是本轮诊断时加的，默认仍是 `auto,sleep=on`（**开**），而本节早已写「门不再用
`-icount`」——即记载是对的、代码是错的。本轮把默认改成**关闭**，要复现「同 seed 同轨迹」时才显式
`EXAMINE_ICOUNT=auto,sleep=on`。改后复验连跑 3 次 **3/3**。

下一步的 nu 重写产出 **`scripts/examine.nu`**（替掉 `.sh` 这版），三条缺失断言（`redeem` / `wipe` /
`prune`）随之一次加进 nu 版。

### 门重写为 nu（`scripts/examine.nu`）

`scripts/examine.sh` → **`scripts/examine.nu`**（297 行；bash 版删除）。四类判据逐条保留——逐步
expect / 自退（非 124）/ 无 panic / 九个 marker；八条命令与九个 marker 与被删版**逐字一致**（从 git
取旧版做 diff 核过）。独立复核：`--ide-ast` 通过、本人实跑 **3/3**。

**输入的喂法必须换（原设计在 nu 里不可达）**：nu **没有 `<` 重定向**——`^cat < f` 里 `<` 只是普通
实参（实测报 `cat: '<': 没有那个文件或目录`），FIFO 接不到 qemu 的 stdin。改用**同性质的长驻管道**：

```
job spawn { try { ^tail -f -n +1 <命令文件> | ^nu scripts/boot.nu <elf> o+e> <捕获> } catch {}; … }
```

tail 端长驻（不 EOF ⇒ 不会被 guest 当 Ctrl-D），读端自 job 启动起由 nu→timeout→qemu 持有（不
SIGPIPE），门只 `save --append` 逐条追加命令。最小实验（`trace/examine-exp/`）证过：管道直通逐条到达；
真 guest 收到 `spawn`/`exit` 并自然停机；job 里不包 `try` 就永远不写 rc 文件。

**验收**：`EXAMINE_REPEAT=3` → **3/3**；`EXAMINE_REPEAT=10` → **10/10**（icount 关）。
**故障注入**（把 `hole` 那步期望改坏、marker 不动）：`步骤[hole] 超时（15s 内没等到 …）` + 原因串指到
`步骤 hole` + 现场四个文件（console.log / cmds.txt / qemu.rc / diag.txt）留在目录 + `rc=1`；改回后 hash
与冻结值一致并复跑通过。**门真的会拦，且指到具体命令。**

**顺带修掉 `.sh` 里一处恒不成立的检查**：诊断用的「回显计数」写成 `grep -ac '^sq > '`（**行首锚定**），
而 guest 提示符前**恒有 ANSI 色码** ⇒ 该计数**永远是 0**——本轮早前我读诊断时正是被它误导（把「guest
没收到输入」判断得更死）。nu 版去掉锚定（故障轮活快照报 20 行含 `sq > `）。它只作诊断字段、不参与判定，
但「永远为 0 的观测量」本身就是飞线。

**改行为处（全列，供复查）**：FIFO→管道；收尾无 `exec 3>&-` ⇒ 失败轮不再因 EOF 提前自退、要等满
`QEMU_TIMEOUT`（实测 60.2s，判据不受影响）；`输入写失败`/`write.err` 无对应物，替代证据是 diag 里的
条数与字节数；diag 内容改写（去 fd3/FIFO，加 seed/icount/qemu 命令行/rc）；`只写出 N/8 条` 补了 ` + `
分隔；新增 nu-only 失败理由「rc 取不到」；`1..0` 与 `QEMU_TIMEOUT=0` 两处边界显式对齐。

### 三条缺失断言落地

各自做过**改坏就挂**的反向验证（反向证据比正向通过值钱）：

| 断言 | 过 | 挂（反向） |
|---|---|---|
| `redeem`：追加 `sleep 700`，断言两条 `sleep …ms` 与两次 `woke` **按序**出现 | `sleep 300ms / woke / sleep 700ms / woke` | 期望改成永不出现的 marker ⇒ `步骤 sleep 700 …` FAIL |
| `wipe`：`hole` 探针把 seal **前/后**两次等待的结局分开打 | `hole: wait-pre wake=timeout …` / `hole: wait-seal sealed=1 wake=seal` | 只打 timeout ⇒ `缺[hole: wait-seal …] + 顺序[…]` FAIL |
| `prune`：audit 档只读计数，门判 `orphan == 0` | `[audit] sites 34 live 0 tomb 34 orphan 0 waiters 0` | 关掉 `prune` ⇒ `sites 36 … orphan 2` ⇒ FAIL `孤儿站点[2]` |

**判据改用 `orphan == 0`，而非原建议的「站点表已空」**（理由是实测）：真值不是 0——`sites=34` 且
**全是墓碑**（`tomb=34, live=0, orphan=0`）；关掉 `prune` 时总数只 34→36，而**孤儿 0→2**。墓碑
（`wipe` 留下的 `pend=true` 空站点）按判据**不许**被 `prune` 删，用总数断言既被稀释、又几乎无牙。
`orphan`（队列空且无信标，正是 `prune` 该删的那一类）才是对 `prune` 的直接断言。

**顺带量出一条实质缺陷（待裁决）**：那 34 个残留**全是墓碑**（28 hole / 6 task）——`wipe` 用
`or_insert_with(Site::new)` 建站点并置 `pend=true`，而 `take_beacon` 消费信标后**不删空站点**、
`prune` 又因 `!pend` 为假不删 ⇒ **每次 hole 封印、每次任务回收各留一个墓碑**；hole id 与 task id
都单调 ⇒ **站点表随运行单调增长**。这是 A2「站点永不回收」修好之后的**一个漏口**，也正是 `prune`
当初要消灭的东西。候选最小修法（**未做**，属行为改动）：`take_beacon` 消费信标后若队列空即删该站点
（键不复用 ⇒ 不会误伤后续等待者）。

**audit 档轮次在本机跑不通（既存缺陷，非本轮引入）**：`[integrity] CanaryBroken … 0x0 != 0x51a70d1ecafebeef`
——docs §C3.8 那条：`KernelHeap` slack canary 被清零 ⇒ `report()` ⇒ panic ⇒ **系统无法完成启动**。
已用 `git stash` 在**原始**内核上复现（3 个 seed 全中、空命令文件也中）；默认档看不见（自检 cfg out）。
影响：shell 起不来 ⇒ audit 轮**一条断言都跑不到**。临时旁路该 canary 后，audit 轮里三条断言**全部
通过**（旁路已完全还原）。其后还有第二条既存违规：关机审计 `AuditDivergence … task lifecycle leak at
shutdown: 19 frames, 9 blocks` + `table frames 150 != kernel-walk count 141` ⇒ 即便 canary 修好，
「无 panic」判据仍会挂。**两条都与本次计数无关**（计数在它们之前的钩子里已打印完，且 `orphan=0`）。

**默认档行为不变**：非 audit 下内核不打印、不计数；门仍是 8 步 9 marker（独立复核 3/3），新步只追加。

### A2 归因：墓碑漏口就是 A2 没做完的那一半

实测（audit 只读计数）：关机时刻 `sites 34 / live 0 / tomb 34 / orphan 0 / waiters 0`——34 个残留
**全是墓碑**（28 hole + 6 task）。**这不是手滑，是在飞窗口的同步点**：

- `take_beacon`（`wait/site.rs:169`）用 `sites.get_mut(&key)`：**不建站点**，缺站点即返回 false；
- `block()` 的时序是 ① `take_beacon` → ② 离核 → ④ 持锁入队时再查信标。①不建站点 ⇒ 一个正在 ①④
  之间飞着的等待者，此刻站点表里**可能没有它的站点**；若 `wipe` 那时不建墓碑，④ 的复查就看不到任何
  「键已死」的记录 ⇒ 它照样入队 ⇒ **永久挂住**。所以 `wipe`（`wait/mod.rs:181`）里的
  `or_insert_with(Site::new)` + `pend = true` 是**无条件**的，不是疏忽；
- 对照 `wake`（`mod.rs:205`）：摘完 waiter 会 `prune(&mut sites, key)`；`wipe` 没有这一步——但它也
  **不能**有：墓碑正是为那个在飞窗口留的。`mod.rs:177` 那句「窗口最多一人」是同一件事的另一半说明。

要收回墓碑只有两条路：① 观测「在飞窗口已关闭」（今天没有这个量）；② 把「键已死」从**墓碑**搬到
**键/资源**本身，于是站点根本不必留——正是本节 A2 的原话：「站点寿命＝资源寿命」「在 `Space::drop` /
`seal` 处调 `wipe`/`void`，结构不动」。⇒ **这是 A2 未做完留下的漏口，不是本次新 bug**：`prune` 的
设计没错，缺的是「键寿命」那一半。

安全且**有界**的两小块（并入 A2 一起做，不单独打补丁）：④ 消费信标后若队列空即 `prune`（安全：信标
已被这个等待者吃掉，且窗口最多一人）；关机期 `clear_loop` 的目标回收可「不建墓碑」（那一刻全任务已退、
无在飞等待者），而 hole 封印那半**不行**（那时是并发中）。

### A2 结构裁决：(b) 令牌是引用

用户裁决：room 拥有**存活单元**，**站点值持 `Weak`**，资源侧（`HolePie` / `Space` / 任务）持强引用；
死亡靠 `Weak::upgrade` 失败**观察**得到，**不靠 `Drop` 回调** ⇒ 锁序零风险（(a) 案的
「资源 drop 必须在 room 锁外」那条难保证的前提因此不必存在）。

**一落地就浮出的结构点：键怎么找到它的存活单元？**

- 键是 `HashMap` 的 key，必须 Copy/Eq ⇒ **`Weak` 不能放进键**（同时破坏 `Copy` 与哈希语义）；
- 于是只剩一条自然路：**调用方随键一起把弱引用交进来**（形如 `wait(key, alive: &Weak<Alive>, dur)`）——
  调用方（hole / space / task 侧）**本来就握着强引用**（它就是那个退休者），交一枚弱引用最自然；
  且保持 **mail → room 单向**、键仍是 Copy 小枚举、**无需任何旁路表**；
- 代价：五个入口的签名各多一个参数（`wait` / `wake` / `wipe` / `join` / `park`；`Alarm` 键由任务自己
  退休，同一形状）。

**由此确定的结构**：`Site` 值变为 `{ pend, waiters, alive: Weak<Alive> }`；`prune` 判据从「队列空 ∧
无信标」扩成「… ∧ `alive` 已死」⇒ 墓碑不再需要、残留自然归零；`block()` 的 ④ 在持站点锁时先判
`alive.upgrade().is_none()` ⇒ 键已死则不入队、直接 `Handoff::Resume`（在飞窗口由此关上）。

### A2 原语与签名（已裁 / 待裁）

**原语（已裁）**：

1. **`Life`** —— 键的存活单元。`Life::new() -> Arc<Life>` 由**资源的创建者**调用；读法一对
   `live()` / `dead()`（4/4 等长对偶；room 内部只用 `dead`）。不变式：**它只回答「资源还在不在」**，
   不回答「谁在等」——后者仍归站点表。死亡 = 强引用落地 ⇒ 弱引用 `upgrade` 失败，**没有写路径、
   没有回调**。命名避开了 `Task` 那侧已有的 `alive` 语义。
2. **`Site` 值加 `life: Weak<Life>`**（不是新原语）。`Weak` 放**值**里而非键里：键是 map key，
   必须 Copy/Eq。
3. **room 侧只加一个读法**（在站点锁内）：`prune` 判据由「队列空 ∧ 无信标」扩为「… ∧ `life` 已死」；
   `block()` ④ 持锁先判 `dead` ⇒ 键已死不入队、走既有回滚。

**不新增写路径**（这条是 (b) 的自洽性前提）：除 `Life::new` 与资源侧**显式的** `wipe(key)`（今天
`seal`/`drop` 处已经在调），room 再不接受任何来自外部的「这个键死了」的说法——全部靠**读**。
故 A2 不引入第二张表，也就不会重演墓碑。

**签名（本轮稿，待裁）**：四个入口加参数，`park` 例外自取：

```
wait(key: WakeKey, life: &Weak<Life>, dur: Duration) -> Handoff<()>
join(tid: TaskId,   life: &Weak<Life>, dur: Duration) -> Result<Handoff<bool>, GateError>
wake(key: WakeKey,  life: &Weak<Life>) -> bool        // 键已死 ⇒ false，且不建站点
wipe(key: WakeKey,  life: &Weak<Life>) -> usize       // 全放；站点可直接删，不留墓碑
park(dur: Duration) -> usize                          // 形状不变，内部自取本任务的 Life
```

资源侧（`Space` / `HolePie` / 任务）各增一枚 `life: Arc<Life>`，并在创建时留一份 `Weak` 交调用方——
避免每次等待一次的原子操作。「键 → 存活单元」的解析在**调用方那一层**（envcall / mail / scheduler），
**不在 room** ⇒ `mail → room` 单向不变。

**④ 的判死走现成的回滚分支**（不新增机制）：`tock` 在 ③ 已做，故「决定不阻塞」这条必须撤销它，
而那正是既有 ⑤（`void(ticket)` + `rise`）在做的事；`Blocked` 只在 push 那一支被写 ⇒「容器 ⇔ 状态」
不出现破口。

### §C3.8 已定根因并修复（1 行合约修正）

根因（`kernel/src/memory/allocator/block.rs:352-355`）：`impl Allocator for BlockAllocator::allocate` 返回
`NonNull::slice_from_raw_parts(addr, 1usize << power)`——**交付长度报的是整块 size class**，而非请求的
`layout.size()`。而 `core::alloc::Allocator::allocate_zeroed` 的默认实现按**返回切片的 len** 清零
（`write_bytes(0, ptr.len())`）⇒ 任何零化分配都把零写进**请求区之外的 slack**，而 fence 的 slack canary
恰住在 `addr + align8(size)`（写 `fence/ledger.rs:87-94`、读 `:252`）⇒ canary 归零 ⇒ `unmark` 时报
`CanaryBroken` ⇒ `report()` ⇒ panic ⇒ audit 档**起不了 shell**。

§C3.8 原本的三个候选方向**被逐一排除**：探针显示「分配一返回 canary 就已经是 0x0」⇒ 清零在分配**内部**；
同页另一块同 size class 的 canary 完好；整块 64B 全零而页仍 tally-owned / banker-held、页内非零字
394/512 ⇒ 严格本块范围。boot 第一枪是启动期 `envcall/mail.rs:134` 的 `alloc::vec![0u8; max]`
（`REFER_MTU = 0x29` → 0x40 块，canary 槽 +0x30）。

**修法（1 行 + 注释）**：`1usize << power` → `layout.size()`。理由：`NonNull<[u8]>` 的 len 是「交付给调用方
的字节数」这条合约的载体；请求区外的 slack 属分配器内部（canary 住那儿），不能报成交付物（frame 侧本来
就只报 `max(size, PAGE_SIZE)`）。**没有关或放宽任何 canary / ledger / banker 检查**；不动 ABI、不动锁序。

**复验（本人独立跑）**：默认档 3/3；audit 档 `CanaryBroken` **归零**、9 步全过，控制台里
`sleep 300ms`/`sleep 700ms`/两次 `woke`、`hole: wait-seal sealed=1 wake=seal`、
`[audit] sites 34 live 0 tomb 34 orphan 0 waiters 0` 全部出现。默认档看不见的原因：canary / ledger /
banker 全在 `cfg(feature = "audit")` 下。

**副作用（待改）**：`EXAMINE_FEATURES=audit` 配默认 repeat 会「按构造」挂掉默认轮——门只构建一次 ELF
（带 audit），而默认轮判据含「默认档不该有 audit 输出」（audit ELF 每次关机都打 `[audit] sites`）⇒ 验默认档
必须单独跑不带 feature 的门。这是门的设计瑕疵：应当**按档分别构建**，或把 audit 轮独立成一档。

### 关机审计第二条违规：已收缩到一个因（属 A2 线，未修）

`task lifecycle leak at shutdown: 19 frames, 9 blocks` + `table frames 150 != kernel-walk count 141` 同源：
**一个 state 已 `Reaped` 却仍有 3 个 `Arc<Task>` 强引用的任务**（id 5、`u-thread`、space asid 4；reap 前
strong 4、bury 后 3，其余任务 bury 后恒为 1）⇒ Task→TaskIdent→Team→Space 整条链都不 drop ⇒
`Space::drop` 里的 `fence::retire(asid 4)` 不跑 ⇒ 三类账同时留下（19 frames = `frame.classes[Task]`；
9 blocks = 6–7 条 `Arc<Task>` 0x88 + 1 条 `Arc<TaskIdent>` 0x80 + 2 条 asid 4 的 UserHeap 0x1000；
150 vs 141 = `frame.classes[Table]` 对内核根 walk，差 9 页是该任务空间页表）。多出的强引用**无活主人**
（running 槽 / starved / by_id / Team.held / 站点队列 / HUSKS / 票根逐项排除）⇒ **泄漏的克隆**，正是
`wait/holder.rs` 注释预言的形态。

**与 A2 同源，故并入 A2、不单独打补丁**：这条账的根就是 A2 正在定的「挂起任务的强持有者只能是它所在的
站点队列 / 键寿命＝资源寿命」。下一步：给 room 线的 `Arc<Task>` 取用点加计数探针（`running_task()` 约 20 处、
`lookup_task_by_id()` 的 Join/Hatch/doom），抓哪一次 +1 不回落；优先怀疑「跨挂起持有」（`block()` 里
`Handoff::Switch` 前的临时强引用、`join`/`wipe`/`redeem`、`quit`/`bury`、`doom::suspend`）。

**旁枝（另案，未处理）**：关机屏障不挡「败者核继续跑任务」——慢探针期间 hart 1 报
`user page fault without running task`（`trap.rs:228`）而 panic。

### 门按档分别构建（修掉「一次构建、两种期望」）

`EXAMINE_FEATURES=audit` 原先只构建**一次** ELF，而默认轮判据里有一条哨兵「默认档不该出现 audit 输出」
（audit ELF 每次关机都打 `[audit] sites …`）⇒ 默认轮**按构造必挂**，两个档不能在一次运行里各得其所。
改成**按档构建、按档跑**：默认轮恒跑不带 feature 的 ELF、audit 轮跑带 feature 的 ELF；每档产物**建完立刻**
搬进本档目录（`<OUT>/elf-default`、`<OUT>/elf-audit`），并**连 `initrd.img` 一起搬**——`boot.nu` 在 ELF 同
目录找它，不搬等于把 initrd 弄丢（症状与 feature 无关，极难查）；只建有轮次要跑的档（`REPEAT=0` 且不带
feature 时不再空构建）。

**默认档的 feature 写成 `const DEFAULT_FEATURES = ""`，不是环境变量**：加旋钮等于在门里开一条「让默认轮跑
别档 ELF」的合法通路，正是本轮修掉的瑕疵的可配置版。

**判定没有被改成和事佬**（逐条自查）：那条既存违规（`task lifecycle leak at shutdown: 19 frames, 9 blocks`，
A2 线）**仍由「无 panic」判据原样判死** ⇒ audit 轮 FAIL、整体非零退出；新增的只是**纯标注**（前件
`$why != ""` ⇒ 只可能加在已判 FAIL 的轮上，`ok` 的算法没动），数从捕获 grep 而来、不写死，并指向本节
「关机审计第二条违规」。没有开关、没有白名单、没摘 marker、没放宽阈值。另把 audit 档 PASS 标签
`站点表已空` 改成 `站点表无孤儿`——**旧标签与本档判据自相矛盾**（判的从来是 `orphan==0 ∧ live==0 ∧
waiters==0`；`sites=34` 全是墓碑，总数从来不作判据），是文字修正。

**复验（本人独立跑）**：默认档 3/3（每轮构建行显示 `--features ''`）；`EXAMINE_FEATURES=audit REPEAT=3`
⇒ **3 条默认轮 PASS ＋ audit 轮九步全过、仅因既存违规 FAIL**、`3/4` rc=1，原因串里没有任何
`缺[…]`/`顺序[…]`/`孤儿站点`。**反向验证**（把 `DEFAULT_FEATURES` 临时改成 `audit`）⇒ 哨兵仍拦
（`默认档出现了 audit 输出`，捕获里 10 行 `[audit]`），改回后指纹一致。

### A2 实施落地

新增 `kernel/src/work/unit/life.rs`（72 行）+ 12 文件适配，`+252/-90`。

- **`Life` 是无字段零尺寸标记**：没有字段 ⇒ **没有锁可持** ⇒ 结构上不可能是 L1/L2/L3 任何一层的持有者
  （比「加把锁保护状态」强的地方就在这里）；判死只读 `ArcInner::strong` 一个原子量 ⇒ 在站点锁（L3）内
  判死是合法的，而那正是 `prune` 需要的那一条读法。
- **`Weak` 的传递**：强引用在资源侧（`Space.life` / `HoleMeta.life` / `Task.life`），弱引用由**调用方层**
  在入口交给 room（envcall `Wait`/`Wake` 取 `team.space.life()`、`Join` 取 `lookup_task_by_id` 的 `Arc`、
  hole 的五个调用点取 `meta.life()`、`park` 内部自取）——room 只收 `(键, 弱引用)`，**不认识 mail**。
- **判据实测**：`[audit] sites 0 live 0 tomb 0 orphan 0 waiters 0`（**tomb 34 → 0**）；自建量具
  `trace/a2-press.nu` 在 extra=0 与 extra=8 两档都是 `sites 0` ⇒ **总数持平且真值为 0**（不是「持平在
  34」）；三条断言照旧（两次 `woke`、`wait-seal sealed=1 wake=seal`、`orphan==0`）；**默认档控制台归一化
  后六份同一个 md5、diff 0 行** ⇒ 默认档无可观测行为变化。
- **反向验证**：把 `wipe` 改回留墓碑 + `prune` 判据还原 ⇒ `sites 34 / tomb 34 / hole 28 / task 6`，与改前
  **逐字相同**；中途版本（判据换了但仍留站点）实测 `52 → 88`，量具当场抓到漏口 ⇒ 判据有牙。

**两处对设计原稿的偏离（已采纳，理由都是实测）**：

1. **`wipe(key)` 不带 `&Weak<Life>`**：语义已变成「当场删站点」，而三个调用点（`HoleMeta::drop` /
   `hole::seal` / `bury`）都在资源退役那一刻，站点里那枚弱引用本就指向同一个 `Arc` ⇒ 参数无处可用；
   逐字保签名只是多一次未使用的 clone。
2. **`prune` 判据落成 `队列空 ∧ (无信标 ∨ 键已死)`**，比原稿的「… ∧ `life` 已死」更进一步：`wipe` 不再留
   墓碑后，死键上**落单的信标**（`wake` 在无人在等时置的那种）成了新的墓碑——按原话写实测
   `sites 34 / tomb 14 / orphan 20`。死键的信标**永远无人认领**（资源侧的 `alive()` 检查已拒绝后来的操作），
   故归入孤儿。这是**谓词形状**的改变，不是新增机制。

**泄漏（判据 4）仍在，并有了新的收缩结论**：`task lifecycle leak at shutdown: 19 frames, 9 blocks` 一字未变
⇒ audit 轮仍未转绿。临时探针（已删）显示 `#5 'u-thread' state=Reaped strong=3`，而 **A2 侧容器全空**
（sites 0 / husks 0 / holders 0 / 四个 info 槽 None）⇒ 强引用不在 A2 碰过的任何容器里；且
**`extra=0 → 21 frames / 13 blocks`、`extra=4 → 23 frames / 17 blocks`** ⇒ 它**按活动量增长**（每个
hole+spawn 轮次约 +0.5 帧 / +1 块），不是固定残留。**下一步**：给 room 线的 `Arc<Task>` 取用点加计数探针
（`running_task()` 约 20 处、`lookup_task_by_id()` 的 Join/Hatch/doom），抓哪一次 +1 不回落。

**门的缺口（待修）**：门的 PASS 判据只判 `orphan`/`live`/`waiters`，**不判 `tomb`** ⇒ 反向验证那一轮门照样
PASS——`tomb 34 → 0` 目前只是**仪器读数**，还不是**门判据**。要让它有牙，须在 `scripts/examine.nu` 的
audit 轮加 `tomb != 0 ⇒ FAIL`。

---

## 10 · scheduler 模块清理（`work/room/scheduler`，7 文件 1148 行）

巡检（A2 之后、门按档构建之后）：把模块连同它的消费者（`envcall` / `gate` / `messenger` /
`lock::depend`）读了一遍，并做了两次决定性的字符串核对（见 §10.1）。问题分四类，按处置排成四轮；
用户裁决全取，序为 **①→③→④→②**。

### 10.1 判据不在被测构建里（最重的一条 · 轮 ②）— ✅ 已执行，见 §10.11

门的构建**只有** `--release`（`scripts/examine.nu` 的 `build_flavor`；`runner.nu` 的用法也写
`cargo run --release`），而 `[profile.release]` 只有 `opt-level=2, debug=1` ⇒ `debug_assertions` 全关。
**双向实测**（阳性对照在内）：

| 字符串 | release ELF（门跑的那份） | debug ELF |
|---|---|---|
| `装槽前 running 必须为空`（`seat` 的容器断言） | **0** | 1 |
| `starved 容器只收 Starved 任务`（`push`） | **0** | — |
| `new level must exceed max(held)`（lockdep 报文体） | **0** | 1 |
| `[depend]` | **0** | — |
| 对照：`task lifecycle leak at shutdown` | 2 | — |
| 对照：`no running task` | 3 | — |

release 份取 `trace/acc-audit0/elf-audit/sqware`；debug 份取
`target/riscv64gc-unknown-none-elf/debug/sqware`（同日构建，14 MB）。

后果全落在本模块与它的纪律上：

- **容器 ⇔ 状态不变量**（`push` 只收 Starved、`seat` 前 running 必空、`shed` 旧载荷不带标签）
  在门跑过的每一个产物里都只是注释；`Task::exclusive` 的 `assert!(strong_count >= 1)` 是**真断言**
  （release 也在）——同一个文件里两种纪律，前者没人验。
- **L1/L3 锁序**：`lock/depend.rs` 整文件 `#[cfg(debug_assertions)]` ⇒ 门也从不校验它。
  `docs/root.md` §8 的「debug 档同路径跑通（无 lockdep 违规）」是**一次手工跑**，不是覆盖；
  而历史上 lockdep **抓到过真违规**（`trace/diag9.log`：`lock-order level violation … (Space)
  <-- max held`），说明这条校验有牙、只是现在不进门。
- `kernel/Cargo.toml` 的 `audit` 注释提到一个「**debug_assertions 硬化开关**」，features 里
  没有这个开关——这句话是这次发现的第一条线索。

**处置**：加一档 `harden`（✅ 已做，见 §10.11）——`[profile.harden] inherits = "release"` + `debug-assertions = true`
（opt-level 2 保行为可比），门跑第三个 flavor，判据与默认档同形（`[depend]`/断言 panic 即 FAIL）。
预期代价：这一档会把 `health/*`（§D6 的「把整个 frame 池抽干再还」）与全部 84 处 `debug_assert`
一起跑起来，第一次很可能当场红——那正是要量的东西，不是要绕的东西。

### 10.2 说了但没做（同一份文件里文档与代码相反 · 轮 ①）

| # | 位置 | 文档说 | 代码是 |
|---|---|---|---|
| A1 | `core.rs` `rip()` 头注 vs 内联注释 | 「强制释放 scheduler 持有的**全部** task 引用」 | 只清 `starved` + info 槽；`running` 明确不动；`by_id` 只存 `Weak` |
| A1b | `rip()` 的 `starved.clear()` | 计数镜像「从唯一事实来源派生」 | 6 个改点里**唯一**漏 `set_len` 的一处 ⇒ 关机后镜像停在旧值 |
| ~~A2~~ | ~~`ktask.rs:96-102` vs `utask.rs:61`~~ | ~~「存活单元**不是参数**，在 callee 内自取」~~ | 曾记：callee 是 `(WakeKey, &Weak<Life>)`，asm 只递 a0 且来源是裸 `usize` ⇒ 双重不符。**已随 §10.8 的删除消失** |
| ~~A3~~ | ~~`mod.rs` 头注「命名三面同词」~~ | — | **已随 §10.8 重写**：三个面文件删除后，`mod.rs` 只剩 core/boot/trap 两个入口面 |
| A4 | `core.rs` 头注 | 「`starved` 字段私有，唯一修改路径是 push/pull」 | 实际 6 条（push/rotate/pull/try_pull/disown/remove/rip） |
| A5 | `core.rs` 头注 | 「`Team.tasks(3)` 与 `Space.inner(2)`」 | `Level::L3` 的**数值是 4**（3 是删掉的旧槽位）；括号里一个写名字一个写数值 |
| A6 | `core.rs` `lookup_id_weak` 头注 | 「避免撞上 `strong_count == 1` 断言」 | `Task::exclusive` 早已放宽成 `>= 1` 并明写「envcall 可短暂持额外强引用」；两个调用点拿到弱引用后**立刻 upgrade** ⇒ 理由是化石 |
| A7 | `core.rs` `wait()` 内联注释 | 「有任务即正常出口（清位交外层）」 | 两行后就是 `conductor::wake(me)`——清位就在本分支 |

另有一处**命名撞车**（不属文档漂移，待裁）：核心的 WFI/取活入口叫 `wait()`，而冻结表把
`wait`/`wake` 这对词给了 messenger 的事件等待与唤醒——一个词两个意思，正名须用户给词。
→ **已裁决并消解**（§10.15）：名字**不动**，`wait` 降为 `core/fetch.rs` 内部私有步骤
（对外入口是 `fetch`）——同一个词不再有两个意思。

### 10.3 并发面（轮 ④）— ✅ 已执行，见 §10.10

- **C1 · `Task.state` 被裸读，且 kill 会静默丢**。`messenger::doom::suspend` 无锁读 `state()` 后
  按它分派容器动作；`messenger::wait::target_dead` 在 `by_id` 交出的强 Arc 上读 `t.state()`。
  `Task::exclusive` 的 SAFETY 论证明写「临时持有者**不触字段**」——这两处正是触字段：他核
  `transform` 写 `state` 时这里是未同步读。更实的是 TOCTOU：读到 `Starved` 与
  `remove_from_starved` 之间被别核 seat 走 ⇒ 返 false ⇒ **这次 kill 被丢掉**（`doomed`+SSIP
  兜底只覆盖 Running 分支，`cull` 的 `filter(|t| suspend(t))` 把它当「没动它」扔了，无重试）。
- **C2 · `by_id` 交出的强 Arc 是有意违反「唯一强持有」**，现状安全只靠「调用方立刻 drop」的口头
  约定（envcall `Join` 恰好 drop 了；`doom::doom` 把 `task` 持过整趟级联）。没有类型、没有断言。

### 10.4 冗余与可删（轮 ③ + §D3）— ✅ D1 已执行（§10.9）、D2/D4 随 §10.8 消失、D3 已执行（§10.9）

- ✅ **D1 · N 张完全相同的表**（已执行，见 §10.9）：`register_task_id` 往**所有** hart 的 `by_id` 各插一份，没有第二条
  插入路径 ⇒ 每张表都是全世界的完整副本：`lookup_task_by_id` 的循环只有第一张可能命中，
  `snap()` 把每个任务返回 **H 份**。取 `snap()` 的 `find`/`holder`/`vestable` 对重复免疫；
  `heirs` 返回重复对，靠 `take` 的幂等被吃掉（`cull` 的 `removed` 只 +1）——**是侥幸不是设计**。
  代价形状更值钱：`by_id` **从不清理**，而 `gate::snap()` 在 `pie.rs` 的 5 处
  （Vest/Accord/Collect/Revoke/Release）各拍一张、每张逐条 `upgrade` ⇒ 每次 envcall 的成本随
  **历史 spawn 总数 × hart 数**增长，不随活任务数。
  **可检验的预测（并到泄漏线）**：「从不清理」还有第二层代价——每个已回收任务的
  `ArcInner<Task>` 因为表里还留着一枚 `Weak` 而**不能归还**，那些块会一直留在类别账上。
  实测的泄漏增长是「每个 hole+spawn 轮次约 +1 块」，与「每次 spawn 多一枚不死 `Weak`」的形态
  吻合。故轮 ③ 把表合一并在关机时清掉它之后，**预测 `blocks` 会下降**（强引用那份不变）；
  不降则这条假设被否掉——那也是收获，因为泄漏线就少了一个候选。
- ~~**D2 · `ktask.rs` 4 份逐字相同的存帧序**（约 35 条 `sd`/`csrr`，245 行里约 140 行）~~ ⇒
  **随 §10.8 的删除一并消失**（不再需要宏）。
- **D3 · `starved_len` 镜像 6 个改点靠人记**，已漏 1 处（A1b）。
- ~~**D4 · 4 处 `allow(dead_code)`**（`ktask` 的 park/starve/wait_forever + `utask::wait_forever`）~~ ⇒
  **随 §10.8 一并消失**（34 → 32；本轮代码提交里含另外 2 处）。

### 10.5 顺带（出模块、同族）— E1 已修（§10.7 的 Cargo.toml 一笔）

- `kernel/Cargo.toml` 说 semihosting「**默认开启**（default 引入）」，而 `default = []` ⇒ 相反；
  `cargo check` 的 `unused dependency semihosting` 是同一事实的旁证。门的两档都不带它 ⇒
  **门跑过的产物没有结构化导出**（`boot.nu` 的 `-semihosting` 只管 QEMU 侧、且要显式请求）。
- `Level::Block = 7` 零引用（§D4 已记），`Level::L3` 的数值洞 3（§10.2 A5 的同一件事）。

### 10.6 已核对干净（不要动）

`seat`/`shed`/`clear_slot` 三处 `into_raw`↔`from_raw` 配对与标签回收**账面平衡**（含
`!prev.is_null()` 分支可达）；`disown_and_install_next` 的「放锁窗口内无人能写本核 `running`」
自述成立（别核只能 `try_pull`）；`wait()` 的每条出口各清一次睡眠位、无重复；
`remove_from_starved`/`running_hart` 逐 hart 顺序取放锁、不嵌套；轮转分支「唯一任务不减预算」与
「Switch 事件落在 seat 之后」是对的；§7.4 的拆分计划方向正确——而它要逼出的那个问题（A3「一张还是
N 张」）现在**已有实测答案**（N 张完全相同）。

### 10.7 轮 ① 执行记录（文档/契约对齐 + `starved_len` 收口）

改了 A1–A7、D3、§10.5 的 manifest 一句，外加 §10.3 C1 在 `remove_from_starved` 上的一句话指针：

- `core.rs`：就绪队列的改动收成四个 `starved_*` 方法（**镜像在方法体内与队列操作同一处派生**），
  `SchedulerInner.starved` 对 core.rs 之外私有、跨文件只留 `starved_is_empty` 一个读法；
  `rip` 走 `starved_clear` ⇒ A1b 的漏点从结构上消失；`rip` 头注改成事实（`running` **有意**不清
  + 关机屏障为什么不保证「没有核还在任务上下文」+ 代价归旁枝的账）；`lookup_id_weak` 的理由改成
  事实（旧理由依赖的 `strong_count == 1` 约束已不存在）；`wait()` 的内联注释改正；头注里
  `Level` 的数值与「唯一修改路径是 push/pull」两处改正。
- `mod.rs`：面清单与「三面同词」改正（**事件面**的 `Scheduler::{park,wait,reap}` 已整体移入
  messenger），死岛存废指向 §D3，并**显式记下** `wait` 一词两义这处待裁的撞车。
- `ktask.rs` / `utask.rs`：`wait_forever` 的失效写成事实（asm 与 callee 双重不符、树内零调用者、
  修它得先定「裸 usize → `WakeKey` 哪一支」的编码）——**不假装它能用**。
- `lock/depend.rs`：`L3` 的清单改正（`blocked` / `reaped` / `TIMER_DEADLINES` 三张表早已不在，
  改成现存的同级八类）。
- `kernel/Cargo.toml`：semihosting 头注改成与 manifest 一致（**非默认**），并记下门的两档都不带它
  ⇒ 门跑过的产物没有结构化导出。

**本轮的判据就是"无可观测变化"**：

| 判据 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | 干净 |
| `cargo check --workspace --all-targets` | warnings **13**（与记账基线同） |
| release 档 warnings | 新建 `HEAD` 对照工作树各建一次再 diff：**23 行两侧逐条相同** |
| 默认门 `scripts/examine.nu`（3 轮） | **3/3 PASS** |
| 默认档控制台归一化 md5（3 份） | **183133960fd82a1ff0f4a4c3f8863355** = A2 记账基线（六份同值） |
| audit 轮（`EXAMINE_FEATURES=audit REPEAT=0`） | 九步全过；仍只因既存违规 FAIL；`sites 0 live 0 tomb 0 orphan 0 waiters 0`；`19 frames, 9 blocks` 与 `table frames 150 != 141` **逐字未变**；**全部 `[audit]` 判据行与 A2 验收那份逐行相同**（差异只在 `[trace]`/`[scene]` 转储的内部顺序——那是非确定性段，不是行为） |

证据：`trace/sched-r1-default/`（`run1..3` 的 `console.log` / `norm2.txt` / `qemu.rc`）、
`trace/sched-r1-audit/run1/`、`trace/warn-head.txt` vs `trace/warn-r1.txt`。

**本轮故意不动**（各有归属轮）：`running` 槽的释放（属旁枝的账，理由写进 `rip` 头注）、
`by_id` 合一（轮 ③）、`state` 裸读与 kill 丢失（轮 ④）、harden 档与 `tomb` 判据（轮 ②）、
`wait` 的正名（**待用户给词**）。

### 10.8 内核任务面删除（用户裁决：内核任务不再支持）

裁决一句话：**把 `task` / `ktask` / `utask` 删了，内核任务不再支持。** 执行的落点与理由：

| 删掉的 | 落到哪 | 理由 |
|---|---|---|
| `scheduler/ktask.rs`（整文件，250 行） | — | 4 个 asm 面里 3 个树内零调用者；第 4 个（`reap`）的唯一调用者是 `ktask_trampoline`，而它自己也是死的 |
| `TaskBuilder::closure`（内核闭包任务唯一生产者，0 调用者）+ `ktask_trampoline` | — | §D3 的闭合死链；随内核任务面一起删（另收掉 2 处 `allow(dead_code)`） |
| `scheduler/utask.rs`（整文件，71 行） | `park`/`wait`/`wake` → 调用方直呼 `messenger::*`；`starve` → `current().starve()`；`wait_forever` → 删 | 五个 1:1 转发没有信息含量（转发层的名字只对「两个任务面」这个二分有意义，二分没了它就没意义） |
| `utask::reap` 的合成（`quit` + `bury` + `run`） | **收进 `messenger::quit()`** | 「排空躯壳必须发生在再次取活之前」原是**调用方义务**（靠两处注释维持）；收进 `quit` 后由结构保证 ⇒ 四个调用点各写一行。同时 `quit() -> Option<usize>`（旧返回值全仓无人用）改为 `-> usize`（就是要恢复的帧） |
| `scheduler/task.rs`（整文件，23 行） | `core.rs` 的模块级 `push`（入队 + 踢醒） | 「踢醒」不能并进 `Scheduler::push`——`messenger::rise` 批量唤醒后只踢一次（单 tick 的 IPI 量 O(N)→O(1)），并进去会让批量路径退化成 N 次踢；而 `conductor::kick` 只在 room 内可见（`pub(super)`）⇒ 入口必须留在 room 里 |
| `messenger::bury` 的导出 | 转私有 | 唯一调用者就是那个合成 ⇒ 收掉一个零引用导出（§D4 同类） |

**结构**：`scheduler/` = `mod.rs` + `core.rs` + `boot.rs` + `trap.rs`——「面」的划分随「用户任务 /
内核任务」二分一起消失，其头注写明这一段历史。`Scheduler::starve` 随之从 `pub(super)` 提到
`pub(crate)`（唯一调用方 envcall 现在直呼它）。

**判据（本轮要求"删除后无可观测变化"）**：

| 判据 | 结果 |
|---|---|
| 默认门 3/3 | **PASS** |
| 默认档控制台归一化 md5（3 份） | **183133960fd82a1ff0f4a4c3f8863355** = A2 基线（六份同值）⇒ 删除本身零行为变化 |
| audit 轮 | 九步全过；`sites 0 live 0 tomb 0 orphan 0 waiters 0`；**`[audit]` 判据行与删前逐行相同**（`19 frames, 9 blocks` + `table frames 150 != 141` 一字未变）；仍只因既存违规 FAIL |
| `cargo fmt --check` / `cargo check --workspace --all-targets` | 干净 / warnings **13**（与基线同） |
| release 档 warnings | 与删前逐条相同（0 行 diff） |

证据：`trace/del-default/run1..3/`、`trace/del-audit/run1/`、`trace/warn-del.txt`（对 `trace/warn-r1.txt`）。

**记账两笔**：

1. **`quit()` 丢掉已算好的后继帧**：`disown_and_install_next` 装槽时已经交出了后继帧 PA，`quit`
   仍走 `run()` 取活——两条路等价，差别只在 `run()` 会替后继再扣 1 个量子（8 → 7）。为保持与改前
   **逐字相同的调度行为**（上面那条 md5 判据要的就是这个），本轮**不动它**，已写进 `quit` 的文档
   待单独裁决。这是"顺手改掉"会让判据失去意义的典型例子。
2. **⚠ 操作教训（本轮踩到、写下来）**：为了对比 warnings，我用 `git worktree` 在主仓里建了一棵
   `HEAD` 对照树、并**共用同一个 `--target-dir`**。`kernel/build.rs` 的 `-T…/link.ld` 是
   `env!("CARGO_MANIFEST_DIR")` 在**编译 build script 时**烧进去的 ⇒ 缓存下来的 build-script
   可执行文件里留着 `trace/wt-head/kernel/link.ld`；对照树删掉之后，主树重建时才在**链接**阶段报
   `rust-lld: cannot find linker script`。`cargo clean -p kernel` **清不掉**它（host 侧
   `target/release/build/kernel/` 的那份要手动删），最后是 `cargo clean` + 全量重建解决的。
   结论：**跨树对照实验必须用各自的 `--target-dir`**，否则缓存会污染到很久以后的一次构建。

### 10.9 轮 ③ 执行记录：`by_id` 合一 —— 名册（enlist / muster / roster）

**命名（用户裁决）**：`enlist` 入册 / `muster` 点名 / `roster` 名册；对偶 `delist` 除名是
**保留名、本轮不写**。理由（也是本轮唯一一处对裁决的偏离，已当面记账）：名册里「条目在」
这件事本身就是**「这个 id 存在过」的唯一事实源**——「已回收」与「从未分配」靠它分开
（`muster` 为 `None` ⇔ 从未入册）。除名会把这两态重新糊在一起，而 §A3 判的正是「合成一条
判活」。A2 已在站点表上教过一遍：**删掉承载事实的东西，就只剩墓碑**。`delist` 出现的两个
触发条件：①「存在过」改由单调计数承担（即 §A3 说要消掉的那个第二真相源）；②名册不再兼职
判活。**表落点**：留在 `core.rs` 的全局表段（模块只剩 4 文件，表本来就住那儿）。

**结构**：`Scheduler.by_id`（每 hart 一张、每张插全量副本）⇒ 模块级 `ROSTER` 一张
（`Level::L3`）。原先「查表遍历所有 hart、快照把每个任务返回 H 份、每条查询成本随 hart 数
放大」三件事一并消失——名册是全局事实，本来就只该有一份。

**判活只剩一条来源**：`task::allocated`（`id < NEXT_ID`，第二个真相源）删除；`muster` 的返回值
本身是三态（`None` = 从未入册；`Some` 升不起来 = 已消失；升得起来 = 活）。`join` 因此**去掉
`Result`**、改收边界当场读出的 `reaped: bool`。

**本轮顺出来的活缺陷（已修，有实测）**：非法 id 的 `Err(Denied)` **曾经永远不可达**——
它需要 `target_dead ∧ ¬allocated` 同时成立，而 `target_dead` 为真的两条来路（查到且 `Reaped` /
查不到走 `allocated`）都蕴含 `allocated`。后果是「从未分配」与「已回收」在 `Join` 入口被折成
同一支，非法 id 走「目标仍活但永远不会结束」那条路。**先量后修**（新增用户态 `stray` 自检 +
门里一条断言，只挂 audit 档、默认档八步逐字未动）：

| | `join(9999, 0)` | `join(9999, MAX)` | `join(0, 0)` | 汇总 |
|---|---|---|---|---|
| 改前 | accepted | accepted | accepted（哨兵 0 甚至被答成「已回收」） | `stray: 0/3 illegal-id joins denied` |
| 改后 | denied | denied | denied | `stray: 3/3 illegal-id joins denied` |

**顺带证到的一条泄漏线结论**（§10.4 D1 的可检验预测，命中）：名册从不清理 ⇒ 每条已回收任务
的 `ArcInner<Task>` 因为还留着一枚 `Weak` 而**不能归还**，被类别账记成泄漏。把名册放到关机
最后一步清掉（强引用先全放，弱引用才是 `ArcInner` 的最后一道门）：

| | `frames` | `blocks` |
|---|---|---|
| 改前 | 19 | **9** |
| 改后 | 19 | **5** |
| 反向验证（临时不清名册） | 19 | **9** |

即：9 块里有 **4 块是名册合法持有的**（不是漏），清掉它们是把"表还在持有"与"漏了"分开；
`frames` 一字未变 ⇒ **真漏（`strong=3` 那个任务）不受影响，audit 轮仍 FAIL** —— 判据没有被
放水，只是账更准了。剩下的 `19 frames / 5 blocks` 仍是本线要追的东西。

**判据**：默认门 3/3；默认档控制台归一化 md5 三份仍是 `183133960fd82a1ff0f4a4c3f8863355`
（名册合一不改查询语义）；audit 轮九步全过、`sites 0 live 0 tomb 0 orphan 0 waiters 0` 不变、
`stray 3/3`、仍只因既存违规 FAIL；fmt 干净；warnings 13（基线同）。

证据：`trace/r3-default/run1..3/`、`trace/r3-audit/run1/`（`blocks 5`）、`trace/r3-reverse/run1/`
（`blocks 9`）、`trace/stray-before/run1/`（`0/3` 的改前读数）。

### 10.10 轮 ④ 执行记录：观察者只读判别式（案 B′）

**根因是一条分界**：读状态的人有两类，此前**能力却一样**。

| 类别 | 谁 | 同步从哪来 |
|---|---|---|
| **持有者** | `trap::run` 续跑、`seat`/`push` 的断言、`redeem` 之外的四条、`reap`、`block` ④ | 任务在本核手上的容器里（或刚被摘出）⇒ 与 `transform` 天然互斥 |
| **观察者** | `doom::suspend`、`Join` 边界、`redeem`（定时到点） | **没有**——容器锁保护容器操作、名册 L3 锁保护名册，都与 `state` 的写没有同步边 |

`Task::exclusive` 的 SAFETY 注记明写「临时持有者**不触字段**」，而这三处触的正是字段；
`task.rs` 里 `Reaped` 的注释还写着「`Join` 的判据因此不含竞态」——按内存模型那句话是**假的**。
更实的一条是 TOCTOU：`suspend` 读到 `Starved` → 摘容器之间被别核 `seat` 走 ⇒ 返 false ⇒
`cull` 的 `filter(|t| suspend(t))` 把它当"没动它"扔掉 ⇒ **这次 kill 静默丢失**。

**落地**（判据与反向验证见代码提交）：

1. **判别式原子化**：`Task.tag: AtomicU8` + 无载荷枚举 `TaskTag`（`TaskState::tag()` 穷尽
   match 投影）；`transform` 写 payload 后 Release store，观察者 `tag()` Acquire 读 ⇒
   观察者与写者之间有了正式的 happens-before 边。
2. **`Task::state()` 收紧成 `&mut self`**（字段转私有）：`&mut` 只能经 `Task::exclusive`
   拿到 ⇒「不触字段」从注释变成**编译期约束**。改这一行，编译器**一处不落地点名**了三处
   观察者——这正是本方案的价值：**旧写法现在连编译都过不了**。
3. **三处观察者各归其容器**：`Join` 读 `tag() == Reaped`；`suspend` 用 tag 当提示、容器操作
   当结论（`Blocked` 的键与票改扫分片问出来），不一致就重来、重试耗尽按 `Running` 兜底 ⇒
   「要么当场摘掉、要么注定自退」；`redeem` 的键改由**票根**携带（`hold(ticket, key, &task)`）
   ⇒ 到点路径不再读任务 payload。

**门覆盖**：`kill / suspend / doom` 这条路径此前 **两档控制台里 `killed` 出现 0 次**——零覆盖。
把 `cascade` 挂进 audit 轮（它覆盖 `doom → cull → suspend/reap`）。

**反向验证（如实记账）**：把 `suspend` 整个失效 ⇒ 门**在 `exit` 步挂到超时**（受害者一个都摘不
掉 ⇒ `REAPED` 永不配平 ⇒ 系统不停机）。故 kill 路径的牙长在**关机判据**上；`cascade: ok` 只
证明「任务自退 + 退出钩子级联」那一段——两件事不要混着读。

**新覆盖当场抓到的第一个真缺陷（已修，另一笔提交）**：`take_beacon` 把站点清成空壳后没有
`prune`——`wake` / `wipe` / `redeem` 三条路都记得做，只有它漏了。判据 `orphan 2 → 0`
（反向：去掉那行 ⇒ 原样回到 2）；`prune` 文档「两种形态」与探针三种 `live`/`tomb`/`orphan`
的漂移一并对齐。

**顺带一条可用作仪器的读数**：cascade 让 spawn 总数上台阶后，泄漏线跟着走
（`19 frames/5 blocks` → `29 frames/15 blocks`）⇒ 这轮把「泄漏 ≈ 每 spawn 任务 +2 帧 +2 块」
量化出来了，而 `cascade` 恰好给了一个**可控的 spawn 旋钮**（多跑一次 = 多一批任务），
比此前"按 hole+spawn 轮次加量"更干净。留给泄漏线用。

### 10.11 轮 ② 执行记录：harden 档（把断言与 lockdep 放回被测产物）+ `dead` 判据

#### (a) 判据不再"在门外"：门加第三档 `harden`

§10.1 记的那件事本轮关掉：门的构建只有 `--release` ⇒ `debug_assertions` 全关 ⇒ 容器⇔状态
断言与整条 lockdep（`lock/depend.rs` 全文件 `#[cfg(debug_assertions)]`）在门跑过的每个产物里
都被编译掉。现在：

- `[profile.harden] inherits = "release"` + `debug-assertions = true`——**只多开这一个开关**，
  opt-level/debug 与 release 一致，行为可比；
- 门加第三档：`EXAMINE_HARDEN=1` ⇒ 构建 `--profile harden` 的 ELF（产物独立搬进
  `<OUT>/elf-harden/`），跑**全部十步**（步骤越多，lockdep 与断言能验到的路径越多），
  判据 = 通用那套（自退 / 无 panic / marker）+ **`[depend]` 不许出现**（锁序违规的报文体，
  单列一条是为了在原因串里点名「这是锁序违规」而不是别的 panic）。

**正向对照**（这一档若不带断言就退化成"又跑了一遍默认档"而没人发现）：构建后**当场 grep ELF**，
必须能搜到某条断言串（取 `Scheduler::push` 的「starved 容器只收 Starved 任务」，非 cfg 代码、
任何构建都编得进去）；搜不到即 `exit 1`。实测：harden ELF **1 次**、release ELF **0 次**；
把 `debug-assertions` 临时改成 `false` 再建 ⇒ harden ELF 也变 **0 次**（对照会据此退出）⇒ 有牙。

**首次实测（本轮）**：`run 2: PASS (harden 档：自退 + 无 panic + 无 lockdep 违规 + 10 步全过 +
11 marker 齐)`——**84 处 `debug_assert` + 全套 lockdep + health 全在跑**的产物里，十步走完、
自然停机、锁序零违规。这是 §10.1 那条"纪律没人验"的正面收口。

#### (b) 站点表判据换成**精确的那一条**：`dead == 0`

原计划是"audit 轮加 `tomb != 0 ⇒ FAIL`"（A2 记账里那条缺口）。轮 ④ 的 `cascade` 覆盖率一挂上，
就发现**这条判据会误报**：`tomb`（队列空 ∧ 有信标）里混着两种东西——「活键上留着一枚等未来
认领的信号」（doorbell 的正常状态，`cascade` 之后稳定是 1）与「死键上的残留」。按老办法断言
`tomb == 0`，门会在一个**合法**状态上判红。

改成给探针加一个正交计数 **`dead`（键的存活单元已死）**——那才是 A2「站点寿命＝资源寿命」的
**精确**形式：资源退役时 `wipe` 当场删站点 ⇒ 一个死键站点存在 ⇔ 某条退役路径漏了 `wipe`。
audit 轮判据：`live == 0`、`orphan == 0`、**`dead == 0`**、`waiters == 0`；`tomb` 与 `sites`
总数只报数、不作判据（理由写进门里，连同 ④ 的实测）。

**反向验证（有牙的实证）**：把 `wipe` 改回"留墓碑"⇒
`[audit] sites 19 live 0 tomb 19 orphan 0 dead 17 waiters 0` ⇒ 门判红 **`死键站点[17]`**。
注意这组数里 **`orphan` 是 0**——也就是说**旧的孤儿判据在这个状态下抓不到**（站点带信标 ⇒
归 `tomb`），`dead` 才抓得到。正/反向读数都已留档。

#### (c) 判据（本节全绿）

| 判据 | 结果 |
|---|---|
| 默认档 ×3 | **3/3 PASS**，默认档控制台归一化 md5 三份仍是 A2 基线 |
| audit 档 | 十步全过；`sites=1 live=0 tomb=1 orphan=0 dead=0 waiters=0`；仍只因既存泄漏 FAIL |
| harden 档 | **PASS**（无 lockdep 违规 + 十步全过 + 自退）|
| 正向对照 | harden ELF 含断言串 1 次 / release 0 次（断言关掉后 harden 也 0）|
| 反向对照 | `wipe` 留墓碑 ⇒ `dead 17` ⇒ 判红；`take_beacon` 去掉 prune ⇒ `orphan 2`（④ 已记）|
| `cargo fmt` / `cargo check` | 干净 / warnings 13 |

### 10.12 泄漏线定位：任务退场把栈上的强引用一起丢了（机制已实证，修法待裁）

§9.3 记的那条账（`task lifecycle leak at shutdown: 29 frames, 15 blocks` + `table frames` 同源）
本轮**定位到机制**，并且让它有了**说出名字的判据**。

#### 先纠正一条我自己的误读

上一条记账说"每个 spawn 任务 +2 帧 +2 块"。实测否掉了它：`cascade=0` 的一次跑给出
`39 frames, 25 blocks`，而名册里**只有一个**任务活着（id=5 `u-thread`, `strong=3`, `Reaped`）。
⇒ 不是"每任务漏一对"，而是**这一个任务钉住了它整个 Team/Space**，于是那个域此后分配过的每一页
都留在类别账上——**账目随该域的活动量涨**，与 spawn 次数只是相关而非因果。这也是为什么不同负载
下同一个门的数字会不一样（29/15 与 39/25 是同一件事的两副面孔）。

#### 一条条排除：容器全空

在 `scheduler::rip` 末尾（此时就绪队列 / 站点 / 躯壳 / 槽都已清）插探针，量出：

```
[probe] 名册 8 条 / 仍活着 1 条 / 强计数合计 3        ← 其中 1 份是探针自己的 upgrade
[probe] hart 0..3 running = none                     ← running 槽空（rip 有意不清，但确实是空的）
[probe] leak team.held = None                        ← 未放行容器空
```

即：`Arc<Task>` 能**存住**的地方（`running` / `starved` / `HUSKS` / 站点队列 / `Team.held`）
**全部为空**，却仍有 2 份强引用 —— 与 §9.3 早先"多出的强引用无活主人"一致，但这次找到了主人。

#### 工具：**逐页先问映射、再问账本**的指针扫描

`Arc<Task>` 克隆存的指针 = `Arc::as_ptr`（数据指针）；`Weak` 存的是 ArcInner 基址 ⇒
在内存里扫这个字就**只命中强克隆**。两个坑都踩过并记下：

1. **直接扫 DRAM 会吃 LoadPageFault**：恒等映射不覆盖整个 DRAM（实测故障地址 `0x87c29000`，
   而且慢探针还会把 §9.3 那条旁枝（败者核继续跑任务）叫起来 ⇒ panic）。修法：逐页
   `team::kernel().space.translate(VirtualAddr::from_raw(pa))` **先问映射**，通过才读。
2. **账本按块登记、不含帧**：命中地址过 `fence::ledger::LEDGER.for_each` 查"驻在哪个块、
   谁分配的"；返回 `block=0x0` 即说明它在**帧**（栈 / trap 帧）里而不是堆块里。

实测（同一趟门）：

```
[probe] leak id=5 strong=3 arc_ptr=0x87c6ff10
[probe]   stack va=0x21000 size=20480 → pa=None      ← 它的栈**已经释放**（translate 返回 None）
[probe]   frame pa=0x87c6a000
[probe]   holder @ 0x87c39700 block=0x0（帧内）
[probe]   holder @ 0x87c49530 block=0x0（帧内）
```

两处持有者都在**帧**里、且都不是它自己的帧或栈 ⇒ 是**别的任务**的栈页。

#### 机制（结论）

任务退场是"**切走**"而不是"展开栈"：`quit` → `swap` → 装下一帧 → `restore`，被切走那份上下文
（含 **callee-saved 寄存器与调用者栈帧**）**永不回退**。于是：

> 切走那一刻栈上**还活着**的 `Arc<Task>` 局部量，随它的栈一起被释放（`bury` 归还 StackWindow /
> FrameWindow），**引用计数永不回落** ⇒ 被指向的任务被永久钉住 ⇒ 它的 Team/Space 不 drop ⇒
> 该域的帧与页全部留在类别账上（`task lifecycle leak`），`table frames != kernel-walk` 同源。

这也解释了 §9.3 记的"reap 前 strong 4、bury 后 3、其余任务 bury 后恒为 1"：多出来的那几份就是
**别的核/别的任务切走时留在栈上的**。u-thread 被 `running_task()` 交出过 **14298 次**（临时探针
实测），只要其中任意两次发生在"持引用时切走"的路径上，它就再也走不掉。

**这是一条纪律缺口，不是一个孤立的 bug**：`Task::exclusive` 的注记管的是"临时持有者不解引用
字段"（别名安全），**没人管"能不能跨切换持有"**（生命周期安全）。

#### 本轮落地：判据先有名字

新增 audit 观测量 `[audit] roster N alive M`（`Weak::strong_count()` 数活口——**只读、不升强
引用**，观测不改被观测的事实），门里立判据 **`alive == 0`**。实测 `roster 8 alive 1` ⇒ audit 轮的
原因串现在直接写出 **`名册活任务[1]`**，不再只有"29 frames, 15 blocks"那种只说现象的数。

#### 修法（待裁，三条）

1. **纪律 + 逐点收口（推荐）**：把"**跨切换不得持强引用**"写成显式纪律，并逐点收口可能切换的
   路径（envcall 的 `running_task()` 克隆在 `park`/`wait`/`join`/`quit` 之前必须 drop；`block`
   之前的 `Handoff` 计算不得残留克隆）。**可验**：`alive == 0` 就是判据，且上面的扫描工具可复查。
2. **退场路径显式收尾**：让 `quit` 走一条"先把可控引用放掉、再切"的窄尾（`#[inline(never)]` 的
   极小函数），把"切走时栈上没有活克隆"变成结构而非纪律。代价：编译器仍可能把克隆留在
   callee-saved 寄存器里，纪律无法完全消掉 ⇒ 需要与 1 合用。
3. **躯壳回收时"擦栈"**：`bury` 归还栈帧前把栈页清零 —— **不解决**问题（清零不递减计数），
   只是把证据擦掉。**否**。

工具代码（约 40 行）本轮用完已撤出树（探针纪律：不留半成品），需要时按上面两步骤重建即可。

### 10.13 泄漏线收口（修法 1+2 落地）：退场窄尾 + 跨挂起不留强引用

§10.12 定位到机制后，用户裁「1 + 2」都做。两处改动，各自对应机制的一半：

1. **退场窄尾（结构）**：原来自退是在 `envcall::dispatch` 里调 `quit()` ⇒ **dispatch 的整帧被
   `restore` 丢掉**，帧里全部临时值的引用计数随之永不回落。改成 `dispatch` 只**交回标记**
   （`Option<*mut TrapContext>`，`None` = 本任务退场），退场由 `trap_handler` 在最浅的 Rust 帧里
   做——此刻 `dispatch` 的帧**已正常归还**，`trap_handler` 手里只有 frame 与几个标量（`ident`
   早已移交给 dispatch）。三条 kill 路径本来就已在 `trap_handler` 里退场，未受影响。
2. **跨挂起不留强引用（纪律）**：`messenger::block` 入队成功后**显式 `drop(task)`**——队列里那份
   才是权威持有者。留着它不影响"正常唤醒"（帧会恢复、局部量照常 drop），但会在**被别核 kill**
   时随栈一起被丢弃（栈没了，计数永不回落）。纪律写进注释：**跨挂起/退场不得持强引用** ——
   `Task::exclusive` 的注记管的是"临时持有者不解引用字段"（别名安全），这一条补的是生命周期安全。

**判据（全门实测）**：

| | 改前 | 改后（连跑 4 轮一致） |
|---|---|---|
| `[audit] roster N alive M` | `8 alive 1` | **`8 alive 0`** |
| `[audit] task lifecycle leak …` | `29 frames, 15 blocks` | **消失**（不再出现） |
| `[audit] table frames != kernel-walk` | `150 != 141` | **消失**（确认与上面同源） |
| audit 轮 FAIL 原因 | 三条（名册活任务 + 泄漏 + table） | **一条**：`死键站点[1]` |
| 默认档控制台归一化 md5 | `183133960…` | 不变（行为无变化） |
| harden 档 | PASS | PASS |

**新暴露的一条（更小、且是被判据抓出来的）**：`死键站点[1]`（`sites 1 live 0 tomb 1 orphan 0
dead 1`，`by kind: space 1`）——**一个 `WakeKey::Space` 站点，它的空间已经死了却还在表里**。
成因清楚：`wipe` 的三个调用点（`HoleMeta::drop` / `hole::seal` / `bury`）分别覆盖 hole 键与
task 键，**空间键没有任何退役调用点**；而 `prune` 只在"那个键再次被碰到"时才会跑 ⇒ 空间死了
就再没人碰它 ⇒ 站点永留。改前它被更大的泄漏遮住（那时空间还活着，所以它算 `tomb` 不算 `dead`）；
修法 1+2 让空间真的死了，这条才露出来——**这正是 `dead == 0` 这条判据存在的意义**。

**处置（待裁）**：给空间键补一条退役路径——`bury` 在归还栈/帧之前，若 `Arc::strong_count(&space) == 1`
（唯一持有者就是这个将死的任务 ⇒ 空间随之而亡），就按 asid 扫站点分片删掉它的 `Space{space: asid, ..}`
站点（新原语 `wipe_space(asid)`，与 `wipe(key)` 同一族）。用强计数当判据的理由与仓里既有做法一致
（"唯一强持有"）；不用 `Drop` 回调的理由见 A2（死亡靠 `Weak` 观察，不靠回调）。

### 10.14 空间键退役：`wipe_space(asid)` —— **audit 轮首次转绿**

§10.13 收口后只剩一条判据红着：`死键站点[1]`（`by kind: space 1`）。成因与处置都已在 §10.13
写清，本轮落地：

- **新原语 `wipe_space(asid)`**（与 `wipe(key)` 同族）：删掉该空间名下的**全部**空间键站点，
  放行其等待者（`void(ticket)` 消音到点 + `rise` 放回就绪）。遍历全部分片、**逐片取放**
  （绝不持跨片锁）、锁外 drop（`Arc<Task>` 的 drop 链会取 L2）——与 `wipe` 同一套锁纪律。
- **触发点 = `bury`**：`Arc::strong_count(&z.ident.team.space) == 1` ⇒ 这个壳是空间的最后
  一份持有者，`drop(z)` 之后空间才真死 ⇒ 趁 asid 还在手上，把它名下的空间键站点一起退役。
  判据用"唯一强持有"与仓里既有做法一致；**不用 `Drop` 回调**的理由见 A2（死亡靠 `Weak` 观察，
  不靠回调——那正是墓碑的来源）。
- 为什么空间键此前没有退役面：hole 键（`HoleMeta::drop` / `hole::seal`）与 task 键（`bury`）
  各有自己的退役调用点，**空间键没有**；而 `prune` 只在"那个键再次被碰到"时跑，空间死了就
  再没人碰它 ⇒ 站点永留。它此前被更大的泄漏遮住（空间还活着时算 `tomb`，不算 `dead`）。

**判据（首次全绿）**：

| | §10.13 之后 | 本轮 |
|---|---|---|
| `[audit] sites …` | `1 live 0 tomb 1 orphan 0 dead 1` | **`0 live 0 tomb 0 orphan 0 dead 0 waiters 0`**（四项全零） |
| `[audit] roster` | `8 alive 0` | `8 alive 0` |
| audit 轮 | FAIL：死键站点[1] | **PASS**（自退 + 无 panic + 十步全过 + 13 marker 齐） |
| 反向验证 | — | 去掉 `wipe_space` 调用 ⇒ 原样回到 `sites 1 / tomb 1 / dead 1` ⇒ 判红 `死键站点[1]` |

**至此 §9.3 起记的那条主线违规全部消掉**：`task lifecycle leak at shutdown`（曾经 19 frames/9
blocks → 29/15）、`table frames != kernel-walk`、`名册活任务[1]`、`死键站点[1]` 四样一起归零。
一路的账：A2（站点寿命＝资源寿命，墓碑 34→0）→ 轮③（名册合一，blocks 9→5）→ 轮④（观察者只读
判别式 + kill 路径覆盖）→ 轮②（harden 档把断言与 lockdep 放回被测产物；`dead == 0` 判据）→
§10.12/§10.13（退场窄尾 + 跨挂起纪律，alive 1→0、泄漏行消失）→ 本节（空间键退役，死键站点归零）。

### 10.15 `scheduler/core.rs` 拆刀 + 正名（用户裁决：乙 + 丁）

§9.3 目录图里挂着的那句「core.rs 的 §7.4 拆分另刀」，本轮落地。**拆的判据是接缝，不是行数**
（§7.6 的方法论）：读完之后找出三条接缝，其中两条是「同一份知识抄了多遍」，一条是
「核心策略长在适配面里」——后者正是本仓核心/适配分离纪律的直接违规。

#### 三条接缝

| # | 接缝 | 事实 | 处置 |
|---|---|---|---|
| S1 | **核心策略在适配面里** | `core.rs` 头注自称「纯功能，无适配代码」，而时间片决策（预算判定 `ticks_left > 1 \|\| starved_is_empty()` / 续跑递减 / 轮转 / `Switch` 事件，27 行）整段在 `trap.rs`——能写出来只因 `inner` 是 `pub(super)`（该文件 5 处伸手） | 收成 `Scheduler::advance() -> Option<usize>`（None = 槽空，交取活）与 `core::fetch()`（取活循环 + 其内部的 WFI 步骤 `wait`）。`trap.rs` **69 → 19 行**，只剩 `hush()` + 二选一转发 |
| S2 | **计数协议手抄三遍** | `seat`(:239-253) / `shed`(:290-296) / `clear_slot`(:307-317) 各写一份「swap 取旧指针 → 判 bit0 → `from_raw` 归还」，两份逐字相同 | 抽成 `Badge`：三个写点 `seat` / `shed` / `clear`，回收只有一处 `reclaim`；带标签指针这套协议只活在 `ident.rs`（做法同 §10.7 的 `starved_len` 收口：**不变量进类型**） |
| S3 | **「帧必有 PA」重述 6 遍** | `.frame.pa.expect("frame span has pa")` 全树 6 处**全在调度器**（core 5 + trap 1）。真相是 `space/salvage.rs:32` 的一句注释（trap 帧恒 `Some`；栈/懒区恒 `None`） | 收成一个 `frame_pa(&TaskIdent) -> PhysAddr` |

#### 结构（裁决 ①＝乙：嵌套，外部路径零改）

```
scheduler/
├── mod.rs            薄壳 + 术语表（28 → 32）
├── core/mod.rs       薄壳 + 重导出（32）
├── core/hart.rs      Scheduler / SchedulerInner / 容器四改点 / seat / swap / starve / advance（340）
├── core/ident.rs     Badge / Identity / LastIdent / ident()（193）
├── core/table.rs     SCHEDULERS / current / rip / launch / 名册 / 全机扫描（176）
├── core/fetch.rs     steal / wait / fetch（139）
├── boot.rs           入口面（34，未动）
└── trap.rs           入口面（69 → 19）
```

- `core/mod.rs` 重导出 ⇒ **22 处 `scheduler::core::X` 引用里只有两处因正名而改**
  （`push` → `launch`、`Current` → `Identity`），路径本身一行未改。
- 跨到 `scheduler` 一级的条目取 `pub(in super::super)`——与
  `messenger/wait/{site,holder}.rs` 同一条纪律（那里也写着「刚好到 `messenger`，不放宽到
  `pub(crate)`」）。核心里只自用的条目降到 `pub(super)` 或私有：`inner` / `rotate` /
  `starved_is_empty` / `starved_push` / `starved_pop` **不再对入口面开放**（S1 的结构性保证：
  `trap.rs` 现在连 `inner` 都够不着）。
- 拆开后总行数 **743 + 69 → 899**（+87）：全是按文件重分的头注、`Badge` 的文档与 `advance`
  的文档；**代码本体零增益**——这不是「拆了就短」，是「拆了才看得见那三条」。

#### 命名（裁决 ③④⑤⑥⑦）

| 项 | 裁决 | 落地 |
|---|---|---|
| `wait()`（WFI 取活入口）与冻结表 `wait`/`wake` 撞车（§10.2 记的「待裁」） | **不动** | 采丁之后 `wait` 降为 `core/fetch.rs` **内部私有**步骤，对外入口是 `fetch` ——**同一个词不再有两个意思**，撞车结构性消解（不是靠改名躲开） |
| `core::push`（自由函数）/ `Scheduler::push`（容器动词）/ `starved_push`（私有改点）三义 | 自由函数正名 | `launch(task)`：名字说它独有的事（新任务出现 → 入就绪 + **踢醒一个休眠核**）；容器动词 `push` 与私有四改点不动 |
| `get_len` / `set_len` | 按推荐 | `backlog()`（锁外预检：还有多少活）/ `recount(&inner)`（按事实重清点——原先的名字说它「设」长度，与事实相反；`get_` 也是全树唯一一处此前缀） |
| `Current` / `Current::Last`（当前=末次，自相矛盾） | 按推荐 | 枚举正名 `Identity`（`Live` / `Last` 不变）；`LastIdent` 类型与 `LAST_TAG` 常量不动 |
| `TaskState::Starved` 只覆盖四条进入路径里的一条（放行 / 轮转 / 主动让出 / 唤醒） | **不动** | 只把「预算耗尽」那句头注改成四条路径的事实（改名字要连门脚本的正控串 `scripts/examine.nu:200` 一起动，收益不值；定义在 `unit/task.rs`，本就越界一处） |

#### 顺带

- `starved_remove` 由「收下标」改成「收目标」：就绪队列对 `hart.rs` 之外私有，「找 + 摘」
  一起留在队列的主人家，`table.rs` 的全机扫描不必看队列内部。
- `ident()` 与本核取用**合一**：原先 `current()` 走 tp 直达、`ident()` 走
  `schedulers()[hart_id()]` 索引（两条件不同：后者有 boot 前兜底），而 `current()` 的头注
  却写「替代那三步」——名不副实。现在 `ident()` = 查表兜底 + `current().badge.read()`：
  唯一一条取本核路径（`boot::init` 先填每核直达指针、再发布表 ⇒ **表在即指针在**）。
- `lock/depend.rs` 的 L3 清单里 `scheduler.by_id` 早已不存在（轮③ 名册合一）⇒ 改成名册。
- `docs/dispatch.md:172` 的 `scheduler::core::snap()` 同理 ⇒ 改成 `roster()`（旧名括注）。

#### 判据

| | 值 |
|---|---|
| **examine** | **5/5** —— 默认 3/3 PASS；audit 轮 PASS（`sites 0 live 0 tomb 0 orphan 0 dead 0 waiters 0` + `roster 8 alive 0`，§10.14 的四零态保持）；harden 轮 PASS（无 `[depend]`，正控串 1 次 / release 0 次） |
| **行为零变化** | 默认档三轮控制台归一化 md5 = `183133960fd82a1ff0f4a4c3f8863355`（A2 基线，逐字节不变）。这是本次唯一的「纯搬家 + 收口」判据：**结构动了、字节没动** |
| 构建 | `cargo fmt --check` 干净；`cargo check --workspace --all-targets` 警告数与基线一致（11 条 + 2 条汇总行）；audit / harden 两档各自单编通过 |

#### §7.4 计划的处置（记账）

§7.4 当年那份四刀切法里，`hart.rs` / `ident.rs` / `table.rs` 三个名字沿用；`vital.rs` **否**
（`vital`＝生命，与「取活」不相干）⇒ 改 `fetch.rs`。更要紧的是那份计划的原动机已经消失：
它说「独立出 `table.rs` 会逼出『一张表还是 N 张』这个决定（§A3）」——A3 已随轮③ 落地
（名册只有一张），§10.6 当时就记下「已有实测答案：N 张完全相同」，故本轮 `table.rs` 的存在
理由换成现行的那条：**名册与全机扫描（`remove_from_starved` / `running_hart` / `rip`）都需要
「全世界的核」，与表同居一处**。

#### 仍未做（记账，不是遗漏）

- `starve()`（主动让出）与 `advance()` 的轮转分支是同一条「rotate → 放锁 → seat」，差别只有
  trace 事件（`Starve` vs `Switch`）与「无视预算」这一条语义。合并要加一个事件参数 ⇒ 属接口
  变更，本轮不动。
- S3 的**根治**是类型化（`TaskIdent.frame` 用恒有 `pa` 的 span 类型），落在 `unit/space`，
  越界；调度器内只做了「说一次」。
- `Badge::read` 目前私有（唯一读者 `ident()`）。将来若要第二读者，先裁再开——
  本轮刻意没把它放进 `core/mod.rs` 的重导出。

### 10.16 逐对象种类记账（用户裁决：甲 / 直接对应 / 收进种类 / 帧来源＝闭包）

上一关（§9.1-2 `fence` 终点）勘察完之后，用户提了另一条更靠前的问题——「内核的分配对象
已经稳定，可以考虑逐对象记账？」。裁下来是：**对象种类成为记账的唯一维度**（甲）、名字
**直接对应**、键域与多页粒度**收进种类**、帧来源用**闭包参数**（`frame(kind)` 被否）。

#### 一个维度取代两个枚举

`Class`（4 值："关机时怎么核账"）与 `OwnerKind`（2 值：毒化策略 + 账键域）本是一个维度的
碎片——`OwnerKind::KernelHeap` 恒等于"账本侧 + 地址键"。于是"这是什么"在代码里**无处安放**，
三条实证：

| 实证 | 位置 |
|---|---|
| 一个标注点盖**五种对象**（trap 帧 / 懒页 / 堆页 / 栈 / COW）——注释只列了四种 | `SpaceInner::frame()`（自述"全模块唯一帧分配点"） |
| 自检数据帧只能假标 `Persistent`（生命周期维度里没有它的位置） | `health/pagetable.rs:40` |
| `spare` / `trap-stack` / `hart-frame` 只能靠**旁路字符串名**区分 | `register_persistent(pa, "…")` |

#### 结构：`Kind` 16 种，属性由种类自带

| kind | 侧 | 键 | end | 落点 |
|---|---|---|---|---|
| `Trap` `Lazy` `Heap` `Stack` | 帧 | 地址 | Zero | `window/frame`、`core::materialize`、`window/heap`、`window/stack`（`Cow` 已删，§10.20） |
| `Image` | 帧 | 地址 | Zero | `loader` |
| `Ring` | 帧 | 地址 | Zero | `mail/pole` |
| `Table` | 帧 | 地址 | Walk | `manager/table` |
| `TrapStack` `HartFrame` `Spare` | 帧 | 地址 | Held | `trap/stack`、`unit/mod`、`spare` |
| `Prime` `Probe` | 帧 | 地址 | Report | `block::prime`、`health/pagetable` |
| `Task` | 账 | 地址 | Zero | `task`（`tagged_alloc`） |
| `UserHeap` | 账 | 页索引 | Retire | `envcall`（键 = asid+页索引） |
| `Plain` | — | 地址 | Report | 未标注：容器增长 / 大块直取 |

四条属性：`side()`（帧表 / 账本 / 未标注）· `keys()`（地址 / 页索引）· `end()`（期望终值）·
`poison()`（**派生**：账本侧 ∧ 地址键——它就是 `OwnerKind` 原先的全部内容）。
存储代价≈0：per-page 仍是 1 byte（值域 4 → 16）、ledger 记录**少**一个字段、计数数组 4 → 16。

#### 帧不认识种类（用户否决 `frame(kind)`）

我第一版把种类做成参数（`frame(kind)`），被否——理由与模块头自己的设计原则一致：
**资源原语不携带对象语义**。改成：`SpaceInner::frame()` 回到"裸零化帧"，`claim` 的帧来源
**由调用者以闭包给出**，而这不是新机制——**`attach` 早就是这个形状**，且它的调用者本来就
是标注点（`loader` 的 `tag!(Image, …)`、`unit/mod` 的 `tag!(HartFrame, …)`）：

```rust
inner.claim(va, PAGE_SIZE, flags, || Ok(crate::tag!(Trap, SpaceInner::frame()?)))
```

| 装配动作 | 帧来源 |
|---|---|
| `attach` | 调用者给（一直如此） |
| `claim` | **调用者给**（服务 trap 帧 / 堆页 / 栈三种对象） |
| `materialize` | 就地给（只服务懒页一种对象） |

闭包**按页**调用 ⇒ 多页对象（栈 / 堆 span / Pole 环）每页都带种类，与"逐页收入"的裁决一致。
被否的第二案（装配完成后按 PA 事后标注）理由记下：多页路径要在**产品档**多走 N 次
translate（栈一次 64 页），且"标注"与"造对象"分家 ⇒ 漏标就静默落 `Plain`。

#### 关机判据：从 4 类变逐种类终值

报告从 `task lifecycle leak at shutdown: 19 frames, 9 blocks` 变成**点名**：

```
[audit] leak: ring 1                                   ← End::Zero 组逐种类（有才打）
[audit] persistent spare @0x83fea000 is tagged ring …   ← Held 组：声明种类须与帧表一致
[audit] shutdown checks ok: zero 8/8 held 6/6 tables 141/141 report-only prime 14 probe 0 plain 31 user-heap 0 pool 15
```

`Held` 组保留**逐项**核 held（不用"按种类计数 vs boot 基线"：计数对"等量还借"不敏感），
且登记表的字符串删掉——名字从种类来，声明即受核。

#### 新牙（连带堵住一类旧事故）

`tag()` 不再用"在不在账本里"**猜**种类落哪张表，而是按 `Kind::side()` **核对**，不符报
`IntegrityViolation::MisplacedKind`（新追加类目，`repr(u8)` 顺序即 ABI）——**覆盖 docs
记过的那次 FRAME_CLASS 污染**（块级 Arc 误用数据指针）。另得一条：`on_free` 时"说的种类"
必须与账上一致（旧版传错无人发现）。同类重复标注改为幂等（旧版会再 relabel 一次，把计数
多搬一遍）。

#### 判据

| | 值 |
|---|---|
| examine | **5/5**：默认 3/3 + audit 轮 PASS（`zero 8/8 held 6/6 tables 141/141`、无 leak 行、站点四零态保持）+ harden 轮 PASS（无 lockdep） |
| 行为零变化 | 默认档三轮归一化 md5 = `183133960fd82a1ff0f4a4c3f8863355`（A2 基线，逐字节不变） |
| **反向验证** | 把 spare 仓错标成 `Ring`（一个零终值种类）⇒ 关机如实报 `[audit] leak: ring 1` + `persistent spare … tagged ring`，门判红 `关机终值违约[ring 1]`；还原即复绿 |
| **新牙自证** | 本轮自己的一个 bug 被当场抓住：`on_alloc` 起初没把传入种类写进账本（用户堆记录落成 `Plain`）⇒ 释放时 `MisplacedKind: free: said UserHeap, ledger says Plain` ⇒ 修复即绿。**这正是旧版"传错无人发现"的那条** |
| 构建 | `cargo fmt --check` 干净；三档警告数与基线一致（默认 7 条死码 + 4 条 manifest） |

#### 顺带

- `hybrid` 大块路径的 `tag(.., Plain)` **删除**：那是一次空转（`relabel(Persistent, Persistent)`
  成对抵消）——"标注"什么都没做，留着就是飞线。
- `statistics`：`classes` → `kinds`；`record_block_{take,give}_for_class` → `record_block_{take,give}`；
  池维度改名 `record_pool_{take,give}`（与帧侧同形）。
- `examine.nu` 的 FAIL 归因分支改读新格式（`[audit] leak: <kind> N`）。
- 用户手工正名：`SpaceInner::materialize_map` → `materialize`（适配面仍名 `materialize_map`）。

#### 块侧的实况（诚实记账，不是遗漏）

产品档里 **`plain 31`**：31 个未标注的帧/块活到关机——内核自身的长期结构（roster、messenger
四表、gate 表、console/trace 缓冲、timer 堆…）。它们**能标但还没标**；而**容器增长**
（`Vec::push` / `HashMap` 扩容）结构上标不了：`GlobalAlloc` 只拿到 `(size, align)`。
故块侧的"逐对象"本轮只做到能标的本体 + `Plain` 兜底。另一半（`ledger.Record.site` 早就存着
分配点返回地址、从没拿来做账）仍是一条可走的路——它回答"在哪"而不是"是什么"，
两件事正交。**「块侧全覆盖」单列成项目**，前提是先把内核自身长期结构逐个声明（约 14 个族）。

#### 与 §9.1-2（`fence` 终点）的关系

per-page 那 1 byte 从"配额分类"变成**对象身份**之后，「在不在手」（`pagemeta` / banker）与
「是什么」（种类）各有唯一真相 ⇒ `banker` 存废那一关的 (i) 方案（吸收进单一真相）
**内容已就位**，只剩"要不要保留独立第二账"这一个问题。另有一条已实测的事实待裁：
**产品档（default）里一条护栏串都没有**（audit 带 banker/ledger、harden 带 checker/lockdep，
没有任何一档同时带两半），而 `fence/mod.rs` 的头注写着"内嵌在生产路径"——那句已按实测改写，
但它指向的"哪一档才是验收构造"仍待裁决。

### 10.17 `fence` 终点第一刀：三档合成（用户裁决「先做第二件」）

先量化再裁。`cargo build -p kernel --profile harden --features audit` **建得出来、跑得绿**，
四串同时在场：

| 档 | banker | ledger | checker | lockdep | 实测 |
|---|---|---|---|---|---|
| default（产品档） | ✗ | ✗ | ✗ | ✗ | 四条串全 0 |
| audit | ✓ | ✓ | ✗ | ✗ | `debit on already-held page` / `unmark: no record` |
| harden（旧） | ✗ | ✗ | ✓ | ✓ | `allocated non-free frame` / `lock-order level violation` |
| **harden + audit（新）** | **✓** | **✓** | **✓** | **✓** | 四串齐；ELF 7899048 B（纯 harden 7433648 B，+6%） |

于是 §10.16 记下的那个病——「帧分配器的同一条不变量有两份实现，而**没有任何一档同时带着
两半**」——直接消掉：融合档里 banker 的每页位与 `pagemeta` 的链式检查**同档同跑**，
两个时刻的计数交叉核对（`banker held == frame.occupied`）也因此第一次真正有意义。

**门侧三处**（都是判据自身的修正，不动内核）：

| # | 改动 | 为什么 |
|---|---|---|
| 1 | harden 档构建参数 `$DEFAULT_FEATURES` → `$features` | 融合档的入口 |
| 2 | 哨兵「默认档不该有 audit 输出」**收窄到 `flavor == "default"`** | 原先对一切非 audit 档生效 ⇒ 融合档必红（实测那次 FAIL 就是这条，**不是内核问题**：同轮 `[depend]` 无违规、audit 报告完整） |
| 3 | 正向对照 1 串 → **4 串**（`HARDEN_PROBES`：断言 / banker / ledger / lockdep），少任何一串即退出 | "两半同档"从此是门**每次都要核**的事实，不再是一句声明；旧版只查一句断言串，所以"两半从不在一起"一直没被门盯住 |

**判据**：全门 **5/5**（默认 3/3 + audit 轮 PASS + 融合 harden 轮 PASS 无 `[depend]`）；
默认档三轮 md5 = `183133960fd82a1ff0f4a4c3f8863355`（A2 基线，逐字节不变）。
全门调用：`EXAMINE_FEATURES=audit EXAMINE_HARDEN=1 scripts/examine.nu`。

#### 顺带抓到的**新泄漏**：`[audit] leak: task 1`（间歇）

融合档实验的第一轮里，audit 轮 FAIL，原因是本轮的逐对象判据：

```
[audit] sites 0 live 0 tomb 0 orphan 0 dead 0 waiters 0
[audit] roster 8 alive 0
[audit] leak: task 1
```

`Task` 是**账本侧**种类 ⇒ 泄漏物是一个 `Arc<Task>` / `TaskIdent` **块**（不是帧）。
同一轮：站点四零态、名册 `alive 0`。**名册看不见它**——名册只存 `Weak<Task>`，
不存 `Weak<TaskIdent>`：一个 `Arc<TaskIdent>` 可以在 `Task` 本体已回收之后继续钉住那个块。
这正是逐对象记账多出来的那一维所看见的东西。

复跑 5 轮 audit 全绿（另加融合档那轮的 audit 轮）⇒ **间歇**（约 1/8）。旧判据（`task_blocks`）
理论上也能看见它（会报"0 frames, N blocks"），但它从未在最近几十轮里出现——现在它有了名字。

**诊断它需要什么**：账本记录里的 `site`（分配点返回地址）**早就在存**，却从没拿来做账
（§10.16 记的"另一半"）。所以下一步是明确的：泄漏时按种类把 `site` 打出来，host 侧
`addr2line` 符号化 ⇒ 直接指到"哪个分配点漏了一份强引用"。这与 §10.12 那条线的机制
（退场是切走、不是退栈 ⇒ 被丢弃帧里的强引用永远不还）同源，故很可能是同一族里的第三处。

#### 残题（比上一次窄了很多）

- **(i) 删 `banker`**：融合档之后它的存在理由从"唯一能查每页在不在手的账"变成"独立第二账"
  ——两份独立实现同档同跑正是审计的价值。要删，理由是"pagemeta 已是唯一真相"；要留，
  理由是"独立验证"。**裁决点从三条收成一条**。
- **(ii) 接受融合档为唯一验收构造**：现在只剩"要不要真的只留融合档 + 产品档两档"
  （audit 轮作为"无断言但有记账"的中间档是否还值得单独跑）。

### 10.18 删 `banker`（用户裁决）—— 帧侧只剩一份账，两半护栏同档第一次真的咬到东西

§10.17 之后残题只剩一条：`banker` 留作独立第二账，还是删。裁决是**删**。

#### 删了什么、覆盖去哪了

`banker`（110 行无锁原子位图，free 区每页 1 bit）回答的是两件事，删后各有接位者——
**都在 `frame::pagemeta` 这一份真相上**：

| banker 的检查 | 接位 | 位置 |
|---|---|---|
| `debit`：取出已 held 页（双取出） | `checker::check_frame_free`（弹链出的帧在 pagemeta 里必须 free） | `frame::pop_link` |
| `credit`：存入陌生页（双释放） | **新** `checker::check_frame_held`（释放的帧在 pagemeta 里必须 held） | `frame::deallocate` |
| `held_count()`（③ 用） | `statistics::view_frame().occupied`（同一量，一份账） | `audit.rs` |
| `is_held(pa)`（② 与账本落页核验） | **新** `frame::is_held(pa)`（pagemeta 读侧，取 Frame 锁） | `frame.rs` |
| `banker held == frame.occupied`（⑤ 交叉核对） | **删除**——那是两份账记同一件事；一份账无所谓"交叉" | — |

删掉的是 **105 行文件** + 一处 `init` + 两条检查；fence 目录 1838 → 1733 行。

#### 门分两档：O(1) 进 audit 档，O(链长) 只留 debug 档

删 banker **不能**只删了事：audit 档（release）里 `checker` 的函数体原本整段 `debug-gated`
⇒ 不补的话"每次取还的核对"会**从有变没**（覆盖率不升反降）。第一版把 `checker` 的检查
**整片**扩到 `any(debug_assertions, feature = "audit")`，实测踩坑：`check_not_in_chain` /
`check_in_chain` 是 **O(链长)** 遍历，而 `walk_chain` 的 `1<<14` "成环"上限是按**块链**
长度定的——帧链可以更长 ⇒ audit 档把合法的长链假报成环并当场 panic（控制台 58 MB 刷屏）。

于是按**代价**分档：

| 检查 | 代价 | 档 |
|---|---|---|
| `check_dram_addr` / `check_bounds` / `check_frame_free` / `check_frame_held` | O(1) / O(order) | debug **与** audit |
| `check_not_in_chain` / `check_in_chain`（+ `walk_chain`/`dump_chain`）/ `log_*` | O(链长) / 纯观测 | 只 debug |

**融合档（debug + audit）是这个划分成立的前提**：链式遍历在那里照样跑（§10.17）。

#### 两条新检查当场咬到的三处真问题（都是本轮自己的）

1. **`poison()` 的派生漏了 `Plain`**：`Plain` 的 `side()` 是 `None`，而 `poison` 写成
   `side == Ledger ∧ keys == Addr` ⇒ 未标注的**内核堆**块被判成"非内核堆" ⇒ boot 三源核对
   假报 `user-heap record VA on non-held page`。修法：`keys == Addr ∧ side != Frame`
   （账本里未标注的记录恒是内核堆块——用户堆一律标 `UserHeap`）。
2. **用户堆记录本来就不该问帧侧**：它的键是 `(asid, 页索引)`，不是地址。旧代码在这里问
   `banker`，那会撞上 `banker.idx` 的范围断言——只因为关机前用户堆账已被 `retire` 清空，
   那个分支**从未被走到**（化石分支）。种类分开后这里不再假装能查。
3. **`held()` 的"对齐命中 ≠ 包含"**：按 order 从大到小找块首时，`index & !(2^power-1)`
   命中一个块基址**不等于**那个块包含该页（同址可能站着另一个 order 的块）⇒ index 5004
   撞上"从 0 起的 4096 页空闲块" ⇒ 把合法释放报成 `freeing non-held frame`。修法：用表项
   自带的 power 复核覆盖关系。

#### 融合档咬到的第四处：一条**新锁序边**（本轮最有价值的一条）

删掉 banker 之后 `audit()` 的账本核验改成 `frame::is_held`（**取 Frame 锁=L6**），而它是在
`LEDGER.for_each`（**Ledger 锁=L8**）的闭包里调的 ⇒ **持高取低** ⇒ `lock/depend.rs` 的
`lock-order level violation` ⇒ `report()` ⇒ panic ⇒ 其后的报错现场把控制台刷成一片
`trap stack overflow`（首次跑到的现象是"第一发用户缺页落在无 running 任务的核上"，
那是 panic 级联，不是独立缺陷——修完即消失）。

**这条边只有融合档看得见**：audit 档是 release（lockdep 是 `debug_assertions` 门控）⇒
装作没看见。这正是 §10.17 把两半合成一档的**直接证据**。修法：锁内只把地址抄进**预先
分配好**的缓冲（锁内零分配纪律照旧），放锁后再问帧分配器。

#### 判据

| | 值 |
|---|---|
| examine | **5/5**（默认 3/3 + audit 轮 PASS + 融合 harden 轮 PASS 无 lockdep） |
| 行为零变化 | 默认档三轮归一化 md5 = `183133960fd82a1ff0f4a4c3f8863355`（A2 基线，逐字节不变） |
| 串普查 | `debit on already-held page` 三档**全 0**（banker 的报文体随文件一起消失）；audit 档现在带 ledger 的 `unmark: no record` + checker 的两条（`allocated non-free frame` / `freeing non-held frame`）；融合档再加 `lock-order level violation` |
| 反向验证 | 上面第 3 条即是：把 `held()` 写成错的（对齐命中就当包含）⇒ `freeing non-held frame` 当场报 ⇒ 修好即绿 |
| 警告 | 三档与基线一致（默认 `cargo check` 7 / audit 1 / harden 7）。**附记**：默认 **release** 档另有 10 条死码警告（`health` / `view_*` 在 `debug_assertions` 关时不可达）——§9.0 那句"两档 0 dead-code warning"是 **dev（`cargo check`）口径**下的结论，release 口径从来不成立（实测：release 17 条，调用点与 HEAD 逐字相同 ⇒ 非本轮引入）。

## 10.19 泄漏现场取证：让「哪种对象」变成「哪一条」

§10.16 把关机判据从「帧/块各还差多少」推进到**逐种类点名**（`[audit] leak: task 1`），
方向是对的，但那条报告只说得出**哪一类**没归零，说不出**哪一条分配点、哪一个对象**。
而它抓到的那条既存违规（audit 档的 `leak: task 1`）恰好是**间歇**的：二十余轮里只见过一次
（`trace/fuse-r1/run1`，seed 336576081），随后连记录在案的种子都复现不出来。间歇 +
名字不详 = 既定位不了也修不了。

本轮的处置是**补上那两级信息**，而不是继续盲扫。

### 加了什么

`fence::audit::dump_records(kind)`——挂点就在那条报告的下一行（`check_baseline` 里
`if n != 0` 的分支内，`report` 之前）：

```
[audit] leak: task 1
[audit]   task @ 0x8479f200 size 48 site 0x8023fe60
[audit]     <- TaskIdent: strong 1, id 7
[audit]     name "u-thread"
```

**它补的量**：记录**地址**、**尺寸**、分配点 **`site`**（host `addr2line` 可符号化
——`alloc_site` 的回溯栈早就把它存进账本了，只是从来没人读），以及 `Kind::Task` 记录
的**对象身份**。

`Kind::Task` 的记录就是 `ArcInner<{Task, TaskIdent}>`（两种都经
`tagged_alloc(Kind::Task)` 标注，尺寸不同故可分辨）：头 16 字节是 strong/weak 计数、+16
起是载荷。`Task` 的载荷首字是指向 `TaskIdent` 的 `Arc` 数据指针；`TaskIdent` 的载荷首字
是 `id`、次字是 `name`（`&'static str` 二元组 = 指针 + 长度）。于是**名字**能直接打出来
——名册（`roster_live`）只说"还有条目活着"，本条说得出"是哪一个"。`name` 只在指针落在内核
镜像内（`_kernel_start`..`_kernel_edge`，新加的两个读法 `image_base()` / `image_edge()`）时
才读，且是只读诊断。

### 三条纪律（都不新造机制，只是把已有的约束在这条路上重申一次）

1. **不加判据**。它只打印：不改任何计数、不写任何表、不参与 `report` 与否。泄漏的判定
   仍然只有一条（`End::Zero` 逐种类归零）。
2. **锁纪律**：本条要两次问账本（先抄记录、再按地址指认内层 `TaskIdent`），而
   `LEDGER.for_each` 是**持有** Ledger 锁的遍历 ⇒ 嵌套调用即自锁死（`ledger::retire` 的头注
   记过同一个坑）。故先抄进**预先分配好**的缓冲（锁内零分配），放锁后再互相指认。
3. **有上限**：最多打印 64 条（超了打一行 `... N records, first 64:`）。取证输出不许自己
   变成刷屏源——同类事故在本文件里已实证过一次（58 MB 控制台，§10.18）。

### 判据

| | 值 |
|---|---|
| examine | **5/5**（默认 3/3 + audit 轮 PASS + 融合 harden 轮 PASS 无 lockdep） |
| 行为零变化 | 默认档三轮归一化 md5 = `183133960fd82a1ff0f4a4c3f8863355`（A2 基线，逐字节不变）；**audit 轮**归一化 md5 = `efd03cb45e01a4acefef90a34ba32735`，与 §10.18 那一轮的基线**逐字节相同** |
| 串普查 | 两轮 `[audit]` 行数同为 10、词汇表 `diff` 完全一致——即本轮在**没有泄漏的关机**上**一个新字符串都不产生**（这是"只在已判泄漏时打印"的直接证据，不是承诺） |
| 正向对照 | 临时把泄漏分支短路（每种有记录的 kind 都无条件走一遍转储）后单跑一轮：转储打出 **32 条**，同帧的关机行是 `plain 32` ⇒ **账本侧记录数与统计侧计数对得上**；32 条**全是 `plain`**、`task` 一行没有，而同帧 `zero 8/8` ⇒ 有记录才打印、无记录不打印，两个方向都在同一次输出里成立；随手挑的 `site` 符号化正常（`0x8023fe60` → `<RawVecInner>::non_null::<(usize, TableNode)>`，`0x8021b344` → `scheduler::core::table::current`）。补丁**已还原**（`audit.rs` 与补丁前逐字节一致） |
| 警告 | 三档与基线一致（默认 `cargo check` 7 / audit 1 / harden 7；7 条全部落在 `statistics.rs`(6) 与 `diagnose/trace.rs`(1)，**无一来自本轮改动的文件**） |

#### 未验到的一处（如实记下）

**`Kind::Task` 的载荷解码一次都没跑到**——对照恰好证明了「没有记录时它就是安静的」，
而那条真泄漏在本轮二十余轮里一次都没复现（含记录在案的失败种子）。所以"尺寸 48/88
分流、`strong` 计数、`id`、`name`"这几步仍是**读代码的结论**，不是实测；要实测只能等它
现形，或人为造一条 Task 记录（那是伪造账，没做）。

同一轮尝试里还撞到一件与泄漏无关的事实：**主动压测（连续产/销任务）在 `-m 128` 下会
先把 128 MB 的内核堆吃满**（`memory allocation of 114688 bytes failed`，来自
`env::ecall::trap` 即用户堆请求），与这台机器的长跑上限有关、与本轮改动无关，未追。

## 10.20 COW 控制面删除：共享在这台机器上只有一个形态

**裁决**（用户）：`⑥ COW 控制面` 由"留 + 立刻修"改为**删**。触发它的是一个问题——
"COW 在现在的模型里能用来干什么"——逐条走完之后，答案是**没有合法用途**。

### 为什么不是"留一块备用机制"

COW 的全部价值前提是"**同一物理页先共享、之后按写分裂**"，它服务的是 fork 的
`exec 后接着跑`。逐条排查：

| 候选 | 判定 |
|---|---|
| 程序启动（新域跑同一份程序） | **不需要**：`.text` 只读 ⇒ 共享只读就够（initrd 区 + `borrow`）；`.data` 必须私有，而装载器**今天已经在拷**（`loader.rs:81` 逐页新建 + `copy_from_slice`），`.bss` 走懒零页 |
| 快照 / 回滚 | **不需要**：那是"拷贝 + 回收"，COW 只省"还没改的那部分"，是可选优化不是需求满足 |
| 内存去重 | **不是 COW**：去重的前提是只读，写不触发拷贝 ⇒ 需要的是内容寻址，COW 帮不上 |
| 事务式私有变更 | **不够用**：COW 只给"一共享 + 一私有"两版，回滚要多版 |
| 一次性克隆运行中的域 | 唯一沾边的一条，但它要求**源域停止**（否则共享一方还在写）⇒ 那就是 fork 的语义，也就是第一行的判定；而"停止 + 拷贝"今天用现有原语就能做 |

而且**共享在这台机器上已经通了，靠的不是 COW**——两条活的跨空间共享都是 `borrow`：
initrd 清单视图映射进 root 的用户段（`boot.rs:217`，帧属"持久保留区"）、pole 环缓冲映射进
每个调用方的空间（`pole.rs:109`，帧属 `HoleMeta`）。它们的共同前提是
**帧的所有者活得比所有借用者长**。这就是这个模型里共享的唯一形态：**只读借用 + 长寿所有者**；
"写共享"不是没实现，是**被放弃**（那样消息传递的隔离叙事就破了）。`borrow` 只写 PTE、
`Map.frames` 留空——它**没有位置**表达"我打算写它、写时再变私有"；要接 COW，第一件事是给共享页
补一个所有权对象，而那个对象存在的唯一理由（共享页会被写）恰恰是被否掉的那条。

### 删了什么（实测）

| 面 | 量 |
|---|---|
| `SpaceInner::share`（唯一构造 `FrameState::Shared` 的地方） | 57 行 |
| `SpaceInner::own`（**唯一调用点是那个 COW 分支** ⇒ 随之全死） | 24 行 |
| `Space::share` / `Space::is_shared` / `Space::own` 三个包装 | 32 行 |
| `fault.rs` 的 COW 分支 + `FrameState` 定义与 `pa()` | 35 行 |
| `Kind::Cow` | 1 个变体（`KIND_COUNT` 16 → 15，其余编号整体下移，**不留空洞**） |
| 合计 | **−228 / +160**（内核侧 −195/+47） |

`FrameState` 因此不再是枚举（只剩一臂），整个类型消失，`Map.frames` 直接持 `Frame`。

### 拍下去之后撞到的一件事（如实记）

一度按"把槽位语义留在类型上"改成 `Page(Frame)` newtype（用户批过这个名字），**实测付不出**：
它唯一的读点 `pa()` 只被 `SpaceInner::audit()` 调用，而后者整段是 `#[cfg(feature = "audit")]`
（`core.rs:418`，两个 boot 调用点也同门控）⇒ **默认档里那个字段没有任何读点**，直接产生两条
dead_code 警告，而"两档零警告、零 allow"是既有纪律。试过的三条补法都不通：`Deref` 服务不了
元组字段、给 `Page` 写 `Drop` 会在 `inject` 的移出处编译失败（E0509）、显式 `drop_in_place`
同样不算"读"。**故撤回该名字**（用户复裁"行"）：`Map.frames` 直接放 `Frame`，取址做成 audit 档里的
自由函数 `page_pa(&Frame)`。教训记在这里：**在这个仓里，一个只服务 audit 档的类型包装，会以
默认档 dead_code 的形式付账**。

### 判据

| | 值 |
|---|---|
| examine | **5/5**（默认 3/3 + audit 轮 + 融合 harden 轮无 lockdep） |
| 行为零变化 | 默认档三轮归一化 md5 = `183133960fd82a1ff0f4a4c3f8863355`（A2 基线）；audit 轮 = `efd03cb45e01a4acefef90a34ba32735`（与 §10.18/§10.19 基线**逐字节相同**） |
| 警告 | 三档**回到基线**：默认 7（`statistics.rs` 6 + `diagnose/trace.rs` 1）/ audit 1 / harden 1——删掉的面没有留下任何新警告 |
| `allow(dead_code)` | 少一条（`Space::share` 那条"fork 后端预留"） |
| ELF | 融合档 7 892 344 → 7 924 608 字节（含被删函数的 debug 信息） |
| 残留 | `grep -rn "Cow\|FrameState\|is_shared\|Shared"` 在内核/用户态/crates 里**零命中** |

### 明确的代价（唯一一条）

失去"同一物理页可写共享"的语义。今天**零个调用方**表达过这个需求。将来若真要（真 fork 或
快照），正确的形状是**显式共享内存对象**（命名、有主、有生命周期，像 pole 的环），
而不是让 COW 悄悄把页变私有——那时写共享是**契约**（所有方看得见），不是**意外**。
从这次删除里留下两条结论供将来复用：
1. 共享的唯一形态是**只读借用 + 所有者长寿**（`borrow` + `Map.frames` 留空）；
2. 帧归还要求 **4 KB 对齐的块基址**（`frame_index`/`merge_block`），所以任何"每页引用计数"
   的设计**都不能把计数长在帧里**——原 `Arc::new_in(…, frame::allocator())` 正是这么写的
   （数据指针 = 基址 + 16），一旦真跑起来就是帧泄漏 + `check_frame_held` 停摆；这也是它
   从没被执行过的原因。

### 顺带记录的一处现存缺口（与本次删除无关，未修）

`Mprotect` 把**私有页**收紧成只读之后，用户写它会在 `fault.rs` 走「已物化但权限不足」——
那条路今天**不处理**（判 `false` ⇒ 空间故障隔离）。删掉的 `own` 本来像是它的解药，但
`own` 在旧代码里被 `is_shared` 门控、永远够不到这条路径 ⇒ **删前删后行为一致**。
要不要让"私有只读页的写缺页"可恢复（把 W 翻回来），是一次独立裁决。

## 10.21 §9.1 收尾：协议独立成 crate + 拷贝契约 + 删 datagram

三件都不大，但各自把一颗飞线拔了。

### (1) `crates/protocol`：内核不知道目录协议，交给依赖方向保证

**裁决**（用户）："新建 protocol 目录"——不是搬进 `task/src/core`，而是独立成一个
crate `crates/protocol`（`crates/env/src/dispatch.rs` → `crates/protocol/src/dispatch.rs`）。

`crates/env` 是**内核也依赖的 ABI crate**，而目录协议（`Request`/`Reply`/`MSG_LEN`）
是**纯用户态**的东西。它住在那儿，只是让内核多编译几百行它永远读不到的协议，
并把"这是用户态的东西"这句话从结构上抹掉。独立成 crate 之后，这条事实变成**编译期
保证**：`kernel/Cargo.toml` 里没有 `protocol`，想引用也引用不到。

依赖方向：`protocol → env`（单向，用 `env::wire::{Name, NAME_LEN, PieToken}`）、
`task → protocol`。

**"零内核引用"用探针验过，两个方向都跑了**（一次性实验，验完即删）：

| | 做法 | 内核 ELF 里 `PROBE-DISPATCH-REACHED` 出现次数 |
|---|---|---|
| 正向 | kernel **不**依赖 protocol（现状） | **0** |
| 反向 | 临时给 `kernel/Cargo.toml` 加 `protocol` + `static PROBE_LINK: usize = protocol::dispatch::MSG_LEN;` | **1** |

两行加起来才说明问题：**"0"不是测不出来，是内核真的没链接它**。反向那一步用完即撤
（`kernel/Cargo.toml`/`main.rs` 已还原，探针串也已删除）。

顺带一处连带收口：`env::Name`/`NAME_LEN`/`NameError` 原先是从 `dispatch` 转口的，
现在直接由 `wire` 出（`pub use wire::{…}`）——**同一个类型不留两个出口**。
`Name::from_bytes`（线格式解码）由 `pub(crate)` 升为 `pub`：它是 `Name` 的**线格式
对偶**，语义属于 `env`，不随协议搬家。

### (2) B2：拷贝契约升级成「要么全写，要么一个字节都不动」

`mail/mod.rs` 的 `copy_out` 原本**边写边判**权限：中段缺 W 或越界时，前面几段**已经
写进用户缓冲区了**，函数才返 false ⇒ 模块头那句"（不部分写入）"是假话。今天没炸只因
所有调用方都传精确长度的缓冲——**靠调用方纪律掩盖的假契约**。

修法是抽出共用前置 `whole(space, va, len, need)`：先整段验完（每段在、权限含 `need`、
段长之和恰为 `len`）再动第一个字节。两遍之间映射可能变（他核 unmap）——那是**既有**
窗口（单遍实现同样逐页取放 Space 锁），不是本契约引入的；先验后写只是让"失败"不再
留下半截数据。代价是区间多走一遍 `Segments`（64 B 消息通常落在一两页内）。

### (3) 删 `core/datagram.rs`

实测消费者数 = **0**（全树引用只剩 `core/mod.rs` 的模块声明本身；demo bin 早随 18 个
死 bin 删除）。`docs/supervisor.md:298` 说的"降到 bin 目录"是**有消费者之后**的处置。

### 判据

| | 值 |
|---|---|
| examine | **5/5**（默认 3/3 + audit 轮 + 融合 harden 轮无 lockdep） |
| 行为零变化 | 默认档三轮 md5 = `183133960fd82a1ff0f4a4c3f8863355`、audit 轮 = `efd03cb45e01a4acefef90a34ba32735` —— 与基线**逐字节相同**（对 B2 而言这条尤其有牙：契约一变，失败路径的字节数就会变） |
| 依赖图 | `protocol → env`、`task → protocol`；`kernel` 的依赖里**没有** `protocol`（探针正反两向实证） |
| 警告 | 三档与基线一致（默认 7 / audit 1 / harden 1） |
| 词法 | `grep -rn datagram task/src/` 零命中；`env::dispatch` 零命中 |

### 带出来的一处小事（未修，记一笔）

`env` crate 的 `Name::from_bytes` 在协议搬走的那一刻立刻变成"never used"——因为它的
**唯一**使用者就是那份协议。这本身是个好信号（说明 `pub(crate)` 的边界画得准），
按上面的处置升成 `pub` 留在 `env`。若将来 `wire` 里出现第二个"只给协议用"的东西，
就该问一句它是不是也该搬去 `protocol`。
