use crate::FlatReference;

#[derive(Clone, Copy)]
pub struct ReferenceState {
    pub handle: FlatReference,
    pub offset: usize,
    pub count: usize,
    pub released: bool,
    pub unsealed: bool,
}
