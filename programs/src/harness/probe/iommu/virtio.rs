//! virtio-pci 1.x 那面：配置空间里的能力表 → 四个区（common / notify / ISR / device）
//! 与一个 virtqueue 的建立与门铃。
//!
//! 走 1.x 那一套（能力表 + 64 位队列地址）有两个好处：队列与描述符的地址由**本域挑**，
//! 于是“设备看到的是 IOMMU 那个空间、不是物理空间”这件事在代码里看得出来；
//! 而且不存在传统 I/O BAR 那一格（本机 PCI 的 I/O 窗口没有门闩）。
//!
//! 本层只到“队列立起来、能打响门铃”；块请求（头/数据/状态）在 `dma` 那一层。
//!
//! 两段用法各有它的一侧：**驱动那一侧**（`Virtio`）自己发现能力表、把队列立起来；
//! **发布那一侧**（`ring`）不建驱动——队列与描述符由别人布好，它只做“推一下”那件事。

use super::ecam::{self, Ecam};
use super::order;

/// 那一张能力表里属 virtio 的四格：BAR 内偏移与通知乘数（**不含 BAR 号**——
/// 那是发现那一侧的事，`read` 把它一并交出来）。
///
/// QEMU 把四格都放在同一个 BAR 里（实测 `virtio-blk-pci` 是 BAR4，0/0x1000/0x2000/0x3000）。
/// 两个入口：`read` 从配置空间发现；`from_parts` 从**已经发现过的**那一组数（跨主体交接时，
/// 发现方把坐标交给持有 BAR 的那一方）。
#[derive(Clone, Copy)]
pub struct Caps {
    common: u32,
    notify: u32,
    multiplier: u32,
    device: u32,
}

impl Caps {
    /// 把“别处已经发现过的那四个数”收进来（本域不重发现）。
    ///
    /// `span` = 拿到的那一段有多大：三个区都必须在里面，且**各占一处**（同一段 BAR 里三个结构
    /// 挤在同一偏移上不是本层认的那张能力表）。偏移本身可以是 0（`common` 实测就在 BAR 头上）。
    pub const fn from_parts(
        common: u32,
        notify: u32,
        multiplier: u32,
        device: u32,
        span: usize,
    ) -> Option<Self> {
        if common as usize >= span
            || notify as usize >= span
            || device as usize >= span
            || common == notify
            || notify == device
            || common == device
        {
            return None;
        }
        Some(Self {
            common,
            notify,
            multiplier,
            device,
        })
    }

    /// 那四个数（发现方把它们交出去时用：接收方不重发现，只收下这一组坐标）。
    pub const fn parts(&self) -> (u32, u32, u32, u32) {
        (self.common, self.notify, self.multiplier, self.device)
    }

    /// 从配置空间的能力链表上把那四格读出来（连同它们所在那一根 BAR 的号）。
    ///
    /// 链表从 `CAPABILITY`（0x34）出发，`cap_next` 串联；只认 `cap_vndr = 0x09` 且
    /// `cap_len ≥ 16` 的项，`cfg_type` 1/2/4 各归各位。缺一不可（本层只会打开一个窗口）。
    /// 跳数有界，链里绕圈也出得来。
    pub fn read(ecam: &Ecam) -> Result<(u8, Self), &'static str> {
        let mut at = ecam.read8(ecam::CAPABILITY) & 0xFC;
        let mut common = None;
        let mut notify = None;
        let mut device = None;
        let mut bar = None;
        let mut multiplier = 0;
        for _ in 0..CAP_STEPS {
            if at == 0 {
                break;
            }
            let vendor = ecam.read8(at as u16);
            let next = ecam.read8(at as u16 + 1) & 0xFC;
            let length = ecam.read8(at as u16 + 2);
            if vendor == ecam::CAP_VENDOR && length >= CAP_BYTES {
                let kind = ecam.read8(at as u16 + 3);
                let owner = ecam.read8(at as u16 + 4);
                let offset = ecam.read32(at as u16 + 8);
                let span = ecam.read32(at as u16 + 12);
                if span != 0 {
                    bar = Some(owner);
                    match kind {
                        CAP_COMMON => common = Some(offset),
                        CAP_DEVICE => device = Some(offset),
                        CAP_NOTIFY => {
                            notify = Some(offset);
                            if length >= CAP_NOTIFY_BYTES {
                                multiplier = ecam.read32(at as u16 + 16);
                            }
                        }
                        _ => {}
                    }
                }
            }
            at = next;
        }
        match (bar, common, notify, device) {
            (Some(bar), Some(common), Some(notify), Some(device)) => Ok((
                bar,
                Self {
                    common,
                    notify,
                    multiplier,
                    device,
                },
            )),
            _ => Err("iommu-dma: virtio caps"),
        }
    }
}

