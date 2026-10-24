//! DMA 绑定那一面的坐标与 Hub 那一面的客户。
//!
//! 绑定编号由 Hub 签发；坐标本身（窗口记号、坐标孔记号、IOVA 窗起址）定义在
//! `hub_api::coord`（那一本账的契约里）；本模块只把它转出来并给出客户面。
//! 认领不在这里：它走**设备门牌**（见 `hub_api::frame::dma`），本面只管
//! 登记（PCI）、关闭/查看/结清与放行时推坐标。

use env::{PieToken, TaskId, Wait};
use hub_api::{
    BindingState, Close, DmaCall, DmaSaid, DmaWire, Enroll, Failure, DMA_BACK,
    DMA_CLOSE, DMA_ENROLL, DMA_INSPECT, DMA_SETTLE,
};
use ipc::rpc::request::Sender;
use ipc::session::establish;
use ipc::time::Deadline;
use system_api::operator::Path;
use system_client::operator::Face;

/// 坐标由 Hub 那一本账的契约给（各域按同一个绑定号算出同一组数）。
pub use hub_api::{
    binding_mark, coord_mark, iova_base, window_mark, BINDING_MAX, IOVA_BASE, IOVA_LIMIT, IOVA_SPAN,
};

pub use super::coords::Coords;

/// Hub 那一扇 DMA 面的客户：每一趟自己借回信孔，失败码原样过线。
pub struct HubDma {
    sender: Sender<DmaCall>,
    host: TaskId,
}

impl HubDma {
    /// 树上找那一面（`/svc/hub/dma`）。
    pub fn find(tree: &Face, wait: Wait) -> Result<Self, &'static str> {
        let service = tree
            .resolve(system_api::namespace::SERVICE, wait)
            .map_err(|_| "hub: service namespace")?;
        let road = Path::new("hub")
            .try_join("dma")
            .ok_or("hub: dma road")?;
        let token = tree
            .tile_from(service, &road, wait)
            .and_then(|tile| tile.token(wait))
            .map_err(|_| "hub: dma face")?;
        Self::open(token)
    }

    /// 把一枚门牌收成一面（开者必须是内核给出的那一台）。
    pub fn open(entry: PieToken) -> Result<Self, &'static str> {
        let host = establish::opened_by(entry).ok_or("hub: dma host")?;
        let sender = Sender::<DmaCall>::from_raw(entry, DMA_BACK).map_err(|_| "hub: dma face")?;
        Ok(Self { sender, host })
    }

    pub fn host(&self) -> TaskId {
        self.host
    }

    fn call(
        &self,
        wait: Wait,
        build: impl FnOnce(PieToken) -> DmaWire,
    ) -> Result<DmaSaid, &'static str> {
        let said = self
            .sender
            .call(Deadline::new(wait), build)
            .map_err(|_| "hub: dma call")?;
        if said.status != wire::OK {
            return Err("hub: dma refused");
        }
        Ok(said)
    }

    /// Register a device key under this Driver's authenticated sender identity.
    pub fn enroll(
        &self,
        wait: Wait,
        key: u64,
        line: u32,
        name: &str,
        class: &str,
    ) -> Result<(), &'static str> {
        self.call(wait, |back| {
            DmaWire::Enroll(Enroll {
                op: DMA_ENROLL,
                key,
                line,
                name: name.into(),
                class: class.into(),
                back,
            })
        })?;
        Ok(())
    }

    /// 关闭本次绑定：重复请求继续同一责任，`Fault` 可覆盖 `Normal`。
    pub fn close(&self, wait: Wait, serial: u64, reason: Close) -> Result<(), &'static str> {
        self.call(wait, |back| {
            DmaWire::Close(hub_api::CloseBinding {
                op: DMA_CLOSE,
                serial,
                reason: reason.code(),
                back,
            })
        })?;
        Ok(())
    }

    /// 查看同一绑定的协调进度或原始失败。
    pub fn inspect(
        &self,
        wait: Wait,
        serial: u64,
    ) -> Result<(BindingState, Option<Failure>), &'static str> {
        let said = self.call(wait, |back| {
            DmaWire::Inspect(hub_api::Inspect {
                op: DMA_INSPECT,
                serial,
                back,
            })
        })?;
        let state = BindingState::of_code(said.state).ok_or("hub: binding state")?;
        Ok((state, said.failure()))
    }

    /// 整机退场门：关闭执行实例与开放绑定，答未结清责任数。
    pub fn settle(&self, wait: Wait) -> Result<u8, &'static str> {
        let said = self.call(wait, |back| {
            DmaWire::Settle(hub_api::Settle {
                op: DMA_SETTLE,
                back,
            })
        })?;
        Ok(said.count)
    }
}
