//! uart 客户端：连上驱动 → **同步**写一段字节。
//!
//! 与 `dispatch`/`console`/`doom`/`irq` 的客户端同形：入口门闩由目录 `Connect` 转授，
//! 回信孔由**一次往返**（[`Port`]）自备，它对驱动侧的那枚句柄随每条请求交出。
//!
//! **推是有界的**（[`push_within`]）：`Port::push` 只有"睡到有位置"一种策略，而这条链上
//! 任何一跳睡死，落屏就跟着停。
//!
//! # 收的那一路不在这里
//!
//! 本会话**只有写**。设备收到的字节由驱动投进一枚**配给下来的投递孔**（root 开辟、
//! 两端各一份副本）——收不是协议，故客户端侧只有一枚读写方向相反的孔，不需要会话对象。

use env::EnvResult;

use runtime::core::port::Port;
use runtime::env::mail::HolePie;

use super::wire::{CAP, PAYLOAD_MAX, Query, Status, denied};
use super::ACK_MS;
use crate::session::push_within;

/// 写通道的一条会话：**请求那一枚孔**（有界推）+ **回执那一侧**（[`Port`]：有界收、核来源）。
pub struct Uart {
    entry: HolePie,
    port: Port,
}

impl Uart {
    /// 开：`entry` = 目录 `Connect("uart")` 的入口副本（root 直接配给亦可）。
    ///
    /// 驱动 id 取 `Reserve(entry).owner`——门闩的**开辟者**（`vestor` 会被转发改写成
    /// root，`owner` 不会；与 `console`/`doom`/`irq` 认服务同一个办法）。
    pub fn open(entry: HolePie) -> EnvResult<Uart> {
        let port = Port::open(&entry)?;
        Ok(Uart {
            entry: HolePie::from_token(entry.token()),
            port,
        })
    }

    /// 本协议的一次往返：编帧（回信地址按**本协议**那一格填）→ 推 → 收（[`Port::pull`]
    /// 内建核对来源）→ 解回执。
    ///
    /// 这一段是**协议自己的**：帧怎么编、回执怎么解只有它知道；机制只管把那两枚孔配成一对。
    fn ask(&self, req: &Query) -> EnvResult<Status> {
        let (frame, n) = req.encode(self.port.seed());
        push_within(&self.entry, frame.get(..n).ok_or_else(denied)?, ACK_MS)?;
        let mut out = [0u8; CAP];
        Status::decode(self.port.pull(&mut out, ACK_MS)?)
    }

    /// 写一段字节：**返回 `Ok` 即"设备已吃下"**（驱动侧逐字节轮询 `THRE` 才回执）。
    ///
    /// 超过 [`PAYLOAD_MAX`] 自行分片，每片一次往返；中途失败即止——**已发出的片收不回**，
    /// 故调用方不要把"写成功"当成"全部原子落下"。
    ///
    /// 错误：`Busy`（超时）或 `Denied`（来源不符）⇒ **本会话不可复用**（迟到的回执会
    /// 污染下一次收，与 dispatch 的 `Directory::call` 同一条契约）；`Denied`（入口
    /// 已失效）；`Dead`（驱动没了）。
    pub fn write(&self, bytes: &[u8]) -> EnvResult<()> {
        for chunk in bytes.chunks(PAYLOAD_MAX) {
            let request = Query::write(chunk).ok_or_else(denied)?;
            match self.ask(&request)? {
                Status::Ok => {}
                _ => return Err(denied()),
            }
        }
        Ok(())
    }
}
