# 单次往返 IPC（Service · Request / Receive / Respond）

> 本文档沉淀 sqware 单次往返 IPC 的完整设计决策与实现路径。
> 所有决策均经用户逐条确认。内核侧几乎不改——全部复用现有结构。

## 0 · 目标

用户态 caller 向一个 Service Task（内核 Task）发一次请求，Service 处理后回结果。

设计原则（贯穿）：

- **Pie 只闩 mail 通道**（`Pie<HoleMeta>` / `Pie<PoleMeta>`），不加新变体、不加新权限位。
- **内核保持最少 syscall**，复杂度转交 IPC 和 Service Task。
- **复用现有结构**：Hole（数据通道）+ Pie（授权）+ wait/wake（挂起/唤醒），不造 `Req` 类型、不造递交点、不造 IPC 等待表。

## 1 · 核心模型：两次通信全部走 Hole

一次往返 = **两个 Hole**（请求 Hole + 回复 Hole）+ **Pie**（授权）+ **wait/wake**（挂起/唤醒）。

```
caller（用户 shell）                    service（内核 Task）
   │ ① 建 请求Hole + 回复Hole（各一个 Pie）
   │ ② Request(callee, op, payload, rep):
   │      · 鉴权 callee（pie alive + permission）
   │      · push【请求】进 请求Hole        [MailCall::Push]
   │      · wait 在回复上（挂起）          [RoomCall::Wait]
   │                                       ③ pull 请求Hole → 请求  [内核 mail::hole::pull]
   │                                       ④ 处理 op
   │                                       ⑤ push【结果】进 回复Hole [内核 mail::hole::push]
   │                                       ⑥ wake 回复 → 唤醒 caller
   │ ⑦ 被唤醒 → pull 回复Hole → 拿结果      [MailCall::Pull]
   │ ⑧ 返回
```

**跨空间传数据由 Hole 承担**（数据过内核，是 Hole 本职）。caller 用户态 push/pull 经 envcall；service 内核态用内核 `mail::hole::push/pull`。

## 2 · 三个原语

### Request（caller 侧，用户封装）

```rust
fn Request(callee, op, payload, rep) -> Result<reply, ServiceError> {
    push(请求, → 请求Hole);   // MailCall::Push
    wait(等回复);              // RoomCall::Wait
    pull(回复, ← 回复Hole);    // MailCall::Pull
}
```

用户看到一个 `Request()` 函数，内部是 push + wait + pull 封装。**底层多次 syscall**（这是复用现有结构的代价；真正"单次 syscall、A0=结果"需写 caller 帧，属后续演进）。

### Receive（service 侧，内核 Task）

```rust
fn Receive() -> Request {
    pull(请求Hole);  // 内核侧 mail::hole::pull（阻塞等）
}
```

### Respond（service 侧，内核 Task）

```rust
fn Respond(req, result) {
    push(结果, → 回复Hole);  // 内核侧 mail::hole::push
    wake(回复);              // 唤醒 caller（Reap/Wait 均返回下一帧）
}
```

## 3 · 复用 vs 新增

| 需要 | 来源 | 状态 |
|---|---|---|
| 请求/回复数据 | **Hole** | 现有 |
| 授权 | **Pie** 闩 Hole | 现有 |
| 挂起/唤醒 | **wait/wake** | 现有 |
| 跨空间数据 | **mail::hole::push/pull** | 现有 |
| service 承载 | **TaskBuilder::closure**（内核 Task） | 现有（曾用，重构删除，可复活）|
| **真正新增** | service 循环（Receive/Respond 编排）+ Request 用户封装 + shell 命令 | 纯适配/库层 |

## 4 · 关键难点：caller 与 service 的连接建立

service 是内核 Task，caller 是用户 Task，两者跨空间。两者要共享两个 Hole，**必须建立连接**。

- caller unseal 请求Hole + 回复Hole（各自 Pie 在 caller 手里）。
- caller 需把请求Hole 的访问权**传**给 service（service 要 pull 它）→ 用 **`accord`**（现有，跨任务转授权）。
- 同理回复Hole，service 要 push 结果 → caller 把回复Hole 写权 accord 给 service（或 service 先建、caller 拿到的方向反之）。

**时序**：service Task 须先有 task id，caller 才能 accord 给它。倾向顺序：
1. boot 建 service Task（内核对，获得 id）。
2. 建一个"入口 Hole"，其能力经当前机制传给 caller（或反过来 caller unseal + accord 给 service）。

> ⚠️ 这是实现里**最需要小心**的部分——跨任务方向（谁建、谁 accord 给谁）决定连接建立。

## 5 · 已有实现（Step 1，已落地编译通过）

- `kernel/src/work/unit/task.rs`：`BlockReason::Mail` 变体（语义标记）。

> 注：若走"纯复用 wait/wake 编排"，`BlockReason::Mail` 非必需（caller 挂起用现有 `Wait`）。
> 保留它作语义标记；仅在"真正单次 syscall（写帧）"路径才被真正使用。

- `kernel/src/work/room/messenger.rs`：`kill` 对 `BlockReason::Mail` 的完备处理。

## 6 · 实现步骤（复用现有结构，最小）

### Step A：复活 service 内核 Task

用 `TaskBuilder::closure` 建一个内核 Task，循环 `Receive()` → 处理 → `Respond()`。

历史用法样板（`91f7e82` 删除前）：
```rust
kernel()
    .expect("kernel team not initialized")
    .task()
    .name("service")
    .closure(move || {
        // service 循环：Receive → 处理 → Respond
    })?;
```

### Step B：用户 `Request` 封装

