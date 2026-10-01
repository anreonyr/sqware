//! principal::bridge — **装配侧**：名册那一面（身份面）与"认下它交给生我者的门牌"。
//!
//! 装配期每一条服务的身份都从这条路上来：**放行前** `derive(ROOT)` + `bind`（负证客人除外）；
//! **名册自己放行之后**装配者才认下面，并补绑它自己与树（它们起来时名册还没在）。
//!
//! 这两手原先散在装配那一趟里（`face_of` 也住那一处）。它们问的是**名册的语义**
//! ——门牌怎么认、谁补绑——故收进名册这一间；装配者那一侧只留一个 [`Roster`] 手柄。

use core::time::Duration;

use env::{TaskId, Wait};
use protocol::communication::establish;
use protocol::debug;
use protocol::service::principal as pcall;
use protocol::service::principal::client::Face;
use runtime::env::room;

use crate::unit::UnitFile;
use crate::system::Assembly;
use crate::system::control::{BOOT_MS, READY_MS, RETRY_MS, Service};

/// **名册在装配者这一侧的状态**：那一面（`None` = 名册还没起来）。
#[derive(Default)]
pub struct Roster {
    face: Option<Face>,
}

impl Roster {
    /// **放行前**给这一条服务派一条号、绑到它那一枚线程上。
    ///
    /// `on` = 装配表上 `bind` 那一格（`false` 是负证客人：不绑，它自己 `resolve(self)` 答
    /// `None`）。名册还没在（名册自己与树）⇒ 什么都不做——那两条由 [`Roster::adopt`] 补绑。
    pub fn bind(&self, task: TaskId) -> Result<(), &'static str> {
        // **照实记（第 71 刀：`Relation::bind` 那一格退场 ⇒ 这里"照旧全绑"，`on` 那个形参没了）**：
        // 身份从那一天起由**持有者自己在运行期丢掉**（[`Principal::drop`]），声明不再说"给不给"。
        let Some(face) = self.face.as_ref() else {
            // **照实记（第 68 轮量出来的那一笔：这两条跳过路各自会跳过谁）**
            //
            // 两条读数都留在这里（`debug!` 设门 ⇒ 不打搅 release 的串口），实测（debug 档、16 条
            // 读数齐的那一跑）**一共只跳 4 台**：
            //
            // | 那一台 | 走哪条路 | 为什么 |
            // |---|---|---|
            // | `operator` / `passer` / `principal`（task 12/13/14） | 这一支（**名册那面还没到**） | 最早那三台——`principal` 正是"自己没法在放行前绑自己"；那两位由 [`Roster::adopt`] 补 |
            // | `probe-denied`（task 19） | 上面 `!on` 那一支 | **声明说 `false`**（`Relation::bind` 那一格） |
            //
            // ⇒ **其余 19 台都真的被绑了**，故"`bind: true` 不兑现"**不是普遍的**，而是**恰好那三台**
            // （时间上的先后：名册的面立在它们之后）。**这一笔把第 67 轮那个矛盾摆到台面上**：
            // 那一刀**去掉** `probe-denied` 的 `bind: false` 之后，它**按这张表本该被绑**，
            // 可它自己的读数却是 `probe-denied: no identity to drop`（`mine.principal()` 没给出
            // 那一格）⇒ **下一刀先把那一手答的到底是 `None` 还是 `Err(哪一支)` 印出来**，再谈撤。
            debug!("principal: bind skip(no face) task={}", task.get());
            return Ok(());
        };
        let root = face.new_principal();
        // **照实记（第 34 轮抓到的那一支红的落点就在这两手）**：debug 档偶发一条红，实测
        // `probe-rule-other`（`reason=0x16`）折在 `bind` 这一步，而 `system: minted probe-rule-other`
        // **在** ⇒ 它铸出来了、折在**放行前问名册**这一趟。额度从前是 `READY_MS`（1 s）——
        // 与上一轮治好的那一族**同一个形状**（debug 档慢，1 s 装不下），故按同一条口径放大：
        // **死已由表侧那一扫独自认**（`supervise` 的 `Watch`）⇒ 这几问的额度宽了只会慢，不会误判。
        let mine = root
            .derive(Wait::AtMost(BOOT_MS))
            .map_err(|fail| {
                debug!("principal: bind derive {:?} task={}", fail, task.get());
                "derive"
            })?;
        face.task(task)
            .bind(mine.id(), Wait::AtMost(BOOT_MS))
            .map_err(|fail| {
                debug!("principal: bind failed {:?} task={}", fail, task.get());
                "bind"
            })?;
        Ok(())
    }

    /// **名册自己放行之后**：认下它交给生我者的那一面，补绑它自己与树，返它的号。
    ///
    /// 走到这里时树**必已就位**（持树者排第一）；`tree = None` 只报一句读数（那是唯一的响声
    /// ——原来它静默跳过）。
    pub fn adopt(&mut self, task: TaskId, tree: Option<TaskId>) -> Result<TaskId, &'static str> {
        let f = face_of(task).ok_or("no identity face")?;
        let root = f.new_principal();
        let mine = root.derive(Wait::AtMost(BOOT_MS)).map_err(|fail| {
            debug!("principal: adopt derive self {:?}", fail);
            "derive self"
        })?;
        f.task(task)
            .bind(mine.id(), Wait::AtMost(BOOT_MS))
            .map_err(|fail| {
                debug!(
                    "principal: adopt bind self {:?} at={}",
                    fail,
                    mine.id().get()
                );
                "bind self"
            })?;
        match tree {
            Some(t) => {
                let pt = root
                    .derive(Wait::AtMost(BOOT_MS))
                    .map_err(|_| "derive tree")?;
                f.task(t)
                    .bind(pt.id(), Wait::AtMost(BOOT_MS))
                    .map_err(|_| "bind tree")?;
            }
            None => debug!("principal: no tree to bind"),
        }
        self.face = Some(f);
        Ok(task)
    }
}

