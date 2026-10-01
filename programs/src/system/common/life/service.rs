//! 建域 / 放行 / 等就绪 / 收 / 盯。
//! ready / stop / until / watch。**纯转发那一层没有了**——`build` / `spawn` /

use env::{Mark, Permission, PieFail, PieToken, ProgramKind, TaskId, UnitFail, Wait};
use runtime::env::mail;
use runtime::env::room;
use runtime::env::unit as utask;

use crate::system::common::life::table::{Announce, Service, Slot, State, Table};
use crate::system::common::life::verdict::{Fail, Ready, Reaped, admit_start, probe_ready};
use protocol::communication::establish::Endpoint;

use crate::unit::{
    Died, coalition::E_COALITION, hub::E_HUB, operator::E_TREE, principal::E_PRINCIPAL,
};

/// **穷尽 match 在域词表上**（env::UnitFail）：装不上 = UnitFail::BadImage；
/// 内存不够 / 产不出来 = UnitFail::OoM（本协议名 Fail::Full）；其余
/// （`Denied` 不在我 heir 里 / 启动参数读不出来；`Busy` 条件未就绪）落 Fail::Unknown
/// ——"不认识的失败"不该猜成某一种。**没有表外那一格**：域词表是穷尽的。
fn unit_fail(e: erra::Error<UnitFail>) -> Fail {
    match e.source {
        UnitFail::BadImage => Fail::BadImage,
        UnitFail::OoM => Fail::Full,
        UnitFail::Denied | UnitFail::Busy => Fail::Unknown,
    }
}

fn pie_fail(e: erra::Error<PieFail>) -> Fail {
    match e.source {
        PieFail::OoM => Fail::Full,
        PieFail::Denied | PieFail::Dead | PieFail::HandedOver | PieFail::NotAligned => {
            Fail::Unknown
        }
    }
}

/// **四枚服务起手失败**（持树者 / 名册 / 盟册 / 设备账各一个 bin，共用这一枚词表）。
/// **名字为什么不叫 `Fail`**：`operator/server.rs` 已经 `use protocol::service::operator::{…,
/// Fail}`（那是**核心**的失败域），两个 `Fail` 在同一份文件里撞名。起手这几格与核心那几格
/// 不是一回事，故按"死在起手的哪一步"取名 Start。
/// **它自己就是出口**（`impl Exit`）：三个 bin 的 `main` 直接答 `Result<(), Start>`——
/// 不需要再有一层 `said` / `exit` 的转发。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    Tree(Died),
    /// 起手要备的那两样备不下：持树者**收帧那一页**（`Pile` 之外的那一样）。
    Room(Died),
    /// 常驻那一问：那只组没立起来，或收帧那一页备不下。
    Desk(Died),
    /// 身份服务的两张表（谱系 + 名册）没立起来；**只有名册那一台有**。
    Book(Died),
    /// 身份服务那份门牌找不到（盟册是它的客人，**按名字找**）；**只有盟册有**。
    Face(Died),
    /// **起手那一段物料没收到**（整机物料那条通道上没来东西 / 来的东西解不动）；
    /// **只有设备账那一台有**（Setup::Machine 那一格）。
    Load(Died),
    /// **常驻期**：组坏了（`await_` 答不出）——与 Start::Desk 分开，是因为它不在"起手那
    /// 几步"里（起手已经过完了），而号同那一族。
    Dead(Died),
}

impl Start {
    pub fn code(self) -> env::Reason {
        match self {
            Start::Tree(d)
            | Start::Room(d)
            | Start::Desk(d)
            | Start::Book(d)
            | Start::Face(d)
            | Start::Load(d)
            | Start::Dead(d) => d,
        }
    }

