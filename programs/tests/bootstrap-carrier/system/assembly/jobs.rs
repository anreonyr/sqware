use super::{Launch, Reference};

pub struct Jobs {
    pub active: Option<Reference>,
    pub launches: Vec<Launch>,
}

impl Jobs {
    pub fn active_hook_reference(&self) -> Option<Reference> {
        self.active
    }
}
