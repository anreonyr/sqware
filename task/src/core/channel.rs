//! Channel — 跨任务 Hole 通道：自己的 hole + 它在对端的 token（Accord 派生）。
//!
//! 用途：服务调用方自带回信通道（directory Connect 后用）、服务间直接通信。
//! 与 [`crate::core::service::Service`] 的 `channel` 字段共用此类型——
//! `Service::call` 把 `at_peer` 写进请求消息，调用方 reply 经 `mine` 到达。
//!
//! 生命周期：`open(peer)` 造 hole + Accord 副本 → 使用（`mine.push` / `mine.pull`）→
//! `close(peer)` revoke 副本 + release 我的 hole。**资源本体不动**——多用户场景
//! 下还有他人持有 pie 副本，需要 `seal` 才会真正销毁 HoleMeta。
//!
//! `from_receipt` 给 Phase B 的 per-caller reply 场景预留：调用方已经 unseal 出
//! 自己的 hole，Accord 给目录得到 `at_peer`，用这个构造器把状态收进 Channel。

use env::{EnvResult, Permission, PieToken, TaskId};

use crate::env::mail::{self, HolePie};

///通信通道：我自己的 hole + 它在对端的 token。
pub struct Channel {
    /// 我的 hole（push/pull 走它）。
    pub(crate) mine: HolePie,
    /// 我 Accord 给 peer 的副本的 token（`close` 时用它 revoke）。
    pub(crate) at_peer: usize,
}

impl Channel {
    /// 建通道：我造一个 hole，把 R|W 副本授给 peer。
    ///
    /// `mtu` 取 `HOLE_MTU_MAX`（4096）——单消息上限：dispatch 等当前载荷远小于此，
    /// 留上限以备未来协议扩展。
    pub fn open(peer: TaskId) -> EnvResult<Channel> {
        let mine = HolePie::unseal(crate::env::mail::HOLE_MTU_MAX)?;
        let at_peer = mine.accord(peer, Permission::READ | Permission::WRITE)?;
        Ok(Channel::from_receipt(mine, at_peer))
    }

    /// 由已有 hole + 对端 token 构造（per-caller reply 场景：调用方已 unseal + accord）。
    pub fn from_receipt(mine: HolePie, at_peer: usize) -> Channel {
        Channel { mine, at_peer }
    }

    /// 关闭：revoke 我授给 peer 的副本 + release 我自己的 hole。
    ///
    /// revoke 失败（peer 已死、token 不在 peer 表里）返 `Denied`——资源本身仍由
    /// `HoleMeta::drop` 在所有副本释放后自动封印，故无需手工 seal。
    pub fn close(self, peer: TaskId) -> EnvResult<()> {
        mail::revoke(peer, PieToken::new(self.at_peer))?;
        self.mine.release()
    }

    /// 取我在对端的 token（写进请求消息前 8 字节）。
    pub fn at_peer(&self) -> usize {
        self.at_peer
    }

    /// 取我自己这端的 hole（直接 push/pull 走它）。
    pub fn mine(&self) -> &HolePie {
        &self.mine
    }
}
