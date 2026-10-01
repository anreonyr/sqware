//! hub::core::league — **类 → 盟**：那一行的形状（[`League`]）＋ 两枚（铸或取 / 读）。
//! 盟由适配层铸（`mint` 是喂进来的闭包）——核心不叫盟册。

use alloc::string::String;
use protocol::service::coalition::CoalitionId;

use super::Ledger;

/// 一类 → 它那枚盟。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct League {
    class: String,
    coalition: CoalitionId,
}

impl Ledger {
    /// 册 · 写：这一类那枚盟；没铸过就铸（`mint` 由适配层给——核心不叫盟册）。
    /// **一处定义**：写 permit 与"代报名"都从这里取，故同一类不会有两枚盟。
    pub fn league(&mut self, class: String, mint: impl FnOnce() -> CoalitionId) -> CoalitionId {
        if let Some(league) = self.leagues.iter().find(|l| l.class == class) {
            return league.coalition;
        }
        let coalition = mint();
        if self.leagues.try_reserve(1).is_ok() {
            self.leagues.push(League { class, coalition });
        }
        coalition
    }

    /// 册 · 读：这一类那枚盟（没铸过 ⇒ `None`）。
    pub fn coalition_of(&self, class: String) -> Option<CoalitionId> {
        self.leagues
            .iter()
            .find(|l| l.class == class)
            .map(|l| l.coalition)
    }
}