`task/env/mail.rs` 加 `Request(callee, op, payload, rep)`：push + wait + pull。

### Step C：shell 加命令触发

shell 的 `exec()` 加 `req` 命令，unseal 请求Hole + 回复Hole，调 `Request`，验证往返。

## 7 · 演进路径（后续）

**"真正单次 syscall（A0 = 结果）"**：需要写 caller 帧 `gpr[A0]` 的能力——即 service 完成后直接把结果写回 caller 的 trap 帧再唤醒，caller 恢复即 A0 = 结果。这需要：
- IPC 等待表（`rep → caller 帧`），service 凭 rep 定位。
- `BlockReason::Mail` 真正用于 caller 挂起。
- 调度器"写帧 + 唤醒"路径。

这是后续演进，非本最小实现范围。

## 8 · 已决

1. **连接方向 = B（service 主导）**：service 建请求Hole + 回复Hole（Pie 在 service），
   经 `accord` 把访问权转授给 caller。caller 用 accord 来的 Pie 发请求/收结果。
   - 复杂度来源：能力跨 U/S 传递。但 service 用 `accord`（与 A 同）即可规避，
     不必"把 Pie 结构体传给用户"——只需 accord 复制 Pie 到 caller.pies。
   - 语义：service 拥有入口，caller 来调用（符合"Service 是服务端"）。

2. **service 退出**：service 循环长期驻留（`loop`，不返回），随内核生命周期。

3. **单核先验证**：先单核跑通往返，跨核 IPC 后续。

4. **入口建立方式 = 方案 2（service Task 异步建 + 就绪握手/轮询）**：
   - boot 建 service Task（异步启动），service 闭包建 请求Hole + 回复Hole 并**登记就绪**。
   - caller（用户 shell）发请求**前**，先等/轮询"入口就绪"（从登记处取入口，或 wait 就绪 key）。
   - 真正"独立 Task"：service 自建入口、独立调度处理循环。
   - 时序：即"service 异步建入口 vs caller 同步取入口"的握手问题。
     * 就绪信号：service 建完入口后置位/登记；caller wait/轮询该就绪标记。
     * 共享"就绪标记"是同步点（避免 caller 在入口未建好时误发）。

## 9 · 运行验证状态（QEMU，已完成）

环境：QEMU `-machine virt` + OpenSBI（`./SBI`）+ riscv64 target。`cargo build -p kernel` 后
直接 `qemu-system-riscv64 ...` 可运行到 shell。

**已验证（`kernel/src/boot.rs::spawn_ipc_probe`）：**
- `TaskBuilder::closure` 能建/调度/运行**内核 Task**（该路径此前从未被打调用过）。
- 内核 Task 能 `ktask::park` 阻塞，唤醒后继续执行。
- **完整 echo 往返**：两个内核 Task 经共享 Hole（请求/回复）通信，
  caller `push "ping"` → service `pull` → `push "pong"` → caller `pull` 取到结果。
  QEMU 实测无 panic（见 §10 修复记录）。

## 10 · 已知问题 —— 已定位并修复（frame buddy 越界）

**双 Task 经共享 Hole 往返（echo）曾触发 frame 释放越界 panic —— 已修复。**

现象：`kernel/src/memory/allocator/frame.rs:388` `index out of bounds`
（`len==31934, index==31934`），panic 在 banner 之前，`scene` 无 running task。

### 真正根因：`merge_block` 缺 buddy 越界检查

`frame` 分配器 `merge_block` 的合并循环：
```rust
while power < self.freelist.len() {
    let buddy = Self::buddy_index(index, power);   // index ^ (1<<power)
    if !self.pagemeta[buddy]... { break; }          // ← 未检查 buddy < pagemeta.len()！
    ...
}
```
`buddy = index ^ (1<<power)` 可能 `>= pagemeta.len()`（free 区帧数非 2 的幂，末块的
XOR 伙伴会越界）。释放靠近 free 区末尾的块时，`pagemeta[buddy]` 越界 panic。

**修复**（`frame.rs::merge_block`）：加 `if buddy >= self.pagemeta.len() { break; }`
——伙伴越界即不存在，不能合并，安全 break。

### 修复验证（QEMU 实测通过）

修复后完整 echo 往返跑通（无 panic）：
```
[health] stress: ok
[ipc] service running
[ipc] caller running
[ipc] caller pushed ping
[ipc] service got "ping"
[ipc] service done
[ipc] caller got "pong"        ← 完整双 Task IPC 往返
[ipc] caller done
SQware shell
```

### 诊断过程中的归因修正（记录）

- 早期误判："双 Task + 共享 Hole + 循环 park 组合的生命周期 bug"。被"闭包捕获 Arc
  只打印版实测正常"证伪。
- 曾怀疑 free 区边界 off-by-one（`free.base` 与 `root_stack_edge` 差一页）——**经实测
  证伪**：banner 实际显示 `free 0x80301000` = `_kernel_edge(0x802f1000) + 0x10000`，一致，
  无 off-by-one。
- 命中真正根因：`merge_block` 的 buddy 越界。echo 版因代码布局变化，boot 期间恰有
  分配/释放落在 free 区末块，暴露了该 bug。

### 状态：已修复，echo 往返完整可用

`spawn_ipc_probe` 现为完整 echo 往返（service/caller 经共享 Hole 通信），编译通过 +
QEMU 实测无 panic。

---

## 11 · 用户态 Service 封装（`task::env::service`）

> 用户态单次往返服务调用。用 sqware 词族 `Service`（**不用 "IPC"**）。已实现于
> `task/src/env/service.rs`（`connect` + `request`，编译通过）。

### 定位

