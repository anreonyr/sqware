//! 环境调用号（EnvCall）枚举 + 载荷 codec。
//!
//! 方案 3（typed payload）：每个原语是一个**带类型载荷的 variant**，字段类型是
//! envcall 语义句柄（`PieToken`/`TaskId`/`VirtAddr`）或 `Permission`/裸量。调用号
//! （a7）不再 `#[repr(usize)]`+手写 `as usize`，而由 `[derive(Envcall)]` 生成的
//! codec 现算：`(class << 32) | index`，`index` 是**声明顺序**判别号（重排即改
//! ABI，写进本文件注释即文档）。
//!
//! 返回类型（R3）：每个 variant 标 `#[ret(T)]`，derive 生成域 `*Ret` 枚举与
//! `call()`（负值即 `EnvError`，非负蒸馏为 Ret）。`call()` 绑定 envcall 汇编入口，
//! `slot/pack/unpack` 只依赖 `Wire`——sbi 未来可复用同一 derive。
//!
//! 分类按**操作的归属轴**一一对应（class=高 32 位）：Room=0, Unit=1, Memory=2,
//! IO=3, Chrono=4, Mail=5, Control=6, **Pie=7**。命名与调度词族（conductor）、
//! `runtime::chrono` 域及用户侧 `runtime::env` 同词。
//!
//! **5 与 7 的分界是两条正交的轴**（不是按资源种类分，也不是按新旧分）：
//! - **class 5 `Mail` = 数据轴**：消息穿孔。`Push`/`Pull`/`Wait`——传的是**内容**。
//! - **class 7 `Pie` = 权柄轴**：权柄的生死与流动。`Unseal*`/`Seal`/`Open`/`Shut`
//!   /`Accord`/`Narrow`/`Revoke`/`Collect`/`Reserve`/`Release`——传的是**许可**。
//!
//! 两轴正交的判据在代码里：数据轴的臂从**不**调用 `gate` 的权柄函数
//! （`accord`/`narrow`/`revoke`/`release`/`vestor`/`snap`），权柄轴的臂从**不**搬运
//! 载荷。原先 12 个操作同居 class 5，是这两轴的混合——本次拆分即为此。
//! `Pie` 复用原 `ServiceCall` 的空出的 7 号（后者是入口策略而非原语：目录入口门闩
//! 改由父任务 `Accord` 下发，见 `docs/dispatch.md`）。
//!
//! **未知调用号的运行时契约（ABI 的一部分，不是实现细节）**：`a7` 由调用方
//! 完全控制，故它是**输入**而非可信标识。未声明的 class / index（含 class 1 的
//! 空号 index 5、未分配的 class 8、越界索引）一律 decoded 为 `Decode::BadSlot`，
//! 内核侧按**被拒绝**处理：写回负码（`GateError::Denied`）并**续跑调用方**——
//! 与其它用户引起的异常同走故障隔离，绝不 panic（否则用户态一发 `ebreak`
//! 即可停摆整机）。想主动终止有正规原语 `RoomCall::Reap { reason }`——它**只终止
//! 调用方所在的那个域**，内核照旧活着（§10.34 修的就是这条：那条路一度是内核
//! 自己的 `panic!`，即"合法退场比非法调用更危险"）。
//! 本文件是这条契约的**单一真相**：`slot` 的生成与解码都在此处。
//!
//! 根除的两处 L3' 漏洞：`Permission`/`PteFlags` 的 unpack 走 `from_bits(...)`
//! `.ok_or(...)` 校验（见 [`Wire`](crate::wire::Wire)），非法位 → `Err`，不再
//! `from_bits_truncate` 静默截断。

use envmacros::Envcall;

use crate::wire::{PieToken, TaskId, TeamId, VirtAddr};

