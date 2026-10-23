use super::Instance;

pub struct Control;

impl Control {
    pub fn done(&self) -> bool {
        false
    }

    pub fn instances(&self) -> impl Iterator<Item = &Instance> {
        core::iter::empty()
    }
}