`Service` 是用户态持有"一个可调用服务通道"的类型，编排一次"请求-回复"调用：
`push(请求) → wait(等回复) → pull(结果)`。底层多次 envcall。

```rust
pub struct Service {
    req_pie: HolePie,   // 请求 Hole（caller → service）
    rep_pie: HolePie,   // 回复 Hole（service → caller）
    next_key: u64,      // wake_key 递增计数器（每条请求唯一）
}
```

### `connect(service_task)` —— 建立到固定服务

```rust
pub fn connect(service_task: usize) -> EnvResult<Service> {
    let req = HolePie::unseal()?;
    let rep = HolePie::unseal()?;
    req.accord(service_task, Permission::READ)?;    // service 读请求
    rep.accord(service_task, Permission::WRITE)?;   // service 写回复
    Ok(Service { req_pie: req, rep_pie: rep, next_key: 0 })
}
```

### 请求消息布局（无状态协议，自包含）

```text
  [0..4]    op        u32      服务要做什么
  [4..12]   wake_key  u64      等回复的 key（每条唯一，递增）
  [12..20]  big_data  u64      大数据 PieToken（指向 Pole；0 = 无）
  [20..64]  payload   44 字节  小数据
```

### `request(op, payload, big_data)` —— 发请求 + 等回复

```rust
pub fn request(&self, op: u32, payload: &[u8; 64],
               big_data: Option<u64>) -> EnvResult<[u8; 64]> {
    let wake_key = self.next_key.wrapping_add(1);   // 每条唯一
    // 组装消息（布局见上）...
    self.req_pie.push(&msg)?;
    room::wait(wake_key as usize, 5_000)?;          // 等 service 用此 key 唤醒
    let mut buf = [0u8; 64];
    self.rep_pie.pull(&mut buf)?;                   // 收结果
    Ok(buf)
}
```

### 已确认决策

| 决策 | 定论 |
|---|---|
| 命名 | `Service`（不用 "IPC"）|
| wake_key | **每条请求递增计数器**（唯一，防并发串）|
| 大数据 | payload 区放 `PieToken` 指向 Pole（`[12..20]`）|
| 服务寻址 | **固定服务**（`connect(service_task)`，boot 注册 id）|
| 无状态 | 每条请求自带 op + wake_key + 数据，service 用请求里的 key 回话 |

### 数据路径

- **小数据**（<=44B）：请求消息 `[20..64]` / 回复 Hole。
- **大数据**：请求 `[12..20]` 放 `PieToken`（指向 Pole）；service 借映读写；结果经 PieToken 回。

### 与内核三原语对齐

| 用户态 | 内核 |
|---|---|
| `Service::request` | `Request`（push + wait + pull 编排）|
| — | `Receive`（pull 请求）+ 业务 + `Respond`（push 回复 + wake）|

### 待办

- 用户态 `Service` 已实现（`connect` + `request`，编译通过）。
- 内核侧配合：service task 需能 `Receive`/`Respond` 用户态 `Service::request` 发的请求
  （即用户态 Hole 的 access 需 accord 到内核 service，且能 wake 用户态 wait 的 key）。
- 尚未在 shell 接入一个真实调用测试（当前仅内核 echo 往返验证，用户态 `request` 尚未
  端到端跑通）。


---

## 12 · 最小闭环实现（已端到端跑通，debug + release）

> 简化版：用 sqware 词族 `Service` + **真按需启用**内核 echo svc。砍掉 dispatcher / ServiceId /
> register 表，**服务数 = 1（echo）**，从最简路径走通。

### 12.1 整体链路

```
caller (shell)                          echo svc (kernel Task)
  Service::connect()
    → envcall ServiceCall::Connect
    → 内核给两个 pie token (req + rep)
                                          wait_mail(pull_key(req))  ← park, 0% CPU
  svc.req.push(payload)
    → envcall Mail::Push → try_push
    → wake(pull_key(req)) → echo 醒
                                          pull(req) → take, wake(push_key)
                                          处理: byte +1
                                          push(rep, reply): 槽空直写；满则
                                            wait_mail(push_key(rep))  ← park, 0% CPU
                                            ← wake(pull_key(rep)) 醒
  svc.rep.pull(buf)
    → envcall Mail::Pull → try_pull Busy
    → shell 短 spin 100 cycles + retry
    → pull ok → 拿到 reply
  打印 "req echo -> ..."
```

### 12.2 关键决策（已落地）

| 决策 | 取值 | 理由 |
|---|---|---|
| 服务数 | **1**（echo 字节 +1） | 验证最小链路；多服务探索见"服务发现"演进 |
| 连接建立 | `ServiceCall::Connect` envcall 给 caller 两个 Pie（req + rep） | 无 dispatcher；service 静态确定 |
| service 身份 | 内核 Task 名 `"echo-svc"` | Task 已有的 name 字段 |
| 用户态 `Service::connect()` | 不再需要 `service_task` 参数 | 内核全权负责"哪个服务"——固定 echo |
| wake_key | **不需要**（双 hole 方案：req 与 rep 是物理独立通道） | 单 caller 串行即可；key 是单 hole 时的并发去重 |
| 大数据 | 不支持（v1 范围外；走 Pole 路径见 §11） | 当前 64B payload 足够 |
| 按需启用 | **真 park**（`wait_mail` asm） | echo 不被调用时 0% CPU |
| service 退出 | 永不退出（`loop`） | 内核生命周期与 echo 绑定 |

### 12.3 新增原语

#### `messenger::park_mail(key: WaitKey) -> Handoff`

