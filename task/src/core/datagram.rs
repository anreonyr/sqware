//! MailDatagram — Hole 之上的端口多路复用（UDP 形态、无 checksum）。
//!
//! 一条 datagram = `[header: 6][payload...]`，头含 src/dst 端口、长度。
//! 多 task 可经 Accord 拿到同一 HoleMeta 的写 pie（各人不同 src 端口）；
//! 收方按 `dst` 端口派发。
//!
//! **不可靠**：Hole 仍是单槽、无重传、无序；同地址空间无传输噪声，无需 checksum。
//!
//! **不开新 envcall**：纯用户态库——`Hole`（变长，C 已落地）之上套编解码层。

use env::EnvResult;
use env::EnvError;

use crate::env::mail::{HolePie, HOLE_MTU_MAX};

/// datagram 头长（3 个 u16：src / dst / length）。
pub const HEADER_LEN: usize = 6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Header {
    pub src: u16,
    pub dst: u16,
    pub length: u16,
}

/// datagram 解析错误。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DatagramError {
    /// 缓冲太短，编码或解码失败。
    TooShort,
    /// 长度字段越界或与载荷不匹配。
    BadLength,
}

const E_DENIED: isize = -1;

fn denied() -> erra::Error<EnvError> {
    env::make_err(EnvError::from_raw(E_DENIED))
}

/// 把一条 datagram（`src` + `dst` 端口 + 载荷）编码进 `dst`：
/// `dst[..6]` 头，`dst[6..6+payload.len()]` 载荷。返写入字节数（= 头 + 载荷）。
pub fn encode_into(
    dst: &mut [u8],
    src: u16,
    dst_port: u16,
    payload: &[u8],
) -> Result<usize, DatagramError> {
    let total = HEADER_LEN + payload.len();
    if total > u16::MAX as usize {
        return Err(DatagramError::TooShort);
    }
    if dst.len() < total {
        return Err(DatagramError::TooShort);
    }
    dst[0..2].copy_from_slice(&src.to_le_bytes());
    dst[2..4].copy_from_slice(&dst_port.to_le_bytes());
    dst[4..6].copy_from_slice(&(total as u16).to_le_bytes());
    dst[HEADER_LEN..total].copy_from_slice(payload);
    Ok(total)
}

/// 解码 `msg`：返 (Header, payload 在 `msg` 中的起点偏移)。
/// `msg.len()` 可大于 datagram 长度（接收方按 `Header.length` 截取）。
pub fn decode(msg: &[u8]) -> Result<(Header, usize), DatagramError> {
    if msg.len() < HEADER_LEN {
        return Err(DatagramError::TooShort);
    }
    let src = u16::from_le_bytes(msg[0..2].try_into().unwrap());
    let dst = u16::from_le_bytes(msg[2..4].try_into().unwrap());
    let len = u16::from_le_bytes(msg[4..6].try_into().unwrap()) as usize;
    if len < HEADER_LEN || len > msg.len() {
        return Err(DatagramError::BadLength);
    }
    Ok((Header { src, dst, length: len as u16 }, HEADER_LEN))
}

/// 数据报封装：自己的端口 + 一条 Hole。
pub struct MailDatagram {
    hole: HolePie,
    port: u16,
}

impl MailDatagram {
    pub fn new(hole: HolePie, port: u16) -> Self {
        Self { hole, port }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn hole(&self) -> &HolePie {
        &self.hole
    }

    pub fn seal(&self) -> EnvResult<()> {
        self.hole.seal()
    }

    /// 发一条 datagram 到 `dst_port`。
    pub fn send_to(&self, dst_port: u16, payload: &[u8]) -> EnvResult<()> {
        let mut buf = [0u8; HOLE_MTU_MAX];
        let n = encode_into(&mut buf, self.port, dst_port, payload).map_err(|_| denied())?;
        self.hole.push(&buf[..n])
    }

    /// 收一条 datagram：把载荷复制到 `buf[0..payload_len]`，返 `(src_port, payload_len)`。
    /// `buf.len()` 须 ≥ MTU，否则大载荷会被截断为 0。
    pub fn recv(&self, buf: &mut [u8]) -> EnvResult<(u16, usize)> {
        let n = self.hole.pull(buf)?;
        let (hdr, off) = decode(&buf[..n]).map_err(|_| denied())?;
        let payload_len = n - off;
        if buf.len() < payload_len {
            return Err(denied());
        }
        buf.copy_within(off..n, 0);
        Ok((hdr.src, payload_len))
    }
}