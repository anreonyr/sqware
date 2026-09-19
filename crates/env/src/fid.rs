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
//! Chrono=4, Mail=5, Control=6, **Pie=7**, Debug=8, **Tole=9**。命名与调度词族
//! （conductor）、`runtime::chrono` 域及用户侧 `runtime::env` 同词。
//!
//! **class 3（原 `IO`：`Put`/`Get`）已删**：设备不再是
//! 内核的事——域持门闩、自己读写寄存器，控制台是服务。号段空着不补：**判别号是声明
//! 顺序**，把 4..9 挪下来只会在 ABI 里制造一次无意义的位移。空号即"这条路上没有
//! 内核的入口"，这比复用更准确。
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
//! 改由父任务 `Accord` 下发）。
//!
//! **未知调用号的运行时契约（ABI 的一部分，不是实现细节）**：`a7` 由调用方
//! 完全控制，故它是**输入**而非可信标识。未声明的 class / index（今天只有 class 3
//! 与 ≥ 10 的号段是空的、以及越界索引）一律 decoded 为 `Decode::BadSlot`，
//! 内核侧按**被拒绝**处理：写回负码（`GateError::Denied`）并**续跑调用方**——
//! 与其它用户引起的异常同走故障隔离，绝不 panic（否则用户态一发 `ebreak`
//! 即可停摆整机）。想主动终止有正规原语 `RoomCall::Reap { reason }`——它退的是
//! **调用方那一枚线程**（同域其它线程照旧；域亡与否由成员清零决定），内核照旧活着
//! （那条路一度是内核自己的 `panic!`，即"合法退场比非法调用更危险"）。
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
/// **时间参数的定式**（全树唯一一份，其它处只引用它）：
///
/// - **上限**（"等某事发生，至多等这么久"）：`Wait` / `Fall` / `Join` / `MailCall::Wait` /
///   `ToleCall::Await`，以及协议层的 `service::until/watch`、`Pier::pull`、`Quay::claim`。
///   三态一律：`0` = **只探测**（当场答，不挂起）、`usize::MAX` = **永久**、其余 = 至多毫秒数。
///   超时与"条件成立"**按返回值区分**（各自的 `bool` / 预置值），不另立错误码。
/// - **下限**（"至少这么久不回到我"）：只有 [`RoomCall::Park`]。它的保证成对写成
///   **"不早于 `millis`，且至多晚一拍"**——晚的那一拍由内核的失明上限兜（见
///   `chrono::timer::BLIND_MS`），故 `0` 在 `Park` 上没有"只探测"的含义。
///
/// 两族语义相反（一个谈"至少"，一个谈"至多"），故**不许只写"毫秒数"**：每个时间参数
/// 的文档都要能一眼看出它属于哪一族。
pub enum RoomCall {
    /// 主动让出处理器（词族 starve）。
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
    /// 见 `runtime::env::room::sleep` 的照实记。
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
    #[ret(())]
    Reap {
        reason: usize,
        note: VirtAddr,
        len: usize,
    },
    /// 事件等待（词族 wait）：key + 毫秒——**上限族**（三态见文件头的定式）。
    #[ret(())]
    Wait { key: usize, millis: usize },
    /// 事件唤醒（词族 wake）：key；返回是否唤到人。
    #[ret(bool)]
    Wake { key: usize },
    /// 他杀（词族 doom）：把一个任务送进既有的死亡路径——与 [`RoomCall::Reap`] 成对，
    /// **自杀 ↔ 他杀**。
    ///
    /// **语义 = 杀它所属的域连同它的子树**（不是"只杀这一枚线程"）：与 Linux
    /// `kill <pid>` 同款——pid 指进程，进程的全部线程一并走。`task` 只是"指认域"的手柄。
    ///
    /// 判据只有**判活**——**没有血缘门**：收一个域是"命令"，不是"血缘特权"。曾经要
    /// "目标域沿 `sire` 可达发起者的域"，现在那道门归 `protocol::system` 的编排者
    /// （该协议里的动词叫 `Ruin`）；内核只回答"能不能收"（答"能"）。
    ///
    /// 代价照实记：**服务之间因此没有护栏**——任何域都能拆任何域。收窄只能在编排侧做。
    ///
    /// 失败：`Dead`(-2) 目标从未入册 / 它所属的域已不在世——**"不在世"按域判**：域里
    /// 已经没有还没收尾的线程时也答 `Dead`（与"入土之后名册升不起"同一条口径）。
    /// **不等它回收**——要等用 [`UnitCall::Join`]。
    ///
    /// [`UnitCall::Join`]: crate::fid::UnitCall::Join
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

/// 程序装成的空间（`Build` 的特权级参数）：S 态页表 / U 态页表。
///
/// 它是**内核打包表的产物**，不是程序自述：`build.rs::INITRD_BINS` 决定，root 服务
/// 读取清单后原样转交。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProgramKind {
    /// U 态页表（页带 U 位）。
    User,
    /// S 态域（页不带 U 位，S 态 SUM=0）。
    Supervisor,
}

