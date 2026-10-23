use env::{PieToken, TaskId};
use hub_api::{DeviceId, Enroll, Fail, INVENTORY_MAX};
use system_api::operator::OperationId;

use super::{Cell, Enrollment, EnrollmentEffects, EnrollmentStep, Entry, Ledger};

impl Ledger {
    /// Reserve both device and class responsibilities before acquiring external resources.
    pub fn prepare_enrollment(&mut self, from: TaskId, enroll: Enroll) -> Result<(), Fail> {
        if self.settling {
            return Err(Fail::Denied);
        }
        if !system_api::operator::name::valid(&enroll.name)
            || !system_api::operator::name::valid(&enroll.class)
        {
            return Err(Fail::Bad);
        }
        if let Some(cell) = self.cells.iter().find(|cell| {
            cell.device().is_some_and(|device| device.requester == enroll.requester)
        }) {
            let entry = cell.entry();
            return if cell.device().is_some_and(|device| device.pci == from) && entry.name == enroll.name
                && entry.class == enroll.class && entry.line == enroll.line
            {
                Ok(())
            } else {
                Err(Fail::Conflict)
            };
        }
        if self.cells.iter().any(|cell| cell.entry().name == enroll.name) {
            return Err(Fail::Conflict);
        }
        if self.cells.len() >= INVENTORY_MAX {
            return Err(Fail::Full);
        }
        self.cells.try_reserve(1).map_err(|_| Fail::Full)?;
        self.prepare_league(enroll.class.clone())?;
        self.cells.push(Cell::Enrolling(Enrollment {
            entry: Entry {
                name: enroll.name,
                class: enroll.class,
                line: enroll.line,
                page: PieToken::NONE,
                door: PieToken::NONE,
            },
            origin: Some(DeviceId { pci: from, requester: enroll.requester }),
            step: EnrollmentStep::Door,
        }));
        Ok(())
    }

    /// A failed or unknown reply leaves the same resource handles and operation IDs in place.
    pub fn advance_enrollment<E: EnrollmentEffects>(
        &mut self,
        requester: u32,
        effects: &mut E,
    ) -> Option<Result<(), E::Error>> {
        if self.settling {
            return None;
        }
        let at = self.cells.iter().position(|cell| {
            cell.device().is_some_and(|device| device.requester == requester)
        })?;
        // Inventory rows are append-only; publish pending rows in reservation order.
        if !self.cells[at].ready() && self.cells[..at].iter().any(|cell| !cell.ready()) {
            return None;
        }
        let cell = &mut self.cells[at];
        let Cell::Enrolling(enrollment) = cell else {
            return Some(Ok(()));
        };
        let league = self.leagues.iter_mut()
            .find(|league| league.class == enrollment.entry.class)?;
        let result = (|| {
            let coalition = league.advance(effects)?;
            let operation = (1u64 << 32) + at as u64 * 2;
            loop {
                enrollment.step = match enrollment.step {
                    EnrollmentStep::Door => {
                        enrollment.entry.door = effects.door()?;
                        EnrollmentStep::Part
                    }
                    EnrollmentStep::Part => EnrollmentStep::Bind(
                        effects.part(&enrollment.entry.class, OperationId(operation))?
                    ),
                    EnrollmentStep::Bind(group) => {
                        effects.bind(group, &enrollment.entry, coalition, OperationId(operation + 1))?;
                        return Ok(enrollment.entry.clone());
                    }
                };
            }
        })();
        let origin = enrollment.origin;
        Some(result.map(|entry| *cell = Cell::Vacant { entry, origin }))
    }
}
