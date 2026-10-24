//! 设备恢复执行器：精确 BAR 附着映射、reset/drain 与释放。
//!
//! `attach` 必须在发布之前、且**附着映射已在手里**之后才把精确 BAR 子授权交给执行方；
//! 业务 CPU 控制切断之前本域不写设备寄存器（`stop` 才写）。本域不解释 IOMMU、不碰 ECAM。

use env::{Permission, PieKind, PieToken, TaskId};
use resource::dock::Dock;

use crate::driver::virtio;
use crate::driver::virtio::blk::recovery::api;
use crate::driver::virtio::blk::recovery::attach_request::AttachRequest;
use crate::driver::virtio::blk::recovery::attachment::Attachment;
use crate::driver::shared::dma;

/// 交给执行方那一枚窗口的形态：读写 ＋ 跟源（aperture 那一枚带 `ONLY`），**不给**再授出权。
const BAR_SUBSET: Permission = Permission::FETCH
    .union(Permission::STORE)
    .union(Permission::ONLY);

pub struct Executor {
    attachment: Option<Attachment>,
    mapping: Option<Dock>,
    floor: u64,
    /// PCI 那一台的号：BAR 窗的来源方（内核给出的 `vestor` 要对上它）。
    pci: TaskId,
}

impl Executor {
    pub fn new(pci: TaskId) -> Self {
        Self {
            attachment: None,
            mapping: None,
            floor: 0,
            pci,
        }
    }

    /// 取得附着映射：核本表里那一枚精确 BAR，量到它正好是 `pages` 页，
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
        if let Some(attachment) = self.attachment.as_mut() {
            let mapping = self.mapping.as_ref().ok_or(api::Fail::Unavailable)?;
            if mapping.pie_token() != request.bar
                || attachment.pages != request.pages
                || attachment.caps.parts() != caps.parts()
                || (attachment.serial != 0
                    && (attachment.serial != request.serial || attachment.execution != request.execution))
            {
                return Err(api::Fail::Conflict);
            }
            attachment.serial = request.serial;
            attachment.execution = request.execution;
            return Self::grant(attachment, mapping);
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
        self.attachment = Some(Attachment {
            serial: request.serial,
            execution: request.execution,
            pages: request.pages,
            caps,
            stopped: false,
            reset_pending: false,
            bar_grant: None,
        });
        Self::grant(self.attachment.as_mut().unwrap(), mapping)
    }

    fn grant(attachment: &mut Attachment, mapping: &Dock) -> Result<(), api::Fail> {
        if attachment.bar_grant.is_some() {
            return Ok(());
        }
        // 映射已量到正好 `pages * PAGE_SIZE`；普通 Accord 继承这一段。
        // 附着映射跨绑定保留；每次执行只重交子授权。
        let receipt = env::pie::accord(
            mapping.pie_token(),
            attachment.execution,
            BAR_SUBSET,
            dma::binding_mark(attachment.serial),
        )
        .map_err(|_| api::Fail::Unavailable)?;
        attachment.bar_grant = Some(receipt);
        attachment.stopped = false;
        crate::debug::put(&alloc::format!(
            "virtio-blk: binding {} attached bar={:#x} pages={} execution={}",
            attachment.serial,
            mapping.view().base(),
            attachment.pages,
            attachment.execution.get(),
        ));
        Ok(())
    }

    pub fn bar_grant(&self, serial: u64) -> Result<PieToken, api::Fail> {
        self.attachment.as_ref()
            .filter(|attachment| attachment.serial == serial)
            .and_then(|attachment| attachment.bar_grant)
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
    /// （顺序在协调入口那一侧）。写入与回读各占一拍；`None` 表示等待下一拍。
    pub fn stop(&mut self, serial: u64) -> Result<Option<u8>, api::Fail> {
        if serial <= self.floor {
            return Ok(Some(0));
        }
        if self.attachment.is_none() {
            self.close_mapping()?;
        }
        let Some(attachment) = self.attachment.as_mut() else {
            self.floor = serial;
            return Ok(Some(0));
        };
        if attachment.serial == 0 && attachment.stopped {
            self.floor = serial;
            return Ok(Some(0));
        }
        if attachment.serial != serial {
            return Err(api::Fail::Conflict);
        }
        let base = self.mapping.as_ref().ok_or(api::Fail::Unavailable)?.view().base();
        if attachment.stopped {
            let status = virtio::status(base, &attachment.caps);
            let queue_enable = virtio::queue_enable(base, &attachment.caps);
            if status != 0 || queue_enable != 0 {
                return Err(api::Fail::Unavailable);
            }
            return Ok(Some(status));
        }
        if !attachment.reset_pending {
            virtio::device::reset_begin(base, &attachment.caps);
            attachment.reset_pending = true;
            return Ok(None);
        }
        let status = virtio::status(base, &attachment.caps);
        let queue_enable = virtio::queue_enable(base, &attachment.caps);
        if status != 0 || queue_enable != 0 {
            return Err(api::Fail::Unavailable);
        }
        attachment.reset_pending = false;
        attachment.stopped = true;
        crate::debug::put(&alloc::format!(
            "virtio-blk: binding {serial} stopped reset=1 status={status} queue_enable={queue_enable}",
        ));
        Ok(Some(status))
    }

    /// 停链完成后清除本次绑定；设备附着映射保留到本驱动退场。
    pub fn detach(&mut self, serial: u64) -> Result<(), api::Fail> {
        if serial <= self.floor {
            return Ok(());
        }
        let Some(attachment) = self.attachment.as_mut() else {
            self.close_mapping()?;
            self.floor = serial;
            return Ok(());
        };
        if attachment.serial != serial || !attachment.stopped {
            return Err(api::Fail::Conflict);
        }
        attachment.serial = 0;
        attachment.execution = TaskId::new(0);
        attachment.bar_grant = None;
        self.floor = serial;
        crate::debug::put(&alloc::format!("virtio-blk: binding {serial} detached"));
        Ok(())
    }

    /// 此刻还挂着几笔（读数用）。
    #[cfg(test)]
    pub fn attached(&self) -> usize {
        usize::from(
            self.attachment
                .as_ref()
                .is_some_and(|attachment| attachment.serial != 0),
        )
    }
}