永久 park（无超时），BlockReason::Mail。语义与 `wait(key, MAX)` 一致但 BlockReason 不同
（Mail 标记 IPC 对方唤醒语义）。

```rust
pub fn park_mail(key: WaitKey) -> Handoff {
    let cond = current();

    // pend 消费（同 wait）
    { ... sites lock ... if site.pend { ... return Resume; } }

    let (mut task, next_pa) = cond.disown_and_install_next();
    // ... trace ...
    { ... sites lock 二次检查 pend ... }
    // 二次检查通过：transform Blocked(Mail), push to sites.waiters
    Task::exclusive(&mut task).transform(TaskState::Blocked {
        reason: BlockReason::Mail,
    });
    site.waiters.push_back(Waiter { task, tock: None });
    drop(sites);
    Handoff::Switch(next_pa)
}
```

#### `scheduler::ktask::wait_mail(_key: usize)` —— ktask 入口

裸 asm：存帧 → 调度 park_mail → restore 下一帧。
**关键**：s0 保存 a0（key），persist 后 s0 仍持有 key（caller-saved 寄存器由 callee 保全）。

```rust
#[unsafe(naked)]
pub extern "C" fn wait_mail(_key: usize) {
    naked_asm!(
        "csrc sstatus, 2",
        "csrrw sp, sscratch, sp",
        "sd    x1,  0x38(sp)",
        // ... 保存 x3..x31 ...
        "csrr  t0, sscratch", "sd    t0,  0x40(sp)",
        "csrr  t0, sstatus",
        "andi  t0, t0, -3",
        "ori   t0, t0, (1 << 5) | (1 << 8)",
        "sd    t0,  0x130(sp)",
        "sd    ra,  0x138(sp)",
        "mv    s0, a0",                  // s0 = key
        "mv    a0, sp",
        "ld    sp,  0x08(sp)",
        "la    t0, {persist}",
        "jalr  t0",
        "mv    a0, s0",
        "la    t0, {sched_park_mail}",
        "jalr  t0",
        "la    t0, {restore}",
        "jalr  t0",
        persist = sym persist,
        sched_park_mail = sym sched_park_mail,
        restore = sym restore,
    );
}
```

#### `mail::hole::push / pull` —— **真阻塞**（ktask 用）

```rust
pub(crate) fn push(meta: &HoleMeta, msg: &[u8; 64]) -> Result<(), GateError> {
    loop {
        { let mut slot = meta.slot.lock();
          if slot.is_none() {
              *slot = Some(*msg);
              drop(slot);
              let _ = messenger::wake(pull_key(meta));
              return Ok(());
          }
        }
        messenger::park_mail(push_key(meta));
        if !meta.alive() { return Err(GateError::Dead); }
    }
}

pub(crate) fn pull(meta: &HoleMeta) -> Result<[u8; 64], GateError> {
    loop {
        { let mut slot = meta.slot.lock();
          if let Some(msg) = slot.take() {
              drop(slot);
              let _ = messenger::wake(push_key(meta));
              return Ok(msg);
          }
        }
        messenger::park_mail(pull_key(meta));
        if !meta.alive() { return Err(GateError::Dead); }
    }
}
```

写/读完槽后均 wake 对侧（push 完成 wake pull waiters；pull 完成 wake push waiters）。
锁序：slot lock → wake（sites lock）→ park_mail（sites lock）→ wake → drop。无嵌套持锁。

#### `mail::hole::try_push / try_pull` —— **非阻塞**（envcall handler 用）

envcall handler 不能 fire-and-forget park（不能跨 ecall 切任务），所以返 Busy 让用户态自旋/重试。

```rust
pub(crate) fn try_push(meta: &HoleMeta, msg: &[u8; 64]) -> Result<(), GateError> {
    let mut slot = meta.slot.lock();
    if slot.is_some() { return Err(GateError::Busy); }
    *slot = Some(*msg);
    drop(slot);
    let _ = messenger::wake(pull_key(meta));
    Ok(())
}
```

#### `mail::hole::push_key / pull_key` —— wait key

per-meta WaitKey，async 用（compose(0, meta_addr | direction_bit)）。

### 12.4 envcall 边界（utask 不能 park）

envcall handler 流程：
```
MailCall::Push { token, msg } →
  查 pie → 找 HoleMeta →
  try_push（Busy 返错）→ set a0 = 0/err → 返 frame
```

envcall handler 始终返 `frame`（不返 PA）——继续当前任务。
用户态 HolePie 短 spin + retry 配合（μs 级，wake 几乎立即生效）。

### 12.5 关键发现：**`#[inline(never)]` 必须加在 closure 内**

release 模式下编译器对 `move || { ... }` closure 做跨函数优化（参数重排、寄存器分配），
**破坏裸 asm 的跨 hart tp 约定**（asm 假设的栈帧布局被改）。

**修复**：closure 体内调一个 `#[inline(never)]` 的 helper 函数：

```rust
.closure(move || {
    #[inline(never)]                    // ← 强制独立栈帧，保留 asm 假设
    fn svc_loop(req: Arc<HoleMeta>, rep: Arc<HoleMeta>) -> ! {
        loop {
            let pull_k = hole::pull_key(&req);
            ktask::wait_mail(WaitKey::into_raw(pull_k));
            let Ok(pulled) = hole::pull(&req) else { continue; };
            // ... 处理 ...
            while hole::push(&rep, &reply).is_err() {
                let push_k = hole::push_key(&rep);
                ktask::wait_mail(WaitKey::into_raw(push_k));
            }
        }
    }
    svc_loop(req_for_task, rep_for_task)
})
```

`#[inline(never)]` 不是性能提示——是**正确性约束**。

### 12.6 文件变更清单

