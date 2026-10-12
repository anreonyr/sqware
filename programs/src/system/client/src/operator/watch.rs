//! `client` 的订阅那一柄（`Watch`）：**一枚孔** ＋ 那条订过的路 ＋ 已读到哪一号。
//!
//! # 事件怎么过来（与从前那一形的分界）
//! 从前本端铸**一页**（`Pole`）交给持树者、由它直接往里写：树上因此按订户数挂着 N 段映射
//! （而那一段在订户退场之后**谁也不回收**——实测：册里那一条只增不减、写一直成功）。
//! 现在本端只铸**一枚孔**：持树者把事件写进**它自己**那一具架的下一格，再往这枚孔上
//! **递一只手**（只登记"那一格在哪"，不复制字节）；本端 `pull` 那一刻内核从持树者那段内存
//! 复制**一次**过来。树上于是只有它自己那一页，与订户数无关。
//!
//! # 号（`seq`）为什么本端也要记
//! 手所指的那一格在环绕回来之后可能已经是**更新的内容**（同一只手取到的是后来的那一条），
//! 故判据是载荷里那一格号：**跳了** ⇒ 中间丢了几条（自己算）；**没前进** ⇒ 这一条读过
//! （环绕回来之前那一手），丢掉继续。
//!
//! # 序是契约
//! [`Watch::of`] 返回（回执 = `OK`）**之后**发生的改动才发得过来——订阅之前那些不在通知
//! 义务内（要那时的状态用 `list` / `seek` / `name`，那是另一回事）。

use alloc::string::String;

use env::{MailFail, Wait};

use ::resource::port::{self, Access, Policy};

use crate::operator::frame::Event;
use crate::operator::frame::watch::EventFrame;
use crate::operator::path::{Path, PathBuf};
use crate::operator::{Entry, EntryId, Fail};
use ipc::hand::{Receiver, RecvFail, SourceFail};

use super::Face;
use super::face::map_code;
use crate::operator as ocall;
use ::resource::raw::Capability;

/// 本端铸的那一枚孔叫什么（记号只在本地认领那一格用；持树者认的是**号**，不是记号）。
use crate::operator::marks::WATCH_MARK;

/// **一位订户的柄**：那一枚孔（本端持有）＋ 那条订过的路 ＋ 已读到哪一号。
///
/// 与 `Pane` / `Tile` 同侪：都是"一个值决定后续操作的宾语"。差别只在它管的是**此后**——
/// 树上真变了，持树者往这枚孔上递一只手；本端 `next` 就取一手。
pub struct Watch<'a> {
    face: &'a Face,
    /// 订的那条路（读数与撤订用）。
    road: String,
    filter: PathBuf,
    base: Option<Entry>,
    base_path: Option<PathBuf>,
    /// **本端那一枚孔**：持着它，退场即作废（树上那一侧下一次递手就把他摘掉）。
    _hole: EventHole,
    /// 收那一侧：等"有手"＋ 取一手 ＋ 解一条。
    read: Receiver<Event>,
    /// 取回来那一手的落点（`Receiver::recv` 要调用方给缓冲）。
    buf: [u8; EventFrame::LEN],
    /// 已读到哪一号。
    seq: u64,
}

/// Closing the root also invalidates the server's delivery capability.
struct EventHole(Capability);
impl Drop for EventHole {
    fn drop(&mut self) {
        let _ = self.0.seal();
    }
}

