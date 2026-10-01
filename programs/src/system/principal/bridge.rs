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
use protocol::system::principal as pcall;
use protocol::system::principal::client::Face;
use runtime::env::room;

use crate::program::Program;
use crate::system::Assembly;
use crate::system::control::{READY_MS, RETRY_MS, Service};

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
    pub fn bind(&self, task: TaskId, on: bool) -> Result<(), &'static str> {
        if !on {
            return Ok(());
        }
        let Some(face) = self.face.as_ref() else {
            return Ok(());
        };
        let root = face.new_principal();
        let mine = root.derive(Wait::AtMost(READY_MS)).map_err(|_| "derive")?;
        face.task(task)
            .bind(mine.id(), Wait::AtMost(READY_MS))
            .map_err(|_| "bind")?;
        Ok(())
    }

    /// **名册自己放行之后**：认下它交给生我者的那一面，补绑它自己与树，返它的号。
    ///
    /// 走到这里时树**必已就位**（持树者排第一）；`tree = None` 只报一句读数（那是唯一的响声
    /// ——原来它静默跳过）。
    pub fn adopt(&mut self, task: TaskId, tree: Option<TaskId>) -> Result<TaskId, &'static str> {
        let f = face_of(task).ok_or("no identity face")?;
        let root = f.new_principal();
        let mine = root.derive(Wait::AtMost(READY_MS)).map_err(|fail| {
            debug!("principal: adopt derive self {:?}", fail);
            "derive self"
        })?;
        f.task(task)
            .bind(mine.id(), Wait::AtMost(READY_MS))
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
                    .derive(Wait::AtMost(READY_MS))
                    .map_err(|_| "derive tree")?;
                f.task(t)
                    .bind(pt.id(), Wait::AtMost(READY_MS))
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
    program: &Program,
    service: &mut Service,
) -> Result<(), &'static str> {
    assembly.roster.bind(service.0, program.relation.bind)
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
    let mut left = READY_MS;
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
