//! system::server — **适配**：把内核答的事实写回表（建域 / 放行 / 等就绪 / 收 / 盯）
//!
//! 正文见 [`super`]；三档（判定 / 账 / 适配）分家的理由见 `system` 模块头注。

use env::{Name, Permission, PieToken, ProgramKind, TaskId};

use crate::session::Quay;

use super::core::{Fail, Ready, Reaped, admit_start, probe_ready};
use super::desk::{Announce, Service, Slot, State, Table};

// ── 适配：原语（转发到运行时那几件）──────────────────────────

/// 起跑前要交出去的一枚门闩：给哪一枚、多大权。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Grant {
    /// 要交出去的那一枚（**在我表里**的句柄）。
    pub token: PieToken,
    /// 交出去的权限子集。
    pub perm: Permission,
}

/// 起一个 Service。
///
/// `image` = 镜像字节（v1 口径）；`kind` = 特权级（**由清单决定**，调用方转交）；
/// `grants` = **放行前**要交到它手里的门闩（空 = 什么都不预先给）；
/// `quay` = 与它的会话（`Announce::Channel` 那一种才有：它就绪的凭据长在这里）；
/// `ms` = 就绪等待（`0` 只探、`usize::MAX` 无期限、其余毫秒）。
///
/// **失败时不留下半行**：产线程**之前**失败 ⇒ 表不动；产线程**之后**失败 ⇒ 实例与
/// 状态都如实留在表里（它确实在跑），调用方用 [`stop`] 收尾。
pub fn spawn(
    table: &mut Table,
    name: Name,
    image: &[u8],
    kind: ProgramKind,
) -> Result<TaskId, Fail> {
    admit_start(table, name)?;

    let team = crate::system::call::mint(image, kind, name)?;
    let Ok(rep) = crate::system::call::bear(team) else {
        return Err(Fail::NoRoom);
    };
    table.attach(name, team, rep)?;
    table.set_state(name, State::Starting);
    Ok(rep)
}

/// 第二相：**放行**，并在放行前塞门闩、定会话。
///
/// `grants` = 放行前要交到它手里的门闩（空 = 什么都不预先给）；`quay` = 与它的会话
/// （`Announce::Channel` 那一种要用：它就绪的凭据长在这里）；`marks` = 放行后要逐条
/// 认领的**记号**（顺序无关；记号即泊位名，装配单给的通道名就是它）；`ms` = 就绪等待
/// （`0` 只探、`usize::MAX` 无期限、其余毫秒）。
///
/// **两相之间的窗口就是"它一步都还没跑"**——塞门闩、定会话都发生在这段窗口里，这与
/// 载体的"恒产未放行"是同一条。
///
/// **失败时不留下半行**：`spawn` 失败 ⇒ 表不动；本相失败 ⇒ 实例与状态如实留在表里
/// （它确实在跑），调用方用 [`stop`] 收尾。
///
/// # Errors
/// 见 [`Fail`]。
/// `ms` = 就绪判定的**上限族**毫秒（口径见 `env::fid` 文件头的定式：`0` 只探测、
/// `usize::MAX` 永久）——超时与"成立了"按返回值区分。
pub fn start(
    table: &mut Table,
    name: Name,
    rep: TaskId,
    grants: &[Grant],
    quay: Option<&mut Quay>,
    marks: &[Name],
    ms: usize,
) -> Result<(), Fail> {
    let launched = (|| -> Result<(), Fail> {
        for g in grants {
            crate::system::call::accord(g.token, rep, g.perm)?;
        }
        crate::system::call::hatch(rep)
    })();
    if let Err(e) = launched {
        crate::system::call::ruin(rep);
        table.detach(name);
        table.set_state(name, State::Dead);
        return Err(e);
    }
    ready(table, name, quay, marks, ms)?;
    Ok(())
}