/// 调度词族调用（class 0；域 = work/room）。
#[derive(Envcall)]
#[call(class = 0)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RoomCall {
    /// 主动让出处理器（词族 starve）。
    #[ret(())]
    Starve,
    /// 睡眠指定毫秒数（词族 park）。
    #[ret(())]
    Park { millis: usize },
    /// 退出当前任务（不返回；词族 reap）。发散，无 Ret。
    ///
    /// `reason` = **退出原因码**（数据，不是策略）：`0` = 自愿/正常结束；非 0 = 域自己的
    /// 诊断编号。内核**只记录不解释**，把它写进 trace 的 `RoomEvent::Exit`。
    ///
    /// 为什么原因码长在本原语上、而不是另立一个"panic 调用"：**"域不可续"是域的判断，
    /// 内核只需要知道"这个任务不再续跑 + 为什么"**。另立入口等于把域的策略写进 ABI，
    /// 且让"任务终止"这条不变量在 ABI 里有两个出口（§10.36）。
    #[ret(())]
    Reap { reason: usize },
    /// 事件等待（词族 wait）：key + 毫秒（usize::MAX = 永久）。
    #[ret(())]
    Wait { key: usize, millis: usize },
    /// 事件唤醒（词族 wake）：key；返回是否唤到人。
    #[ret(bool)]
    Wake { key: usize },
}

/// 程序装成的空间（`Build` 的特权级参数）：S 态页表 / U 态页表。
///
/// 它是**内核打包表的产物**，不是程序自述：`build.rs::INITRD_BINS` 决定，root 服务
/// 读取清单后原样转交（见 `docs/supervisor.md` §13、`docs/root.md`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProgramKind {
    /// U 态页表（页带 U 位）。
    User,
    /// S 态域（页不带 U 位，S 态 SUM=0）。
    Supervisor,
}

/// 执行单元调用（class 1）—— unit 域：`Build`（装域）/ `Spawn`（产线程）/ `Hatch`
/// （放行）/ `Join`（等结束），外加血缘观察（`Sire`/`HeirCount`/`Heir`）。
///
/// **index 5 是空号**（原 `SpawnTask` 已并入 `Spawn`）——保留不复用；index 是声明
/// 顺序判别号，见文件头。
#[derive(Envcall)]
#[call(class = 1)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnitCall {
    /// 产线程（**Held**，未放行）：`team`（`TeamId(0)` = 当前域）+ `entry`（0 = 域默认
    /// 入口）+ `args`/`count`（父方空间里的标量参数，内核拷到新任务栈顶；子方
    /// `a0 = args VA`、`a1 = count`）+ `stack`（0 = 默认栈）。
    ///
    /// 产出的线程**一定不会先于 `Hatch` 运行**——父方可以先 `Accord` 再放行。
    #[ret(TaskId)]
    Spawn {
        team: TeamId,
        entry: usize,
        args: VirtAddr,
        count: usize,
        stack: usize,
    },
    /// 取当前 task id（无参 → 0 = 无上下文）。
    #[ret(TaskId)]
    SelfId,
    /// 溯源：生我者的 task id（0 = 顶级域 / 父已亡）。
    #[ret(TaskId)]
    Sire,
    /// 我生的子域数量（heir 枚举的 first pass；0 = 无子域）。
    #[ret(usize)]
    HeirCount,
    /// 按索引取子域 TeamId（heir 枚举的 second pass；越界 → 0）。
    #[ret(TeamId)]
    Heir { index: usize },
    // index 5：原 SpawnTask —— 空号，不复用。
    /// 装域：镜像字节区间 + 特权级 + 名字 → 新域（Space + Team，**无线程**）。
    ///
    /// 名字 ≤ 31 字节（`Name` 的定长上限）。
    ///
    /// # 两道门
    ///
    /// 1. **建域权**：`build` 必须是**调用方自己表里**一枚活着的 `Nole`（存在权的
    ///    载体，见 `PieCall::UnsealNole`）。token 不自证——内核只在调用方的表里找它，
    ///    故"拿别人的 token"不是绕过面。带它是为了让权威**显式可审计**（同
    ///    `Reserve`/`Release` 的形态："你说的是哪一枚"）。
    /// 2. **S 态兜底**：调用方仍须是 supervisor 域。
    ///
    /// 两道门不是冗余：能力回答"**谁有权**"，S 态回答"**血缘树能不能伸进沙箱外**"
    /// ——`Build` 出来的域以调用方为 `sire`，若允许 U 态域建域，沙箱里的任务就成了
    /// 别的域的父亲，那是本仓没有的形态。先别开这个口子。
    #[ret(TeamId)]
    Build {
        elf: VirtAddr,
        len: usize,
        kind: ProgramKind,
        name: VirtAddr,
        name_len: usize,
        build: PieToken,
    },
    /// 放行：`Held → Starved`。放行只发生一次——重复调用返回 `-1 Denied`。
    #[ret(())]
    Hatch { task: TaskId },
    /// 等目标回收：`millis`（0 = 只探测，`usize::MAX` = 永久）。
    ///
    /// `true` = **调用开始时**目标已回收（未挂起）；`false` = 未回收（可能挂起过）。
    /// 调用模式（与 `MailCall::Wait` 同款）：
    /// `loop { if Join{task,0} { break } Join{task,MAX} }`。
    #[ret(bool)]
    Join { task: TaskId, millis: usize },
}

