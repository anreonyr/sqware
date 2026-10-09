use super::value::Handle;
use alloc::vec::Vec;
#[derive(Clone)]
pub(crate) struct Environment {
    pub parent: Option<Handle>,
    pub cells: Vec<Handle>,
}
