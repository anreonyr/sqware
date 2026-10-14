use alloc::sync::Weak;
use env::TaskId;

use crate::core::work::unit::life::Life;

pub(crate) struct TaskLife {
    pub(crate) id: TaskId,
    pub(crate) life: Weak<Life>,
}
