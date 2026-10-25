#![no_std]

extern crate alloc;

pub mod coord;
pub mod coordination;
pub mod declaration;
pub mod device_id;
pub mod frame;
pub mod grant;
pub mod marks;

pub use declaration::{
    CHANNEL, CHANNEL_MARK, E_HUB, INVENTORY_MARK, READY, READY_MARK,
};

pub use coord::{
    binding_mark, coord_mark, window_mark,
    LAUNCH_MARK,
};
pub use frame::{code_to_fail, fail_to_code};
pub use device_id::DeviceId;

pub use frame::{
    ALIVE_MARK, BACK_MARK, BAD, BOND, BOOT, CLAIM, DEAD, DENIED, DTB, Deed, Fail, INVENTORY_MAX,
    Inventory, InventoryItem, LIST, LIST_MAX, OK, SERVICE_PATH, SUPERVISOR_EXTERNAL, Said, TAKEN,
    UNKNOWN, Window, Wire, inventory_slot,
};
pub use frame::{
    BindingId, BindingState, Close, CloseBinding, DmaCall, DmaClaim, DmaSaid,
    DmaWire, Enroll, Failure, Inspect, Settle, DMA_BACK, DMA_CLAIM, DMA_CLOSE, DMA_ENROLL, DMA_INSPECT, DMA_SETTLE, PARTY_CONTROL, PARTY_HUB, PARTY_IOMMU, PARTY_PCI,
    PARTY_RECOVERY,
};
pub use grant::{Grant, grant_of};
pub use interface::{INTERFACE_ID, PUBLICATIONS, REGISTRY};
pub const NAME: &str = "hub";

#[mold::interface(id = "sqware.service.hub.v1", metadata)]
mod interface {
    #[channels]
    pub enum Channel {
        #[channel(key = "back", legacy = "hub-back", constant = BACK_MARK)]
        Back,
        #[channel(key = "alive", legacy = "hub-alive", constant = ALIVE_MARK)]
        Alive,
    }

    #[grants]
    pub enum Grant {
        #[grant(
            code = 1,
            key = "bond",
            legacy = "hub-entry-bond",
            publication = "bond"
        )]
        Bond,
        #[grant(
            code = 2,
            key = "list",
            legacy = "hub-entry-list",
            publication = "list"
        )]
        List,
        #[grant(
            code = 3,
            key = "claim",
            legacy = "hub-entry-claim",
            publication = "claim"
        )]
        Claim,
        /// DMA 安全绑定那一面：发现、认领、关闭、查看与整机结清共用一扇门
        /// （一面一原语那一族是普通设备账；DMA 那一族的协调进度是一个整体，
        /// 故不拆成五扇）。
        #[grant(
            code = 4,
            key = "dma",
            legacy = "hub-entry-dma",
            publication = "dma"
        )]
        Dma,
        /// Generic retirement face: the provider alone maps its own responsibility to Done.
        #[grant(
            code = 5,
            key = "retire",
            legacy = "hub-entry-retire",
            publication = "retire"
        )]
        Retire,
    }
}
