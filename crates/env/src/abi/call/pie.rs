//! call::pie — **Pie 域（class 7，权柄轴：许可的生死与流动）**：调用表（[`PieCall`]）与失败词汇（[`PieFail`]）。

use crate::abi::permission::Permission;
use crate::wire::{Mark, PieToken, TaskId, VirtAddr};
use mold::{Envcall, Fail};

/// Pie 域（class 7：权柄轴）的失败词汇。
#[derive(Fail)]
pub enum PieFail {
    /// 不是开辟者 / 表里没这枚 / 类型不符。
    Denied = -1,
    /// 资源已封印。
    Dead = -2,
    /// 门闩表备不下。
    OoM = -3,
    /// Pole 的 `size` / 区间非法（未页对齐）。
    NotAligned = -4,
    /// 这一枚已交出去（交回即复原）。
    HandedOver = -5,
    #[busy]
    Busy = -6,
}

/// `PieFail` 的结果别名。
pub type PieResult<T> = Result<T, erra::Error<PieFail>>;

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
#[call(class = 7, fail = PieFail)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PieCall {
    /// 解封 Hole（数据过内核管道）：孔上刻**一枚记号**（[`Mark`]）。
    ///
    /// 记号由铸它的那一方在开门这一刻刻上（构造期定型，往后无 setter）。它随副本过线、
    /// 转手不变，故"同一位开的多枚孔"也分辨得出（读它走 [`PieCall::Reserve`]）。
    /// **内核不解释它**：不校验、不比较、不显示，只保管、只递回。
    ///
    /// **消息本身仍不预设上限**，也不预分配槽：这里多出来的只有记号，解封仍是零字节，
    /// 消息多长由每条 `Push` 自己带。
    #[ret(PieToken)]
    UnsealHole { mark: Mark },
    /// 解封 Pole（页级安全内存；大小页对齐）。
    /// shared=false 在创建时带 ONLY；两种形式均自动 RW Open。
    #[ret(PieToken)]
    UnsealPole { size: usize, shared: bool },
    /// 解封 Nole（**无数据面的权柄载体**）：造一枚只有身份与存活的许可载体。
    ///
    /// **无参数**——没有 mtu、没有字节数、没有对齐可校验。它的全部内容就是"这一枚
    /// 存在"，故它是**无载荷通信**的载体（门铃，见 `work::mail::nole::NoleMeta`）。
    /// 与 `UnsealHole`/`UnsealPole` 并列，不是它们的特例。
    ///
    /// **谁都能铸**（与那两处同一口径：**没有特权级门**）：一枚 Nole 能换来的只有"一次唤醒"，
    /// 凭证是"谁把它交给你"（[`PieCall::Accord`]），不是"谁造的"——自铸不构成提权。
    /// 与 [`crate::call::unit::Build`] 那一格的注同一条理由。
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
    /// 转授子集给其他 Task：src_token + dst_id + subset + **记号** → 新 pie 的 token（撤销句柄）。
    ///
    /// `mark` = 给**子枚**刻的那一枚记号（badge）；**`Mark::NONE` = 照源枚**。
    /// 于是"授出时不给记号"（今天全部调用点）与"授出时另刻一枚"共用一个入口，
    /// 而前者与记号还在资源上时的行为逐字相同。解析只在 `gate::accord` 一处。
    #[ret(PieToken)]
    Accord {
        src: PieToken,
        dst: TaskId,
        subset: Permission,
        mark: Mark,
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
    Narrow { token: PieToken, subset: Permission },
    /// 收回授与他人的副本：dst_id + token（`token` = 该副本在**对端表里**的句柄）。
    #[ret(())]
    Revoke { dst: TaskId, token: PieToken },
    /// 收拢：报出本任务权限表第 `index` 份——**这一枚是几号 / 谁开的 / 刻的什么**。
    /// 越界 → 三格全哨兵（`PieToken::NONE` / `TaskId(0)` / `Mark::NONE`），**不报错**。
    ///
    /// **唯一的枚举手段**（[`PieCall::Reserve`] 是它的对偶：一个**按位置**问，一个**按句柄**问）。
    /// 四格一起答，是为了让"扫一遍表"这件事**不必每一枚再问一次 `Reserve`**：那一问是
    /// 一次 envcall（~55 µs），表 16 枚 ⇒ 一趟扫描 6.5 ms 的读数就是这么来的
    /// （见 `programs/src/driver/rtc/adapt/desk.rs`）。
    ///
    /// **宽返回（本枚举唯一一格 [`FromTriple`](crate::wire::FromTriple)）**：
    /// `a0` = token；`a1` = `owner`（这扇门谁开的）；`a2` = **整一枚记号**（64 位）。
    /// 记号整枚独占一格的理由：`a0` 兼作"成 / 不成"那一格，64 位记号有一半最高位是 1，
    /// 挤进任何"按符号读"的寄存器都会被读成出错。
    ///
    /// **哨兵的三重含义与 `Reserve` 同一份判据**：越界 / **这一枚不是活着的孔**（已封印、
    /// 或本来就无记号——Pole/Nole/Tole）⇒ `owner` 与记号都答 `0` / `NONE`。故**收拢这一格
    /// 从不判死活**：答不出的那两格与"没有"同形，读的人只须知道"这一条候选不成立"。
    #[infallible]
    #[ret3((PieToken, TaskId, Mark))]
    Collect { index: usize },
    /// 查这枚门闩的来历：`vestor`（谁授的）+ `owner`（资源谁开的）+ **记号**（第三格）。
    ///
    /// 三个身份不可混用：`vestor` 是**门闩**的来历，转手（Accord）即改写；
    /// `owner` 是**资源**的来历，任意副本共享同一事实——故「目录是谁」经
    /// `owner` 求得，root 转发门闩也不会把身份转丢。
    ///
    /// 第三格是随副本过线的**记号**（[`PieCall::UnsealHole`] 刻的那一格），由返回值直接
    /// 给出——它答的是"**这枚孔是干什么用的**"，与 `owner` 合起来才认得出"同一位的哪一枚孔"。
    ///
    /// 错误：token 不在本任务表 → `-1 Denied`；资源已封印 → `-2 Dead`；**记号只长在孔上**
    /// ——别的资源（Pole/Nole/Tole）问不到记号 ⇒ `-1 Denied`。
    /// **两格打包**：`a0` = `owner` 高 32 位 | `vestor` 低 32 位（两个号都远小于 2^32）；
    /// `a1` = **整一枚记号**（64 位）。
    ///
    /// 两处不能换位，各栽过一次：① 记号挤在 `a0` 的高半或整个放 `a0`——`a0` 是"成 / 不成"
    /// 那一格（用户态按它的**符号**读词表），64 位记号有一半最高位是 1 ⇒ 每次查询都
    /// 被读成出错；② 记号挤在 `a1` 的高 32 位——静默截断，所有认领孔都"记号对不上"。
    #[ret((usize, usize))]
    Reserve { token: PieToken },
    /// 放下：自释本任务的一份门闩（含其全部后代；Pole 同步 unmap）。表里无此 token → -1。
    ///
    /// **不判存活**——与 [`PieCall::Shut`] 并列，是仅有的两处例外：`Seal` 不摘表项，
    /// 若本操作也判存活，封印后的表项就永远摘不掉。语义 =「你总得能放下手里的东西」。
    #[ret(())]
    Release { token: PieToken },
    /// **这一枚还在不在**：在本任务表里（`gate::locate`）**且**没被封印（存活闸）
    /// → `true` / `false`。
    ///
    /// **与 `Reserve` / `Collect` 的分工**：那两格答的是**孔的**来历（`vestor` / `owner`）
    /// 与记号（"这条路的名字"），故对页与铃（Pole / Nole）**答 `Denied`、记号也答不出**
    /// （见那两格的注）；这一格只答**存活**这一件事实，与资源是哪种无关。
    ///
    /// **为什么要有它**：树那一层"把门牌后面那一枚交出去"（`operator` 的 `find`）判的是
    /// "这一枚还能不能交出去"——那是存活，不是"孔的父手还在不在"。从前它借 `Reserve`
    /// 代理，于是**页与铃当门牌时一律被判死**（真机量到：`land=Ok` 而 `find=Err(Dead)`），
    /// 而且顺手把那一格剔掉。补上这一格之后，树对四种资源一视同仁。
    ///
    /// **本格不失败**：不在表里、已封印、号是野的——一律答 `false`（"不在"就是答案，
    /// 不拿负码当第二个说法）。
    #[ret(bool)]
    Alive { token: PieToken },
    /// Drop a borrowed, non-exclusive reference. Direct descendants inherit
    /// its parent, retaining upstream revocation without closing the resource.
    #[ret(())]
    Forget { token: PieToken },
    /// Compare resources behind two live references held by this Task.
    #[ret(bool)]
    Same { a: PieToken, b: PieToken },
    /// Inspect the live resource's immediate transferor, owner and mark, for every kind.
    /// Uses the same packed return layout as Reserve; Reserve remains Hole-only.
    #[ret((usize, usize))]
    Inspect { token: PieToken },
}
