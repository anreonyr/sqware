//! uart 客户端：连上驱动 → **同步**写一段字节。
//!
//! 与 `dispatch`/`console`/`doom`/`irq` 的客户端同形：入口门闩由目录 `Connect` 转授，
//! 回信孔由**一次往返**（[`Port`]）自备，它对驱动侧的那枚句柄随每条请求交出。
//!
//! # 收的那一路不在这里
//!
//! 本会话**只有写**。设备收到的字节由驱动投进一枚**配给下来的投递孔**（root 开辟、
//! 两端各一份副本）——收不是协议，故客户端侧只有一枚读写方向相反的孔，不需要会话对象。

use env::EnvResult;

use runtime::core::port::Port;
use runtime::env::mail::HolePie;

use super::wire::{PAYLOAD_MAX, Request, Status, denied};

/// 等回执的上界（毫秒）。驱动在同一台机器上做一次逐字节的 `THRE` 轮询，远超实际耗时；
/// 有上界才能把"回执被丢"暴露成 `Busy`，而不是永久挂起（与 console/irq 同值）。
const ACK_WAIT_MS: usize = 1000;

/// 写通道的一条会话：一次往返的机制。
pub struct Uart {
    port: Port,
}

impl Uart {
    /// 开：`entry` = 目录 `Connect("uart")` 的入口副本（root 直接配给亦可）。
    ///
    /// 驱动 id 取 `Reserve(entry).owner`——门闩的**开辟者**（`vestor` 会被转发改写成
    /// root，`owner` 不会；与 `console`/`doom`/`irq` 认服务同一个办法）。
    pub fn open(entry: HolePie) -> EnvResult<Uart> {
        Ok(Uart {
            port: Port::open(&entry)?,
        })
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
            let request = Request::write(chunk).ok_or_else(denied)?;
            match self.port.call::<Request>(&request, ACK_WAIT_MS)? {
                Status::Ok => {}
                _ => return Err(denied()),
            }
        }
        Ok(())
    }
}