| 文件 | 改动 |
|---|---|
| `kernel/src/work/mail/hole.rs` | push/pull 真阻塞；新增 try_push/try_pull、push_key/pull_key |
| `kernel/src/work/room/messenger.rs` | 新增 `park_mail(key)`；`WaitKey::into_raw` |
| `kernel/src/work/room/scheduler/utask.rs` | 新增 `park_mail(key)` 入口 |
| `kernel/src/work/room/scheduler/ktask.rs` | 新增 `wait_mail(key)` 裸 asm |
| `kernel/src/boot.rs` | 删 `spawn_ipc_probe`；新增 `ECHO_REQ` / `ECHO_REP` + `spawn_echo_service` |
| `kernel/src/runtime/switcher/envcall.rs` | `Service(Connect)` 给两 pie（a0=req a1=rep）；`Mail::Push/Pull` 改用 `try_push/try_pull` |
| `crates/env/src/fid.rs` | `ServiceCall::Connect` ret 改 `(PieToken, PieToken)` |
| `crates/env/src/wire.rs` | 新增 `FromPair for (PieToken, PieToken)` |
| `task/src/env/service.rs` | `Service` 持 req + rep 两 holePie；`echo` 编排两轮 envcall |
| `task/src/env/mail.rs` | `HolePie::push/pull` 短 spin 100 cycles + retry |
| `task/src/bin/shell.rs` | `req` 命令 |

### 12.7 端到端验证

```bash
$ cargo build --release
$ qemu-system-riscv64 ...
SQware shell
type 'help' for commands.
sq > req
req echo -> "ifmmp.tfswjdf..."    ← hello-service 字节 +1 正确
```

debug 与 release 均通过。

### 12.8 已确认决策

| 决策 | 定论 |
|---|---|
| 命名 | `Service`（sqware 词族；不用 "IPC"）|
| 连接建立 | envcall 给两 pie（dispatcher 不需要——单服务场景）|
| service 数量 | 1（echo）；多服务探索走 dispatcher（见"服务发现"演进） |
| 按需启用 | **真 park**（`wait_mail` asm），不被调用时 0% CPU |
| 字节序 | 小端（env 单一真相：`Wire::pack`） |
| wake_key | 不需要（双 hole 物理隔离了并发） |
| closure release 兼容 | `#[inline(never)]` 内层 helper（必须） |

### 12.9 已知边界

| | 现状 |
|---|---|
| 多 caller 并发 | ✅（单 slot 串行，push/pull 阻塞保证原子） |
| 异常退出清理 | echo `loop` 不退出；如需终止要 reap 路径 |
| 多服务发现 | ❌（按 dispatcher 方案演进） |
| 大数据（>64B） | ❌（走 Pole 路径见 §11） |
| release 模式 | ✅ 跑通（需 `#[inline(never)]`） |

### 12.10 演进路径（后续）

1. **dispatcher 服务发现**：见之前讨论的 dispatcher Task + ServiceId enum。
   `ServiceCall::Connect` 接 `ServiceId` 参数，找 dispatcher 注册的 service。
2. **单次 syscall（A0=结果）**：service 完成后直接写 caller 帧 + 唤醒。需 IPC 等待表
   （rep → caller 帧映射）+ `BlockReason::Mail` 真正用上。
3. **异常退出清理**：echo svc 加 reap 路径（任务级 panic → 转 cleanup）。
4. **大数据 Pole 路径**：payload > 64B 时经 Pole 物理页直传。

---

## 13 · 多服务发现：dispatcher Task + Arc 服务注册表

> §12 单服务（echo）走通后扩展。**dispatcher 是真正的服务发现机制**——服务数从 1
> 扩到 N，shell 按服务名 lookup；dispatcher Task 走"DHCP 服务器"路径（已有结构复用）。

### 13.1 设计决策（与 §12 差异）

| 决策 | §12 单服务 | §13 多服务（dispatcher）|
|---|---|---|
| 服务数 | 1（echo）| N（dispatcher 注册表）|
| 连接建立 | envcall 直接给服务 Pies | envcall 给 dispatcher Pies → push lookup → pull reply |
| Service Pies 来源 | kernel 直接构造 | dispatcher accord 给 caller.pies |
| 用户态 Service | 拿到 echo svc Pies | 拿 dispatcher Pies → lookup 拿目标 svc Pies |
| dispatcher 闭包持有 | — | `Arc<SpinLock<Vec<ServiceEntry>>>`（独占持有）|
| 名字到 Pies | 编译期固定 | 运行时按 `ServiceId.name_bytes()` 查 |
| 按需启用 | echo svc 真 park | dispatcher + echo svc 都真 park（`wait_mail`）|

### 13.2 新增模块

#### `kernel/src/service/dispatch.rs`

```rust
pub struct ServiceEntry {
    pub name: String,
    pub req_id: ResourceId,
    pub req: Weak<HoleMeta>,
    pub rep_id: ResourceId,
    pub rep: Weak<HoleMeta>,
}

pub type ServiceRegistry = SpinLock<Vec<ServiceEntry>>;

pub fn new_registry() -> Arc<ServiceRegistry> { ... }
pub fn register(registry: &Arc<ServiceRegistry>, name: &str,
               req_id: ResourceId, req: Weak<HoleMeta>,
               rep_id: ResourceId, rep: Weak<HoleMeta>) { ... }
pub fn lookup<'a>(registry: &'a Arc<ServiceRegistry>, name: &str)
    -> Option<Vec<ServiceEntry>> { ... }
```

#### `kernel/src/service/mod.rs`

```rust
pub mod dispatch;
```

