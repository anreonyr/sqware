use env::{PieToken, TaskId};

pub struct AttachRequest {
    pub serial: u64,
    pub execution: TaskId,
    pub bar: PieToken,
    pub pages: u32,
    pub common: u32,
    pub notify: u32,
    pub multiplier: u32,
    pub device: u32,
}
