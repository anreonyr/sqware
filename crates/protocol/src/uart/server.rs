//! uart·server — 服务侧：认动词、验字段。**不做 I/O、无状态**。
//!
//! 回复**由调用方推**——与 `doom::serve` 同一条规矩（`doom/server.rs` 头注：
//! 「本函数不做 I/O——回复由调用方推送」）。写进设备那一件事由装配层做：它是设备动作，
//! 协议层不认识串口（§5.2）。
//!
//! # 为什么无状态
//!
//! 收的那一路不在这条协议上（走投递孔，见模块头）。于是服务侧只剩"解码 → 说该写什么"，
//! 没有表、没有会话、没有未决读——**一个纯函数**。

use env::PieToken;

use super::wire::{Query, Status, address_of};

/// 一条报文解出来的动作。
pub enum Action<'a> {
    /// 把这批字节写进设备，然后回 `Ok`。
    Write { reply: PieToken, bytes: &'a [u8] },
    /// 只回一个状态（坏报文 / 超长）。
    Reply { reply: PieToken, status: Status },
    /// 连回信地址都没有：丢弃。
    Ignore,
}

/// 认动词、验字段。**不碰设备、不推孔**。
pub fn serve(msg: &[u8]) -> Action<'_> {
    match Query::view(msg) {
        Some(bytes) => match address_of(msg) {
            Some(reply) => Action::Write { reply, bytes },
            None => Action::Ignore,
        },
        // 坏报文也欠对方一个答复：能抠出回信地址就答 `Denied`，抠不出来就丢。
        None => match address_of(msg) {
            Some(reply) => Action::Reply {
                reply,
                status: Status::Denied,
            },
            None => Action::Ignore,
        },
    }
}
