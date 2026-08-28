//! 用户侧 mail 封装：port 阻塞语义（条件循环 + 调度域 wait/wake）。
//!
//! push/pull 的阻塞 = 用户侧循环：尝试 → Busy（-2）则 `wait(条件键)` → 重试；
//! 条件变更方（成功 push 的一方唤醒 pull 端，反之亦然）`wake` 对端。内核只
//! 管槽与拷贝（定长 [`MSG_LEN`]、单槽）。

use ubi::UResult;

use crate::env;

/// port 消息定长（与内核 `mail::port::MSG_LEN` 同步）。
pub const MSG_LEN: usize = 64;

/// port 用户侧句柄：持有 (句柄, 条件键)。
#[derive(Clone, Copy)]
pub struct Port {
    handle: usize,
    key: usize,
}

impl Port {
    /// 建 port：返回句柄（两端同持；条件键供 wait/wake）。
    pub fn open() -> UResult<Port> {
        let (handle, key) = env::port_open()?;
        Ok(Port { handle, key })
    }

    /// 条件键（wait/wake 用；两端同值）。
    pub fn key(&self) -> usize {
        self.key
    }

    /// 终止通道（Dead：对端 push/pull 返回 -1）。
    pub fn shut(&self) -> UResult<()> {
        env::port_shut(self.handle)
    }

    /// 投递（阻塞）：`msg` 长度须为 [`MSG_LEN`]。
    pub fn push(&self, msg: &[u8]) -> UResult<()> {
        debug_assert_eq!(msg.len(), MSG_LEN);
        loop {
            match env::port_try_push(self.handle, msg.as_ptr()) {
                Ok(()) => {
                    // 存入槽 → 槽满：唤醒可能阻塞的 pull 端（条件变更方负责 wake）
                    let _ = env::wake(self.key);
                    return Ok(());
                }
                Err(e) if e.source.code() == -2 => {
                    // 槽满：阻塞等槽空
                    let _ = env::wait(self.key, usize::MAX);
                }
                Err(e) => return Err(e), // -1 Dead（断开感知）等
            }
        }
    }

    /// 收取（阻塞）：`buf` 长度须为 [`MSG_LEN`]。
    pub fn pull(&self, buf: &mut [u8]) -> UResult<()> {
        debug_assert_eq!(buf.len(), MSG_LEN);
        loop {
            match env::port_try_pull(self.handle, buf.as_mut_ptr()) {
                Ok(()) => {
                    // 取走槽 → 槽空：唤醒可能阻塞的 push 端
                    let _ = env::wake(self.key);
                    return Ok(());
                }
                Err(e) if e.source.code() == -2 => {
                    // 槽空：阻塞等投递
                    let _ = env::wait(self.key, usize::MAX);
                }
                Err(e) => return Err(e),
            }
        }
    }
}
