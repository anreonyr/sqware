use crate::{FlatReference, Mark, Permission, PieToken};

#[derive(Clone, Copy)]
pub struct Unseal {
    pub reference: FlatReference,
    pub shared: bool,
    pub token: PieToken,
    pub permission: Permission,
    pub mark: Mark,
    pub offset: usize,
    pub count: usize,
}
