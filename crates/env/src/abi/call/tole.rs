//! call::tole — **Tole 域（class 9：多路等待）**：调用表（[`ToleCall`]）与失败词汇（[`ToleFail`]）。

use super::HoleDir;
use crate::abi::wait::Wait;
use crate::wire::{PieToken, TaskId};
use mold::{Envcall, Fail};

/// Tole 域（class 9：多路等待）的失败词汇。
#[derive(Fail)]
pub enum ToleFail {
    /// token 不在表里 / 不是组 / 权不够。
    Denied = -1,
    /// 组已封印。
    Dead = -2,
    /// 组表备不下。
    OoM = -3,
    /// 这一枚已交出去（交回即复原）。
    HandedOver = -4,
    /// 条件未就绪（`Await` 的等待位在退化上下文里答这一枚）。
    #[busy]
    Busy = -5,
}

/// `ToleFail` 的结果别名。
pub type ToleResult<T> = Result<T, erra::Error<ToleFail>>;

/// **状态订阅观察的来源**（线上那一格：`0` = 任务收尾、`1` = 能力变化）。
///
/// 判别号只写一处（[`Source::wire`] / [`Source::of`] 在类型自己身上，`crate::wire` 那一对
/// impl 只转调）——与 `PieKind::of` 同一条理由：两处各写一遍 `match 0/1` 就是两份判别号表，
/// 日后加一格必漏一处。
///
/// **两格的 `target` 不同类**：`TaskCompleted` 的那一格是**被观察的任务**；
/// `CapabilitiesChanged` 的那一格必须等于**调用者自己**——内核核，不认别人。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// 观察指定 Task 的**退出收尾完成**。
    ///
    /// 通知时刻**不早于**退出钩子跑完且 `Reaped` 已发布；它不宣称栈、帧、Team、Space
    /// 已经归还，也不改动 conductor 的恰好一次计数。观察范围**不比 `Join` 宽**
    /// （授权判据逐条复用 `Join` 那一条）。
    TaskCompleted,
    /// 观察**调用者自己能力的可观察状态改变**。
    ///
    /// 覆盖会影响下一次 `Reserve`/`Alive`、LINK 选择或权限表枚举结果的**成功**变更：
    /// 外来 Accord 到达、`Release`/`Forget`/`Revoke` 及级联摘除、`Narrow`、`Seal`、
    /// 本地创建与移交。失败与纯查询（`Collect`/`Reserve`/`Alive`）不产生通知。
    /// 通知只要求**复核**，不代替任何判据。
    CapabilitiesChanged,
}

impl Source {
    /// 本类型 → 线上那一格。
    pub const fn wire(self) -> usize {
        match self {
            Source::TaskCompleted => 0,
            Source::CapabilitiesChanged => 1,
        }
    }

    /// 线上那一格 → 本类型（**判别号不认识 ⇒ `None`**：读的人按"这一帧读不懂"处置，不猜）。
    pub const fn of(raw: usize) -> Option<Source> {
        match raw {
            0 => Some(Source::TaskCompleted),
            1 => Some(Source::CapabilitiesChanged),
            _ => None,
        }
    }
}