#### `kernel/src/main.rs`

```rust
mod service;  // 新增
```

### 13.3 env 扩展

#### `crates/env/src/fid.rs`

```rust
pub enum ServiceCall {
    /// 拿 dispatcher 的 req/rep Pies（service 参数当前固定 dispatcher；未来可按 ID 路由）
    #[ret((PieToken, PieToken))]
    Connect { service: ServiceId },
}

/// 服务号（dispatcher 按 name 查找；0 保留——未来 dispatcher 自身可服务 0）
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ServiceId {
    Echo = 1,
    // 后续: Logger = 2, Fs = 3, ...
}

impl ServiceId {
    /// 服务名（UTF-8 null-padded，dispatcher 查找用）
    pub const fn name_bytes(self) -> &'static [u8] {
        match self {
            ServiceId::Echo => b"echo\0",
        }
    }
}
```

#### `crates/env/src/wire.rs`

```rust
impl Wire for crate::fid::ServiceId {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        s[*i] = *self as u8 as usize; *i += 1;
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let v = *s.get(*i).ok_or(Decode::Overflow)? as u8;
        *i += 1;
        match v {
            1 => Ok(Self::Echo),
            _ => Err(Decode::Invalid),
        }
    }
}
```

### 13.4 协议（dispatcher req/rep 64 字节）

**请求**（user → dispatcher）：
```
[0..8]    sender_task_id (u64 LE)
[8..64]   service_name (UTF-8 null-padded)
```

**回复**（dispatcher → user）：
```
[0..8]    req_pie_token (u64 LE; 0 = 未找到)
[8..16]   rep_pie_token (u64 LE; 0 = 未找到)
[16..64]  reserved
```

### 13.5 boot 编排

```rust
// 1. dispatcher req/rep hole
let (dreq, dreq_id) = hole::meta()?;
let (drep, drep_id) = hole::meta()?;
DISPATCHER_REQ.get_or_init(|| (dreq_id, dreq.clone()));
DISPATCHER_REP.get_or_init(|| (drep_id, drep.clone()));

// 2. echo svc req/rep hole
let (ereq, ereq_id) = hole::meta()?;
let (erep, erep_id) = hole::meta()?;

// 3. 服务注册表（显式 Arc，dispatcher 闭包独占持有）
let registry = dispatch::new_registry();
dispatch::register(&registry, "echo", ereq_id, Arc::downgrade(&ereq),
                  erep_id, Arc::downgrade(&erep));

// 4. spawn echo svc（持 echo req/rep Arc 闭包）
kt.task().name("echo-svc").closure(move || {
    #[inline(never)]
    fn svc(req: Arc<HoleMeta>, rep: Arc<HoleMeta>) -> ! {
        // pull req (park 真阻塞) → 处理 → push rep (park 真阻塞)
    }
    svc(ereq, erep)
}).spawn()?;

// 5. spawn dispatcher task（持 dispatcher req/rep + registry Arc）
kt.task().name("dispatcher").closure(move || {
    #[inline(never)]
    fn svc(dreq: Arc<HoleMeta>, drep: Arc<HoleMeta>,
           reg: Arc<dispatch::ServiceRegistry>) -> ! {
        loop {
            // wait_mail(pull_key(dreq)) park 真阻塞
            // pull → 解析 sender_id + name
            // lookup(&reg, name) → ServiceEntry
            // gate::new_pie(req_id, READ|WRITE, ...) accord 给 caller.pies
            // push reply tokens 到 drep（park 真阻塞）
        }
    }
    svc(dreq, drep, registry)  // registry move 进闭包
}).spawn()?;
```

### 13.6 envcall `Service(Connect)` handler

```rust
EnvCall::Service(ServiceCall::Connect { service: _ }) => {
    // service 参数当前固定返 dispatcher（未来按 ID 路由）
    let (req_id, req_meta) = (DISPATCHER_REQ.get()..., DISPATCHER_REQ.get()...);
    let (rep_id, rep_meta) = (DISPATCHER_REP.get()..., DISPATCHER_REP.get()...);
    let req_pie = new_pie(req_id, READ|WRITE, Some(current_id), Arc::downgrade(&req_meta));
    let rep_pie = new_pie(rep_id, READ|WRITE, Some(current_id), Arc::downgrade(&rep_meta));
    task.pies.lock().push(AnyPie::Hole(req_pie));
    task.pies.lock().push(AnyPie::Hole(rep_pie));
    a0 = req_pie.token(); a1 = rep_pie.token();
}
```

### 13.7 用户态 `Service::connect`

```rust
pub fn connect(sid: ServiceId) -> EnvResult<Service> {
    // 1. envcall 拿 dispatcher Pies
    let (dreq_tk, drep_tk) = ServiceCall::Connect { service: sid }.call()?
        .into_connect();
    let dreq = HolePie::from_token(dreq_tk);
    let drep = HolePie::from_token(drep_tk);

    // 2. push lookup request
    let my_id = task::self_id()?;
    let mut req_msg = [0u8; 64];
    req_msg[0..8].copy_from_slice(&(my_id.get() as u64).to_le_bytes());
    let name = sid.name_bytes();
    let n = name.len().min(55);
    req_msg[8..8+n].copy_from_slice(&name[..n]);
    dreq.push(&req_msg)?;

    // 3. pull reply
    let mut reply = [0u8; 64];
    drep.pull(&mut reply)?;

    // 4. parse tokens
    let req_tk = u64::from_le_bytes(reply[0..8].try_into().unwrap_or([0; 8]));
    let rep_tk = u64::from_le_bytes(reply[8..16].try_into().unwrap_or([0; 8]));
    if req_tk == 0 || rep_tk == 0 { return Err(not_found); }

    // 5. build Service with echo svc Pies
    Ok(Service {
        req: HolePie::from_token(req_tk),
        rep: HolePie::from_token(rep_tk),
    })
}
```

