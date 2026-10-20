//! 设备目录那几件：把新登记的一台挂到 `/device/<类>/<名>` 上、并立这一类那枚盟。
//!
//! 它是**协调入口手上的装置**（不是第二本账）：设备事实仍在 Hub 那一本 `Ledger` 里，
//! 这里只拿装配期发下的 `devices` 能力、本册那一台的主体与盟册那几面，
//! 把一台新设备的门牌按既有 Operator/Identity 准入挂出去。

use env::{Access, PieToken, Wait};
use system_api::identity::{CoalitionId, CoalitionSet, FoundId, PrincipalId, Reply, Selector, Wire};
use system_api::operator::{Entry, OperationId, Permit, ResourceView};
use system_client::operator::Capability;

use crate::service::hub::core::{EnrollmentEffects, Entry as DeviceEntry};
use super::enrollment_failure::EnrollmentFailure;
use super::League;

const MS: usize = 3000;

/// 挂一台新设备的门牌、立它那一类的盟。
pub struct Directory<'a> {
    pub devices: &'a Capability,
    pub anchor: Entry,
    pub league: &'a League,
    /// 本册那一台的主体（立盟时把自己放进类里）。
    pub subject: PrincipalId,
}

impl EnrollmentEffects for &Directory<'_> {
    type Error = EnrollmentFailure;

    fn found(&mut self, id: FoundId) -> Result<CoalitionId, Self::Error> {
        self.league.organization.found(id, Wait::AtMost(MS))
            .map_err(EnrollmentFailure::Identity)
    }

    fn admit(&mut self, coalition: CoalitionId) -> Result<(), Self::Error> {
        self.league.organization.admit(coalition, self.subject, Wait::AtMost(MS))
            .map_err(EnrollmentFailure::Identity)
    }

    fn activate(&mut self, coalition: CoalitionId) -> Result<(), Self::Error> {
        let coalitions = CoalitionSet::new(&[coalition])
            .map_err(|error| EnrollmentFailure::Identity(system_client::identity::CallError::Service(error)))?;
        match self.league.activation
            .call(Wire::Activate(env::unit::self_id(), coalitions), Wait::AtMost(MS))
            .map_err(EnrollmentFailure::Identity)?
        {
            Reply::Unit => Ok(()),
            _ => Err(EnrollmentFailure::Identity(system_client::identity::CallError::Malformed)),
        }
    }

    fn door(&mut self) -> Result<PieToken, Self::Error> {
        env::hole::unseal(hub_api::Grant::Claim.mark(), env::HoleLimits::default())
            .map_err(|error| EnrollmentFailure::Door(error.source))
    }

    fn part(&mut self, class: &str, id: OperationId) -> Result<Entry, Self::Error> {
        self.devices.part(self.anchor, class, id, Wait::AtMost(MS))
            .map(|part| part.entry)
            .map_err(EnrollmentFailure::Operator)
    }

    fn bind(
        &mut self,
        group: Entry,
        entry: &DeviceEntry,
        coalition: CoalitionId,
        id: OperationId,
    ) -> Result<(), Self::Error> {
        let view = ResourceView::new(Access::STORE).ok_or(EnrollmentFailure::Layout)?;
        self.devices.bind(
            group, &entry.name, entry.door, view,
            Permit::Identity(Selector::MemberOf(coalition)), id, Wait::AtMost(MS),
        ).map(|_| ()).map_err(EnrollmentFailure::Operator)
    }
}
