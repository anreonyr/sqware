use env::PieToken;
use system_api::identity::{CoalitionId, FoundId};
use system_api::operator::{Entry as DirectoryEntry, OperationId};

use super::Entry;

pub trait EnrollmentEffects {
    type Error;

    fn found(&mut self, id: FoundId) -> Result<CoalitionId, Self::Error>;
    fn admit(&mut self, coalition: CoalitionId) -> Result<(), Self::Error>;
    fn activate(&mut self, coalition: CoalitionId) -> Result<(), Self::Error>;
    fn door(&mut self) -> Result<PieToken, Self::Error>;
    fn part(&mut self, class: &str, id: OperationId) -> Result<DirectoryEntry, Self::Error>;
    fn bind(
        &mut self,
        group: DirectoryEntry,
        entry: &Entry,
        coalition: CoalitionId,
        id: OperationId,
    ) -> Result<(), Self::Error>;
}