    /// **这一句话怎么念**：族名 ＋ 步名。
    pub fn text(self) -> &'static str {
        match self {
            // 持树者（`E_TREE`）：它会走到的那三步。
            Start::Tree(E_TREE) => "operator: tree",
            Start::Room(E_TREE) => "operator: no room",
            Start::Desk(E_TREE) => "operator: desk",
            // 名册（`E_PRINCIPAL`）：树 / 两张表，加上 `carrier` 那三格。
            Start::Tree(E_PRINCIPAL) => "principal: tree",
            Start::Book(E_PRINCIPAL) => "principal: no book",
            Start::Room(E_PRINCIPAL) => "principal: no room",
            Start::Desk(E_PRINCIPAL) => "principal: desk",
            Start::Dead(E_PRINCIPAL) => "inner: group dead",
            // 盟册（`E_COALITION`）：树 / 门牌，加上 `carrier` 那三格。
            Start::Tree(E_COALITION) => "coalition: tree",
            Start::Face(E_COALITION) => "coalition: no identity plate",
            Start::Room(E_COALITION) => "coalition: no room",
            Start::Desk(E_COALITION) => "coalition: desk",
            Start::Dead(E_COALITION) => "inner: group dead",
            // 设备账（`E_HUB`）：树 / 物料 / 盟册那面 / 自带的常驻圈（它不用 `carrier`：
            // 两个来路——那只组 ＋ 探活那一拍，见 `system/hub/serve/mod.rs`）。
            Start::Tree(E_HUB) => "hub: tree",
            Start::Load(E_HUB) => "hub: no machine",
            Start::Face(E_HUB) => "hub: no league plate",
            Start::Room(E_HUB) => "hub: no room",
            Start::Desk(E_HUB) => "hub: desk",
            Start::Dead(E_HUB) => "inner: group dead",
            // **构造上到不了**（上面三组把三个族的全部产点摆齐了）；真到这一步就照实说"不知道"，
            // 不顺手套一个好听的名字。
            _ => "start: ?",
        }
    }
}

impl crate::Exit for Start {
    fn report(&self) -> crate::Report<'_> {
        crate::Report::note(self.code(), self.text())
    }
}

/// 起跑前要交出去的一枚门闩：给哪一枚、多大权。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Grant {
    /// 要交出去的那一枚（**在我表里**的句柄）。
    pub token: PieToken,
    /// 交出去的权限子集。
    pub perm: Permission,
}

/// 起一个 Service：**建域 → 产线程 → 挂身子**。
/// `image` = 镜像字节（v1 口径）；`kind` = 特权级（**由清单决定**，调用方转交）。
pub fn mint(
    table: &mut Table,
    name: &str,
    image: &[u8],
    kind: ProgramKind,
) -> Result<TaskId, Fail> {
    admit_start(table, name)?;

    let team = utask::build(image, kind).map_err(unit_fail)?;
    let Ok(task) = utask::spawn(team, 0, &[], 0) else {
        return Err(Fail::Full);
    };
    table.attach(name, Some(team), task)?;
    table.set_state(name, State::Starting);
    protocol::debug::put(&alloc::format!("system: minted {name} tid={}", task.get()));
    Ok(task)
}

/// 第二相：**放行**，并在放行前塞门闩、备通道。
/// `grants` = 放行前要交到它手里的门闩（空 = 什么都不预先给）；`channels` = 与它的那几条通道
/// **记号**（记号即通道名）；`millis` = 就绪等待（上限族，`Wait`）。
/// **两相之间的窗口就是"它一步都还没跑"**——塞门闩、备通道都发生在这段窗口里。
/// **失败时不留下半行**：本相失败 ⇒ 实例与状态如实留在表里（它确实在跑），调用方用
/// stop 收尾。
pub fn start(
    table: &mut Table,
    name: &str,
    task: TaskId,
    grants: &[Grant],
    channels: &mut [Endpoint],
    marks: &[Mark],
    millis: Wait,
) -> Result<(), Fail> {
    let launched = (|| -> Result<(), Fail> {
        for g in grants {
            // 记号照源枚（Mark::NONE）：授下去的这几柄带的是它们原本那条路的名字。
            mail::accord(g.token, task, g.perm, Mark::NONE).map_err(pie_fail)?;
        }
        utask::hatch(task).map_err(unit_fail)
    })();
    if let Err(e) = launched {
        let _ = room::doom(task);
        table.detach(name);
        table.set_state(name, State::Dead);
        return Err(e);
    }
    ready(table, name, channels, marks, millis)?;
    Ok(())
}

