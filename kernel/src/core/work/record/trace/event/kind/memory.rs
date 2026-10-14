use serde::Serialize;

use crate::core::memory::nest::FaultKind;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryEvent {
    PageFault {
        va: usize,
        fault: FaultKind,
        resolved: bool,
    },
}
