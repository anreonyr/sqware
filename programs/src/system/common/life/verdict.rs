//! common::life::verdict — **判定（纯）**：起不起、起来了没有、收尾完了没有——只读表，不碰内核
//! 正文见 [`protocol::system`]；三档（判定 / 账 / 适配）分家的理由见那一份模块头注。

use crate::unit::Ending;

use super::table::{Announce, Slot, State, Table};

/// 起一个 Service 的**准许**：只有它没在跑、也不在停，才准起。
/// 两种拒绝的理由不同，故不压成一个：名字不在表里 = 调用方写错了；
/// 它已经在跑/在停 = 时机不对。
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

/// **该收了**：账上活着的都是常驻台——会走的都走了、听令的已经发过话。
/// **穷尽 `match`**：`Ending` 多一种结束方式，这里就编译不过（不至于静默归成某一类）。
pub fn due(table: &Table) -> bool {
    table.living().all(|r| match r.restart {
        Ending::Resident => true,
        Ending::Transient | Ending::Told => false,
    })
}

/// **还有"会自己走"的活着吗**——收场那一相那条**静默兜底**看这一句。
/// 与 [`due`] 的分别只在主体：闸问"还剩谁在等"（常驻不算），本句问"卡住的是不是那种
/// **会自己走**的"。**听令的那一台不算**——它的沉默是正常的，它的终止词从外面来。
pub fn walking(table: &Table) -> bool {
    table.living().any(|r| match r.restart {
        Ending::Transient => true,
        Ending::Resident | Ending::Told => false,
    })
}

/// **收讫了**：账上一个不剩。
/// 与内核那一层的收场判决（`conductor::done`：`PUSHED == REAPED`）**同名同形**——四个量词
/// （一台 / 一张单 / 一个域 / 全部任务）共用一个形状，缺的只是这一处。
pub fn done(table: &Table) -> bool {
    table.living().next().is_none()
}

/// "收尾完了没有"的三态判定 —— 与 [`Ready`] 同形：**判决 + 判决的来路**。
/// 为什么判决还要带"来路"：`Join{millis}` 挂起过之后的返回值是**挂起前的预置值**，
/// 不是判决（它只说"醒过"，不说"为什么醒"），故判决只认**非阻塞那一问**；而"问了几次"
/// 正是调用方要写进读数的那一格（`wait=now|waited`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reaped {
    /// 收尾早已完成：**问的那一次**就有结论。
    Now,
    /// 问时还没完成，**挂起等到事件后复探**确认（唯一可靠的那条路）。
    Waited,
    Unsettled,
}

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