/// 等它就绪。`true` = **调用开始时就已就绪**（挂起过的一律 `false`）。
/// 唯一会改表的地方就是这个函数：确认它的宣布之后置 State::Ready，确认它死了之后
/// **归建立那一手**：通道在对方交回孔并归位（`claim`）时成立，本函数只去问那句"成立了没有"。
pub fn ready(
    table: &mut Table,
    name: &str,
    channels: &mut [Endpoint],
    marks: &[Mark],
    millis: Wait,
) -> Result<bool, Fail> {
    // 先看表：上一次问过的事实（按这一行自己声明的说法解读）。
    if let Ready::Up = probe_ready(table, name) {
        return Ok(true);
    }
    let Some(Service {
        slot: Slot::Live { task, .. },
        announce,
        ..
    }) = table.find(name)
    else {
        return Err(Fail::NotReady);
    };
    let (task, announce) = (*task, *announce);

    // 它不宣布的那一种：放行之后只要还活着就算起来了，没有可等的东西。
    if announce == Announce::None {
        if !utask::join(task, Wait::POLL).unwrap_or(true) {
            table.set_state(name, State::Ready);
            return Ok(false);
        }
        table.detach(name);
        table.set_state(name, State::Dead);
        return Err(Fail::NotReady);
    }

    // 它会交回一枚孔 ⇒ 那件事归建立那一手：`claim` 把它认下来（逐条凑齐了才算起来）。
    // **两条数要一样**：`marks` 是装配表上那几条通道，`channels` 是放行前逐条装上的
    // （`connect` 一次一件）——对不上就是装配错，与"没认齐"同一落点：**不算起来**。
    if !marks.is_empty()
        && channels.len() == marks.len()
        && channels
            .iter_mut()
            .zip(marks)
            .all(|(channel, mark)| channel.claim(task, *mark, millis))
    {
        table.set_state(name, State::Ready);
        return Ok(false);
    }
    if utask::join(task, Wait::POLL).unwrap_or(true) {
        table.detach(name);
        table.set_state(name, State::Dead);
        return Err(Fail::NotReady);
    }
    // 还活着、只是没宣布：它确实在跑，**实例如实留在表里**（调用方用 `stop` 收尾）。
    if millis == Wait::POLL {
        return Ok(false);
    }
    Err(Fail::NotReady)
}

pub fn stop(table: &mut Table, name: &str) -> Result<(), Fail> {
    let Some(Service {
        slot: Slot::Live { task, .. },
        ..
    }) = table.find(name)
    else {
        return Err(Fail::Unknown);
    };
    let task = *task;
    let _ = room::doom(task);
    table.set_state(name, State::Stopping);
    Ok(())
}

/// 等它收尾：`millis` 与 ready 同款（上限族，`Wait`）。
/// **只读：不动表**。
/// 形状是 **问 → 等 → 问**，判决只认两次**非阻塞问**（`Join{task, 0}`）；等只是为了少问几次。
/// `Err(Fail::Unknown)` = 表里没这一行、或这一行还没有身子的坐标。问不出（`Denied` =
/// 已入土 / 从未入册）按"收尾了"处理。
pub fn until(table: &Table, name: &str, millis: Wait) -> Result<Reaped, Fail> {
    let Some(task) = live_task(table, name) else {
        return Err(Fail::Unknown);
    };
    if utask::join(task, Wait::POLL).unwrap_or(true) {
        return Ok(Reaped::Now);
    }
    if millis == Wait::POLL {
        return Ok(Reaped::Unsettled);
    }
    // 挂起等一记：醒来自收尾（`wipe`）或到点，两者当场分不开 ⇒ 醒来复探，判决只认它。
    let _ = utask::join(task, millis);
    if utask::join(task, Wait::POLL).unwrap_or(true) {
        Ok(Reaped::Unsettled)
    } else {
        Ok(Reaped::Waited)
    }
}

/// 这一行身子那一枚线程（没有身子 = 没有可等的坐标）。
fn live_task(table: &Table, name: &str) -> Option<TaskId> {
    match table.find(name) {
        Some(Service {
            slot: Slot::Live { task, .. },
            ..
        }) => Some(*task),
        _ => None,
    }
}

/// 盯着它：`true` = 它收尾了（`Now` / `Waited`）。
/// 内核的事实优先：它说收了就是收了，表随之落定 `Dead`——**坐标留着**（清了就没得放下、
/// 也没得重启）。`Unsettled`（有界期内没等出来）**一个字都不写**：那是"还没收干净"，
/// 不是"收了"。
pub fn watch(table: &mut Table, name: &str, millis: Wait) -> Result<bool, Fail> {
    match until(table, name, millis)? {
        Reaped::Now | Reaped::Waited => {
            table.set_state(name, State::Dead);
            Ok(true)
        }
        Reaped::Unsettled => Ok(false),
    }
}
