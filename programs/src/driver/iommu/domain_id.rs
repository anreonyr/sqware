//! 本引擎签发的私有 Domain 身份：实际引擎任务号 ＋ 本引擎单调铸出的键。
//!
//! 键、PSCID 与 Hub 的绑定号是三件不同的事实：键只在本引擎的 `Vec<Domain>` 里解析，
//! PSCID 只用于硬件缓存标签，绑定号只证明 Hub 那一侧的协调。

use env::TaskId;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct DomainId {
    engine: TaskId,
    key: u64,
}

impl DomainId {
    pub const fn new(engine: TaskId, key: u64) -> Self {
        Self { engine, key }
    }

    pub const fn engine(self) -> TaskId {
        self.engine
    }

    pub const fn key(self) -> u64 {
        self.key
    }
}