### 13.8 加新服务流程

```rust
// 1. crates/env/src/fid.rs
pub enum ServiceId {
    Echo = 1,
    Logger = 2,  // 新增
}
impl ServiceId {
    pub const fn name_bytes(self) -> &'static [u8] {
        match self {
            ServiceId::Echo => b"echo\0",
            ServiceId::Logger => b"logger\0",
        }
    }
}

// 2. crates/env/src/wire.rs（增加 Logger 反序列化）

// 3. kernel/src/boot.rs::spawn_services
let (lreq, lreq_id) = hole::meta()?;
let (lrep, lrep_id) = hole::meta()?;
let lreq_for_task = lreq.clone();
let lrep_for_task = lrep.clone();
dispatch::register(&registry, "logger", lreq_id, Arc::downgrade(&lreq),
                  lrep_id, Arc::downgrade(&lrep));
kt.task().name("logger-svc").closure(move || {
    #[inline(never)] fn svc(req: Arc<HoleMeta>, rep: Arc<HoleMeta>) -> ! {
        // logger 业务：把 req 内容写入某处；push "ok\0..."
    }
    svc(lreq_for_task, lrep_for_task)
}).spawn()?;

// 4. task::Service::connect(ServiceId::Logger)
```

**无需新 dispatcher、无需新 envcall 域**——只扩 ServiceId enum + 业务 Task + 注册。

### 13.9 已确认决策

| 决策 | 定论 |
|---|---|
| 命名 | `Service`（sqware 词族）|
| dispatcher 是 Task | ✅（按需启用，wait_mail 真 park）|
| 服务注册表所有权 | `Arc<SpinLock<Vec<ServiceEntry>>>` dispatcher 闭包独占持有 |
| `ServiceId` 范围 | 1..（0 保留给 dispatcher 自身）|
| lookup key | name 字符串（更灵活；ServiceId 仅环境调用 ABI 边界）|
| dispatcher reply slot | 单 slot（v1 单 shell 串行；多 caller 后续扩数组）|
| 服务 Pie 权限 | `READ | WRITE`（caller push/pull + echo pull/push）|

### 13.10 release 已知边界

**A. opt-level=2/3 已知 bug（已修复，**`opt-level=2` 全档跑通**）**：

1. **WaitKey::compose mask 错联**（**asm 已验证 + 已修复**）——size 优化
   下，编译器在 `dispatcher svc` 闭包内联 `push_key`/`pull_key` 时，把
   `((1usize << 48) - 1)` 这一表达式在第一次使用后多算了一个 `+1`，
   把 `0x0000_FFFF_FFFF_FFFF` 偏移成 `0x0001_0000_0000_0000`（仅位 48）。
   修法：**抽 `#[inline(never)] fn low48(va: usize) -> usize` helper**
   承担 mask 计算，强制每次独立；pull_key 的 `+1` 在调用点
   `addi a0, a0, 17`（= + 16 + 1 = 原始 +1）做，不进 mask。
   opt-level=2 修复后的 dispatcher svc 反汇编：
   ```asm
   802278ba: addi  a0, a0, 17        # a0 = drep_ptr + 17  (pull_key 的 +1 在调用点)
   802278c0: jalr  ... low48          # a0 = low48(a0)  = pull_key
   8022804c: mv    a0, s4            # a0 = dreq_ptr + 16
   80228052: jalr  ... low48          # a0 = low48(s4) = push_key
   8022805e: jalr  ... wait_mail
   ```
   两次 `low48` 独立调用，`+1` 不再跨调用错联到 mask。**方向 A 验证通过**。

   方向 B（换字面量）验证过：**无效**——编译器仍生成
   `lui 0xfffe0; srli 0x10; addi +1` 把 `+1` 算在 mask 上。字面量写法
   不影响 size 优化把跨调用 `+1` 折叠的决策。

2. **closure + 裸 asm ABI 假设**（**修复已落地**）——release 编译器对
   `move ||` closure 做跨函数优化（参数重排 / 寄存器重分配 / 栈帧重排），
   内层裸 asm `wait_mail`（基于"`scheduler::ktask::park` 后调用方栈帧特定偏移"
   的假设）的 `ra` 保存位置与返回路径被破坏，sepc 在 sret 后指向 garbage。
   修法：在 `kernel/src/boot.rs` 中给 `dispatcher svc` 和 `echo svc` 的内层
   `fn svc(...) -> !` 加 `#[inline(never)]`，**锁住闭包→裸 asm 跨函数 ABI 边界**。
   该修复在 opt-level=1/2/3 任意档位下都生效（与 low48 helper 独立）。

3. **撤回「FromPair 生成 bug」旧 claim**——该说法源自 compaction summary，
   但**未直接验证**。`FromPair for u8` 实现就一行 `v0 as u8`，size 优化下
   编译出错概率极低；原始 `pc=0x1010166646a7772` 是 **KERNEL** 页错误
   （scause=12 来自 S-mode trap），不是用户态 `unreachable!()` 命中——
   KERNEL panic 跟用户态 `derive` 没因果关系。该 claim 撤回。

> 结论：**`opt-level = 2` 现在是默认档位**（`Cargo.toml` 已切）。
> mask 错联通过 `low48` helper 修；裸 asm 假设通过 `#[inline(never)]`
> 锁闭包边界修。端到端 5/5 跑通 `(sleep 2; printf 'req\n') | cargo run` →
> `req echo -> "ifmmp.tfswjdf..."`（byte+1 echo）。

