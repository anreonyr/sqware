//! 用户侧 Hole 封装：pie 门闩的阻塞 push/pull（条件循环 + 调度域 wait/wake）。

use ubi::UResult;

use crate::env::{mail as env_mail, room};

pub const MSG_LEN: usize = 64;

/// Hole 句柄（pie_idx 持有者）。`Drop` 触发内核 shut。
pub struct Hole {
    idx: usize,
}

impl Hole {
    pub fn open() -> UResult<Self> {
        let idx = env_mail::hole_open()?;
        Ok(Self { idx })
    }

    pub fn idx(&self) -> usize {
        self.idx
    }

    pub fn try_push(&self, msg: &[u8]) -> UResult<()> {
        debug_assert_eq!(msg.len(), MSG_LEN);
        env_mail::hole_push(self.idx, msg as *const _ as *const [u8; MSG_LEN])
    }

    pub fn try_pull(&self, buf: &mut [u8]) -> UResult<()> {
        debug_assert_eq!(buf.len(), MSG_LEN);
        env_mail::hole_pull(self.idx, buf as *mut _ as *mut [u8; MSG_LEN])
    }

    pub fn push(&self, msg: &[u8]) -> UResult<()> {
        debug_assert_eq!(msg.len(), MSG_LEN);
        loop {
            match self.try_push(msg) {
                Ok(()) => {
                    let _ = room::wake(self.idx);
                    return Ok(());
                }
                Err(e) if e.source.code() == -3 => {
                    let _ = room::wait(self.idx, usize::MAX);
                }
                Err(e) => return Err(e),
            }
        }
    }

    pub fn pull(&self, buf: &mut [u8]) -> UResult<()> {
        debug_assert_eq!(buf.len(), MSG_LEN);
        loop {
            match self.try_pull(buf) {
                Ok(()) => {
                    let _ = room::wake(self.idx);
                    return Ok(());
                }
                Err(e) if e.source.code() == -3 => {
                    let _ = room::wait(self.idx, usize::MAX);
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for Hole {
    fn drop(&mut self) {
        let _ = env_mail::shut(self.idx);
    }
}