use env::{MailCondition, Permission};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Need {
    Fetch,
    Store,
    Vest,
}

impl Need {
    pub(crate) fn of(condition: MailCondition) -> Need {
        match condition {
            MailCondition::Pull | MailCondition::Signal(_) => Need::Fetch,
            MailCondition::Push | MailCondition::Empty => Need::Store,
        }
    }

    pub(crate) fn allowed_by(self, permission: Permission) -> bool {
        permission.contains(match self {
            Self::Fetch => Permission::FETCH,
            Self::Store => Permission::STORE,
            Self::Vest => Permission::VEST,
        })
    }
}
