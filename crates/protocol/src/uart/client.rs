//! uart 客户端：连上驱动 → **同步**写一段字节。
//!
//! 与 `dispatch`/`console`/`doom`/`irq` 的客户端同形：入口门闩由目录 `Connect` 转授，
//! 回信孔由**调用方自备**（`unseal` + `Accord` 给服务，把对端那个 token 随每条请求交出去）。
//!
//! # 收的那一路不在这里
//!
//! 本会话**只有写**。设备收到的字节由驱动投进一枚**配给下来的投递孔**（root 开辟、
//! 两端各一份副本）——收不是协议，故客户端侧只有一枚读写方向相反的孔，不需要会话对象。

use env::{EnvError, EnvResult, Permission, PieToken, make_err};

use runtime::env::mail::AnyPie as _;
use runtime::env::mail::{self, HolePie};

use super::wire::{ACK_LEN, PAYLOAD_MAX, Request, Status};

/// 等回执的上界（毫秒）。驱动在同一台机器上做一次逐字节的 `THRE` 轮询，远超实际耗时；
/// 有上界才能把"回执被丢"暴露成 `Busy`，而不是永久挂起（与 console/irq 同值）。
const ACK_WAIT_MS: usize = 1000;

fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(-1))
}

/// 写通道的一条会话：入口门闩 + 自备的回信孔 + 回信孔在**驱动侧**的 token。
pub struct Uart {
    entry: HolePie,
    reply: HolePie,
    at_driver: PieToken,
}

impl Uart {
    /// 开：`entry` = 目录 `Connect("uart")` 的入口副本（root 直接配给亦可）。
    ///
    /// 驱动 id 取 `Reserve(entry).owner`——门闩的**开辟者**（`vestor` 会被转发改写成
    /// root，`owner` 不会；与 `console`/`doom`/`irq` 认服务同一个办法）。
    pub fn open(entry: HolePie) -> EnvResult<Uart> {
        let owner = mail::reserve(PieToken::new(entry.token()))?.1;
        if owner.get() == 0 {
            return Err(denied());
        }
        let reply = HolePie::unseal()?;
        let at_driver = reply.accord(owner, Permission::READ | Permission::WRITE)?;
        Ok(Uart {
            entry,
            reply,
            at_driver,
        })
    }

    /// 写一段字节：**返回 `Ok` 即"设备已吃下"**（驱动侧逐字节轮询 `THRE` 才回执）。
    ///
    /// 超过 [`PAYLOAD_MAX`] 自行分片，每片一次往返；中途失败即止——**已发出的片收不回**，
    /// 故调用方不要把"写成功"当成"全部原子落下"。
    ///
    /// 错误：`Busy`（超时）⇒ **本会话不可复用**（迟到的回执会污染下一次 pull，
    /// 与 dispatch 的 `Directory::call` 同一条契约）；`Denied`（入口/回执地址已失效）；
    /// `Dead`（驱动没了）。
    pub fn write(&self, bytes: &[u8]) -> EnvResult<()> {
        for chunk in bytes.chunks(PAYLOAD_MAX) {
            let msg = Request::write(chunk, self.at_driver).ok_or_else(denied)?;
            self.entry.push(&msg.encode())?;
            let mut ack = [0u8; ACK_LEN];
            self.reply.pull_timeout(&mut ack, ACK_WAIT_MS)?;
            match Status::from_byte(ack[0]) {
                Some(Status::Ok) => {}
                _ => return Err(denied()),
            }
        }
        Ok(())
    }
}
