# SQware：Channel

> 状态：Channel 部分 = **已恢复**（A7 全部裁决 + 第 4 关签名定稿，进入第 5 关实现）；异步框架部分 = **进行中**（自 B 起按 design-pipeline 逐关推进）。

---

## A. Channel —— 一对一消息专线

### A0 · 目标与边界

- 目标：内核唯一的「消息传递」载体——**channel = 不同 VA 映射同一 PA 的队列**（共享物理帧，双端映射，AMO 读写）。小对象走槽字消息；大对象走共享内存（描述随通道传）。
- 边界（不进 channel）：SBI/M-mode、调度器内部移交（push/pop/steal/park 因果链）、原子状态信号（PUSHED/REAPED/ALARM）、诊断观察流（trace 消费端可接通道，但通道不取代 trace）。
- **v1 不接管 ucall**（本设计要求不含 ucall 话题；接管与否由日后另行裁决）。
- 用户侧 v1 同步 API；Future/async 层 = v2 候选（waiters 预留「唤醒即继续」）。

### A1 · 功能模型（操作清单）

生命周期：

| 操作 | 语义 |
| --- | --- |
| `spawn`（Builder） | 建 req/resp **两条**通道，四端点**一次吐出**（mpsc「创建即双端」）；无指名、无 `to(Peer)`、无 claim |
| 端点 move | 服务端半对（`req_rx` + `resp_tx`）作为**普通对象** move 进对端任务闭包（同空间 VA 直见，零内核参与） |
| `crush` | 显式终止（消费式 `self`）：置 Dead + 唤醒阻塞者 + 撤本端映射；资源回收 = Arc 归零自动 |
| 任务退出挂钩 | 端点对象随任务 drop → Arc 递减；另一端仍在则通道续活（mpsc 语义）；Arc 归零 → 帧返池 |

通信：

| 操作 | 语义 |
| --- | --- |
| `push` | 推端投递槽字消息；快路径直访共享帧 AMO；满 → park 阻塞 |
| `try_push` | 非阻塞；满 → Full |
| `pull` | 拉端取消息；空 → park；Hang∧空 → CAS(Hang→Gone) → 断开感知 |
| `timeout` | 限时拉；超时 → Timeout |

状态机（四态）：`Live → Hang（推端消逝）`、`Live/Hang → Dead（crush）`、`Hang → Gone（余信取空，拉端钉连）`。

载荷模型：消息 = 槽字（标量/地址/句柄通吃）；对象大小是用户约定（`T` 信息类型），内核 ABI 只有槽字；不设编解码 trait。

### A2 · 结构设计（字段 → 操作）

- `Channel`（内核对象，Arc 管理）持有 `SharedFrame`（一页：header state/head/tail/slot_len + 槽区；双端映射，AMO）。
- `Tx` / `Rx` = `Arc<Channel>` + 方向类型义务（push 只收 Tx、pull 只收 Rx，不可互换）。
- `ChannelBuilder::new().slot_len(…) → spawn → (ClientEnd, ServerEnd)`。
- 同空间 task↔task：端点映射在共享 Space 中，快路径直访 AMO（零 trap）；任务退出 = drop 端点对象（Arc 递减），无窗口退出扫描。
- 复用不重造：`Space::map/map_frames`、`IntervalAllocator`、`park/unpark`、`chrono` deadline、`diagnose::trace`。

### A3 · 原语与命名（单一隐喻：通信链路）

- 生命周期：`spawn` / `crush`；四态 `Live · Hang · Gone · Dead`
- 端点：`tx` / `rx`；半对 `ClientEnd { req_tx, resp_rx }` / `ServerEnd { req_rx, resp_tx }`；`Spawned { client, server }`
- 通信：`push` / `pull` / `try_push` / `timeout`
- 控制器：Builder 链（`.slot_len()` / `.spawn()`；**无** `.to()`）
- 错误域：`Invalid · Gone · Full · Timeout · Dead`（五变体）
- 已删：`Peer`（含 Peer::Kernel）、`claim`、`to(Peer)`、`Endpoint` 枚举、句柄/能力表/generation、`send_bytes`、`SlotCodec`、`reply_addr`/临时邮路、多生产者 `grant`/`key`

