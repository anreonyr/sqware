//! system::core — **判定（纯）**：起不起、起来了没有、还活着没有、收尾完了没有——只读表，不碰内核
//!
//! 正文见 [`super`]；三档（判定 / 账 / 适配）分家的理由见 `system` 模块头注。

use env::{Name, PieToken, TaskId};

use super::desk::{Announce, Service, Slot, State, Table};

// ── 核心：判定（纯函数，只读表）─────────────────────────────

/// 起一个 Service 的**准许**：只有它没在跑、也不在停，才准起。
///
/// 两种拒绝的理由不同，故不压成一个：名字不在表里 = 调用方写错了；
/// 它已经在跑/在停 = 时机不对。
///
/// **重发**：`Dead` 与 `NeverStarted` 同档，故"在同一行上再起"在这道门上是允许的
/// ——但注意 **`stop` 之后状态是 `Stopping`，把 `Dead` 落地的是 `watch`**
/// （`until` 只读不写）。完整序列（stop → watch → Oust → spawn → start）与被踩过的
/// 两处暗礁见 `crate::system` 的 §六。
pub fn admit_start(table: &Table, name: Name) -> Result<(), Fail> {
    let Some(s) = table.find(name) else {
        return Err(Fail::Unknown);
    };
    match s.state {
        State::NeverStarted | State::Dead => Ok(()),
        State::Starting | State::Ready | State::Stopping => Err(Fail::NotReady),
    }
}

/// "起来了没有"的三态判定。**只读表**——"它交回句柄了没有"是内核的事实，由适配写进
/// 表之后这里才读得到（见 `ready`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ready {
    /// 已就绪（它宣布过了，或它这一种根本不需要宣布）。
    Up,
    /// 代表线程已经不在了 ⇒ 它没起来。
    Gone,
    /// 既没宣布、也还活着 ⇒ 继续等。
    Pending,
}

/// 就绪判定（纯）：按**这一行自己声明的**说法解读。
pub fn probe_ready(table: &Table, name: Name) -> Ready {
    let Some(s) = table.find(name) else {
        return Ready::Gone;
    };
    match s.state {
        State::Ready => Ready::Up,
        State::Starting => match (s.announce, s.slot) {
            // 不宣布的那一种：放行就算起来了（它死了会走 `Dead`，那时是 `Gone`）。
            (Announce::None, Slot::Live { .. }) => Ready::Up,
            (Announce::Channel, Slot::Live { .. }) => Ready::Pending,
            (_, Slot::None) => Ready::Gone,
        },
        State::Stopping => Ready::Gone,
        State::NeverStarted | State::Dead => Ready::Gone,
    }
}

/// 还活着没有。
///
/// **照实记（口径未齐）**：本判定读的是"身子里还有没有坐标"，而 [`Slot`] 的裁决是**死亡记账
/// 不清坐标**（留给重启与放下）⇒ 一位已经收尾的 Service 在这里仍答 [`Watch::Alive`]。也就是说
/// 这一对名字（`Alive`/`Gone`）现在名不副实：生死该看 [`State`]。今天没有调用者（`watch` 收尾
/// 时不再 `detach`），故留着不动；要用它的人先裁这一处读法。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Watch {
    Alive,
    Gone,
}

/// 身子判定（纯）：**表里还有没有那对坐标**——不是生死（见 [`Watch`] 的照实记）。
pub fn probe_watch(table: &Table, name: Name) -> Watch {
    match table.find(name) {
        Some(Service {
            slot: Slot::Live { .. },
            ..
        }) => Watch::Alive,
        _ => Watch::Gone,
    }
}

/// "收尾完了没有"的三态判定 —— 与 [`Ready`] 同形：**判决 + 判决的来路**。
///
/// 为什么判决还要带"来路"：`Join{millis}` 挂起过之后的返回值是**挂起前的预置值**，
/// 不是判决（它只说"醒过"，不说"为什么醒"），故判决只认**非阻塞那一问**；而"问了几次"
/// 正是调用方要写进读数的那一格（`wait=now|waited`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reaped {
    /// 收尾早已完成：**问的那一次**就有结论。
    Now,
    /// 问时还没完成，**挂起等到事件后复探**确认（唯一可靠的那条路）。
    Waited,
    /// 有界期内复探仍说没有 ⇒ 收尾未定：收场交给本域退场时的级联。
    Unsettled,
}

// ── 失败域 ──────────────────────────────────────────────────

/// 原语失败的**四种**（对应"调用方接下来该干什么"，不是"内核哪一步坏了"）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 表里没这个名字，或它已经登记过。
    Unknown,
    /// 镜像装不上。
    BadImage,
    /// 表满，或线程/帧产不出来。
    Full,
    /// 没就绪：等到期还没起来、半路死了、或此刻不该起（已在跑）。
    NotReady,
}

// ── 注入的事实：那一枚还答得出吗 ────────────────────────────

/// **活性**：那一枚 Pie 还答得出吗？答不出（`None`）= **它后面的人没了**。
///
/// 它答两件事，而两者在这一格里**不可分**（也不该分）：
///
/// - 那一枚**不在我表里**（令牌越界，或它已被 `Unship` 放下）；
/// - **或**它那扇门**已经封印**：`Reserve` 的 `owner` 那一格带存活闸（内核
///   `envcall/pie.rs` 的 `owner().ok_or(Fail::Dead)`，闸在 `work/unit/gate/pie.rs`
///   的 `alive().then(...)`）⇒ 门一封印就答 `Err(-2 Dead)`，而 `env::fid` 的 `Reserve`
///   注记写着这条契约。**故"答不出"这一格里就有"门封印了"**。
///
/// 返回的 [`TaskId`] 是授与人（原始自持编码为 `TaskId(0)`）。
///
/// **它为什么住这里**（照实记）：板那一份与树那一份原是**两个同名同形、各写一遍的别名**；
/// 两本客人账并成一本之后，共用的那本账要的是**一个**类型 ⇒ 收进 `system::core`，
/// 两处各自 `pub use` 回去（路径照旧，调用点不动）。
pub type VestedBy = fn(PieToken) -> Option<TaskId>;