/// **身份轴在装配那一趟里的那一手**：读这一台声明上 `bind` 那一格。
///
/// **放行之前**就做完——故服务一起来 `resolve(self)` 就答得出。（名册本身与树不走这里：它们起来
/// 时名册还没在；那两条由 [`Roster::adopt`] 在它放行之后补绑。）
pub fn bind(
    assembly: &mut Assembly,
    program: &UnitFile,
    service: &mut Service,
) -> Result<(), &'static str> {
    assembly.roster.bind(service.0)
}

/// **名册这一位要认下面 ＋ 补绑自己与树** —— 判据是**它自己交上来的那一枚门牌**
/// （[`Grant::Set`](protocol::service::principal::Grant)：定面那一枚；只有名册那一族交得出它）。
///
/// **照实记（这一手从前读声明上 `eyes: Some(Eyes::Roster)` 那一格）**：那一格退场了——"谁是
/// 名册"不再由**谁**说，而是名册**自己交上来的东西**（与 `holds_tree` 那一刀同一条纪律：
/// 运行期的事实由运行时的那一枚孔认）。于是这一手不看声明（形参 `_program`），判据从
/// `establish::find` 现问；判不出来 ⇒ 这一台不是名册，**什么都不做**。
///
/// **次序**：它在 [`Assembly::assemble`](crate::system::Assembly::assemble) 的 `AFTER_READY` 那一相
/// ——那时名册已经起完（它起手就把那一枚交出来了），故 `POLL` 就够。**补绑自己与树那一手**
/// 仍是 [`Roster::adopt`]（它同时把名册那一面认下来，此后本域问身份才有一枚门牌在手）。
pub fn adopt_roster(
    assembly: &mut Assembly,
    _program: &UnitFile,
    service: &mut Service,
) -> Result<(), &'static str> {
    if establish::find(service.0, pcall::Grant::Set.mark()).is_none() {
        return Ok(());
    }
    assembly.roster.adopt(service.0, assembly.tree.host())?;
    // **接线那一句推给持树者**（照实记：这一刻之前那道门必须**不接线**——名册自己还没被补绑，
    // 而它起手那一趟要上树；门早接线就会把它自己拒掉，实测 `principal: start failed`）。
    assembly.tree.wire()?;
    Ok(())
}

/// 认下名册**交给生我者**的那一枚门牌（装配者自己的那一份）。
///
/// 装配期**不必上树查自己起的那一枚**：名册起手就把门牌那一枚 `ship` 进本域表里，本域按
/// `(开者 = 它, 记号 = 面)` 两格认出来（[`establish::find`] 的两格正判据）。它起手就交，
/// 故这里是**短等**：还没到就隔一拍再问，问到期限为止。
///
/// **要的是 [`Grant::Set`]（定面）**（开面那一刀）：本间那两手是 `derive` ＋ `bind`——发身份
/// 那一侧要的正是改的权柄，而问面给不了它。名册两面各交一枚、记号不同，故"要哪一面"得说清。
fn face_of(host: TaskId) -> Option<Face> {
    let mut left = BOOT_MS;
    loop {
        if let Some(entry) = establish::find(host, pcall::Grant::Set.mark()) {
            return Face::of(entry).ok();
        }
        if left == 0 {
            return None;
        }
        let _ = room::sleep(Duration::from_millis(RETRY_MS as u64));
        left = left.saturating_sub(RETRY_MS);
    }
}
