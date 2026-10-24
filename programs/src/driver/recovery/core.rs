//! 设备恢复那一域的核心：每一号绑定的精确 BAR 恢复映射、reset/drain 与放下。
//!
//! `attach` 必须在发布之前、且**恢复映射已在手里**之后才把精确 BAR 子授权交给执行方；
//! 业务 CPU 控制切断之前本域不写设备寄存器（`stop` 才写）。本域不解释 IOMMU、不碰 ECAM。

use env::{Permission, PieKind, PieToken, TaskId};
use resource::dock::Dock;

use crate::driver::virtio;
use crate::driver::recovery::api;
use crate::driver::recovery::device::Device;
use crate::driver::shared::dma;
pub use super::attach_request::AttachRequest;

/// 交给执行方那一枚窗口的形态：读写 ＋ 跟源（aperture 那一枚带 `ONLY`），**不给**再授出权。
const BAR_SUBSET: Permission = Permission::FETCH
    .union(Permission::STORE)
    .union(Permission::ONLY);

pub struct Core {
    device: Option<Device>,
    mapping: Option<Dock>,
    floor: u64,
    /// PCI 那一台的号：BAR 窗的来源方（内核给出的 `vestor` 要对上它）。
    pci: TaskId,
}

impl Core {
    pub fn new(pci: TaskId) -> Self {
        Self {
            device: None,
            mapping: None,
            floor: 0,
            pci,
        }
    }

    /// 取得恢复映射：核本表里那一枚精确 BAR，量到它正好是 `pages` 页，
    /// 再把子授权交给执行方。同号同参重试复用原记录。
    pub fn attach(&mut self, request: AttachRequest) -> Result<(), api::Fail> {
        if request.serial <= self.floor {
            return Err(api::Fail::Conflict);
        }
        let bytes = (request.pages as usize)
            .checked_mul(env::PAGE_SIZE)
            .filter(|bytes| *bytes != 0)
            .ok_or(api::Fail::Denied)?;
        let caps = virtio::Caps::from_parts(
            request.common, request.notify, request.multiplier, request.device, bytes,
        ).ok_or(api::Fail::Denied)?;
        if let Some(device) = self.device.as_mut() {
            let mapping = self.mapping.as_ref().ok_or(api::Fail::Unavailable)?;
            if mapping.pie_token() != request.bar
                || device.pages != request.pages
                || device.caps.parts() != caps.parts()
                || (device.serial != 0
                    && (device.serial != request.serial || device.execution != request.execution))
            {
                return Err(api::Fail::Conflict);
            }
            device.serial = request.serial;
            device.execution = request.execution;
            return Self::grant(device, mapping);
        }
        self.close_mapping()?;
        let info = resource::raw::inspect(request.bar).map_err(|_| api::Fail::Denied)?;
        if !info.alive
            || info.kind != PieKind::Pole
            || info.mark != crate::driver::pci::api::BAR_MARK
            || info.permission != BAR_SUBSET.union(Permission::VEST)
            || info.vestor != self.pci
        {
            return Err(api::Fail::Denied);
        }
        self.mapping = Some(Dock::open(request.bar).map_err(|_| api::Fail::Denied)?);
        let mapping = self.mapping.as_ref().ok_or(api::Fail::Unavailable)?;
        if mapping.view().size() != bytes {
            self.close_mapping()?;
            return Err(api::Fail::Denied);
        }
        self.device = Some(Device {
            serial: request.serial,
            execution: request.execution,
            pages: request.pages,
            caps,
            stopped: false,
            bar_grant: None,
        });
        Self::grant(self.device.as_mut().unwrap(), mapping)
    }

    fn grant(device: &mut Device, mapping: &Dock) -> Result<(), api::Fail> {
        if device.bar_grant.is_some() {
            return Ok(());
        }
        // 映射已量到正好 `pages * PAGE_SIZE`；普通 Accord 继承这一段。
        // 恢复映射跨绑定保留；每次执行只重交子授权。
        let receipt = env::pie::accord(
            mapping.pie_token(),
            device.execution,
            BAR_SUBSET,
            dma::binding_mark(device.serial),
        )
        .map_err(|_| api::Fail::Unavailable)?;
        device.bar_grant = Some(receipt);
        device.stopped = false;
        crate::debug::put(&alloc::format!(
            "recovery: binding {} attached bar={:#x} pages={} execution={}",
            device.serial,
            mapping.view().base(),
            device.pages,
            device.execution.get(),
        ));
        Ok(())
    }

    pub fn bar_grant(&self, serial: u64) -> Result<PieToken, api::Fail> {
        self.device.as_ref()
            .filter(|device| device.serial == serial)
            .and_then(|device| device.bar_grant)
            .ok_or(api::Fail::Conflict)
    }

    fn close_mapping(&mut self) -> Result<(), api::Fail> {
        if let Some(mapping) = self.mapping.as_ref() {
            env::pole::shut(mapping.pie_token()).map_err(|_| api::Fail::Unavailable)?;
            self.mapping = None;
        }
        Ok(())
    }

    /// 本设备 reset/drain 并回读：只在业务 CPU 控制已切断、翻译已撤、MASTER 已清之后做
    /// （顺序在协调入口那一侧）。返此刻的状态字节。
    pub fn stop(&mut self, serial: u64) -> Result<u8, api::Fail> {
        if serial <= self.floor {
            return Ok(0);
        }
        if self.device.is_none() {
            self.close_mapping()?;
        }
        let Some(device) = self.device.as_mut() else {
            self.floor = serial;
            return Ok(0);
        };
        if device.serial == 0 && device.stopped {
            self.floor = serial;
            return Ok(0);
        }
        if device.serial != serial {
            return Err(api::Fail::Conflict);
        }
        let base = self.mapping.as_ref().ok_or(api::Fail::Unavailable)?.view().base();
        if device.stopped {
            return Ok(virtio::status(base, &device.caps));
        }
        if !virtio::reset(base, &device.caps) {
            return Err(api::Fail::Unavailable);
        }
        device.stopped = true;
        let status = virtio::status(base, &device.caps);
        crate::debug::put(&alloc::format!(
            "recovery: binding {serial} stopped reset=1 status={status} queue_enable={}",
            virtio::queue_enable(base, &device.caps),
        ));
        Ok(status)
    }

    /// 停链完成后清除本次绑定；设备恢复映射保留到本驱动退场。
    pub fn detach(&mut self, serial: u64) -> Result<(), api::Fail> {
        if serial <= self.floor {
            return Ok(());
        }
        if self.device.is_none() {
            self.close_mapping()?;
        }
        if let Some(device) = self.device.as_mut() {
            if device.serial != serial || !device.stopped {
                return Err(api::Fail::Conflict);
            }
            device.serial = 0;
            device.execution = TaskId::new(0);
            device.bar_grant = None;
        }
        self.floor = serial;
        crate::debug::put(&alloc::format!("recovery: binding {serial} detached"));
        Ok(())
    }

    /// 此刻还挂着几笔（读数用）。
    pub fn attached(&self) -> usize {
        usize::from(
            self.device
                .as_ref()
                .is_some_and(|device| device.serial != 0),
        )
    }
}