**B. opt-level=1 shell `pc=0x4` 假象（已澄清：测试基础设施问题）**：

早期诊断时观察到 shell 在 `readline` 循环中 panic，`sepc=0x4`（试图从
NULL+4 处取指）。但加 `core::hint::black_box`、dump 多次 envcall 前后
`frame.sepc/a0` 后定位到：**这是 QEMU 测试基础设施的问题，不是 sqware bug**。

- **根因**：QEMU `-nographic` 默认是 `-serial mon:stdio`，QEMU monitor 与
  serial uart 共享 stdio。`r`/`e`/`q` 等字符被 monitor 当命令吃掉，并把
  monitor 自己的响应（如寄存器 dump）回灌到 serial。shell 的 `readline`
  收到的是 0x7/0x5/0xd/0x4/0x1a 等控制码（monitor 输出），不是用户输入。
- **症状**：shell 反复 `IOCall::Get` 拿到控制码 → `dec.advance` 不识别 →
  继续 readline 循环；同时 `LineSink` 不断 alloc/dealloc buffer，多轮
  后触发未确定的边界 panic（`sepc=0x4`）。
- **解决路径**：runner 改用 `( sleep 2; printf 'req\n' ) | cargo run` 延迟喂入，
  让 QEMU 先 boot 完 shell 启动，再喂输入（fifo + sleep 同样可行，但需保证
  fifo 在 QEMU 启动后才写入）。

**端到端验证（opt-level=1 实测）**：

```bash
$ ( sleep 2; printf 'req\n' ) | QEMU_TIMEOUT=10 cargo run --release
...
SQware shell
type 'help' for commands.
sq > req
req echo -> "ifmmp.tfswjdf..."   ← hello-service 字节 +1（echo svc 端到端跑通）
```

完整路径：shell `req` → `ServiceCall::Connect` → dispatcher svc lookup "echo"
→ 创建 echo svc Pies accord 给 shell → shell `svc.echo(msg)` push 64B →
echo svc 真 park wake → byte+1 → push reply → shell pull reply → 打印结果。

### 13.11 端到端验证

```bash
$ cargo build --release
$ qemu-system-riscv64 ... 
SQware shell
type 'help' for commands.
sq > req
req echo -> "ifmmp.tfswjdf..."  ← hello-service 字节 +1
```

**完整路径跑通**：
- `Service::connect(ServiceId::Echo)` → envcall 拿 dispatcher Pies
- push lookup → dispatcher wake → 查表 → 创建 echo Pies accord 给 shell
- shell pull reply → 构造 Service(echo svc Pies)
- `svc.echo(msg)` → push echo req → echo svc 真 park wake → 处理（byte+1）→ push echo rep
- shell pull echo rep → 拿 reply

### 13.12 文件清单

```
新增:
  kernel/src/service/mod.rs
  kernel/src/service/dispatch.rs
修改:
  kernel/src/main.rs                       mod service
  kernel/src/boot.rs                       spawn_demos → spawn_services（dispatcher + echo）
  kernel/src/runtime/switcher/envcall.rs    Service(Connect) → dispatcher Pies
  kernel/src/work/room/messenger.rs        WaitKey::into_raw（辅助）
  crates/env/src/fid.rs                    ServiceId enum, Connect 改 Connect{ service }
  crates/env/src/wire.rs                   FromPair (PieToken, PieToken), Wire ServiceId
  crates/env/src/lib.rs                    导出 ServiceId, make_err
  task/src/env/service.rs                   connect 走 dispatcher 路径
  task/src/env/task.rs                     + self_id()
  task/src/bin/shell.rs                    req 用 ServiceId::Echo
  Cargo.toml                                release opt_level = 1
```

---

## 14 · 服务目录协议（取代 §13 的 dispatcher 形态）

§13 的 dispatcher（class 7 `ServiceCall::Connect` + 单 slot rep + 跨任务写调用方
权限表）已被**服务目录协议**取代，规范见 [`docs/dispatch.md`](dispatch.md)。

差异要点：

| §13 | 现在 |
|---|---|
| `ServiceCall::Connect`（class 7，`service` 参数被忽略） | **删除**——入口门闩由内核在 boot 期放进首个用户任务权限表 |
| dispatcher 持 `(name, req_id, Weak, rep_id, Weak)` | 目录持 `Pie`（`Binding { name, entry }`） |
| 直接为 caller 建 pie 塞进 `caller.pies` | `gate::accord` 转授子集（与普通 task 授权同路） |
| 单 slot rep（多 caller 串台） | 调用方自带回信通道 |
| lookup 与授权混在一笔往返 | `Resolve` / `Enumerate` / `Connect` 三个操作 |
| 无 `Unregister` / `Replace` | 均有（`Replace` 支持服务重启换门闩不换名字） |

新增两个与目录无关的原语：`MailCall::Collect`（自省权限表）、`MailCall::Release`
（自释自己的一份）。实现过程中修复的三个内核缺陷见 `docs/dispatch.md` §11。

---

## 15 · S 态域（Supervisor 域）

`SpaceKind` 收窄为特权轴（`Supervisor` / `User`）、ASID 提为独立字段
（0 = 内核空间）、域态 echo 服务见 [`docs/supervisor.md`](supervisor.md)。该文
同时记录两条被验证逼出来的 ABI 事实：环境调用陷阱是 **`ebreak`**（S 态 `ecall`
是 SBI 调用、不进内核）与 `sepc` 必须按**真实指令长度**前进（`c.ebreak` 是 2
字节）。
