//! call::room — **Room 域（class 0：调度词族）**：调用表（[`RoomCall`]）与失败词汇（[`RoomFail`]）。

use crate::abi::wait::Wait;
use crate::wire::{TaskId, VirtAddr};
use mold::{Envcall, Fail};

/// Room 域（class 0：调度词族）的失败词汇。
#[derive(Fail)]
pub enum RoomFail {
    /// 目标域 / 任务已回收（`Doom`）。
    Dead = -1,
    /// 备料失败（`Park` / `ParkUntil` / `Wait` 的等待位备不下）。
    OoM = -2,
    /// 条件未就绪（等待那一路在退化上下文里答这一枚）。
    #[busy]
    Busy = -3,
}

/// `RoomFail` 的结果别名。
pub type RoomResult<T> = Result<T, erra::Error<RoomFail>>;

/// 调度词族调用（class 0；域 = work/room）。
#[derive(Envcall)]
#[call(class = 0, fail = RoomFail)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
/// **时间参数的定式**（全树唯一一份，其它处只引用它）：
///
/// - **上限**（"等某事发生，至多等这么久"）：`Wait` / `Fall` / `Join` / `MailCall::Wait` /
///   `ToleCall::Await`，以及协议层的 `service::until/watch`、`communication::Receiver::recv`、
///   `communication::establish::claim`。
///   **参数类型是 [`Wait`](crate::abi::wait::Wait)**（上限族都在那一格上）：`Wait::AtMost(0)` =
///   **只探测**（当场答，不挂起）、`Wait::Forever` = **永久**、`Wait::AtMost(ms)` = 至多毫秒数。
///   超时与"条件成立"**按返回值区分**（各自的 `bool` / 预置值），不另立错误码。
///
///   **过线那一格仍拼成 `usize`**：`0` = 只探测、`usize::MAX` = 永久、其余 = 毫秒——这一句
///   仍是**唯一一处**；两端各转一次（`Wait::from_wire` / `Wait::to_wire`）。
/// - **下限**（"至少这么久不回到我"）：只有 [`RoomCall::Park`]。它的保证成对写成
///   **"不早于 `millis`，且至多晚一拍"**——晚的那一拍由内核的失明上限兜（见
///   `chrono::timer::BLIND_MS`），故 `0` 在 `Park` 上没有"只探测"的含义。
///
/// 两族语义相反（一个谈"至少"，一个谈"至多"），故**不许只写"毫秒数"**：每个时间参数
/// 的文档都要能一眼看出它属于哪一族。
pub enum RoomCall {
    /// 主动让出处理器（词族 starve）。
    #[infallible]
    #[ret(())]
    Starve,
    /// 睡眠指定毫秒数（词族 park）——**下限**：不早于 `millis` 才可能回到本任务。
    ///
    /// 保证成对：**不早于 `millis`，且至多晚一拍**（那一拍 = 内核的失明上限
    /// `chrono::timer::BLIND_MS`；武装点已收敛为 `min(本核上限, 最近活到点)`，故只有
    /// "没有任何核能替它兑现"时才会吃满那一拍）。`millis = 0` 是"让出一拍"，
    /// **不是**探测——下限族没有三态。
    ///
    /// **域侧换算必须向上取整**：把时长换成 `millis` 时，"至少"要求 **ceil**
    /// （`500µs → 1`、`1.5ms → 2`），否则 `Park{0}` 会把一次亚毫秒睡眠静默变成"让出"。
    /// 见 `runtime::env::room::sleep` 。
    #[ret(())]
    Park { millis: usize },
    /// 退出当前任务（不返回；词族 reap）。发散，无 Ret。
    ///
    /// `reason` = **退出原因码**（数据，不是策略）：`0` = 自愿/正常结束；非 0 = 域自己的
    /// 诊断编号。内核**只记录不解释**，把它写进 trace 的 `RoomEvent::Exit`。
    ///
    /// `note` = 域自己带的一句话（`VirtAddr(0)` + `len = 0` = 无话）：**"哪里算不下去"
    /// 只有域知道**（panic 现场自带的 `file:line` 就是编译器塞进只读段的字面量，不需要
    /// 符号表），而它一旦退场，那段内存也跟着没了——故内核在入口当场 `copy_in` 一段
    /// （至多 [`NOTE_MAX`] 字节，超出部分截断），**由内核自己打印**：不依赖任何服务活着
    /// （"一个算不下去的域先去求一条活路"这条是被拒绝的）。
    ///
    /// 为什么原因码与这句话长在本原语上、而不另立"panic 调用"：**"域不可续"是域的判断，
    /// 内核只需要知道"这个任务不再续跑 + 为什么"**。另立入口等于把域的策略写进 ABI，
    /// 且让"任务终止"这条不变量在 ABI 里有两个出口。
    #[manual]
    #[ret(())]
    Reap {
        reason: usize,
        note: VirtAddr,
        len: usize,
    },
    /// 事件等待（词族 wait）：key + 期限——**上限族**（三态见文件头的定式）。
    #[ret(())]
    Wait { key: usize, millis: Wait },
    /// 事件唤醒（词族 wake）：key；返回是否唤到人。
    #[infallible]
    #[ret(bool)]
    Wake { key: usize },
    /// 他杀（词族 doom）：把一个任务送进既有的死亡路径——与 [`RoomCall::Reap`] 成对，
    /// **自杀 ↔ 他杀**。
    ///
    /// **语义 = 杀它所属的域连同它的子树**（不是"只杀这一枚线程"）：与 Linux
    /// `kill <pid>` 同款——pid 指进程，进程的全部线程一并走。`task` 只是"指认域"的手柄。
    ///
    /// 判据只有**判活**——**没有血缘门**：收一个域是"命令"，不是"血缘特权"。
    /// "该不该收"由 `protocol::system` 的编排者按 `sire` 链的可达性判定（该协议里
    /// 用的是**同一个词**：`doom`）；内核只回答"能不能收"（答"能"）。
    ///
    /// 代价**服务之间因此没有护栏**——任何域都能拆任何域。收窄只能在编排侧做。
    ///
    /// 失败：`Dead`(-2) 目标从未入册 / 它所属的域已不在世——**"不在世"按域判**：域里
    /// 已经没有还没收尾的线程时也答 `Dead`（与"入土之后名册升不起"同一条口径）。
    /// **不等它回收**——要等用 [`UnitCall::Join`]。
    ///
    /// [`UnitCall::Join`]: crate::abi::call::UnitCall::Join
    #[ret(())]
    Doom { task: TaskId },
    /// 睡到**绝对点**（词族 park，与 [`RoomCall::Park`] 成对）：`at` = 自启动基准的
    /// 纳秒标量，与 [`ChronoCall::Clock`] **同基准同单位**。
    ///
    /// **下限族**（口径见文件头的定式）：**不早于 `at` 返回，且至多晚一拍**。
    /// **`at` 已过 ⇒ 当场返回、不挂起**（不是"让出一拍"——让出只留给 `Park{0}`）；
    /// 已过**不报错**：周期任务迟到是常态。
    ///
    /// 周期任务的正确写法（本原语存在的理由）：
    /// `loop { next += period; ParkUntil { at: next }; work(); }` —— 到点是绝对的，
    /// **迟到不累积**。
    #[ret(())]
    ParkUntil { at: u64 },
}

/// `Reap { note }` 的上限：域带的那句话最多这么长，超出部分内核**截断**（不拒绝：
/// 一句被截断的话仍有诊断价值，而拒绝会让"域正在死"这件事多一个失败模式）。
///
/// 为什么是编译期常量：内核在 `Reap` 的入口把它拷进**栈上的定长缓冲**（退场路径不分配），
/// 上限因此必须是常量。128 字节装得下 `消息 + " at file:line:col"`（panic 现场）。
pub const NOTE_MAX: usize = 128;
