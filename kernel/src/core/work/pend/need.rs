use env::Permission;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Need {
    Fetch,
    Store,
    Vest,
}

impl Need {
    pub(crate) fn allowed_by(self, permission: Permission) -> bool {
        permission.contains(match self {
            Self::Fetch => Permission::FETCH,
            Self::Store => Permission::STORE,
            Self::Vest => Permission::VEST,
        })
    }
}
