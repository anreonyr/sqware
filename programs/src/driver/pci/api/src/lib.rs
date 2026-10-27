#![no_std]

//! PCI 驱动那一扇门的协议：落位、开启与清除 MASTER。
//!
//! 落位与 MASTER 是两手：落位只开内存空间（配置不授权发起），开启只在协调入口
//! （设备 Driver）核过全部前置之后发生。可信调用者由内核发送者身份确定。

mod call;
mod fail;
mod place;
mod said;
mod toggle;
mod wire;

pub use call::Call;
pub use fail::{
    BAD, CONFLICT, DENIED, Fail, UNAVAILABLE, UNKNOWN, code_to_fail, fail_to_code,
};
pub use place::Place;
pub use said::Said;
pub use toggle::Toggle;
pub use wire::Wire;

/// 本台那一扇门在树上与服务名下的坐标。
pub use pci_declaration::marks::ENTRY;
pub const ENTRY_NAME: &str = "entry";
pub const ENTRY_PATH: &str = "pci/entry";

/// 三个动作在报文里的码。
pub const PLACE: u8 = 1;
pub const ENABLE: u8 = 2;
pub const DISABLE: u8 = 3;
pub const RELEASE: u8 = 4;
pub const BAR_MARK: env::Mark = env::Mark::of("pci/recovery-bar");

/// 回信孔那一枚上的记号：这一面自己的。
pub const BACK: env::Mark = env::Mark::of("pci-back");