/// 内存调用（class 2；trace 事件名 `MemoryEvent` 同词）。
#[derive(Envcall)]
#[call(class = 2)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MemoryCall {
    /// 用户堆分配（字节数，页对齐向上取整）。
    #[ret(VirtAddr)]
    Allocate { size: usize },
    /// 用户堆释放（VA，字节数，页对齐）。
    #[ret(())]
    Deallocate { addr: VirtAddr, size: usize },
    /// 高位大段懒匿名映射（字节数页对齐；at = 期望 VA，VirtAddr(0) = 窗口自选）。
    #[ret(VirtAddr)]
    Mmap { size: usize, at: VirtAddr },
    /// 释放 mmap/声明区域（VA，字节数，页对齐）。
    #[ret(())]
    Munmap { addr: VirtAddr, size: usize },
    /// 修改映射区域保护标志（VA，字节数页对齐，新权限 PteFlags 位）。
    #[ret(())]
    Mprotect {
        addr: VirtAddr,
        size: usize,
        flags: u64,
    },
}

/// IO 调用（class 3）。
#[derive(Envcall)]
#[call(class = 3)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IOCall {
    /// 写缓冲（len，缓冲 VA）。
    #[ret(())]
    Put { len: usize, buf: VirtAddr },
    /// 非阻塞读一字节；无输入 → -3 Busy。
    ///
    /// **内核侧契约**：成功时 `a0` 恰是那一字节（`console::pull()` 给的 `u8` 零扩展），
    /// 故 `a0 ∈ 0..=255`。这不是"约定"而是本原语的形状——但它**只由内核的实现承载**，
    /// 故 `FromPair for u8` 在 `debug_assertions` 档把它查出来（见 `wire/frompair.rs`）。
    #[ret(u8)]
    Get,
}

/// 时钟调用（class 4；域 = runtime::chrono）。
#[derive(Envcall)]
#[call(class = 4)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChronoCall {
    /// 读取定时器 tick 计数（诊断，非时间单位）。
    #[ret(usize)]
    Ticks,
    /// 读取单调时钟（uptime）：(秒, 亚秒纳秒)。
    #[ret((u64, u64))]
    Clock,
}

/// hole 的等待方向：`Pull` = 等槽里有消息（可取），`Push` = 等槽空（可发）。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HoleDir {
    Pull,
    Push,
}

/// 通信调用（class 5，mail）—— **数据轴**：消息穿孔。
///
/// 三个操作都作用在一枚 Hole 门闩上：`Push` 写入、`Pull` 取出、`Wait` 等方向就绪。
/// 权柄的生死与流动不在此类，见 [`PieCall`]（class 7）。
///
/// **wait 的分界**：事件键等待留 Room（`RoomCall::Wait/Wake` 的键是调用方命名空间
/// 里的裸整数，内核不解释）；**资源就绪**等待归本类——`Wait` 收 `token`，由内核
/// 解引用出 hole 的等待键，键不出内核。
///
/// **变长孔**：`UnsealHole { mtu }` 在 unseal 时定该孔消息上限（1..=4096）；
/// `Push { len }` 与 `Pull { max }` 把长度作为参数传——长度是契约不是约定。
#[derive(Envcall)]
#[call(class = 5)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MailCall {
    /// push msg：token + msg VA + 长度（1..=该孔 mtu）。
    #[ret(())]
    Push {
        token: PieToken,
        msg: VirtAddr,
        len: usize,
    },
    /// pull msg：token + 缓冲 VA + 上限（≥1 且 ≤该孔 mtu）。
    ///
    /// 返 `(实际长度, 发送者 TaskId)`——发送者由**内核在 Push 时盖章**（syscall
    /// 上下文，不可伪造），与消息同槽交付。身份不必再从报文里猜。
    #[ret((usize, TaskId))]
    Pull {
        token: PieToken,
        buf: VirtAddr,
        max: usize,
    },
    /// 等某方向就绪：`millis` 毫秒（`usize::MAX` = 永久，`0` = 只探测不挂起）。
    ///
    /// 返回 `true` = 本次调用**当场就绪**（未挂起）；`false` = 未就绪（探测失败，
    /// 或挂起过——被唤醒与超时不分）。**绝不返 `-3 Busy`**：未就绪的答案就是 `false`。
    /// 权利：`Pull` 需 R、`Push` 需 W。
    #[ret(bool)]
    Wait {
        token: PieToken,
        dir: HoleDir,
        millis: usize,
    },
}

