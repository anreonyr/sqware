//! :client 的门牌那一柄（Tile）：门牌上的坐标与那几手。

use alloc::string::String;

use env::PieToken;
use env::Wait;

use crate::service::operator as ocall;
use crate::service::operator::EntryId;
use crate::service::operator::Fail;

use super::pane::Pane;
use super::{Face, map_code};

/// **一枚砖**：`EntryId` 是固定下来的宾语，那一枚门闩是它背后的东西（到头了）。
/// 它与 Pane 是**同一格的两个方向**，不是一个"二选一"的包装：想往里走就 Tile::pane
/// （`list` 判），想拿那一枚就 Tile::token（`find` 判）。
pub struct Tile<'a> {
    pub(super) face: &'a Face,
    pub(super) id: EntryId,
}

impl Tile<'_> {
    pub fn id(&self) -> EntryId {
        self.id
    }

    pub fn name(&self, wait: Wait) -> Result<String, Fail> {
        let said = self.face.call(ocall::Req::Name(self.id), wait)?;
        said.name().map_err(map_code)
    }

    /// 按**窗格**读它：是窗格 ⇒ 继续往里走；是一枚砖 ⇒ Fail::NotAPane。
    pub fn pane(&self, wait: Wait) -> Result<Pane<'_>, Fail> {
        Pane::at(self.face, self.id, wait)
    }

    /// 按**砖**读它：把那一号背后那一枚 Pie 要过来。
    /// 那一枚**经会话授进本端表**，而**它在本端表里的号随这条答话回来**（ocall::Union::Seed），
    /// 故客人不必再扫表。寻到头是窗格 ⇒ Fail::NotATile。
    pub fn token(self, wait: Wait) -> Result<PieToken, Fail> {
        let said = self.face.call(ocall::Req::Find(self.id), wait)?;
        said.seed().map_err(map_code)
    }
}

// 它们是 Pane / Tile 那几个方法的实现体：那一层只做"补上宾语 + 转发 + 解释返回值"。
// 只借 `&Session`（不是 `&Face`）的那几处调用点（`programs/src/driver/context.rs::line` 那类
// 手里有会话、又拿不出 `Face` 所有权的地方）直接叫它们。
