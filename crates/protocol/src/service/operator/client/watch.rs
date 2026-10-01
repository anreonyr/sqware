//! `client` 的订阅那一柄（`Watch`）：一具架（页 ＋ 铃）＋ 那一条订过的路。

use alloc::string::String;

use env::Wait;

use runtime::core::res::port::{self, Access, Policy};

use crate::common::path::{Path, PathBuf};
use crate::communication::rack::{self, Reader};
use crate::service::operator::frame::watch::Event;
use crate::service::operator::{EntryId, Fail};

use super::{Face, map_code};
use crate::service::operator as ocall;

/// **一位订户的柄**：那一具架（页 ＋ 铃，本端持有）＋ 那一条**订过的路**。
///
/// 与 `Pane` / `Tile` 同侪：都是"一个值决定后续操作的宾语"。差别只在它管的是**此后**——
/// 树上真变了，持树者往本端这一页里记一条、响一次铃；本端 `next` 就读一条。
///
/// **游标只有一枚**（`Reader` 住在本柄里）：`Rack::reader()` 每叫一次都是**从头读**的那一枚，
/// 故这里只取一次、此后一直用它——同一个柄上叫两次不会各读一半。
pub struct Watch<'a> {
    face: &'a Face,
    /// 那一具架（页是事件往哪写，铃是"有事"）；**持有它**，映射才活着。
    _rack: rack::Rack<Event>,
    /// 那一枚游标（读端）。
    reader: Reader<Event>,
    /// 订的那条路（撤订与读数用）。
    road: String,
}

impl<'a> Watch<'a> {
    /// **订一条子树**：本端开一具架（`Rack::open`），把页与铃交给持树者，等它记下这一位。
    ///
    /// 序是契约：**这一手返回（回执 = `OK`）之后**发生的改动才发得过来——订阅之前那些不在
    /// 通知义务内（要那时的状态用 `list` / `seek` / `name`，那是另一回事）。
    pub fn of(face: &'a Face, road: &Path, wait: Wait) -> Result<Watch<'a>, Fail> {
        let rack = rack::Rack::<Event>::open(rack::Mode::Oldest).map_err(|_| Fail::Unknown)?;
        let (page, bell) = rack.ship();
        // **那一页要交给持树者**：它得把这枚页借映进自己那张表（映射是写自己的页表 ⇒ 那一问
        // 要 `STORE`）。只给 `FETCH` 的症状是"对端 `Dock::open` 答 `Denied`"——实测踩过。
        // `Policy::NONE`：接过来的人不必再授出（事件只有持树者写）。
        let shipped = port::ship(
            &runtime::env::mail::PolePie::from_token(page),
            face.host(),
            Access::FETCH | Access::STORE,
            Policy::NONE,
        )
        .map(|to| to.seed())
        .map_err(|_| Fail::Unknown)?;
        let said = face.call(
            ocall::Req::Watch {
                road: road.to_path_buf(),
                // **交出去的那一枚在对面表里的号**（不是我手里那一枚）：两个编号空间不同源。
                page: shipped,
                bell,
            },
            wait,
        )?;
        let code = said.code();
        match code {
            ocall::OK => {
                let reader = rack.reader();
                Ok(Watch {
                    face,
                    _rack: rack,
                    reader,
                    road: String::from(road.as_str()),
                })
            }
            // 那一格码原样说出去（`OK` 以外的每一档各有成因，别折成同一个 `Unknown`）。
            code => {
                crate::debug!("operator: watch refused code={code}");
                Err(map_code(code))
            }
        }
    }

    /// **读一条事件**：架上没有就等铃（`within` 是这一次调用等多久）。
    /// 到期仍没有 ⇒ `Err(RecvFail::Empty)`（与 `Receiver` 那三格同一套词）。
    pub fn next(&mut self, within: Wait) -> Result<Event, rack::RecvFail> {
        self.reader.recv(within)
    }

    /// 看一眼：**不前进**（架上有就取，没有就答 `None`）。
    pub fn try_next(&mut self) -> Result<Option<Event>, rack::RecvFail> {
        self.reader.try_recv()
    }

    /// 订的那条路（读数用）。
    pub fn road(&self) -> &str {
        self.road.as_str()
    }

    /// **订过的那条路此刻译成什么号**（`None` = 译不出）——"事件里那个号对不对"靠它。
    pub fn seek(&self, wait: Wait) -> Result<EntryId, Fail> {
        let road = PathBuf::try_new(self.road.as_str()).ok_or(Fail::Unknown)?;
        self.face.tile(&road, wait).map(|tile| tile.id())
    }
}