/// 一个已经上电、队列已立起来的 virtio-pci 设备面。
///
/// 它就把 BAR 基址与那一组坐标拿着；寄存器的算法在下面几个 `read`/`write` 与
/// [`ring`] 里各只有一处。
pub struct Virtio {
    bar: usize,
    caps: Caps,
}

impl Virtio {
    /// 上电初始化（规范 3.1.1 那一段）：复位 → ACKNOWLEDGE → DRIVER →
    /// 接 `VIRTIO_F_VERSION_1` 与 `VIRTIO_F_ACCESS_PLATFORM` → FEATURES_OK 并回读确认。
    ///
    /// `ACCESS_PLATFORM`（即这一格在设备那一侧的别名 IOMMU_PLATFORM）：设备在**平台**的地址空间里，
    /// 驱动得自己把要发布的地址映好——本景就是手写 IOMMU 那几张表，故这一位我们接下；
    /// 不接就等于说“设备能直接访客内存”，而这里不是。
    /// 只认这两位：别的位不哄它（设备不提供 1.x 也当场拒）。
    pub fn open(bar: usize, caps: &Caps) -> Result<Self, &'static str> {
        let mut virtio = Self { bar, caps: *caps };
        virtio.write8(STATUS, 0);
        virtio.write8(STATUS, STATUS_ACKNOWLEDGE);
        virtio.write8(STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER);
        virtio.write32(FEATURE_SELECT_DEVICE, 1);
        let offered = virtio.read32(FEATURE_DEVICE);
        if offered & FEATURE_VERSION_1 == 0 {
            return Err("iommu-dma: virtio version 1");
        }
        virtio.write32(FEATURE_SELECT_DRIVER, 0);
        virtio.write32(FEATURE_DRIVER, 0);
        virtio.write32(FEATURE_SELECT_DRIVER, 1);
        virtio.write32(
            FEATURE_DRIVER,
            offered & (FEATURE_VERSION_1 | FEATURE_ACCESS_PLATFORM),
        );
        virtio.write8(
            STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK,
        );
        if virtio.read8(STATUS) & STATUS_FEATURES_OK == 0 {
            return Err("iommu-dma: virtio features rejected");
        }
        // `device` 那一格的偏移本景不用（本驱动不读设备配置），但发现它必须成功：
        // 四格缺一就说明这不是本层认的那张能力表。
        if caps.device == 0 {
            return Err("iommu-dma: virtio device region");
        }
        Ok(virtio)
    }

    /// 立第 0 号队列：尺寸先回读再写回（不改设备的取值）、描述符/可用环/已用环三段
    /// 用**调用方挑的那三个地址**（设备真的会照这三段去取），最后 DRIVER_OK。
    ///
    /// 返设备给出的队列长度（描述符表能放几项）——“本域那一页装得下吗”是**布环那一侧**的
    /// 判据（它才知道自己那页有多大），故这一层收下设备给的数、不替它判。
    pub fn setup_queue(&mut self, desc: u64, avail: u64, used: u64) -> Result<u16, &'static str> {
        self.write16(QUEUE_SELECT, 0);
        let size = self.read16(QUEUE_SIZE);
        if size == 0 || !size.is_power_of_two() {
            return Err("iommu-dma: virtio queue size");
        }
        self.write16(QUEUE_SIZE, size);
        self.write16(QUEUE_MSIX, NO_VECTOR);
        self.write32(QUEUE_DESC_LOW, desc as u32);
        self.write32(QUEUE_DESC_HIGH, (desc >> 32) as u32);
        self.write32(QUEUE_AVAIL_LOW, avail as u32);
        self.write32(QUEUE_AVAIL_HIGH, (avail >> 32) as u32);
        self.write32(QUEUE_USED_LOW, used as u32);
        self.write32(QUEUE_USED_HIGH, (used >> 32) as u32);
        self.write16(QUEUE_ENABLE, 1);
        self.write8(
            STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        );
        if self.read16(QUEUE_ENABLE) != 1 {
            return Err("iommu-dma: virtio queue did not come up");
        }
        Ok(size)
    }

    /// 设备此刻的状态字节（出口留一行现场用）。
    pub fn status(&self) -> u8 {
        self.read8(STATUS)
    }

    /// 推一下这一台自己的第 0 号队列（块设备就一条队列）。
    pub fn notify(&self) {
        ring(self.bar, &self.caps, 0);
    }

    fn read8(&self, register: u16) -> u8 {
        // SAFETY: 公共区在本域已映射的那一段 BAR 里，偏移取自规范的表。
        unsafe {
            core::ptr::read_volatile(
                (self.bar + self.caps.common as usize + register as usize) as *const u8,
            )
        }
    }

    fn read16(&self, register: u16) -> u16 {
        // SAFETY: 同 `read8`。
        unsafe {
            core::ptr::read_volatile(
                (self.bar + self.caps.common as usize + register as usize) as *const u16,
            )
        }
    }

    fn read32(&self, register: u16) -> u32 {
        // SAFETY: 同 `read8`。
        unsafe {
            core::ptr::read_volatile(
                (self.bar + self.caps.common as usize + register as usize) as *const u32,
            )
        }
    }

    fn write8(&mut self, register: u16, value: u8) {
        // SAFETY: 同 `read8`。
        unsafe {
            core::ptr::write_volatile(
                (self.bar + self.caps.common as usize + register as usize) as *mut u8,
                value,
            )
        }
    }

    fn write16(&mut self, register: u16, value: u16) {
        // SAFETY: 同 `read8`。
        unsafe {
            core::ptr::write_volatile(
                (self.bar + self.caps.common as usize + register as usize) as *mut u16,
                value,
            )
        }
    }

    fn write32(&mut self, register: u16, value: u32) {
        // SAFETY: 同 `read8`。
        unsafe {
            core::ptr::write_volatile(
                (self.bar + self.caps.common as usize + register as usize) as *mut u32,
                value,
            )
        }
    }
}

