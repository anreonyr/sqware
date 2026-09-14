//! doom 客户端：打开会话 → `kill(名字)`。
//!
//! 与 `dispatch`/`console` 的客户端同形：入口门闩由目录 `Connect` 转授，回信孔由
//! **一次往返**（[`Port`]）自备，它对服务侧的那枚句柄随每条请求交出。

use env::{EnvResult, Name};

use runtime::core::port::Port;
use runtime::env::mail::HolePie;

use super::{Ack, Query};

/// 等回执的上界（毫秒）。服务在同一台机器上做一次 envcall + 有界 `Join`，远超实际
/// 耗时；有上界才能把"回执被丢"暴露成 `Busy`，而不是永久挂起。
const ACK_TIMEOUT_MS: usize = 2000;

/// 他杀服务的一条会话。
pub struct Doom {
    port: Port,
}

impl Doom {
    /// 打开：`entry` = 目录 `Connect("doom")` 给的那枚入口门闩副本。
    ///
    /// 服务 id 取 `Reserve(entry).owner`——门闩的**开辟者**（`vestor` 会被转发改写成
    /// root，`owner` 不会；与 `console` 认 PLIC 驱动同一个办法）。
    pub fn open(entry: HolePie) -> EnvResult<Doom> {
        Ok(Doom {
            port: Port::open(&entry)?,
        })
    }

    /// 杀一个域（按名字）。**名字 → 目标**的解析在服务侧做（名字的账在目录里）。
    ///
    /// 回执四值见 [`Ack`]；`Ok` 的含义是"内核确认它回收完了"，不是一个"收到了"。
    pub fn kill(&self, target: &Name) -> EnvResult<Ack> {
        self.port
            .call::<Query>(&Query::new(*target), ACK_TIMEOUT_MS)
    }
}
