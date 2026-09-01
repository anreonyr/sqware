//! 用户侧 dock — 共享内存邮路（多 pier 生产 / 唯一 quay 消费）。

// 硬不变量：锁外读 state / 锁内仅原子字段 + 槽 memcpy / Hang→Gone CAS 仅 quay 侧 / 成功方 wake(key)。

use erra::ResultExt;
use ubi::dock::{self, DOCK_KEY_TAG};
use ubi::{UError, UResult};

use crate::env::{mail as env_mail, room};
use crate::shared::{DockLayout, SharedBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockState {
    Live, Hang, Gone, Dead,
}

impl DockState {
    fn from_code(code: u8) -> DockState {
        match code {
            dock::state::LIVE => DockState::Live,
            dock::state::HANG => DockState::Hang,
            dock::state::GONE => DockState::Gone,
            _ => DockState::Dead,
        }
    }

    pub const fn pullable(self) -> bool {
        matches!(self, DockState::Live | DockState::Hang)
    }
}

/// 端（side）。
#[allow(dead_code)]
pub enum Side {
    Pier,
    Quay,
}

impl Side {
    #[allow(dead_code)]
    pub(crate) fn as_usize(self) -> usize {
        match self {
            Side::Pier => dock::side::PIER,
            Side::Quay => dock::side::QUAY,
        }
    }
}

pub struct Pier {
    id: usize,
    shared: SharedBuf<DockLayout>,
}

pub struct Quay {
    id: usize,
    shared: SharedBuf<DockLayout>,
}

pub fn open(item_len: usize, slots: usize) -> UResult<(Pier, Quay)> {
    let (id, view) = env_mail::dock_open(item_len, slots)?;
    let shared = SharedBuf::new(view);
    Ok((Pier { id, shared }, Quay { id, shared }))
}

pub fn join_pier(id: usize) -> UResult<Pier> {
    let view = env_mail::dock_join(id, dock::side::PIER)?;
    Ok(Pier { id, shared: SharedBuf::new(view) })
}

pub fn join_quay(id: usize) -> UResult<Quay> {
    let view = env_mail::dock_join(id, dock::side::QUAY)?;
    Ok(Quay { id, shared: SharedBuf::new(view) })
}

pub fn close(id: usize) -> UResult<()> {
    env_mail::dock_shut(id)
}

impl Clone for Pier {
    fn clone(&self) -> Pier {
        let _ = env_mail::dock_clone(self.id);
        Pier { id: self.id, shared: self.shared }
    }
}

impl Drop for Pier {
    fn drop(&mut self) { let _ = env_mail::dock_drop(self.id, dock::side::PIER); }
}

impl Drop for Quay {
    fn drop(&mut self) { let _ = env_mail::dock_drop(self.id, dock::side::QUAY); }
}

impl Pier {
    pub fn key(&self) -> usize {
        DOCK_KEY_TAG | self.id
    }

    pub fn try_push(&self, msg: &[u8]) -> UResult<()> {
        let st = DockState::from_code(self.shared.state().load(core::sync::atomic::Ordering::Acquire));
        if !matches!(st, DockState::Live) {
            return Err(UError::from_raw(dock::err::DEAD)).annotate("dock push (state)");
        }
        self.shared.acquire();
        let code = self.shared.try_push_locked(msg);
        self.shared.release();
        if code == 0 {
            let _ = room::wake(self.key());
            Ok(())
        } else {
            Err(UError::from_raw(code)).annotate("dock push")
        }
    }

    pub fn push(&self, msg: &[u8]) -> UResult<()> {
        loop {
            match self.try_push(msg) {
                Ok(()) => return Ok(()),
                Err(e) if e.source.code() == dock::err::BUSY => { let _ = room::wait(self.key(), usize::MAX); }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Quay {
    pub fn key(&self) -> usize {
        DOCK_KEY_TAG | self.id
    }

    pub fn try_pull(&self, buf: &mut [u8]) -> UResult<()> {
        self.shared.acquire();
        let mut code = self.shared.try_pull_locked(buf);
        // dock 专属：Hang 下取空 → CAS Gone 钉连，连接自然终了。
        if code == dock::err::BUSY {
            let st = DockState::from_code(self.shared.state().load(core::sync::atomic::Ordering::Acquire));
            if st == DockState::Hang {
                let _ = self.shared.state().compare_exchange(
                    dock::state::HANG,
                    dock::state::GONE,
                    core::sync::atomic::Ordering::AcqRel,
                    core::sync::atomic::Ordering::Acquire,
                );
                code = dock::err::GONE;
            }
        }
        self.shared.release();
        if code == 0 {
            let _ = room::wake(self.key());
            Ok(())
        } else {
            Err(UError::from_raw(code)).annotate("dock pull")
        }
    }

    pub fn pull(&self, buf: &mut [u8]) -> UResult<()> {
        loop {
            match self.try_pull(buf) {
                Ok(()) => return Ok(()),
                Err(e) if e.source.code() == dock::err::BUSY => { let _ = room::wait(self.key(), usize::MAX); }
                Err(e) => return Err(e),
            }
        }
    }
}