### A4 · 签名（定稿）

~~~rust
pub struct Tx { channel: Arc<Channel> }
pub struct Rx { channel: Arc<Channel> }

impl Tx {
  pub fn push(&self, slots: &[usize]) -> Result<(), ChannelError>;        // 满 → park
  pub fn try_push(&self, slots: &[usize]) -> Result<(), ChannelError>;    // 满 → Full
  pub fn crush(self);                                                     // 消费式：置 Dead+唤醒+撤本端映射
}
impl Rx {
  pub fn pull(&self, out: &mut [usize]) -> Result<usize, ChannelError>;     // 空 → park；返回实际槽数
  pub fn timeout(&self, out: &mut [usize], deadline: Instant) -> Result<usize, ChannelError>; // 超时 → Timeout
  pub fn crush(self);
}
pub enum ChannelError { Invalid, Gone, Full, Timeout, Dead }

pub struct ChannelBuilder { slot_len: usize }                             // 默认 8
impl ChannelBuilder {
  pub fn new() -> Self;
  pub fn slot_len(mut self, n: usize) -> Self;
  pub fn spawn(self) -> Result<Spawned, ChannelError>;                    // 双通道：四端点一次吐
}

pub struct Spawned {
  pub client: ClientEnd,   // 客户端半对：req_tx + resp_rx（留己用）
  pub server: ServerEnd,   // 服务端半对：req_rx + resp_tx（move 进对端任务）
}
pub struct ClientEnd { pub req_tx: Tx, pub resp_rx: Rx }
pub struct ServerEnd { pub req_rx: Rx, pub resp_tx: Tx }
~~~

### A5 · 裁决记录（A 部分恢复期新增）

1. A7.1：v1 做双通道 + echo 验收；echo 由**用户任务充当**（task↔task 验收），task↔kernel 服务通道**顺延 v2**（Peer::Kernel 随 Peer 删除）
2. A7.2：快路径直访**进 v1**——用户 VA 窗映射共享帧，push/try_push/pull 直访 AMO（零 trap），需定页表属性与 SPSC fence 纪律
3. A7.3：`slot_len` 默认 **8**（2 的幂，索引免取模）
4. A7.4：`crush` = `Tx::crush(self)` / `Rx::crush(self)` 消费式，单通道级（不连带 req/resp 对）
5. A7.5：`pull`/`timeout` 写调用方 `out` 缓冲（零分配），返回 `Result<usize, ChannelError>`（实际槽数）
6. A7.6：错误域**五变体** = Invalid/Gone/Full/Timeout/Dead（槽超限归 Invalid；crush 后操作报 Dead）
7. A7.7：**打开 task↔task**（推翻 A6 原被否项）；端点告知 = **mpsc 式**——创建即双端、端点对象 move 交接，**同空间限定**（跨空间 v1 不开放）

### A6 · 被否项（不做 X 因为…）

| 不做 | 原因 |
| --- | --- |
| grant/key（多生产者/投递权复制） | 一对一无多生产者 |
| 句柄/能力表/id+generation | 端点即操作容器（tx/rx 关联方法）；VA 空间隔离即授权 |
| reply_addr/临时邮路 | 双向 = 一对通道（request/response） |
| send_bytes / 大载荷拷贝 | 载荷 = 槽字；大对象 = 共享内存描述走通道 |
| SlotCodec/编解码 trait | 槽片是内核 ABI 实现细节，用户侧信息类型逐个实现 |
| 内置「小对象/大对象」通道模式 | 模式解耦：一切皆信息，大小是用户约定 |
| per-task 能力表 | 反向索引 = 同空间共享映射 + drop 语义，无需表 |
| CONT/独立登记表 | 生命周期归 Arc + 池，无需第三张表 |
| `Peer`（含 Peer::Kernel）/ `claim` / `to(Peer)` | mpsc 式端点对象 move 交接，无指名无代挂 |
| task↔kernel 服务通道（v1） | 顺延 v2（echo 由用户任务充当） |
| 跨空间通道（v1） | 同空间 mpsc 式限定；跨空间映射/代挂 v2 |
| ucall 接管（v1） | 本设计要求不含；独立落地先行 |
| Future/async 层（v1） | v2 候选；waiters 预留「唤醒即继续」 |