/// 权柄调用（class 7，pie）—— **权柄轴**：许可的生死与流动。
///
/// 用户句柄统一为 per-pie `token`（全局唯一）。本类**不搬运载荷**——传的是许可，
/// 内容走 [`MailCall`]（class 5）。两轴正交，见文件头。
///
/// # 三条轴
///
/// **资源轴**（动的是资源本身）：`Unseal*` ↔ `Seal` 是资源寿命的两端（不可逆）；
/// `Open` ↔ `Shut` 是杆闩的开合（可逆的日常）。`Open`/`Shut` 只对 Pole 成立——
/// Hole 的"开闩"就是 `MailCall::Push`/`Pull`。
///
/// **持有轴**（动的是我表里的那一份）：`Collect`（按 index 枚举出我表里的）↔
/// `Release`（放下我持有的一枚）。两个方向都不需要权限位。
///
/// **转授轴**（跨任务）：`Accord`（授出子集）↔ `Revoke`（收回授出的）。`Narrow`
/// 是就地收窄自己那一份，同属权限大小这一维。
///
/// `Reserve` 与 `Collect` 分工：`Collect` 按 index 枚举（发现未见过的句柄），
/// `Reserve` 按句柄查事实（vestor = 父门闩的持有者，owner 随资源不变）。
#[derive(Envcall)]
#[call(class = 7)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PieCall {
    /// 解封 Hole（数据过内核管道）；`mtu` = 该孔单消息上限（1..=4096）。
    #[ret(PieToken)]
    UnsealHole { mtu: usize },
    /// 解封 Pole（页级安全内存；字节数页对齐）。
    #[ret(PieToken)]
    UnsealPole { bytes: usize },
    /// 解封 Nole（**无数据面的权柄载体**）：造一枚只有身份与存活的许可载体。
    ///
    /// **无参数**——没有 mtu、没有字节数、没有对齐可校验。它的全部内容就是"这一枚
    /// 存在"，故它承载的是**存在权**（第一位消费者：建域权 `UnitCall::Build`）。
    /// 与 `UnsealHole`/`UnsealPole` 并列，不是它们的特例。
    #[ret(PieToken)]
    UnsealNole,
    /// 开闩：借映 Pole 物理页进当前 task.space（同 token 幂等复用）→ VA。
    ///
    /// 仅对 Pole 成立；权利：需 R。
    #[ret(VirtAddr)]
    Open { token: PieToken },
    /// 关闩：从当前 task.space 解除该 token 的映射（幂等）。
    ///
    /// 仅对 Pole 成立；权利：需 R。
    #[ret(())]
    Shut { token: PieToken },
    /// 封印资源（generic on Hole/Pole）：token。**只有资源开辟者**可做。
    ///
    /// 只置死 + 唤醒等待者，**不摘表项**——持有者仍须 `Release` 收尾（否则泄漏）。
    /// 故本操作之后 `Release` 仍须可用：`Release` 是唯一不过存活闸的操作。
    #[ret(())]
    Seal { token: PieToken },
    /// 转授子集给其他 Task：src_token + dst_id + subset → 新 pie 的 token（撤销句柄）。
    #[ret(PieToken)]
    Accord {
        src: PieToken,
        dst: TaskId,
        subset: crate::permission::Permission,
    },
    /// 收窄本 pie 权限（就地改写；Pole 同步降页表）：token + subset。
    #[ret(())]
    Narrow {
        token: PieToken,
        subset: crate::permission::Permission,
    },
    /// 收回授与他人的副本：dst_id + token（`token` = 该副本在**对端表里**的句柄）。
    #[ret(())]
    Revoke { dst: TaskId, token: PieToken },
    /// 收拢：报出本任务权限表第 `index` 份（token + permission + vestor）。
    /// 越界 → `PieToken(0)`（无效哨兵，不报错）；vestor = None 时返 `TaskId(0)`。
    ///
    /// **唯一的枚举手段**：`handshake::moor()` 靠它发现「父域授给我的那枚门闩」
    /// （未知句柄）。已知句柄求事实用 `Reserve`。
    #[ret((PieToken, crate::permission::Permission, TaskId))]
    Collect { index: usize },
    /// 查这枚门闩的来历：`vestor`（谁授的）+ `owner`（资源谁开的）。
    ///
    /// 两个身份不可混用：`vestor` 是**门闩**的来历，转手（Accord）即改写；
    /// `owner` 是**资源**的来历，任意副本共享同一事实——故「目录是谁」经
    /// `owner` 求得，root 转发门闩也不会把身份转丢。
    ///
    /// 错误：token 不在本任务表 → `-1 Denied`；资源已封印 → `-2 Dead`。
    #[ret((TaskId, TaskId))]
    Reserve { token: PieToken },
    /// 放下：自释本任务的一份门闩（含其全部后代；Pole 同步 unmap）。表里无此 token → -1。
    ///
    /// **唯一不判存活的操作**：`Seal` 不摘表项，若本操作也判存活，封印后的表项
    /// 就永远摘不掉。语义 =「你总得能放下手里的东西」。
    #[ret(())]
    Release { token: PieToken },
}

