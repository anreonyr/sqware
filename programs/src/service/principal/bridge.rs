//! 名册自己放行之后装配者才认下面，并补绑它自己与树（它们起来时名册还没在）。

use core::time::Duration;

use env::{TaskId, Wait};
use protocol::communication::session::establish;
use protocol::debug;
use protocol::service::principal as pcall;
use protocol::service::principal::client::Face;
use runtime::env::room;

use crate::system::Assembly;
use crate::system::control::{BOOT_MS, RETRY_MS, Service};
use crate::unit::UnitFile;

#[derive(Default)]
pub struct Roster {
    face: Option<Face>,
}

impl Roster {
    /// **放行前**给这一条服务派一条号、绑到它那一枚线程上
    /// `None`）。名册还没在（名册自己与树）⇒ 什么都不做——那两条由 Roster::adopt 补绑
    pub fn bind(&self, task: TaskId) -> Result<(), &'static str> {
        let Some(face) = self.face.as_ref() else {
            debug!("principal: bind skip(no face) task={}", task.get());
            return Ok(());
        };
        let root = face.new_principal();
        let mine = root.derive(Wait::AtMost(BOOT_MS)).map_err(|fail| {
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

pub fn bind(
    assembly: &mut Assembly,
    program: &UnitFile,
    service: &mut Service,
) -> Result<(), &'static str> {
    assembly.roster.bind(service.0)
}

/// **名册这一位要认下面 ＋ 补绑自己与树** —— 判据是**它自己交上来的那一枚门牌**
/// （Grant::Set：定面那一枚；只有名册那一族交得出它）
pub fn adopt_roster(
    assembly: &mut Assembly,
    _program: &UnitFile,
    service: &mut Service,
) -> Result<(), &'static str> {
    if establish::find(service.0, pcall::Grant::Set.mark()).is_none() {
        return Ok(());
    }
    assembly.roster.adopt(service.0, assembly.tree.host())?;
    assembly.tree.wire()?;
    Ok(())
}

/// 认下名册**交给生我者**的那一枚门牌（装配者自己的那一份）
/// `(开者 = 它, 记号 = 面)` 两格认出来。它起手就交
/// **要的是 Grant::Set（定面）**：本间那两手是 `derive` ＋ `bind`——发身份
/// 那一侧要的正是改的权柄，而问面给不了它。名册两面各交一枚、记号不同，故"要哪一面"得说清
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
