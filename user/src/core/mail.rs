//! 用户侧 mail 封装：port 阻塞语义（条件循环 + 调度域 wait/wake）。

// 硬不变量：push/pull 成功方负责 wake 对端；唤醒后条件未必成立，须重试。

use ubi::UResult;

use crate::env::{mail as env_mail, room};

pub const MSG_LEN: usize = 64;

#[derive(Clone)]
pub struct Port {
    handle: usize,
    key: usize,
}

impl Port {
    pub fn open() -> UResult<Port> {
        let (handle, key) = env_mail::port_open()?;
        Ok(Port { handle, key })
    }

    pub fn join(handle: usize) -> UResult<Port> {
        let key = env_mail::port_join(handle)?;
        Ok(Port { handle, key })
    }

    pub fn handle(&self) -> usize {
        self.handle
    }

    pub fn key(&self) -> usize {
        self.key
    }

    pub fn close(&self) -> UResult<()> {
        env_mail::port_shut(self.handle)
    }

    pub fn try_push(&self, msg: &[u8]) -> UResult<()> {
        env_mail::port_try_push(self.handle, msg.as_ptr())
    }

    pub fn try_pull(&self, buf: &mut [u8]) -> UResult<()> {
        env_mail::port_try_pull(self.handle, buf.as_mut_ptr())
    }

    pub fn push(&self, msg: &[u8]) -> UResult<()> {
        debug_assert_eq!(msg.len(), MSG_LEN);
        loop {
            match self.try_push(msg) {
                Ok(()) => {
                    let _ = room::wake(self.key);
                    return Ok(());
                }
                Err(e) if e.source.code() == -2 => {
                    let _ = room::wait(self.key, usize::MAX);
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
                    let _ = room::wake(self.key);
                    return Ok(());
                }
                Err(e) if e.source.code() == -2 => {
                    let _ = room::wait(self.key, usize::MAX);
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for Port {
    fn drop(&mut self) {
        let _ = env_mail::port_shut(self.handle);
    }
}
