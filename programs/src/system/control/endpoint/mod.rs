//! Control endpoint registration and request adaptation.
mod instance;
pub(crate) mod construction;
pub(super) mod request;
pub(crate) use instance::{answer as answer_instances, receive as receive_instances};

use env::PieToken;
use system_api::control as ccall;
pub(crate) struct Entries {
    instance: Option<PieToken>,
    faces: [Option<PieToken>; ccall::Grant::ALL.len()],
}
impl Entries {
    pub(crate) fn new() -> Self {
        Self {
            instance: None,
            faces: [None; ccall::Grant::ALL.len()],
        }
    }
    pub(crate) fn attach_face(&mut self, grant: ccall::Grant, face: PieToken) {
        self.faces[grant.index()] = Some(face);
    }
    pub(crate) fn attach_instance(&mut self, entry: PieToken) {
        self.instance = Some(entry);
    }
    pub(crate) fn entries(&self) -> impl Iterator<Item = PieToken> + '_ {
        self.faces.iter().flatten().copied().chain(self.instance)
    }
    pub(crate) fn face(&self, grant: ccall::Grant) -> Option<PieToken> {
        self.faces[grant.index()]
    }
    pub(crate) fn instance(&self) -> Option<PieToken> {
        self.instance
    }
}
