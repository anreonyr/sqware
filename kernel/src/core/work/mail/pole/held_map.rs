use alloc::sync::{Arc, Weak};

use crate::core::memory::manager::entry::PteFlags;
use crate::core::memory::manager::{Map, Mapping, Reservation};
use crate::core::work::grant::Grant;
use crate::core::work::unit::space::Space;
use env::{Permission, PieToken, TeamId};

use super::relation::Relation;

pub(super) struct HeldMap {
    pub(super) map: Arc<Map>,
    pub(super) token: Option<PieToken>,
    pub(super) grant: Option<Arc<Grant>>,
    pub(super) placement: Option<(Weak<Space>, Reservation)>,
    pub(super) relation: Relation,
}

impl HeldMap {
    pub(super) fn placed_in(&self, space: &Arc<Space>) -> bool {
        self.placement
            .as_ref()
            .is_some_and(|(weak, _)| weak.ptr_eq(&Arc::downgrade(space)))
    }

    pub(super) fn placed_by(&self, token: PieToken) -> bool {
        self.holds(token) && self.placement.is_some()
    }

    pub(super) fn staged_for(&self, token: PieToken, team: TeamId) -> bool {
        self.holds(token) && self.staged() == Some(team)
    }

    pub(super) fn closable(&self, token: PieToken) -> bool {
        self.placed_by(token) && !self.keeper() && self.staged().is_none()
    }

    pub(super) fn writable(&self) -> bool {
        self.placement.is_some()
            && !self.keeper()
            && self.live()
            && self.map.ranges().map_or(true, |ranges| {
                ranges
                    .iter()
                    .any(|range| range.current.contains(PteFlags::W))
            })
    }

    pub(super) fn permitted(&self) -> Option<Permission> {
        self.grant.as_ref().and_then(|grant| grant.permission())
    }

    pub(super) fn storing(&self) -> bool {
        self.holds_authority()
            && self
                .permitted()
                .is_some_and(|p| p.contains(Permission::STORE))
    }

    pub(super) fn placed(
        mapping: Mapping,
        space: &Arc<Space>,
        token: Option<PieToken>,
        grant: Option<Arc<Grant>>,
        relation: Relation,
    ) -> Self {
        Self {
            map: mapping.map,
            token,
            grant,
            placement: Some((Arc::downgrade(space), mapping.reservation)),
            relation,
        }
    }

    pub(super) fn authority(map: Arc<Map>, token: PieToken, grant: Arc<Grant>) -> Self {
        Self {
            map,
            token: Some(token),
            grant: Some(grant),
            placement: None,
            relation: Relation::Authority,
        }
    }

    pub(super) fn keeper(&self) -> bool {
        self.relation == Relation::Keeper
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
        self.map.is_live()
    }

    pub(super) fn holds(&self, token: PieToken) -> bool {
        self.token == Some(token)
    }

    pub(super) fn holds_authority(&self) -> bool {
        self.relation == Relation::Authority
    }

    pub(super) fn authorizes(&self, token: PieToken) -> bool {
        self.holds_authority() && self.holds(token)
    }

    pub(super) fn granted(&self) -> bool {
        self.holds_authority() && self.permitted().is_some()
    }
}