/// Tole 调用（class 9）—— **多路等待**：一枚"组"的六件事：造、挂、摘、等、订、退。
///
/// 与 class 7（权柄轴）的分界：本类不搬许可的生死，只改"这一组我关心哪几枚可等地"
/// ——**成员只有两种：孔（一个方向）与铃**（两者都有"有一位可读的就绪谓词"），
/// 外加**状态订阅**（观察"某个任务的收尾完成"或"我自己的能力变化"）；
/// 与 class 5（数据轴）的分界：本类不搬载荷。组自己的身份就是 `PieToken`
/// （与 Hole/Pole/Nole 同款：号只在持有它的那张表里有意义），资源实体见
/// `work::mail::tole`。
///
/// 组按**自己的 `ONLY`**分两种，种类**只在造的那一刻定、之后不可变**（`Narrow` 不得撤
/// `ONLY`）：独占组（等待位只有一条：授出即移交、复制不出来）与共享组（可复制给多个
/// 任务；组键的唤醒因此是**提示型**——放行全链，人人自取快照复核）。
///
/// 号段取 9：class 3 是空号、也不去复活它（号段口径见文件头）；
/// 8 已归调试面，故本类顺延到 9——判别号是声明顺序，取号不改任何既有号。
#[derive(Envcall)]
#[call(class = 9, fail = ToleFail)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToleCall {
    /// 造一个空组 → `PieToken`。
    ///
    /// `shared` = **这枚组允不允许多个使用者**（就是 `ONLY` 的取反），**只在创建点定、
    /// 之后不可变**（`Narrow` 不得撤 `ONLY`）：
    /// - `false`（独占组）：门闩带 `ONLY` ⇒ 授出即**移交**（源枚 `HandedOver`）、复制不出来；
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
    Attach {
        tole: PieToken,
        pie: PieToken,
        dir: HoleDir,
    },
    /// 从 `tole` 摘掉一格；没挂过即无事。方向归一规则同 [`ToleCall::Attach`]。
    #[ret(())]
    Detach {
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
    /// **`PieToken::NONE` 不等于"肯定没有变化"**：它也可能是"某个状态订阅变了，去复核"。
    /// 组上装了状态订阅时，一次 `NONE` 只说明"这一次没有成员就绪、或者有来源报过事"，
    /// 从不说明"以后也不会有"。
    ///
    /// 错误：token 不在本任务表 / 权不够（组需 `FETCH`）/ 不是组（递了孔、铃、页）
    /// → `-1 Denied`；组已封印 → `-2 Dead`；**组的等待权已被我过户出去**（`ONLY` 的
    /// 移交）→ `-7 HandedOver`。三个码**不折平**（与数据轴 [`MailCall::Wait`] 同款口径）：
    /// `Denied` 是号拿错了、`Dead` 是组没了该换策略、`HandedOver` 是交回即复原。
    #[ret((PieToken, HoleDir))]
    Await { tole: PieToken, millis: Wait },
    /// 把一个**状态来源**登记进组：同 `(source, target)` 重复登记幂等（不重装转发边、
    /// 不重复留提示）。登记成功**先留一次待复核提示**——已有变化、已完成的任务以及
    /// 登记那一刻的状态，都不依赖"未来再来一个事件"。
    ///
    /// 权利：组需 `STORE`（与 [`ToleCall::Attach`] 同一条）；组必须是**当前 Task 本地持有的
    /// 独占组**（带 `ONLY`）——转授、共享组不受理。`Source::CapabilitiesChanged` 的
    /// `target` **必须等于调用者自己**；`Source::TaskCompleted` 的授权判据逐条复用
    /// `UnitCall::Join`（同域或自己的子域）。
    ///
    /// 订阅由组持有，**不发给调用方任何 token**；取消靠同一份来源描述
    /// （见 [`ToleCall::Unsubscribe`]）。登记失败（组表满 / 转发边满）完整回滚。
    ///
    /// 错误同 `Attach`：`Denied` / `Dead` / `OoM`。
    #[ret(())]
    Subscribe {
        tole: PieToken,
        source: Source,
        target: TaskId,
    },
    /// 按**已安装的订阅描述**取消；同描述重复取消无事。
    ///
    /// 只认 `(source, target)` 这条描述：**不要求目标还在世**、不要求还能升级出它的域、
    /// 也不要求它还在 heir 里。权利同 [`ToleCall::Subscribe`]（组需 `STORE`）。
    ///
    /// 错误：`Denied` / `Dead`。
    #[ret(())]
    Unsubscribe {
        tole: PieToken,
        source: Source,
        target: TaskId,
    },
}
