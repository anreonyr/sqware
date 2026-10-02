//! call::unit — **Unit 域（class 1：装域 / 产线程 / 血缘）**：调用表（[`UnitCall`]）与失败词汇（[`UnitFail`]）。

use crate::abi::wait::Wait;
use crate::wire::program_kind::ProgramKind;
use crate::wire::{TaskId, TeamId, VirtAddr};
use mold::{Envcall, Fail};

/// Unit 域（class 1：装域 / 产线程 / 血缘）的失败词汇。
#[derive(Fail)]
pub enum UnitFail {
    /// 不在我 heir 里 / 名册里没有这个 id / 启动参数读不出来。
    Denied = -1,
    /// 条件未就绪（`Fall` 没等到 / `Oust` 还有没收尾的线程）。
    #[busy]
    Busy = -2,
    /// 备料失败（装载头窗口 / 任务帧 / 等待位）。
    OoM = -3,
    /// 镜像不可装载。
    BadImage = -4,
}

/// `UnitFail` 的结果别名。
pub type UnitResult<T> = Result<T, erra::Error<UnitFail>>;

/// 执行单元调用（class 1）—— unit 域：`Build`（装域）/ `Spawn`（产线程）/ `Hatch`
/// （放行）/ `Join`（等结束）/ `Oust`（放下子域），外加血缘观察
/// （`Sire`/`HeirCount`/`Heir`）。
///
/// **index 是声明顺序判别号**（见文件头）。原 `SpawnTask` 并入 `Spawn` 之后，本枚举
/// 的 index `0..=9` **连续无空号**：`Build` 落在 5、`Hatch`/`Fall`/`Join` 在 6/7/8、
/// `Oust` 在 9——注释占不住槽位，没有变体就没有号。
#[derive(Envcall)]
#[call(class = 1, fail = UnitFail)]
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
    /// 装域：镜像字节区间 + 特权级 → 新域（Space + Team，**无线程**）。
    ///
    /// **不收名字**：域名字归装配账，内核一个名字都不记（身份只有号）。
    ///
    /// U-domain 只能创建 User；Supervisor 可创建两种执行空间。
    /// 镜像必须来自调用者可读的普通映射。
    ///
    /// **镜像字节不被拷走**：内核按 ELF 段现读 `elf` 那几页（一份 ELF 里九成以上是
    /// 符号表与调试信息）。故 `elf` 那段区间在调用期间必须一直映射着。
    ///
    /// 失败：`-4 BadImage`（不可装载）/ `-1 Denied`（镜像区读不出来）/ `-3 OoM`（内存不够）。
    #[ret(TeamId)]
    Build {
        elf: VirtAddr,
        len: usize,
        kind: ProgramKind,
    },
    /// 放行：`Held → Starved`。放行只发生一次——重复调用返回 `-1 Denied`。
    #[ret(())]
    Hatch { task: TaskId },
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
    /// 只可推断"它名下没有活的通道与门闩、也没有线程会再跑"。
    ///
    /// 这条边界是**故意**的：把回收拉进契约就等于把延迟回收变同步（做不到），而需要
    /// "放下/重启它"的那条路（[`UnitCall::Oust`]）本来就**不等**回收——它只要求
    /// "没有还没收尾的线程"。
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
    /// - 目标必须**没有还没收尾的线程**（`Reaped` 只算收尾、不算在世；正在埋的那具壳不挡
    ///   这一格），且没有未放行的引导线程——由内核判，不干净答 `-3 Busy`。
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
    /// - 在表里但还有没收尾的线程 ⇒ `-3 Busy`（"条件未就绪"）。
    ///
    /// 一次调用最多摘一格；重复调用答 `Denied`（第二格起它就不在我表里了）。
    #[ret(())]
    Oust { team: TeamId },
}