/// 执行单元调用（class 1）—— unit 域：`Build`（装域）/ `Spawn`（产线程）/ `Hatch`
/// （放行）/ `Join`（等结束）/ `Oust`（放下子域），外加血缘观察
/// （`Sire`/`HeirCount`/`Heir`）。
///
/// **index 是声明顺序判别号**（见文件头）。原 `SpawnTask` 并入 `Spawn` 之后，本枚举
/// 的 index `0..=9` **连续无空号**：`Build` 落在 5、`Hatch`/`Fall`/`Join` 在 6/7/8、
/// `Oust` 在 9——注释占不住槽位，没有变体就没有号。
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
    /// 装域：镜像字节区间 + 特权级 + 名字 → 新域（Space + Team，**无线程**）。
    ///
    /// 名字 ≤ 31 字节（`Name` 的定长上限）。
    ///
    /// # 门
    ///
    /// **没有门。** 曾经是"建域权就是 S 态"——那是内核替调用方定"该不该起"的时候留下
    /// 的。判据现在分家：能不能起由内核回答（答"能"），该不该起归 `protocol::system`
    /// 的编排者（它拿服务表与策略说话），那一侧的动词叫 `Mint`。
    ///
    /// 放开它**不构成提权**：提权的两条路都还堵着——特权级由内核打包表决定（调用方
    /// 说不上话），镜像仍要调用方交字节。
    ///
    /// 能力面不为建域背书，这条没变：曾有一道"存在权"门收一枚 `Nole`，而 `UnsealNole`
    /// 无代价可自铸 ⇒ 那道门与"是 S 态"等价，白收一个载荷而已。
    #[ret(TeamId)]
    Build {
        elf: VirtAddr,
        len: usize,
        kind: ProgramKind,
        name: VirtAddr,
        name_len: usize,
    },
    /// 放行：`Held → Starved`。放行只发生一次——重复调用返回 `-1 Denied`。
    #[ret(())]
    Hatch { task: TaskId },
    /// 等"我自己这张权限表里落进一枚"：`millis`——**上限族**（三态见文件头的定式）。
    ///
    /// **无参数**——等的是调用者自己的表（同 `SelfId` / `Sire`：没有参数就没有伪造面）。
    /// 只报"多了"：只有 `Accord`（别人把副本交进来）会响；自己铸的与 boot 那批不响。
    /// `true` = **调用开始时**已落过（自你上次取走以来），不保证"就是你要的那一枚"
    /// ——醒来自己扫表分辨。
    #[ret(bool)]
    Fall { millis: usize },
    /// 等目标**死透**：`millis`——**上限族**（三态见文件头的定式）。
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
    Join { task: TaskId, millis: usize },
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
    /// 不留残留（`programs/src/bin/stress/again.rs`）。
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

