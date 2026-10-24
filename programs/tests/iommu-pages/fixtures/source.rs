use crate::{PieKind, Permission, TaskId};

#[derive(Clone)]
pub struct Source {
    pub token: usize,
    pub pages: usize,
    pub vestor: TaskId,
    pub mark: crate::Mark,
    pub permission: Permission,
    pub alive: bool,
    pub reference: Option<usize>,
    pub physical: u64,
}

impl Source {
    pub fn new(
        token: usize,
        pages: usize,
        vestor: TaskId,
        mark: crate::Mark,
        physical: u64,
    ) -> Self {
        Self {
            token,
            pages,
            vestor,
            mark,
            permission: Permission::FETCH | Permission::STORE,
            alive: true,
            reference: None,
            physical,
        }
    }

    pub fn bytes(&self) -> usize {
        self.pages * crate::PAGE_SIZE
    }

    pub fn kind(&self) -> PieKind {
        PieKind::Pole
    }
}