### A7 · 悬而未决（全部已裁，见 A5）

~~1. `Peer` 形态~~ → 删（mpsc 式）；~~2. 快路径直访~~ → 进 v1；~~3. slot_len 默认 7~~ → 8；
~~4. crush 形态~~ → Tx/Rx 各自 `crush(self)`；~~5. out 缓冲~~ → 确认 + 返回槽数；~~6. 错误域~~ → 五变体 +Dead；
~~7. v1 仅 Peer::Kernel~~ → 打开 task↔task（同空间 mpsc 式）

### A8 · 恢复点

**第 5 关实现已完成**（2026-08-26 验收）：

1. ✅ **ktask 自切换原语**（`kernel/src/runtime/switcher/selfpark.rs`，提前实现异步框架 §8 步骤 2 第一步）：闭包体内自 park——SIE=0 捕获现场进任务帧 + sepc 指恢复 thunk + scheduler::park_ms 簿记 + restore；唤醒 sret 回 thunk 续跑 poll 循环。scheduler 新增 `park_ms`/`running_task_pa` 适配。
2. ✅ **Channel 引擎**（`kernel/src/work/unit/channel.rs`）：`Channel`（SPSC 无锁 ring：state/head/tail/slot_len/槽区，长度槽前缀，AMO+Release/Acquire 序）、四态 `Live·Hang·Gone·Dead`、五变体错误、`Tx`/`Rx`（方向进类型，Drop 钩子：Tx→Hang、Rx→Dead 对称补齐）、`ChannelBuilder::new().slot_len().spawn()` → `Spawned { client, server }`、push/try_push/pull/timeout/crush（tick 粒度 park 重试阻塞）。
3. ✅ **同空间双任务 echo 验收**（boot.rs）：A spawn 双通道 → server 半对 move 进 B 闭包 → B echo 回 → A 校验，SMP 1/2/4 五轮全过零 MISMATCH；语义补测独立通道：`try_push Full ✓`、`timeout Timeout ✓`、`crush Dead ✓`；A 退出后 B `pull err Gone`（状态机断开感知隐式验收）。
4. ✅ **顺手修复**：`clock.rs` `checked_duration_since` 用惰性 `then` 替换 eager `then_some`（earlier 在未来时 underflow panic 的潜伏 bug，被 timeout 路径引爆）。

后续（用户接入 / ucall 话题）：端点映射入用户 VA 窗 + 用户库 API（独立轨，另行裁决）。

> 维护：本文件为一次性探讨/挂起文档，决策固化到代码或后续文档后即删。
### A8.4 · 交付验收记录（2026-06 release 全量复核）

**验收配置**：`cargo run --release`（release = optimized + debuginfo，`panic="abort"` 由 profile 提供），QEMU virt `-icount auto,sleep=off`，QEMU_SMP ∈ {1,2,4} × 6 seeds。

- ✅ **SMP1 × 6 seeds**：panic=0；echo 5/5 轮 OK；semantics 4/4（Full/Timeout/Dead/说明）
- ✅ **SMP2 × 6 seeds**：同上全绿
- ⚠️ **SMP4 × 6 seeds**：4/6 全绿；2/6 崩溃（`kernel page fault`，形状=dangling 帧恢复/垃圾 PC，与下述遗留档案同族）
- storm 回归演示从验收 boot 摘除（见下「遗留 1」）