/// 时钟调用（class 4；域 = runtime::chrono）。
#[derive(Envcall)]
#[call(class = 4)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChronoCall {
    /// 读取定时器 tick 计数（诊断，非时间单位）。
    #[ret(usize)]
    Ticks,
    /// 读取单调时钟（自启动基准）：**单字 `u64` 纳秒**。
    ///
    /// 这是本 ABI 的**绝对点**：与 [`RoomCall::ParkUntil`] 的 `at` **同基准、同单位**——
    /// 域读到的点可以（加一个周期之后）原样喂回去。单调不减；实际分辨率是机器
    /// timebase 的一跳（QEMU virt 10 MHz ⇒ **100 ns**，"纳秒"这个单位名比它细）。
    /// `u64` 纳秒覆盖约 584 年，不回绕。展示用的"秒 + 亚秒纳秒"由域自己拆
    /// （`/ 1e9`、`% 1e9`）——ABI 只给一个标量。
    #[ret(u64)]
    Clock,
}

/// hole 的等待方向：`Pull` = 等槽里有消息（可取），`Push` = 等槽空（可发）。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HoleDir {
    Pull,
    Push,
}

/// 通信调用（class 5，mail）—— **数据轴**：消息穿孔 + 门铃。
///
/// 前三个操作作用在一枚 Hole 门闩上：`Push` 写入、`Pull` 取出、`Wait` 等方向就绪；
/// 后两个作用在一枚 Nole 门闩上（门铃）：`Hush` 应铃、`Ring` 响铃。
/// 权柄的生死与流动不在此类，见 [`PieCall`]（class 7）。
///
/// **wait 的分界**：事件键等待留 Room（`RoomCall::Wait/Wake` 的键是调用方命名空间
/// 里的裸整数，内核不解释）；**资源就绪**等待归本类——`Wait` 收 `token`，由内核
/// 解引用出资源自己的等待键，键不出内核。
///
/// **变长孔**：孔**不预设**消息上限——`Push { len }` 与 `Pull { max }` 把长度作为参数
/// 传，长度是契约不是约定。一条消息多大由推者**当时分配得出多少**决定（`try_reserve`
/// 失败即 `OoM`），没有第二条上限。`Pull { max: 0 }` 是"**只问长度**"（不动槽）。
///
/// **空载荷**：门铃没有载荷，故它没有 Push/Pull——"有事"就是那一位本身。`Wait` 复用
/// 在两种资源上，靠 `dir` 分：Hole 两个方向，**Nole 只认 `Pull`**（门铃只有一条方向）。
#[derive(Envcall)]
#[call(class = 5)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MailCall {
    /// push msg：token + msg VA + 长度（≥ 1；**无上限**）。
    #[ret(())]
    Push {
        token: PieToken,
        msg: VirtAddr,
        len: usize,
    },
    /// pull msg：token + 缓冲 VA + 缓冲容量。
    ///
    /// 返 `(实际长度, 发送者 TaskId)`——发送者由**内核在 Push 时盖章**（syscall
    /// 上下文，不可伪造），与消息同槽交付。身份不必再从报文里猜。
    ///
    /// `max == 0` = **只报长度、不动槽**：收方据此备出装得下的缓冲，槽因此总能被
    /// 排空。装不下（`len > max`）答 `-1 Denied` 且槽原样；**空槽（没有可取之事）答
    /// `-3 Busy`**——`max == 0` 的探长也一样，因为"没有可取之事"不是错误而是状态；
    /// **孔已封印答 `-2 Dead`**（内核那一步先过存活闸）。
    /// 这一格正是 [`MailCall::Wait`] 那句"绝不返 `Busy`"的对照面：同一个"未就绪"，
    /// 非阻塞的 `Pull` 用 `Busy` 答、阻塞的 `Wait` 用 `false` 答。
    #[ret((usize, TaskId))]
    Pull {
        token: PieToken,
        buf: VirtAddr,
        max: usize,
    },
    /// 等某方向就绪：`millis`——**上限族**（三态见文件头的定式）。
    ///
    /// 返回 `true` = 本次调用**当场就绪**（未挂起）；`false` = 未就绪（探测失败，
    /// 或挂起过——被唤醒与超时不分）。**绝不返 `-3 Busy`**：未就绪的答案就是 `false`。
    /// 权利：`Pull` 需 R、`Push` 需 W。
    ///
    /// 作用在 Nole（门铃）上时：**`dir` 必须是 `Pull`**——铃只有"响了"这一条方向，
    /// 别的值返 `-1 Denied`（不静默忽略：ABI 不留一个白填的字段）。权利仍按 `dir` 判。
    #[ret(bool)]
    Wait {
        token: PieToken,
        dir: HoleDir,
        millis: usize,
    },
    /// 应铃：清掉"有待取之事"（门铃专用）。权利：R——听与应都在"取"这一侧。
    ///
    /// 未响 ⇒ `-3 Busy`（没有可取之事）。**不唤醒任何人**：没人等"铃不响"。
    #[ret(())]
    Hush { token: PieToken },
    /// 响铃：置"有待取之事"并唤醒听者（门铃专用）。权利：W。
    ///
    /// 已响 ⇒ `-3 Busy`——多 hart 同时响合成一位，第二次起不改变状态。
    /// 这个动词是给**自检**与"自己叫自己"的：没有它，门铃的验证只能等真中断。
    /// 内核响中断那道门铃不走这里（它持着源实体，见 `devices.rs`）。
    #[ret(())]
    Ring { token: PieToken },
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
    /// 解封 Hole（数据过内核管道）：孔上刻**一格记号**（`mark` 处的 `len` 字节）。
    ///
    /// 记号 = **这条路的名字**（[`Name`](crate::wire::Name) 的字节：非空、< 32、无 NUL、
    /// UTF-8），由铸它的那一方在开门这一刻刻上（构造期定型，往后无 setter）。它随副本
    /// 过线、转手不变，故"同一位开的多枚孔"也分辨得出（读它走 [`PieCall::Reserve`]）。
    ///
    /// **消息本身仍不预设上限**，也不预分配槽：这里多出来的只有记号，解封仍是零字节，
    /// 消息多长由每条 `Push` 自己带。
    ///
    /// `len > NAME_LEN`（[`NAME_LEN`](crate::wire::NAME_LEN)）/ 区间未映射 / 名字非法
    /// （空、含 NUL、非 UTF-8）⇒ `Denied`，**不截断、不 panic**。
    #[ret(PieToken)]
    UnsealHole { mark: VirtAddr, len: usize },
    /// 解封 Pole（页级安全内存；大小页对齐）。
    #[ret(PieToken)]
    UnsealPole { size: usize },
    /// 解封 Nole（**无数据面的权柄载体**）：造一枚只有身份与存活的许可载体。
    ///
    /// **无参数**——没有 mtu、没有字节数、没有对齐可校验。它的全部内容就是"这一枚
    /// 存在"，故它是**无载荷通信**的载体（门铃，见 `work::mail::nole::NoleMeta`）。
    /// 与 `UnsealHole`/`UnsealPole` 并列，不是它们的特例。
    #[ret(PieToken)]
    UnsealNole,
    /// 开闩：借映 Pole 物理页进当前 task.space（同 token 幂等复用）→ VA + **整段多大**。
    ///
    /// **两件一起返**：起点与长度是同一段区间的两半，分开取会把"这段有多长"留成
    /// 调用方的猜测——而它恰好只在内核手里（外来区按页界向两侧撑开，`reg` 声明的
    /// 长度内核不知道）。
    ///
    /// 仅对 Pole 成立；权利：需 R。
    #[ret((VirtAddr, usize))]
    Open { token: PieToken },
    /// 关闩：从当前 task.space 解除该 token 的映射（幂等）。
    ///
    /// 仅对 Pole 成立；权利：需 R。
    ///
    /// **不过存活闸**——与 [`PieCall::Release`] 并列，是仅有的两处例外：撤的是**调用方
    /// 自己那张 PTE**，资源已封印也得撤得掉（否则"封印后借入映射撤不掉"）。故已封印的
    /// token 在这里答 `Ok`，不答 `Dead`。
    #[ret(())]
    Shut { token: PieToken },
    /// 封印资源（generic on Hole/Pole）：token。**只有资源开辟者**可做。
    ///
    /// 只置死 + 唤醒等待者，**不摘表项**——持有者仍须 `Release` 收尾（否则泄漏）。
    /// 故本操作之后 `Release` 仍须可用：**`Release` 与 [`PieCall::Shut`] 是仅有的两处
    /// 不过存活闸的操作**（理由各异：一个是"总得能放下手里的东西"，一个是"撤的是
    /// 调用方自己那张 PTE"）。
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
    ///
    /// 错误：token 不在本任务表 → `-1 Denied`；**已封印 → `-2 Dead`**；空子集 / 非单调 /
    /// 撤 `ONLY`（形态位是资源事实）→ `-1 Denied`；Pole 的页表降不下去 → `-1 Denied`。
    ///
    /// **死活先于覆盖子集**：一个已封印的 token **不因为"子集越权"这个判据先撞上就换成
    /// `-1`**——同样的 token 在别的动词上也答 `-2`，答案不该按动词变。这条不是本动词的
    /// 纪律，是共用的取用判据（`gate::locate` + `gate::accede`）的一部分。
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
    /// **唯一的枚举手段**：`protocol::startup::moor()` 靠它发现「父域授给我的那枚门闩」
    /// （未知句柄）。已知句柄求事实用 `Reserve`。
    #[ret((PieToken, crate::permission::Permission, TaskId))]
    Collect { index: usize },
    /// 查这枚门闩的来历：`vestor`（谁授的）+ `owner`（资源谁开的）+ **记号长度**。
    ///
    /// 三个身份不可混用：`vestor` 是**门闩**的来历，转手（Accord）即改写；
    /// `owner` 是**资源**的来历，任意副本共享同一事实——故「目录是谁」经
    /// `owner` 求得，root 转发门闩也不会把身份转丢。
    ///
    /// 第三格是随副本过线的**记号**（[`PieCall::UnsealHole`] 刻的那一格），写进 `mark`
    /// 处至多 `cap` 字节的缓冲，长度由返回值给出（只写内容那一段，尾随填充不上线）。
    /// 它答的是"**这条路上刻的是哪个名字**"——与 `owner` 合起来才认得出"同一位的哪一枚孔"。
    ///
    /// 错误：token 不在本任务表 → `-1 Denied`；资源已封印 → `-2 Dead`；**装不下
    /// （记号长度 > `cap`）→ `-1 Denied` 且缓冲区一个字节都不动**（要么全取、要么一个
    /// 字节都不动，同 [`MailCall::Pull`]）；区间未映射 → `-1 Denied`。**记号只长在孔上**
    /// ——别的资源（Pole/Nole/Tole）问不到记号 ⇒ `-1 Denied`。
    #[ret((TaskId, TaskId, usize))]
    Reserve {
        token: PieToken,
        mark: VirtAddr,
        cap: usize,
    },
    /// 放下：自释本任务的一份门闩（含其全部后代；Pole 同步 unmap）。表里无此 token → -1。
    ///
    /// **不判存活**——与 [`PieCall::Shut`] 并列，是仅有的两处例外：`Seal` 不摘表项，
    /// 若本操作也判存活，封印后的表项就永远摘不掉。语义 =「你总得能放下手里的东西」。
    #[ret(())]
    Release { token: PieToken },
}

