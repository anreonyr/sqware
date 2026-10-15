use alloc::sync::{Arc, Weak};

use crate::core::memory::nest::{Access, View};
use crate::core::work::unit::space::{Mapping, Space};
use env::{PieToken, TeamId};

use super::relation::Relation;

/// 一项活跃关系：真实 `View` 加上它的放在哪、属于哪个 token。
///
/// 来源与授权记录都不在这里：`View` 自己强持来源，权限位归 `Grant`。
pub(super) struct HeldMap {
    pub(super) view: Arc<View>,
    pub(super) token: Option<PieToken>,
    pub(super) placement: Option<Weak<Space>>,
    pub(super) relation: Relation,
}

impl HeldMap {
    pub(super) fn placed_in(&self, space: &Arc<Space>) -> bool {
        self.placement
            .as_ref()
            .is_some_and(|weak| weak.ptr_eq(&Arc::downgrade(space)))
    }

    pub(super) fn placed_by(&self, token: PieToken) -> bool {
        self.holds(token) && self.placement.is_some()
    }

    pub(super) fn staged_for(&self, token: PieToken, team: TeamId) -> bool {
        self.holds(token) && self.staged() == Some(team)
    }

    pub(super) fn closable(&self, token: PieToken) -> bool {
        self.placed_by(token) && self.staged().is_none()
    }

    pub(super) fn writable(&self) -> bool {
        self.placement.is_some()
            && self.live()
            && self.view.active().map_or(true, |ranges| {
                ranges
                    .iter()
                    .any(|range| range.current.contains(Access::WRITE))
            })
    }

    pub(super) fn placed(
        mapping: Mapping,
        space: &Arc<Space>,
        token: Option<PieToken>,
        relation: Relation,
    ) -> Self {
        Self {
            view: mapping.view().clone(),
            token,
            placement: Some(Arc::downgrade(space)),
            relation,
        }
    }

    pub(super) fn input(&self) -> bool {
        self.relation == Relation::Input
    }

    pub(super) fn staged(&self) -> Option<TeamId> {
        if let Relation::Staged(team) = self.relation {
            Some(team)
        } else {
            None
        }
    }

    pub(super) fn opened(&self) -> bool {
        matches!(self.relation, Relation::Input | Relation::Open)
    }

    pub(super) fn mapped(&self) -> bool {
        self.opened() || self.relation == Relation::View
    }

    pub(super) fn live(&self) -> bool {
        self.view.is_live()
    }

    pub(super) fn holds(&self, token: PieToken) -> bool {
        self.token == Some(token)
    }
}