impl<'a> Watch<'a> {
    /// **订一条子树**：本端铸一枚孔，把它交给持树者，等它记下这一位。
    pub fn of(face: &'a Face, road: &Path, wait: Wait) -> Result<Watch<'a>, Fail> {
        Self::create(face, None, road, wait)
    }

    pub fn from(face: &'a Face, base: Entry, road: &Path, wait: Wait) -> Result<Watch<'a>, Fail> {
        if base.operator != face.host() {
            return Err(Fail::Denied);
        }
        Self::create(face, Some(base), road, wait)
    }

    fn create(
        face: &'a Face,
        base: Option<Entry>,
        road: &Path,
        wait: Wait,
    ) -> Result<Watch<'a>, Fail> {
        let hole = EventHole(Capability::unseal_hole(WATCH_MARK).map_err(|_| Fail::Unknown)?);
        // **那一枚孔要交给持树者**：它得推得进来（`Push` 是"把发送方那段登记到孔上"⇒ 要写权）。
        // `Policy::NONE`：接过来的人不必再授出（事件只有持树者递）。
        let shipped = port::ship(hole.0.token(), face.host(), Access::STORE, Policy::NONE)
            .map(|to| to.seed())
            .map_err(|_| Fail::Unknown)?;
        let request = match base {
            Some(base) => ocall::Req::WatchFrom {
                base,
                road: road.to_path_buf(),
                hole: shipped,
            },
            None => ocall::Req::Watch {
                road: road.to_path_buf(),
                // **交出去的那一枚在对面表里的号**（不是我手里那一枚）：两个编号空间不同源。
                hole: shipped,
            },
        };
        let said = face.call(request, wait)?;
        let (filter, base_path) = match base {
            Some(_) => {
                let base_path = said.watch_path().map_err(map_code)?;
                let filter = if road.as_str().is_empty() {
                    base_path.clone()
                } else if base_path.as_str().is_empty() {
                    road.to_path_buf()
                } else {
                    base_path.try_join(road.as_str()).ok_or(Fail::Unknown)?
                };
                (filter, Some(base_path))
            }
            None => {
                let code = said.status().ok_or(Fail::Unknown)?;
                if code != ocall::OK {
                    crate::debug::put(&alloc::format!("operator: watch refused code={code}"));
                    return Err(map_code(code));
                }
                (road.to_path_buf(), None)
            }
        };
        Ok(Watch {
            face,
            road: String::from(road.as_str()),
            filter,
            base,
            base_path,
            read: Receiver::from_raw(hole.0.token()),
            _hole: hole,
            buf: [0u8; EventFrame::LEN],
            seq: 0,
        })
    }

    /// **读一条事件**：孔上没手就等（`within` = 这一次等多久）。
    ///
    /// 取回来的是**持树者那一格的载荷**：号前进了 ⇒ 收下（中间丢了几条自己算）；号没前进 ⇒
    /// 这一条读过，丢掉再看。到期仍没有 ⇒ `Err(RecvFail::Mail(Busy))`（与 `Receiver` 同一套词）。
    pub fn next(&mut self, within: Wait) -> Result<Event, SourceFail> {
        let until = ipc::time::deadline(within);
        loop {
            match self
                .read
                .recv_from(self.face.host(), &mut self.buf, Wait::POLL)
            {
                Ok(ev) => {
                    if let Some(ev) = self.accept(ev) {
                        return Ok(ev);
                    }
                }
                Err(SourceFail::Receive(RecvFail::Mail(MailFail::Busy))) => {}
                Err(error) => return Err(error),
            }
            let remain = ipc::time::remain(until);
            if remain == Wait::POLL {
                return Err(SourceFail::Receive(RecvFail::Mail(MailFail::Busy)));
            }
            let ev = self
                .read
                .recv_from(self.face.host(), &mut self.buf, remain)?;
            if let Some(ev) = self.accept(ev) {
                return Ok(ev);
            }
        }
    }

    /// 看一眼：**不前进**（孔上没手就答 `None`）。
    pub fn try_next(&mut self) -> Result<Option<Event>, SourceFail> {
        loop {
            match self
                .read
                .recv_from(self.face.host(), &mut self.buf, Wait::POLL)
            {
                Ok(ev) => match self.accept(ev) {
                    Some(ev) => return Ok(Some(ev)),
                    None => continue,
                },
                Err(SourceFail::Receive(RecvFail::Mail(MailFail::Busy))) => return Ok(None),
                Err(e) => return Err(e),
            }
        }
    }

    /// 收下一条：**号前进了 ＋ 落在本端订的那条路上**才收。
    ///
    /// 后一个条件不是多余的：手所指的那一格在环绕回来之后可能已经被**别条路**的改动顶掉
    /// ——持树者只保证"递出去那一刻那一格是自己的"，不保证它一直是（`Mode::Oldest` 的代价）。
    /// 不匹配的当"丢了一条"跳过：下一次 `seq` 一跳就看得出来。
    fn accept(&mut self, mut ev: Event) -> Option<Event> {
        if ev.seq > self.seq && self.covers(ev.road.as_str()) {
            self.seq = ev.seq;
            if let Some(base) = &self.base_path {
                let path = ev.road.as_str();
                let relative = if base.as_str().is_empty() {
                    path
                } else if path == base.as_str() {
                    ""
                } else {
                    path.strip_prefix(base.as_str())?.strip_prefix('/')?
                };
                ev.road = PathBuf::try_new(relative)?;
            }
            Some(ev)
        } else {
            None
        }
    }

    /// Event path 落在服务端确认的 canonical subscription prefix 内吗。
    fn covers(&self, road: &str) -> bool {
        let f = self.filter.as_str();
        if f.is_empty() {
            return true;
        }
        road.strip_prefix(f)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    }

    /// 订的那条路（读数用）。
    pub fn road(&self) -> &str {
        self.road.as_str()
    }

    /// **订过的那条路此刻译成什么号**（`None` = 译不出）——"事件里那个号对不对"靠它。
    pub fn seek(&self, wait: Wait) -> Result<EntryId, Fail> {
        let road = PathBuf::try_new(self.road.as_str()).ok_or(Fail::Unknown)?;
        match self.base {
            Some(base) => self.face.tile_from(base, &road, wait).map(|tile| tile.id()),
            None => self.face.tile(&road, wait).map(|tile| tile.id()),
        }
    }
}