/// 调试调用（class 8）：域直接借内核的 DBCN。
///
/// **它不碰设备**：走的是内核自己的调试出口（SBI DBCN），故与「设备不再是内核的事」
/// 不冲突——设备写仍归持设备者，这一格只是**绕过服务**：
/// 引导期服务全都不存在，而"哪里算不下去"只有域知道。
///
/// **它不设构建门**（曾打算挂在 `debug_assertions` 上，实测不成立）：本仓的程序 ELF 由
/// `kernel/build.rs` 用**嵌套 cargo** 打包，那条内层构建与内核那一次**不共享
/// `debug_assertions`** ⇒ ABI 两侧的 `cfg` 会分叉，域调得到一个内核不认的调用号。
/// 一个"只在某些构建里存在"的调用号是**会分叉的 ABI**，代价大于它省下的那点字节。
/// 因此这一格恒在；"生产不该用它"是纪律，不是编译期的事（与 `ControlCall::Backtrace`
/// 同一个折中）。
///
/// 本类落在 **8** 上：class 3（原 `IO`）退役后那个号一直空着，而 8 也没人占——
/// 判别号是声明顺序，取哪个空号都一样，不占任何既有号。
#[derive(Envcall)]
#[call(class = 8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DebugCall {
    /// 把域里的一段字节写进调试控制台（SBI DBCN，**不经过任何服务**）。
    ///
    /// `len == 0` / 区间未映射 ⇒ 负值；返**写出去的字节数**。
    ///
    /// **`len > DBCN_MAX` 是截断，不是错误**：内核那一步是 `len.min(DBCN_MAX)`
    /// （`envcall/debug.rs::put`），只搬前 `DBCN_MAX` 字节，返回值即搬走的长度——
    /// 要"一个字都不少"就调用方自己分段。**与 [`DebugCall::Get`] 的"多出即拒"
    /// 不同形**，这一格是照实测改的（旧注写"⇒ 负值"，与内核不符）。
    #[ret(usize)]
    Put { buf: VirtAddr, len: usize },
    /// 从调试控制台读一段字节写进域：内核一块栈暂存 → `Dbcn::ConsoleRead` → 写回域。
    ///
    /// **不阻塞**：没数据时**当场**返 0 字节——**不是**"等到至少读到一个字节"。
    /// 这是实测口径（OpenSBI v1.9 / QEMU virt；内核侧的照实记见
    /// `kernel/src/runtime/switcher/envcall/debug.rs::get`），旧注写反过，代价是
    /// `programs` 的 `echo` 照它写成紧循环、宿主 99%。拿到 0 的调用方必须自己让一拍
    /// 或等中断，**别立刻再问**（域自己的判断，内核不替它决定）。
    ///
    /// 返**实际写进域的字节数**（`len > DBCN_MAX` ⇒ 负值，不截断）。
    #[ret(usize)]
    Get { buf: VirtAddr, len: usize },
    /// 开关**报文对账**：此后每次一次往返都把"推出去那一帧"与"收回来那一帧"的前
    /// [`DBCN_MAX`] 字节以十六进制打进调试控制台。
    ///
    /// 为什么它是 ABI 的一格而不是一个环境变量：布局错位这类病**只有真实字节能证**，
    /// 而"读代码推布局"正是上一轮连试三轮都没走出来的那条路。
    #[ret(())]
    SetTrace { on: usize },
}

