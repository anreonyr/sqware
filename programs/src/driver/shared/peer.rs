//! 一台对端的**类型化客户面**：门牌 ＋ 开者 ＋ 一问一答。
//!
//! 各驱动与 Hub 都用它：开者由内核给出（`opened_by`），一问的荷载与一答的形状由
//! 契约给，回信孔那一枚由 `ipc::rpc` 铸、交、核（发送者、记号、一次性）。

use env::{PieToken, TaskId, Wait};
use ipc::rpc::request::Sender;
use ipc::session::establish;
use ipc::time::Deadline;
use system_api::operator::Path;
use system_client::operator::Face;
use wire::{Contract, Message};

/// 一枚门牌 → 一面客户。
pub struct Peer<C: Contract> {
    sender: Sender<C>,
    host: TaskId,
}

impl<C: Contract> Peer<C> {
    /// 收一枚门牌；开者就是那一台（内核给出，不看报文自报）。
    ///
    /// 回信孔那一枚的记号由调用方给（`Call::BACK`）：契约类型本身不声明它
    /// （未带 mark/back 的契约也能共用这一面）。
    pub fn open(entry: PieToken, back: env::Mark) -> Result<Self, &'static str> {
        let host = establish::opened_by(entry).ok_or("peer: host")?;
        let sender = Sender::<C>::from_raw(entry, back).map_err(|_| "peer: face")?;
        Ok(Self { sender, host })
    }

    /// 树上沿一条路取那一面。
    pub fn find(
        tree: &Face,
        road: &Path,
        back: env::Mark,
        wait: Wait,
    ) -> Result<Self, &'static str> {
        let service = tree
            .resolve(system_api::namespace::SERVICE, wait)
            .map_err(|_| "peer: service namespace")?;
        let token = tree
            .tile_from(service, road, wait)
            .and_then(|tile| tile.token(wait))
            .map_err(|_| "peer: entry")?;
        Self::open(token, back)
    }

    /// 对端是那一台（读数用）。
    pub fn host(&self) -> TaskId {
        self.host
    }

    /// 问一句、取一句答；"没走到"与"对面拒了"都折进同一个失败格
    /// （调用方按自己的下一步决定怎么报）。
    pub fn call(
        &self,
        wait: Wait,
        build: impl FnOnce(PieToken) -> C::Request,
    ) -> Result<<C::Response as Message>::In, &'static str> {
        self.sender
            .call(Deadline::new(wait), build)
            .map_err(|_| "peer: call")
    }
}
