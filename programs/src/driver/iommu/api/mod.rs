//! IOMMU 驱动那一扇门的协议：建立 Domain、取得私有 IOVA 映射、发布、撤销、关队列与解除。
//!
//! 每一手都是独立效果，不合并。报文只带本引擎签发的 `DomainId` 与 Hub 的完整绑定号；
//! 不接收可信 caller：发送者由内核给出，域主人在门外按实际发送者核。

#[path = "../marks.rs"]
mod marks;

mod acquire;
mod call;
mod establish;
mod fail;
mod said;
mod step;
mod wire;

pub use acquire::Acquire;
pub use call::Call;
pub use establish::Establish;
pub use fail::{Fail, code_to_fail, fail_to_code};
pub use said::Said;
pub use step::Step;
pub use wire::Wire;

pub const ENTRY: env::Mark = marks::ENTRY;
pub const ENTRY_NAME: &str = "entry";
pub const ENTRY_PATH: &str = "iommu/entry";

/// 六个动作在报文里的码：1..5 是既有五步，6 是建立持久 Domain。
pub const ACQUIRE: u8 = 1;
pub const PUBLISH: u8 = 2;
pub const INVALIDATE: u8 = 3;
pub const QUIESCE: u8 = 4;
pub const RELEASE: u8 = 5;
pub const ESTABLISH: u8 = 6;

/// 本台自己的失败方那一格（Hub 那本账据此记原始失败）。
pub const PARTY: u8 = hub_api::PARTY_IOMMU;

/// 回信孔那一枚上的记号：这一面自己的。
pub const BACK: env::Mark = env::Mark::of("iommu-back");
