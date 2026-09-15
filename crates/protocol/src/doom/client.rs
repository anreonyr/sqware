//! doom 客户端：打开会话 → `kill(名字)`。
//!
//! 与 `dispatch`/`console` 的客户端同形：入口门闩由目录 `Connect` 转授，回信孔由
//! **一次往返**（[`Port`]）自备，它对服务侧的那枚句柄随每条请求交出。

use env::{EnvResult, Name};

use runtime::core::port::Port;
use runtime::env::mail::HolePie;

use super::wire::{ACK_LEN, denied};
use super::{Ack, Query};
use crate::session::push_within;

/// 一次往返里**推**与**等回执**各一份的上界（毫秒）。服务在同一台机器上做一次 envcall +
/// 有界 `Join`，远超实际耗时；有上界才能把"服务卡住 / 回执被丢"暴露成 `Busy`，而不是
/// 永久挂起——**推也要有上界**：`Port::push` 只有"睡到有位置"一种策略，服务不排空就把
/// 调用方钉死，而这条链上钉死的那个正是**来杀它的**。
const ACK_TIMEOUT_MS: usize = 2000;

/// 他杀服务的一条会话。
///
/// `entry` 是入口门闩的**一份副本**：往返归 [`Port`]，有界推要自己拿一枚（[`Port`] 里
/// 那份字段私有）。它与 `Port::open` 收下的是同一张表里的同一枚孔。
pub struct Doom {
    entry: HolePie,
    port: Port,
}

impl Doom {
    /// 打开：`entry` = 目录 `Connect("doom")` 给的那枚入口门闩副本。
    ///
    /// 服务 id 取 `Reserve(entry).owner`——门闩的**开辟者**（`vestor` 会被转发改写成
    /// root，`owner` 不会；与 `console` 认 PLIC 驱动同一个办法）。
    pub fn open(entry: HolePie) -> EnvResult<Doom> {
        let port = Port::open(&entry)?;
        Ok(Doom {
            entry: HolePie::from_token(entry.token()),
            port,
        })
    }

    /// 本协议的一次往返：编帧（回信地址按**本协议**那一格填）→ **有界**推 → 收
    /// （[`Port::pull`] 内建核对来源）→ 解回执。
    ///
    /// 推不动（`Busy`）或到点没回（`Busy`）后本会话不可复用——迟到的回执会污染下一次。
    fn ask(&self, req: &Query) -> EnvResult<Ack> {
        let (frame, n) = req.encode(self.port.seed());
        let frame = frame.get(..n).ok_or_else(denied)?;
        push_within(&self.entry, frame, ACK_TIMEOUT_MS)?;
        let mut out = [0u8; ACK_LEN];
        Ack::decode(self.port.pull(&mut out, ACK_TIMEOUT_MS)?)
    }

    /// 杀一个域（按名字）。**名字 → 目标**的解析在服务侧做（名字的账在目录里）。
    ///
    /// 回执四值见 [`Ack`]；`Ok` 的含义是"内核确认它回收完了"，不是一个"收到了"。
    pub fn kill(&self, target: &Name) -> EnvResult<Ack> {
        self.ask(&Query::new(*target))
    }
}
