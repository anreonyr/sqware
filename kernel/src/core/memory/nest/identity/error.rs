use fack::prelude::Error;

use super::Asid;

#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
#[error("remote sfence failed for asid {}", asid.get())]
pub struct Deaf {
    pub asid: Asid,
}
