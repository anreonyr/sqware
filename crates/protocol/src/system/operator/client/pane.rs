//! :client 的窗格那一柄（Pane）：一格子树的取窗、逐格、找名。

use alloc::string::String;

use env::{PieToken, Wait};
use runtime::core::res::port::{self, Access, Policy};
use runtime::env::mail;

use crate::common::path::Path;
use crate::system::operator as ocall;
use crate::system::operator::{Fail, EntryId, Listing, Where, Permit};

use super::tile::Tile;
use super::{Face, Mine, map_code};

/// **一块窗格**：**哪一个容器**是固定下来的宾语，那几手不再重复传它
/// 它能继续分 / 落 / 列——正是"一个值决定后续操作的宾语"那一格，故给它一个柄；一枚砖只需
/// 一枚号 ＋ 取那一枚门闩（见 Tile）
/// 里面存的是 Where：**根与"某一号"是线上两形**，柄必须两形都表达得出（Face::root）
pub struct Pane<'a> {
    pub(super) face: &'a Face,
    pub(super) at: Where,
}

impl<'a> Pane<'a> {
    /// **由一格造柄**（帧那一侧答出来的号）
    /// 着一枚号**的调用点——它们（`programs/src/harness/probe/probe_operator_*/main.rs` 那两位）要的正是"手里有号、
    pub fn of(face: &'a Face, id: EntryId) -> Pane<'a> {
        Pane {
            face,
            at: Where::At(id),
        }
    }

    /// 按**窗格**读一格：`list` 它一下——`list` 的判据正是"这一号是一块 `Pane`"
    /// （是一枚砖 ⇒ Fail::NotAPane；号不在 ⇒ Fail::Unknown）
    /// **它是"把一个已有的号读成窗格"那一格**：Face::pane 与 Pane::tile 的落点
    /// 也是"**手里已经有一枚号**"那一档（`Pane::of` 只是造柄、不问那一号是不是窗格；
    /// 这一手要问）——故公开给测具那两位用（`probe-operator-gate` / `probe-watch`）。
    pub fn at(face: &'a Face, id: EntryId, wait: Wait) -> Result<Pane<'a>, Fail> {
        face.call(ocall::Req::List(Where::At(id)), wait)?
            .list()
            .map_err(map_code)?;
        Ok(Pane::of(face, id))
    }

    pub fn id(&self) -> EntryId {
        match self.at {
            Where::Root => EntryId::new(0),
            Where::At(id) => id,
        }
    }

    /// **分**：在这一块里给 `name` 放一块空窗格；答那一格自己的号
    /// **幂等**：那一格已经是窗格就答它那个号（里面有没有东西不管）；是一枚砖 ⇒
    pub fn open(&self, name: String, wait: Wait) -> Result<Pane<'_>, Fail> {
        let said = self
            .face
            .call(ocall::Req::Part { at: self.at, name }, wait)?;
        let id = said.entry().map_err(map_code)?;
        Ok(Pane::of(self.face, id))
    }

    /// **落**：在这一块里给 `name` 贴一枚 `Tile`；答那一格自己的号
    pub fn bind(
        &self,
        name: String,
        e: PieToken,
        permit: Permit,
        mine: Mine,
        wait: Wait,
    ) -> Result<Tile<'_>, Fail> {
        let pie = mail::HolePie::from_token(e);
        let shipped = port::ship(
            &pie,
            self.face.session.host,
            Access::FETCH | Access::STORE,
            Policy::VEST,
        )
        .map(|to| to.seed())
        .map_err(|_| Fail::Unknown)?;
        let said = self.face.call(
            ocall::Req::Land {
                at: self.at,
                name,
                entry: shipped,
                permit,
                // **归不归自己**是语义；编成那一格 bit 只在这一句（`Mine` 不中途降成 `bool`）。
                mine: matches!(mine, Mine::Yes),
            },
            wait,
        )?;
        let id = said.entry().map_err(map_code)?;
        Ok(Tile {
            face: self.face,
            id,
        })
    }

    /// **列**：这一块里有哪些号（Face::root = 根那一层）
    /// 答的是那一串号（Listing 是定长值、不是借来的迭代器——它自带 `iter`）
    pub fn list(&self, wait: Wait) -> Result<Listing, Fail> {
        let said = self.face.call(ocall::Req::List(self.at), wait)?;
        said.list().map_err(map_code)
    }

    /// **剪**：把 `e` 那一号剪掉
    pub fn trim(&self, e: EntryId, wait: Wait) -> Result<(), Fail> {
        let said = self.face.call(ocall::Req::Trim(e), wait)?;
        match said.code() {
            ocall::OK => Ok(()),
            code => Err(map_code(code)),
        }
    }

    /// **名**：`e` 那一号此刻叫什么
    pub fn name(&self, e: EntryId, wait: Wait) -> Result<String, Fail> {
        let said = self.face.call(ocall::Req::Name(e), wait)?;
        said.name().map_err(map_code)
    }

    /// **一条路** → 一枚砖：只译号（`seek`），**不动树、不补问**——编号由 Tile::token 认
    /// **路从根写起，不是相对这一块**：`Road` 那一帧只带段列表，持树者从**根**解（正文明写
    pub fn tile(&self, road: &Path, wait: Wait) -> Result<Tile<'_>, Fail> {
        let said = self.face.call(ocall::Req::Road(road.to_path_buf()), wait)?;
        let id = said.entry().map_err(map_code)?;
        Ok(Tile {
            face: self.face,
            id,
        })
    }
}
