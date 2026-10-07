//! 报名 / 列册 / 认领。
//! `claim` 打的是 `/dev/<类>/<名>` 那一格上挂的**那一台那一份**。叫错了门由对端答 Fail::Bad
//! （形状不认）——故 Face 只保证"把这一帧送到你手里那一枚"。
//! 报文里那一格是"我给你的那一枚在你表里是几号"，对端据此一次 `Reserve` 就认得出回信的路，
//! 答完当场放下。
//! **传输失败折进 Fail::Bad**：借不出回信孔 / 推不出去 / 超时 / 答话形状不对——四件事对

use alloc::string::String;
use env::{Access, Policy, HoleDir, PieKind, PieToken, TaskId, Wait};
use ::resource::port;

use crate::communication::hand::{Receiver, RecvFail};
use crate::communication::session::establish;
use crate::wire::message::Message;

use super::Fail;
use super::frame::{self, BACK_MARK};
use env::pie;
use ::resource::raw::{Hole};

/// 一面 hub 的门牌：**树上查回来的那一枚** ＋ 它的开者（hub）
pub struct Face {
    entry: PieToken,
    host: TaskId,
}

impl Face {
    /// 把一枚门牌收成一面
    /// 对端从**这一枚门闩自己**问出来（establish::opened_by）——门牌是持表那一侧挂的
    /// 不是本端开的
    pub fn of(entry: PieToken) -> Result<Self, Fail> {
        let host = establish::opened_by(entry).ok_or(Fail::Bad)?;
        Ok(Face { entry, host })
    }

    /// 对端是谁（读数用）
    pub fn host(&self) -> TaskId {
        self.host
    }

    /// **报名**：许我驱这一类。**幂等**（已在那一类里答成）
    pub fn bond(&self, class: String, wait: Wait) -> Result<(), Fail> {
        let said = self.call::<_, frame::Said>(|back| frame::Bond::of(class, back), wait)?;
        read(said.status)
    }

    /// **列册**：从 `from` 起取一窗（越界答空窗——是答案，不是错误）
    pub fn list(&self, class: String, from: u32, wait: Wait) -> Result<frame::Window, Fail> {
        let window =
            self.call::<_, frame::Window>(|back| frame::ListReq::of(class, from, back), wait)?;
        read(window.status)?;
        Ok(window)
    }

    /// **认领**：这一台归我 ⇒ 答一张契
    /// **先交一份过去**、此后一直开着——hub 扫账时问它"主人还在不在"（内核那一问 `Join`
    /// 只许同队或父域，hub 与驱动是兄弟，问不动）
    pub fn claim(
        &self,
        kind: PieKind,
        access: Access,
        policy: Policy,
        sensor: PieToken,
        wait: Wait,
    ) -> Result<frame::Deed, Fail> {
        // 交出去的只是**读的那一份**（`FETCH|STORE`、不给 `VEST`）：hub 只用它 `reserve`
        // 一次（问"这一枚还在不在"），不需要再授给谁。
        let shipped = port::ship(
            sensor,
            self.host,
            Access::FETCH | Access::STORE,
            Policy::NONE,
        )
        .map(|to| to.seed())
        .map_err(|_| Fail::Dead)?;
        let kind = kind as u8;
        let access = access.bits().bits();
        let policy = policy.bits().bits();
        let deed = self.call::<_, frame::Deed>(
            |back| frame::Claim::of(kind, access, policy, shipped, back),
            wait,
        )?;
        read(deed.status)?;
        Ok(deed)
    }

    /// 问一句、取一句答（三手共用）
    /// **先铸、先交，再推**（次序是契约的一半）：那一枚"种在对端表里的号"随帧一起过去
    /// **一枚构造函数**，不是一枚编好的帧
    /// **两个型参各管一头**：`S` 是问那一形（只用来编），`R` 是答那一形（收那一侧要它
    /// 故答话的解码只有一条路）。答那一形只能由调用点写出来——问的那一枚闭包推不出它
    /// **传输失败两格分得开**
    /// ```text
    /// 孔用不动了（Dead / Denied：权限不够 / 资源封印 / 那一枚已交出去）⇒ Fail::Dead
    /// 没消息 / 备不下 / 收下来解不动 ⇒ Fail::Bad
    /// ```
    fn call<S: Message, R: Message>(
        &self,
        with: impl FnOnce(PieToken) -> S,
        wait: Wait,
    ) -> Result<R::In, Fail> {
        fn report(step: &str) {
            crate::debug!("hub: call deny={step}");
        }
        let (back, seed) = match establish::lend_out(self.entry, BACK_MARK) {
            Ok(pair) => pair,
            Err(()) => {
                report("borrow");
                return Err(Fail::Dead);
            }
        };
        let mut ask = S::EMPTY;
        let Some(n) = with(seed).store(ask.as_mut()) else {
            // "这只手被取走"（Sender::Drop），而它等的这一枚只有我手里这一份。
            let _ = pie::seal(back);
            let _ = pie::release(back);
            report("encode");
            return Err(Fail::Bad);
        };
        let door = Hole::from_raw(self.entry);
        if door.push(&ask.as_ref()[..n], Wait::Forever).is_err() {
            // "这只手被取走"（Sender::Drop），而它等的这一枚只有我手里这一份。
            let _ = pie::seal(back);
            let _ = pie::release(back);
            report("push");
            return Err(Fail::Dead);
        }
        let mut said = R::EMPTY;
        let got = match Receiver::<R>::from_raw(back).recv(said.as_mut(), wait) {
            Ok(one) => Ok(one),
            Err(RecvFail::Mail(e)) if !matches!(e, env::MailFail::Dead | env::MailFail::Denied) => {
                report("recv");
                Err(Fail::Bad)
            }
            Err(RecvFail::Mail(_)) => {
                report("recv-dead");
                Err(Fail::Dead)
            }
            Err(RecvFail::Unread(len)) => {
                crate::debug!("hub: call deny=recv-unread len={len}");
                Err(Fail::Bad)
            }
        };
        let _ = door.wait(HoleDir::Push, Wait::Forever);
        // "这只手被取走"（Sender::Drop），而它等的这一枚只有我手里这一份。
        let _ = pie::seal(back);
        let _ = pie::release(back);
        got
    }
}

/// 一句答拆开：**状态先过码表**
/// `OK` ⇒ 成；对端答的那几格 ⇒ 它们自己那个失败；**表外那一格**（连 `OK` 都没读成）⇒
/// 与"没走到"同落 Fail::Bad（同 control 的 `read`）
fn read(status: u8) -> Result<(), Fail> {
    match frame::code_to_fail(status) {
        None if status == frame::OK => Ok(()),
        Some(fail) => Err(fail),
        None => Err(Fail::Bad),
    }
}
