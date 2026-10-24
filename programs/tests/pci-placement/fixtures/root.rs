use crate::{Mark, Permission, PieToken};

pub(crate) struct Root {
    pub token: PieToken,
    pub offset: usize,
    pub count: usize,
    pub permission: Permission,
    pub mark: Mark,
}
