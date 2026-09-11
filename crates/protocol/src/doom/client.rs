//! doom 客户端：打开会话 → `kill(名字)`。
//!
//! 与 `dispatch`/`console` 的客户端同形：入口门闩由目录 `Connect` 转授，回信孔由
//! **调用方自备**（`unseal` + `Accord` 给服务，把对端那个 token 随每条请求交出去）。

use env::{EnvError, EnvResult, Name, Permission, PieToken, TaskId, make_err};

use runtime::env::mail::AnyPie as _;
use runtime::env::mail::{self, HolePie};

use super::{ACK_LEN, Ack, Kill, REQ_LEN};

/// 等回执的上界（毫秒）。服务在同一台机器上做一次 envcall + 有界 `Join`，远超实际
/// 耗时；有上界才能把"回执被丢"暴露成 `Busy`，而不是永久挂起。
const ACK_TIMEOUT_MS: usize = 2000;

fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(-1))
}

/// 他杀服务的一条会话。
pub struct Doom {
    entry: HolePie,
    ack: HolePie,
    /// 回信孔在**服务侧**的 token（每条请求回填进报文）。
    ack_target: PieToken,
}

impl Doom {
    /// 打开：`entry` = 目录 `Connect("doom")` 给的那枚入口门闩副本。
    ///
    /// 服务 id 取 `Reserve(entry).owner`——门闩的**开辟者**（`vestor` 会被转发改写成
    /// root，`owner` 不会；与 `console` 认 PLIC 驱动同一个办法）。
    pub fn open(entry: HolePie) -> EnvResult<Doom> {
        let owner = mail::reserve(PieToken::new(entry.token()))?.1.get();
        if owner == 0 {
            return Err(denied());
        }
        let ack = HolePie::unseal(ACK_LEN)?;
        let ack_target = ack.accord(TaskId::new(owner), Permission::READ | Permission::WRITE)?;
        Ok(Doom {
            entry,
            ack,
            ack_target,
        })
    }

    /// 杀一个域（按名字）。**名字 → 目标**的解析在服务侧做（名字的账在目录里）。
    ///
    /// 回执四值见 [`Ack`]；`Ok` 的含义是"内核确认它回收完了"，不是一个"收到了"。
    pub fn kill(&self, target: &Name) -> EnvResult<Ack> {
        let msg: [u8; REQ_LEN] = Kill::new(*target, self.ack_target).encode();
        self.entry.push(&msg)?;
        let mut buf = [0u8; ACK_LEN];
        self.ack.pull_timeout(&mut buf, ACK_TIMEOUT_MS)?;
        Ok(Ack::from_byte(buf[0]))
    }
}
