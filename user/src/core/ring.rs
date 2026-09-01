//! 用户侧 ring — 一对一共享内存邮路。

// 硬不变量：锁外读 state / 锁内仅原子字段 + 槽 memcpy / 无 Hang → Gone CAS / 成功方 wake(key)。

use erra::ResultExt;
use ubi::ring::{self, RING_KEY_TAG};
use ubi::{UError, UResult};

use crate::env::{mail as env_mail, room};
use crate::shared::{RingLayout, SharedBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingState {
    Live, Dead,
}

impl RingState {
    fn from_code(code: u8) -> RingState {
        match code {
            ring::state::LIVE => RingState::Live,
            _ => RingState::Dead,
        }
    }

    pub const fn pullable(self) -> bool {
        matches!(self, RingState::Live)
    }
}

pub struct Producer {
    id: usize,
    shared: SharedBuf<RingLayout>,
}

pub struct Consumer {
    id: usize,
    shared: SharedBuf<RingLayout>,
}

pub fn open(item_len: usize, slots: usize) -> UResult<(Producer, Consumer)> {
    let (id, view) = env_mail::ring_open(item_len, slots)?;
    let shared = SharedBuf::new(view);
    Ok((Producer { id, shared }, Consumer { id, shared })
    )
}

pub fn close(id: usize) -> UResult<()> {
    env_mail::ring_close(id)
}

impl Drop for Producer {
    fn drop(&mut self) { let _ = env_mail::ring_close(self.id); }
}

impl Drop for Consumer {
    fn drop(&mut self) { let _ = env_mail::ring_close(self.id); }
}

impl Producer {
    pub fn id(&self) -> usize { self.id }

    pub fn join(id: usize) -> UResult<Producer> {
        let view = env_mail::ring_join(id)?;
        Ok(Producer { id, shared: SharedBuf::new(view) })
    }

    pub fn key(&self) -> usize {
        RING_KEY_TAG | self.id
    }

    pub fn try_push(&self, msg: &[u8]) -> UResult<()> {
        let st = RingState::from_code(self.shared.state().load(core::sync::atomic::Ordering::Acquire));
        if !matches!(st, RingState::Live) {
            return Err(UError::from_raw(ring::err::DEAD)).annotate("ring push (state)");
        }
        self.shared.acquire();
        let code = self.shared.try_push_locked(msg);
        self.shared.release();
        if code == 0 {
            let _ = room::wake(self.key());
            Ok(())
        } else {
            Err(UError::from_raw(code)).annotate("ring push")
        }
    }

    pub fn push(&self, msg: &[u8]) -> UResult<()> {
        loop {
            match self.try_push(msg) {
                Ok(()) => return Ok(()),
                Err(e) if e.source.code() == ring::err::BUSY => { let _ = room::wait(self.key(), usize::MAX); }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Consumer {
    pub fn id(&self) -> usize { self.id }

    pub fn join(id: usize) -> UResult<Consumer> {
        let view = env_mail::ring_join(id)?;
        Ok(Consumer { id, shared: SharedBuf::new(view) })
    }

    pub fn key(&self) -> usize {
        RING_KEY_TAG | self.id
    }

    pub fn try_pull(&self, buf: &mut [u8]) -> UResult<()> {
        self.shared.acquire();
        let code = self.shared.try_pull_locked(buf);
        self.shared.release();
        if code == 0 {
            let _ = room::wake(self.key());
            Ok(())
        } else {
            Err(UError::from_raw(code)).annotate("ring pull")
        }
    }

    pub fn pull(&self, buf: &mut [u8]) -> UResult<()> {
        loop {
            match self.try_pull(buf) {
                Ok(()) => return Ok(()),
                Err(e) if e.source.code() == ring::err::BUSY => { let _ = room::wait(self.key(), usize::MAX); }
                Err(e) => return Err(e),
            }
        }
    }
}