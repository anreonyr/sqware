//! control::core — **判定（纯）**：起不起、起来了没有、收尾完了没有——只读表，不碰内核
//!
//! 正文见 [`protocol::system`]；三档（判定 / 账 / 适配）分家的理由见那一份模块头注。

use crate::program::Ending;

use super::desk::{Announce, Slot, State, Table};

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
pub fn admit_start(table: &Table, name: &str) -> Result<(), Fail> {
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
pub fn probe_ready(table: &Table, name: &str) -> Ready {
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

// **照实记（`Watch` / `probe_watch` 已删）**：它们答的是"身子里还有没有那对坐标"，
// 而 [`Slot`] 的裁决是**死亡记账不清坐标**（留给重启与放下）⇒ 一位已经收尾的 Service 在那里
// 仍答 `Alive`——名不副实，生死该看 [`State`]。全仓零调用者（`watch` 收尾时不再 `detach`），
// 故按"没有读者的格不留在面上"删掉整对：要用"它还在不在"，问 [`probe_ready`]。

// ── 核心：整机那两条判定（只读账）─────────────────────────
//
// **照实记（它们为什么住这里）**：与上面三条同类——都是"由账算出来的判决"，签名里只有表，
// 不碰内核（本模块头注那条口径原样成立）。**与上面三条的分别只有量词**：上面答"这一行"，
// 下面两条答"这一批"。账里已经有"谁结束它"那一格（登记时定死，见 [`Service::restart`]），
// 故这两条**不必回头查那一份声明**——"只读表"这句话仍然是字面意义上的真。

/// **该收了**：账上活着的都是常驻台——会走的都走了、听令的已经发过话。
///
/// **穷尽 `match`**：`Ending` 多一种结束方式，这里就编译不过（不至于静默归成某一类）。
pub fn due(table: &Table) -> bool {
    table.living().all(|r| match r.restart {
        Ending::Resident => true,
        Ending::Transient | Ending::Told => false,
    })
}

/// **还有"会自己走"的活着吗**——收场那一相那条**静默兜底**看这一句。
///
/// 与 [`due`] 的分别只在主体：闸问"还剩谁在等"（常驻不算），本句问"卡住的是不是那种
/// **会自己走**的"。**听令的那一台不算**——它的沉默是正常的，它的终止词从外面来。
pub fn walking(table: &Table) -> bool {
    table.living().any(|r| match r.restart {
        Ending::Transient => true,
        Ending::Resident | Ending::Told => false,
    })
}

/// **收讫了**：账上一个不剩。
///
/// 与内核那一层的收场判决（`conductor::done`：`PUSHED == REAPED`）**同名同形**——四个量词
/// （一台 / 一张单 / 一个域 / 全部任务）共用一个形状，缺的只是这一处。
pub fn done(table: &Table) -> bool {
    table.living().next().is_none()
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

// **照实记（`VestedBy` 那枚函数指针已经退场）**：从前这里有一格"探活"的**注入类型**
// （`pub type VestedBy = fn(PieToken) -> Option<TaskId>`）——板与树各 `pub use` 一份回去，
// 由各自的构造点（`board()` / `tree()`）接上身体。判据一字没改，**注入这一层撤了**：
// 它只有一个身体（[`protocol::communication::establish::vested_by`]），
// 而"接上"这件事只是把同一个函数换个名字传一圈（薄封装）。今天要用它的地方**直接叫**。
// 那一条"活性"的口径（答不出 = 不在我表里 **或** 那扇门已经封印）写在
// [`vested_by`](protocol::communication::establish::vested_by) 上。