/// [`DebugCall`] 那两格一次能搬的字节数上限。
///
/// 为什么是编译期常量：内核在入口把它拷进**栈上的定长缓冲**（诊断路径不分配，与
/// [`NOTE_MAX`] 同一条理由）。256 够一句 `file:line + 消息`，也够敲一条命令。
pub const DBCN_MAX: usize = 256;

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
///
/// Tole 调用（class 9）—— **多路等待**：一枚"组"的四件事：造、挂、摘、等。
///
/// 与 class 7（权柄轴）的分界：本类不搬许可的生死，只改"这一组我关心哪几枚可等地"
/// ——**成员只有两种：孔（一个方向）与铃**（两者都有"有一位可读的就绪谓词"）；
/// 与 class 5（数据轴）的分界：本类不搬载荷。组自己的身份就是 `PieToken`
/// （与 Hole/Pole/Nole 同款：号只在持有它的那张表里有意义），资源实体见
/// `work::mail::tole`。
///
/// 组按**自己的 `ONLY`**分两种，种类**只在造的那一刻定、之后不可变**（`Narrow` 不得撤
/// `ONLY`）：独占组（等待位只有一条：授出即移交、复制不出来）与共享组（可复制给多个
/// 任务；组键的唤醒因此是**提示型**——放行全链，人人自取快照复核）。
///
/// 号段取 9：class 3（原 `IO`）退役后一直空着、也不去复活它（号段口径见文件头）；
/// 8 已归调试面，故本类顺延到 9——判别号是声明顺序，取号不改任何既有号。
#[derive(Envcall)]
#[call(class = 9)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToleCall {
    /// 造一个空组 → `PieToken`。
    ///
    /// `shared` = **这枚组允不允许多个使用者**（就是 `ONLY` 的取反），**只在创建点定、
    /// 之后不可变**（`Narrow` 不得撤 `ONLY`）：
    /// - `false`（独占组）：门闩带 `ONLY` ⇒ 授出即**移交**（源枚 `Caged`）、复制不出来；
    ///   等待位只有一条 ⇒ 组键一次兑现一个等待者；
    /// - `true`（共享组）：门闩**不带** `ONLY` ⇒ 可 `Accord` **复制**给多个任务、没有锚
    ///   （等待权不会"被关住"）；多个持有者可同时等 ⇒ 组键的唤醒是**提示型**（放行全链，
    ///   人人自取快照复核）。
    ///
    /// 两种都带 `VEST`：共享组若不可复制，"多个使用者"是空话。
    #[ret(PieToken)]
    Unseal { shared: bool },
    /// 把 `pie` 的**一个方向**挂进 `tole`；同成员幂等。
    ///
    /// 成员是孔或铃：孔两个方向都收（`Pull` / `Push`），**铃只认 `Pull`**——它只有
    /// 一条方向（"响了"），别的值不是"暂时没有"，是不存在这个操作（同 `MailCall::Wait`
    /// 的铃通道）。权利：组需 `STORE`，成员需 `FETCH`。
    #[ret(())]
    Hang {
        tole: PieToken,
        pie: PieToken,
        dir: HoleDir,
    },
    /// 从 `tole` 摘掉一格；没挂过即无事。方向归一规则同 [`ToleCall::Hang`]。
    #[ret(())]
    Unhang {
        tole: PieToken,
        pie: PieToken,
        dir: HoleDir,
    },
    /// 等到组里**任意一格**有事 → `(哪一枚, 哪个方向)`；`millis`——**上限族**
    /// （三态见文件头的定式）。
    ///
    /// **两个 `0` 不是一回事**：入参 `millis = 0` 是"只探测、不挂起"；返回值里
    /// `PieToken::NONE`（= 0）+ 方向 = "这次没等到"。别把"没探测到"读成"没挂起"。
    ///
    /// **挂起过一侧返回恒是预置值**（`PieToken::NONE`）：内核没有第二次执行机会
    /// ——调用方按 deadline 循环、醒来自己按组快照复核（与 `UnitCall::Fall` 同款）。
    ///
    /// 错误：token 不在本任务表 / 权不够（组需 `FETCH`）/ 不是组（递了孔、铃、页）
    /// → `-1 Denied`；组已封印 → `-2 Dead`；**组的等待权已被我过户出去**（`ONLY` 的
    /// 移交）→ `-7 Caged`。三个码**不折平**（与数据轴 [`MailCall::Wait`] 同款口径）：
    /// `Denied` 是号拿错了、`Dead` 是组没了该换策略、`Caged` 是交回即复原。
    #[ret((PieToken, HoleDir))]
    Await { tole: PieToken, millis: usize },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnvCall {
    Room(RoomCall),
    Unit(UnitCall),
    Memory(MemoryCall),
    Chrono(ChronoCall),
    Mail(MailCall),
    Control(ControlCall),
    Pie(PieCall),
    /// 调试面（见 [`DebugCall`]）。
    Debug(DebugCall),
    /// 多路等待（见 [`ToleCall`]）。
    Tole(ToleCall),
}

impl EnvCall {
    /// 由调用号 + 寄存器组解码回带载荷的聚合枚举。
    pub fn from_wire(slot: usize, regs: &[usize; 6]) -> Result<Self, crate::wire::Decode> {
        let class = slot >> 32;
        match class {
            0 => Ok(EnvCall::Room(RoomCall::from_wire(slot, regs)?)),
            1 => Ok(EnvCall::Unit(UnitCall::from_wire(slot, regs)?)),
            2 => Ok(EnvCall::Memory(MemoryCall::from_wire(slot, regs)?)),
            4 => Ok(EnvCall::Chrono(ChronoCall::from_wire(slot, regs)?)),
            5 => Ok(EnvCall::Mail(MailCall::from_wire(slot, regs)?)),
            6 => Ok(EnvCall::Control(ControlCall::from_wire(slot, regs)?)),
            7 => Ok(EnvCall::Pie(PieCall::from_wire(slot, regs)?)),
            8 => Ok(EnvCall::Debug(DebugCall::from_wire(slot, regs)?)),
            9 => Ok(EnvCall::Tole(ToleCall::from_wire(slot, regs)?)),
            _ => Err(crate::wire::Decode::BadSlot),
        }
    }
}
