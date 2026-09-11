//! irq 客户端：连上驱动 → 两个动词（登记 / 写属主）。
//!
//! 与 `dispatch`/`console`/`doom` 的客户端同形：入口门闩由目录 `Connect` 转授，
//! 回信孔由**调用方自备**（`unseal` + `Accord` 给驱动，把对端那个 token 随每条请求
//! 交出去）。

use env::{EnvError, EnvResult, Name, Permission, PieToken, TaskId, make_err};

use runtime::env::mail::AnyPie as _;
use runtime::env::mail::{self, HolePie};

use crate::dispatch::client::Directory;

use super::{ACK_LEN, Ack, Request, SERVICE};

/// 等回执的上界（毫秒）。驱动在同一台机器上做一次查表 + 回执（至多再碰一次 PLIC 寄存器），
/// 远超实际耗时；有上界才能把"回执被丢"暴露成失败，而不是永久挂起。
const ACK_TIMEOUT_MS: usize = 1000;

fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(-1))
}

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
    /// 本函数负责把 `session` 的 WRITE 副本授给驱动——门闩是 per-task 的，这是唯一
    /// 能跨任务交接的方式（"客户端递出门闩"，§3.2.4）。
    pub fn register(&self, name: &Name, session: &HolePie) -> EnvResult<Ack> {
        let at_driver = session.accord(self.owner, Permission::WRITE)?;
        self.ask(|ack| Request::register(*name, at_driver, ack))
    }

    /// 写属主：这个名字归 `who`。**只有 root 会调它**（驱动认推者是不是自己的 `sire`）。
    pub fn refer(&self, name: &Name, who: TaskId) -> EnvResult<Ack> {
        self.ask(|ack| Request::refer(*name, who, ack))
    }

    /// 一次往返：备回信孔 → 报文 → 有界等回执 → **封印回信孔**。
    fn ask(&self, build: impl FnOnce(PieToken) -> Request) -> EnvResult<Ack> {
        let ack = HolePie::unseal(ACK_LEN)?;
        let at_driver = ack.accord(self.owner, Permission::WRITE)?;
        let msg = build(at_driver).encode();
        // 推送**会阻塞**（槽满即等，见 `HolePie::push`）：驱动一定会看到这条报文。
        // 但"看到"不是"办到"——回执才是，故下一步是等它。
        HolePie::from_token(self.entry.get()).push(&msg)?;
        let mut status = [0xFFu8; ACK_LEN];
        let got = ack.pull_timeout(&mut status, ACK_TIMEOUT_MS);
        // 回信孔**用完即封印**：一次往返一条回执，此后它没有读者了。不封印的话，驱动
        // 迟到的 `push` 会落在一条没人排空的槽上——驱动的推送是阻塞的，那会把它挂住。
        let _ = ack.seal();
        match got {
            Ok(ACK_LEN) => Ack::from_byte(status[0]).ok_or_else(denied),
            _ => Err(denied()),
        }
    }
}
