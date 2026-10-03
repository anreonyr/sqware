//! call::unit — **Unit 域（class 1：装域 / 创建 task / 血缘）**：调用表（[`UnitCall`]）与失败词汇（[`UnitFail`]）。

use crate::abi::wait::Wait;
use crate::wire::program_kind::ProgramKind;
use crate::wire::{TaskId, TeamId, VirtAddr};
use mold::{Envcall, Fail};

/// Unit 域（class 1：装域 / 创建 task / 血缘）的失败词汇。
#[derive(Fail)]
pub enum UnitFail {
    /// 不在我 heir 里 / 名册里没有这个 id / 启动参数读不出来。
    Denied = -1,
    /// 条件未就绪（`Fall` 没等到 / `Oust` 还有没收尾的 task）。
    #[busy]
    Busy = -2,
    /// 空域、任务准备或发布所需的资源不足。
    OoM = -3,
    /// 用户态 loader 保留的镜像错误码；内核 Build 不解释镜像。
    BadImage = -4,
    /// 入口不是当前域内有效、对齐的可执行地址。
    BadEntry = -5,
}

/// `UnitFail` 的结果别名。
pub type UnitResult<T> = Result<T, erra::Error<UnitFail>>;

/// 执行单元调用（class 1）—— unit 域：`Build`（装域）/ `Spawn`（创建 task）/ `Embark`
/// （放行）/ `Join`（等结束）/ `Oust`（放下子域），外加血缘观察
/// （`Sire`/`HeirCount`/`Heir`），以及挂起和销毁 task。
#[derive(Envcall)]
#[call(class = 1, fail = UnitFail)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnitCall {
    /// 创建 task（**Held**，未放行）：`team`（`TeamId(0)` = 当前域）+ `entry`（0 = 域默认
    /// 入口）+ `args`/`count`（父方空间里的标量参数，内核拷到新任务栈顶；子方
    /// `a0 = args VA`、`a1 = count`）+ `stack`（0 = 默认栈）。
    ///
    /// 产出的 task**一定不会先于 `Embark` 运行**——父方可以先 `Accord` 再放行。
    #[ret(TaskId)]
    Spawn {
        team: TeamId,
        entry: usize,
        args: VirtAddr,
        count: usize,
        stack: usize,
    },
    /// 取当前 task id（无参 → 0 = 无上下文）。
    #[infallible]
    #[ret(TaskId)]
    SelfId,
    /// 溯源：生我者的 task id（0 = 顶级域 / 父已亡）。
    #[infallible]
    #[ret(TaskId)]
    Sire,
    /// 我生的子域数量（heir 枚举的 first pass；0 = 无子域）。
    #[infallible]
    #[ret(usize)]
    HeirCount,
    /// 按索引取子域 TeamId（heir 枚举的 second pass；越界 → 0）。
    #[infallible]
    #[ret(TeamId)]
    Heir { index: usize },
    /// 创建空的 Constructing 域；调用者通过 Mmap 安装程序页。
    /// User 域只能创建 User 域。首次 Spawn 验证入口并提交构造。
    #[ret(TeamId)]
    Build { kind: ProgramKind },
    /// 首次放行或恢复指定 task。
    #[ret(())]
    Embark { task: TaskId },
    /// 等"我自己这张权限表里落进一枚"：`millis`——**上限族**（定式见文件头）。
    ///
    /// **无参数**——等的是调用者自己的表（同 `SelfId` / `Sire`：没有参数就没有伪造面）。
    /// 只报"多了"：只有 `Accord`（别人把副本交进来）会响；自己铸的与 boot 那批不响。
    /// `true` = **调用开始时**已落过（自你上次取走以来），不保证"就是你要的那一枚"
    /// ——醒来自己扫表分辨。
    #[ret(bool)]
    Fall { millis: Wait },
    /// 等目标**死透**：`millis`——**上限族**（定式见文件头）。
    ///
    /// `true` = **调用开始时**目标已死透（未挂起）；`false` = 还没（可能挂起过）。
    /// 调用模式（与 `MailCall::Wait` 同款）：
    /// `loop { if Join{task,0} { break } Join{task,MAX} }`。
    ///
    /// # "死透"到哪一层（契约边界，用户裁决）
    ///
    /// 真 = **① 收尾**完成：目标已死（`TaskState::Reaped`）**且退出钩子（通道级联 +
    /// 能力级联）已跑完**——返回真时，它名下的门闩与通道**都已消失**。
    ///
    /// **② 回收不在契约里**：栈 / trap 帧 / `Team` / `Space` 的归还**由内核择机做**
    /// （延迟回收：不能在自己正在用的栈上回收自己）。故**调用方不得据此推断内存已归还**，
    /// 只可推断"它名下没有活的通道与门闩、也没有 task会再跑"。
    ///
    /// 这条边界是**故意**的：把回收拉进契约就等于把延迟回收变同步（做不到），而需要
    /// "放下/重启它"的那条路（[`UnitCall::Oust`]）本来就**不等**回收——它只要求
    /// "没有还没收尾的 task"。
    #[ret(bool)]
    Join { task: TaskId, millis: Wait },
    /// 放下一个子域：父方**不再认**自己生的这一格（`heir` 表里那一格）。
    ///
    /// **不是**杀（那是 [`RoomCall::Doom`]）、**不是**等（[`UnitCall::Join`]）、**不是**转交
    /// （协议层 `disown` 的"别人接上"那半边）。它只做一件事：把调用者那张血缘表里的一格
    /// 摘掉，放掉那一份强引用。
    ///
    /// # 前置
    ///
    /// - 目标必须在**调用者自己**的 `heir` 表里——那张表本身就是凭证（与 `Spawn` 的门同源），
    ///   故没有第二道权限判据；
    /// - 目标必须**没有还没收尾的 task**（`Reaped` 只算收尾、不算在世；正在埋的那具壳不挡
    ///   这一格），且没有未放行的引导 task——由内核判，不干净答 `-2 Busy`。
    ///
    /// 注意"**不等回收**"是故意的：放下/重启一条路**不需要**等内核把栈/帧/`Space` 还完
    /// （理由与契约边界见 [`UnitCall::Join`]）；实测三轮"起→停→放下→再起"在同一行上
    /// 不留残留（`programs/src/harness/bench/again/again/main.rs`）。
    ///
    /// # 效果
    ///
    /// 那一格消失 ⇒ `HeirCount` / `Heir` 不再报它、`Spawn { team }` 对这个域再也通不过、
    /// `Doom` 级联不再遍历到它 ⇒ 该域除"正在埋的那具壳"外最后一枚长期强引用落地，域对象
    /// （`Team` 与它的 `Space`）随那具壳埋完而析构。
    ///
    /// # 失败
    ///
    /// - 不在调用者的 `heir` 里（没生过 / 已放下过 / 别人的子域）⇒ `-1 Denied`；
    /// - 在表里但还有没收尾的 task ⇒ `-2 Busy`（"条件未就绪"）。
    ///
    /// 一次调用最多摘一格；重复调用答 `Denied`（第二格起它就不在我表里了）。
    #[ret(())]
    Oust { team: TeamId },
    /// 挂起指定 task；Running 目标已请求挂起时返回 Busy，可重试确认。
    #[ret(())]
    Debark { task: TaskId },
    /// 销毁指定 task，保留资源撤销与其拥有的子 team 级联。
    #[ret(())]
    Slay { task: TaskId },

}
