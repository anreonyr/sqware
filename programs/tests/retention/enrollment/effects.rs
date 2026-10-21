use env::{PieToken, TaskId};
use system_api::identity::{CoalitionId, FoundId};
use system_api::operator::{Entry as DirectoryEntry, EntryId, EntryTarget, OperationId};
use crate::hub_core::{EnrollmentEffects, Entry};
use super::{boundary::Boundary, failure::Failure};

#[derive(Default)]
pub struct Effects {
    pub failure: Option<Failure>,
    pub calls: Vec<Boundary>,
    pub coalitions: Vec<(FoundId, CoalitionId)>,
    pub admitted: Vec<CoalitionId>,
    pub activated: Vec<CoalitionId>,
    pub doors: Vec<PieToken>,
    pub parts: Vec<(OperationId, String, DirectoryEntry)>,
    pub bindings: Vec<(OperationId, DirectoryEntry, Entry, CoalitionId)>,
}

impl Effects {
    fn fresh_operation(&self, id: OperationId) {
        let high = self.parts.iter().map(|(key, _, _)| key.0)
            .chain(self.bindings.iter().map(|(key, _, _, _)| key.0))
            .max().unwrap_or(0);
        assert!(id.0 > high, "an unseen Operator request must exceed its watermark");
    }

    fn begin(&mut self, boundary: Boundary) -> Result<(), Failure> {
        self.calls.push(boundary);
        if self.failure == Some(Failure::Before(boundary)) {
            return Err(self.failure.take().unwrap());
        }
        Ok(())
    }

    fn reply(&mut self, boundary: Boundary) -> Result<(), Failure> {
        if self.failure == Some(Failure::Reply(boundary)) {
            return Err(self.failure.take().unwrap());
        }
        Ok(())
    }
}

impl EnrollmentEffects for Effects {
    type Error = Failure;

    fn found(&mut self, id: FoundId) -> Result<CoalitionId, Failure> {
        self.begin(Boundary::Found)?;
        let coalition = if let Some((_, coalition)) = self.coalitions.iter().find(|(key, _)| *key == id) {
            *coalition
        } else {
            let coalition = CoalitionId::new(TaskId::new(4), self.coalitions.len() as u64);
            self.coalitions.push((id, coalition));
            coalition
        };
        self.reply(Boundary::Found)?;
        Ok(coalition)
    }

    fn admit(&mut self, coalition: CoalitionId) -> Result<(), Failure> {
        self.begin(Boundary::Admit)?;
        if !self.admitted.contains(&coalition) {
            self.admitted.push(coalition);
        }
        self.reply(Boundary::Admit)
    }

    fn activate(&mut self, coalition: CoalitionId) -> Result<(), Failure> {
        self.begin(Boundary::Activate)?;
        assert!(self.admitted.contains(&coalition));
        if !self.activated.contains(&coalition) {
            self.activated.push(coalition);
        }
        self.reply(Boundary::Activate)
    }

    fn door(&mut self) -> Result<PieToken, Failure> {
        self.begin(Boundary::Door)?;
        let door = PieToken::mint(self.doors.len() + 100);
        self.doors.push(door);
        Ok(door)
    }

    fn part(&mut self, class: &str, id: OperationId) -> Result<DirectoryEntry, Failure> {
        self.begin(Boundary::Part)?;
        let group = if let Some((_, name, group)) = self.parts.iter().find(|(key, _, _)| *key == id) {
            assert_eq!(class, name);
            *group
        } else {
            self.fresh_operation(id);
            let group = DirectoryEntry {
                operator: TaskId::new(5),
                target: EntryTarget::Node(EntryId::new(self.parts.len() + 1)),
            };
            self.parts.push((id, class.into(), group));
            group
        };
        self.reply(Boundary::Part)?;
        Ok(group)
    }

    fn bind(
        &mut self,
        group: DirectoryEntry,
        entry: &Entry,
        coalition: CoalitionId,
        id: OperationId,
    ) -> Result<(), Failure> {
        self.begin(Boundary::Bind)?;
        assert!(self.activated.contains(&coalition));
        assert!(self.doors.contains(&entry.door));
        if let Some((_, previous_group, previous_entry, previous_coalition)) =
            self.bindings.iter().find(|(key, _, _, _)| *key == id)
        {
            assert_eq!(group, *previous_group);
            assert_eq!(entry, previous_entry);
            assert_eq!(coalition, *previous_coalition);
        } else {
            self.fresh_operation(id);
            self.bindings.push((id, group, entry.clone(), coalition));
        }
        self.reply(Boundary::Bind)
    }
}
