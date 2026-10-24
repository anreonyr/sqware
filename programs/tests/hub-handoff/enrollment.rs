use env::{PieToken, TaskId};
use system_api::identity::{CoalitionId, FoundId};
use system_api::operator::{Entry as DirectoryEntry, EntryId, EntryTarget, OperationId};
use crate::core::{EnrollmentEffects, Entry};

pub struct Enrollment;

impl EnrollmentEffects for Enrollment {
    type Error = ();

    fn found(&mut self, _: FoundId) -> Result<CoalitionId, ()> {
        Ok(CoalitionId::new(TaskId::new(4), 1))
    }
    fn admit(&mut self, _: CoalitionId) -> Result<(), ()> { Ok(()) }
    fn activate(&mut self, _: CoalitionId) -> Result<(), ()> { Ok(()) }
    fn door(&mut self) -> Result<PieToken, ()> { Ok(PieToken::mint(100)) }
    fn part(&mut self, _: &str, _: OperationId) -> Result<DirectoryEntry, ()> {
        Ok(DirectoryEntry {
            operator: TaskId::new(5),
            target: EntryTarget::Node(EntryId::new(1)),
        })
    }
    fn bind(&mut self, _: DirectoryEntry, _: &Entry, _: CoalitionId, _: OperationId)
        -> Result<(), ()>
    {
        Ok(())
    }
}