/// 队列的通知偏移（只读那一格：设备给的，与谁持有 BAR 无关）。
pub fn notify_offset(bar: usize, caps: &Caps) -> u16 {
    // SAFETY: 公共区在本域已映射的那一段 BAR 里。
    unsafe {
        core::ptr::read_volatile(
            (bar + caps.common as usize + QUEUE_NOTIFY_OFF as usize) as *const u16,
        )
    }
}

/// 打响门铃：通知区基址 ＋ `queue_notify_off × 乘数`，写下去的值是队列号。
///
/// 发布那一侧（点完描述符的那一方）只要这一格就够了——队列由别人布好也行。
/// 前后各一枚栅栏：没有它，设备可能看到半截描述符（QEMU 下碰不上，硬件上会）。
pub fn ring(bar: usize, caps: &Caps, queue: u16) {
    order::sync();
    let at =
        bar + caps.notify as usize + notify_offset(bar, caps) as usize * caps.multiplier as usize;
    // SAFETY: 通知区在本域已映射的那一段 BAR 里，门铃就是这一个 16 位写。
    unsafe { core::ptr::write_volatile(at as *mut u16, queue) };
    order::sync();
}

/// 能力链表最多追几项（链里绕圈也出得来）。
const CAP_STEPS: usize = 48;
/// 一项能力头的最小长度与带通知乘数那一项的长度。
const CAP_BYTES: u8 = 16;
const CAP_NOTIFY_BYTES: u8 = 20;
/// `cfg_type`：1 common、2 notify、4 device（3 ISR 本层不用）。
const CAP_COMMON: u8 = 1;
const CAP_NOTIFY: u8 = 2;
const CAP_DEVICE: u8 = 4;
/// 公共区里那几格的偏移（linux `virtio_pci.h` 的宏表，与 QEMU 同一张）。
const FEATURE_SELECT_DEVICE: u16 = 0x00;
const FEATURE_DEVICE: u16 = 0x04;
const FEATURE_SELECT_DRIVER: u16 = 0x08;
const FEATURE_DRIVER: u16 = 0x0c;
const STATUS: u16 = 0x14;
const QUEUE_SELECT: u16 = 0x16;
const QUEUE_SIZE: u16 = 0x18;
const QUEUE_MSIX: u16 = 0x1a;
const QUEUE_ENABLE: u16 = 0x1c;
const QUEUE_NOTIFY_OFF: u16 = 0x1e;
const QUEUE_DESC_LOW: u16 = 0x20;
const QUEUE_DESC_HIGH: u16 = 0x24;
const QUEUE_AVAIL_LOW: u16 = 0x28;
const QUEUE_AVAIL_HIGH: u16 = 0x2c;
const QUEUE_USED_LOW: u16 = 0x30;
const QUEUE_USED_HIGH: u16 = 0x34;
/// 设备状态位：ACKNOWLEDGE / DRIVER / DRIVER_OK / FEATURES_OK。
const STATUS_ACKNOWLEDGE: u8 = 1;
const STATUS_DRIVER: u8 = 2;
const STATUS_DRIVER_OK: u8 = 4;
const STATUS_FEATURES_OK: u8 = 8;
/// 特性第二页（`device_feature_select = 1`）里那两位：
/// bit 32 = `VIRTIO_F_VERSION_1`、bit 33 = `VIRTIO_F_ACCESS_PLATFORM`（设备侧的叫法 IOMMU_PLATFORM）。
const FEATURE_VERSION_1: u32 = 1;
const FEATURE_ACCESS_PLATFORM: u32 = 2;
/// 不让任何 MSI-X 向量（本驱动轮询，不要中断）。
pub const NO_VECTOR: u16 = 0xFFFF;