/// 控制调用（class 6）。
#[derive(Envcall)]
#[call(class = 6)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ControlCall {
    /// 用户自诊断：采样当前任务调用栈，把 pc 地址数组写进用户 buf，返回帧数。
    ///
    /// `buf` = 用户预分配的 `[usize; N]` 数组 VA；`frames` = 该数组最大容量。
    /// 内核经 `mail::copy_out` 写 `frames` 个 pc 到 buf；返回实际捕获帧数（`usize`），
    /// buf 非法（未映射/不可写）→ 负值（EnvError）。
    #[ret(usize)]
    Backtrace { buf: usize, frames: usize },
}

/// 环境调用号聚合（内核侧解码总入口）。
///
/// `from_wire(slot, regs)` 按 class（高 32 位）分派到各域的 `from_wire`，得到
/// `PieCall::Seal { .. }` 等带载荷 variant，供 `dispatch` match。用户侧不再构造
/// 本枚举——直接 `PieCall::X.call()` 发起（R3+B）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnvCall {
    Room(RoomCall),
    Unit(UnitCall),
    Memory(MemoryCall),
    IO(IOCall),
    Chrono(ChronoCall),
    Mail(MailCall),
    Control(ControlCall),
    Pie(PieCall),
}

impl EnvCall {
    /// 由调用号 + 寄存器组解码回带载荷的聚合枚举。
    pub fn from_wire(slot: usize, regs: &[usize; 6]) -> Result<Self, crate::wire::Decode> {
        let class = slot >> 32;
        match class {
            0 => Ok(EnvCall::Room(RoomCall::from_wire(slot, regs)?)),
            1 => Ok(EnvCall::Unit(UnitCall::from_wire(slot, regs)?)),
            2 => Ok(EnvCall::Memory(MemoryCall::from_wire(slot, regs)?)),
            3 => Ok(EnvCall::IO(IOCall::from_wire(slot, regs)?)),
            4 => Ok(EnvCall::Chrono(ChronoCall::from_wire(slot, regs)?)),
            5 => Ok(EnvCall::Mail(MailCall::from_wire(slot, regs)?)),
            6 => Ok(EnvCall::Control(ControlCall::from_wire(slot, regs)?)),
            7 => Ok(EnvCall::Pie(PieCall::from_wire(slot, regs)?)),
            _ => Err(crate::wire::Decode::BadSlot),
        }
    }
}
