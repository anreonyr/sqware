//! irq 客户端：连上驱动 → 两个动词（登记 / 写属主）。
//!
//! 与 `dispatch`/`console`/`doom` 的客户端同形：入口门闩由目录 `Connect` 转授，
//! 回信孔由**一次往返**（[`Port`]）自备，它对驱动侧的那枚句柄随每条请求交出。
//!
//! **本协议每条请求新开一枚回信孔**（[`Line::ask`]）：驱动不需要会话表，它只认识
//! "这条回执寄到哪"。代价是每次调用多两次 envcall（开孔 / 放下），收益是对"迟到
//! 回复污染下一趟"免疫——与 console/dispatch 那种"开一次、长期用"的会话不同。
//!
//! 线号不在报文里：线 = 名字的函数，由驱动自己解出来（`docs/driver.md` §12 甲）。

use env::{EnvResult, Name, PieToken, TaskId};

use runtime::core::port::{Access, Policy, Port, ship};
use runtime::env::mail::{self, HolePie};

use super::wire::denied;
use super::{Ack, Request, SERVICE};
use crate::dispatch::client::Directory;

/// 等回执的上界（毫秒）。驱动在同一台机器上做一次查表 + 回执（至多再碰一次 PLIC 寄存器），
/// 远超实际耗时；有上界才能把"回执被丢"暴露成失败，而不是永久挂起。
const ACK_TIMEOUT_MS: usize = 1000;

/// 驱动的一条会话：入口门闩的副本 + **驱动是谁**。
pub struct Line {
    entry: PieToken,
    owner: TaskId,
}

impl Line {
    /// 从目录里按 [`SERVICE`] 连一次。
    pub fn connect(dir: &Directory) -> EnvResult<Line> {
        Self::at(dir.connect_token(SERVICE)?)
    }

    /// 已经拿到入口副本时用它（root 的路子：`Refer` 换来的目录门闩再 `Connect`）。
    ///
    /// 驱动 id 取 `Reserve(entry).owner`——门闩的**开辟者**（`vestor` 会被转发改写成
    /// root，`owner` 不会；与 `console`/`doom` 认服务同一个办法）。
    pub fn at(entry: PieToken) -> EnvResult<Line> {
        let owner = mail::reserve(entry)?.1;
        if owner.get() == 0 {
            return Err(denied());
        }
        Ok(Line { entry, owner })
    }

    /// 驱动是谁（任务 id）——客户端据此核投递来源（`docs/driver.md` §5.3）。
    pub const fn owner(&self) -> TaskId {
        self.owner
    }

    /// 登记：`session` 是**本任务**的会话门闩（驱动往它投线号）。
    ///
    /// 本函数负责把 `session` 的 `WRITE` 副本授给驱动——门闩是 per-task 的，这是唯一
    /// 能跨任务交接的方式（"客户端递出门闩"，§3.2.4）。
    pub fn register(&self, name: &Name, session: &HolePie) -> EnvResult<Ack> {
        let to = ship(session, self.owner, Access::WRITE, Policy::NONE)?;
        self.ask(Request::register(*name, to.seed()))
    }

    /// 写属主：这个名字归 `who`。**只有 root 会调它**（驱动认推者是不是自己的 `sire`）。
    pub fn refer(&self, name: &Name, who: TaskId) -> EnvResult<Ack> {
        self.ask(Request::refer(*name, who))
    }

    /// 委托写权：这个名字的属主，从此也可以由 `who` 写。**同样只有 root 会调它**——
    /// 它是"root 把自己那份写权借给自己域里的一个线程"，与 [`Line::refer`] 同一条判据入口。
    pub fn delegate(&self, name: &Name, who: TaskId) -> EnvResult<Ack> {
        self.ask(Request::delegate(*name, who))
    }

    /// 一次往返：开回信孔 → 报文 → 有界等回执 → **放下回信孔**。
    ///
    /// 推送**会阻塞**（槽满即等，见 `HolePie::push`）：驱动一定会看到这条报文。
    /// 但"看到"不是"办到"——回执才是。
    ///
    /// 收尾是 `close`（放下），不是旧版的 `seal`：驱动每请求只推一条回执，迟到的
    /// 那条落在空槽里、下一请求已换新孔，故这里不必再借"封印"去断它的路。
    fn ask(&self, request: Request) -> EnvResult<Ack> {
        let entry = HolePie::from_token(self.entry.get());
        let port = Port::open(&entry)?;
        let ack = port.call::<Request>(&request, ACK_TIMEOUT_MS)?;
        port.close()?;
        Ok(ack)
    }
}