/// 等它就绪。`true` = **调用开始时就已就绪**（挂起过的一律 `false`）。
///
/// 唯一会改表的地方就是这个函数：确认它的宣布之后置 [`State::Ready`]，确认它死了之后
/// 落 [`State::Dead`]——**内核的事实在这里变成表里的事实**。而"它宣布了"那件事本身
/// **归 [`session`](crate::session)**：会话在对方交回孔并归位时成立，本函数只去问那句
/// "成立了没有"。
pub fn ready(
    table: &mut Table,
    name: Name,
    quay: Option<&mut Quay>,
    marks: &[Name],
    ms: usize,
) -> Result<bool, Fail> {
    // 先看表：上一次问过的事实（按这一行自己声明的说法解读）。
    if let Ready::Up = probe_ready(table, name) {
        return Ok(true);
    }
    let Some(Service {
        slot: Slot::Live { rep, .. },
        announce,
        ..
    }) = table.find(name)
    else {
        return Err(Fail::NotReady);
    };
    let (rep, announce) = (*rep, *announce);

    // 它不宣布的那一种：放行之后只要还活着就算起来了，没有可等的东西。
    if announce == Announce::None {
        if crate::system::call::running(rep) {
            table.set_state(name, State::Ready);
            return Ok(false);
        }
        table.detach(name);
        table.set_state(name, State::Dead);
        return Err(Fail::NotReady);
    }

    // 它会交回一枚孔 ⇒ 那件事归会话：`claim` 把它认下来（凑齐了才算）。
    if let Some(q) = quay {
        // 它会交回孔、也会自己装一条 ⇒ 那件事归会话：**逐条按记号认领**（记号 = 那条
        // 泊位的名字 = 装配单给的通道名），每条都配齐才算起来。认领的是**它**交上来的
        // 那一批（`owner` = 这个孩子）：孔交给的是"生我者"（建域那一枚线程），而
        // **"谁的孔"与"我认的对端"是两件事**——见 `Quay::claim` 的正文。
        if !marks.is_empty() && marks.iter().all(|mark| q.claim(rep, *mark, ms).is_ok()) {
            table.set_state(name, State::Ready);
            return Ok(false);
        }
    }
    if !crate::system::call::running(rep) {
        table.detach(name);
        table.set_state(name, State::Dead);
        return Err(Fail::NotReady);
    }
    // 还活着、只是没宣布：它确实在跑，**实例如实留在表里**（调用方用 `stop` 收尾）。
    if ms == 0 {
        return Ok(false);
    }
    Err(Fail::NotReady)
}

/// 收掉一个 Service。**下令即回，不等它收完**。
pub fn stop(table: &mut Table, name: Name) -> Result<(), Fail> {
    let Some(Service {
        slot: Slot::Live { rep, .. },
        ..
    }) = table.find(name)
    else {
        return Err(Fail::Unknown);
    };
    let rep = *rep;
    crate::system::call::ruin(rep);
    table.set_state(name, State::Stopping);
    Ok(())
}

/// 等它收尾：`ms` 三态与 [`ready`] 同款（`0` 只探、`usize::MAX` 挂到它收尾、其余毫秒）。
/// **只读：不动表**。
///
/// 形状是 **问 → 等 → 问**，判决只认两次**非阻塞问**（`Join{rep, 0}`）；等只是为了少问几次。
/// `Join{rep, ms}` 挂起过之后的返回值不含信息（见 [`Reaped`]），故醒来必须复探——**"他杀
/// 偶发不生效"那条错读就是漏了这一步**：把"醒过"当成了"没收到"。
///
/// 为什么有界等不需要 clock、也不必睡满 `ms`：`WakeKey::Task{id}` 上的投信方只有 `wipe`，
/// 而它只在 `bury` 里、`Reaped` 置位**之后**调（`kernel/src/work/room/messenger/reap.rs`）
/// ⇒ 等待里的"早醒"只可能来自收尾；"到点"那一支由复探分出来（答 [`Reaped::Unsettled`]）。
///
/// `Err(Fail::Unknown)` = 表里没这一行、或这一行还没有身子的坐标。问不出（`Denied` =
/// 已入土 / 从未入册）按"收尾了"处理——与 [`running`](crate::system::call) 同一折法。
/// `ms` = **上限族**（口径见 `env::fid` 文件头的定式）；超时那支答 [`Reaped::Unsettled`]，
/// 不写表。
pub fn until(table: &Table, name: Name, ms: usize) -> Result<Reaped, Fail> {
    let Some(rep) = live_rep(table, name) else {
        return Err(Fail::Unknown);
    };
    if !crate::system::call::running(rep) {
        return Ok(Reaped::Now);
    }
    if ms == 0 {
        return Ok(Reaped::Unsettled);
    }
    // 挂起等一记：醒来自收尾（`wipe`）或到点，两者当场分不开 ⇒ 醒来复探，判决只认它。
    let _ = runtime::env::unit::join(rep, ms);
    if crate::system::call::running(rep) {
        Ok(Reaped::Unsettled)
    } else {
        Ok(Reaped::Waited)
    }
}

/// 这一行身子的代表线程（没有身子 = 没有可等的坐标）。
fn live_rep(table: &Table, name: Name) -> Option<TaskId> {
    match table.find(name) {
        Some(Service {
            slot: Slot::Live { rep, .. },
            ..
        }) => Some(*rep),
        _ => None,
    }
}

/// 盯着它：`true` = 它收尾了（`Now` / `Waited`）。
///
/// 内核的事实优先：它说收了就是收了，表随之落定 `Dead`——**坐标留着**（见 [`Slot`]：
/// 清了就没得放下、也没得重启）。`Unsettled`（有界期内没等出来）**一个字都不写**：
/// 那是"还没收干净"，不是"收了"。
/// `ms` = **上限族**（口径见 `env::fid` 文件头的定式）。
pub fn watch(table: &mut Table, name: Name, ms: usize) -> Result<bool, Fail> {
    match until(table, name, ms)? {
        Reaped::Now | Reaped::Waited => {
            table.set_state(name, State::Dead);
            Ok(true)
        }
        Reaped::Unsettled => Ok(false),
    }
}

/// 按名字找那一行的只读视图（枚举的入口）。
pub fn find(table: &Table, name: Name) -> Option<&Service> {
    table.find(name)
}