**遗留 1（storm × channel 交互，既有回归，独立档案）**：风暴任务（60 空闭包子任务连环 spawn/exit/reap/retire）与 ktask 自切换 park 通道组合时，debug 构建 fence 审计出 12 个 0x80-class 内核堆块 canary 被砸 + `Reaped→Running` 非法状态变换 + 垃圾尺寸分配（2.2GB ≈ 堆区地址值）。实测定性：单跑风暴干净、单跑 channel 干净、两者组合崩溃。疑似写者 = 拿到垃圾 SP 的执行流把内核栈线性扫过块池（canary 只防本块拥有者越界、拦不住外来者中部落笔）。boot.rs 已按注释摘除 storm 调用（函数本体与恢复方式保留在注释中）。

**遗留 2（SMP4 跨核自切换恢复，本会话主症，未修复）**：SMP4 下 ktask 自 park 的跨核唤醒/steal 恢复间歇性损坏（更早形态：restore 目标帧 sepc=帧窗 VA / starved 弹出悬垂 Arc；当前形态：`sepc==stval≈时钟量级` 垃圾 PC）。已排除并记录失败方向：
- 桥改道 trap 路径 `park+run()`：唤醒链断裂（单核停滞，echo 不跑）
- 桥切 per-hart trap 栈：park 期间挂起的 WFI 循环帧被同核他任务 S-timer 陷阱处理器经 trap 栈顶覆写（SMP1 亦坏）
- 桥切专用 .bss 停车栈：布局相关行为（部分变体停滞/崩溃），未收敛
- 原版任务栈 WFI 循环：SMP1/2 自洽（echo 全绿），SMP4 跨核场景有上述间歇损坏
根因侧证据链集中于：任务跨核恢复的帧内容与调度簿记脱钩（容器 Arc 身份错位）、时钟/尺寸字段偶发被清零（CYCLE 类静态被写）。后续排查入口：`restore()` 前对 frame_pa 做页归属校验（banker/ledger），或在 trace 环给 Park/Restore 事件补 `pa` 字段做崩溃回放比价。

### A8.5 · trap 栈固定 VA 布局（2026-08-27，独立落地的诊断稳健性基础设施）

**改动**（`space.rs` / `trampoline.rs` / `trap.rs` / `boot.rs` / `scene.rs`）：

1. **trap 栈窗口固定 VA**：`TRAP_STACK_BASE = FRAME_BASE − MAX_HART_SLOTS·64 KiB`（编译期常量 + 断言，窗口恰止于 FRAME_BASE）。hart h 段 = `BASE + h·64 KiB`（guard 页 + 60 KiB 栈体）；物理页仍 frame 分配，boot 时映射到固定 VA（guard 不映射）。
2. **推 hart 纯算术**：`hart = (sp − BASE) >> 16`（O(1)、零表、零堆依赖）——`establish_tp`、`trap_stack_top/bottom/guard_hart`、崩溃场景钳制全部改为算术；**删除 TRAP_STACKS 元数据表 / TrapStackMeta / SyncCell**（堆破坏不再能经表污染 hart 判定）。
3. **sscratch 约定变更**：内核态 sscratch = 本 hart 内核帧 VA（`KERNEL_FRAME_BASE + hart·PAGE`），boot 与 `__restore` SPP=1 分支维护；崩溃场景按值域判定内核/用户并直接反推 hart。
   - ⚠️ **一处与 A 计划的偏差**：`__strap` 入口 **保留 tp+ LUI 重建帧址**，未改为「sscratch 直取」。原因：`__utrap` 把 sscratch 换成用户 sp 的窗口内，若处理中有内核缺页再入 `__strap`，sscratch 已被污染——不读 sscratch，帧址仍由 tp 重建（tp 为内核态不变量，entry/establish_tp 维持）。
4. **副核 HSM 启动栈**改用 `trap_stack_phys_top(hart)`（物理地址，satp=0 阶段可用；旧 `trap_stack_top` 已返固定 VA，不可作 bare 栈）。

**验收**（release，SMP1/2 × 6 seeds）：panic=0、echo 5/5、semantics 4/4 全绿；SMP4 × 6 seeds 3/6 干净与基线（改动前再现 5/6 崩）对比无回归且略优。崩溃形状（若有）仍属「跨核恢复垃圾帧」遗留族（见遗留 2），与本次改动无关。
