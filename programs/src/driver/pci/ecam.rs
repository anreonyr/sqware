//! PCI 配置空间（ECAM）：一个功能格的 8/16/32 位配置读写，加上 BAR 的标准探尺与落位。
//!
//! 设备树里 `pci@…` 那一段 `reg` 就是 ECAM；一个功能落在
//! `bus<<20 | dev<<15 | fn<<12` 上，而它在 IOMMU 眼里的号是 `bus<<8 | dev<<3 | fn`。
//! 本层只说「哪一格、几字节、怎么量」，不说设备是什么（那是 `endpoint` 那一层）。

/// ECAM 段 ＋ 段内**一个功能**：那一个号（`requester`）同时是配置空间的坐标与 IOMMU 的设备号。
pub struct Ecam {
    base: usize,
    requester: u16,
}

impl Ecam {
    /// `base` = ECAM 段起址；`requester` = 这一个功能在 PCI 上的号。
    pub const fn new(base: usize, requester: u16) -> Self {
        Self { base, requester }
    }

    pub const fn requester(&self) -> u16 {
        self.requester
    }

    /// 这一格的段内偏移：bus 占 20 位、dev 占 15 位、fn 占 12 位，`reg` 落在低 4KB 里。
    fn at(&self, register: u16) -> usize {
        let requester = self.requester as usize;
        self.base
            + ((requester >> 8) << 20)
            + ((requester & 0xFF) << 12)
            + (register & 0xFFC) as usize
    }

    pub fn read8(&self, register: u16) -> u8 {
        // SAFETY: `base` 是一段已授权、已映射的 ECAM 窗口，`register` 落在低 4KB 里。
        unsafe {
            core::ptr::read_volatile((self.at(register) + (register as usize & 3)) as *const u8)
        }
    }

    pub fn read16(&self, register: u16) -> u16 {
        // SAFETY: 同 `read8`，偏移按 2 字节格取。
        unsafe {
            core::ptr::read_volatile((self.at(register) + (register as usize & 2)) as *const u16)
        }
    }

    pub fn read32(&self, register: u16) -> u32 {
        // SAFETY: 同 `read8`，偏移按 4 字节格取。
        unsafe { core::ptr::read_volatile(self.at(register) as *const u32) }
    }

    pub fn write16(&self, register: u16, value: u16) {
        // SAFETY: 同 `read8`，偏移按 2 字节格取。
        unsafe {
            core::ptr::write_volatile(
                (self.at(register) + (register as usize & 2)) as *mut u16,
                value,
            )
        }
    }

    pub fn write32(&self, register: u16, value: u32) {
        // SAFETY: 同 `read8`，偏移按 4 字节格取。
        unsafe { core::ptr::write_volatile(self.at(register) as *mut u32, value) }
    }

    /// 命令寄存器里再置上 `set` 那几位（**读改写**：不能把原本开着的位洗掉）。
    pub fn command_or(&self, set: u16) {
        self.write16(COMMAND, self.read16(COMMAND) | set);
    }

    /// 量一根**内存** BAR 的长度（标准那一手：先写全 1、再读回掩码，随后把原值放回去）。
    ///
    /// 返 `(字节数, 是不是 64 位那一根)`；`None` = 这一格不存在（原值 0）或它是 I/O 空间
    /// （bit0）。
    ///
    /// 64 位那一根要把两半**拼成一个 64 位掩码**再取最低那一位（与 Linux `pci_read_bases`
    /// 同一条口径）——把高半当成“必须为 0”是错的：一根 16KiB 的 64 位 BAR 读回去是
    /// `0xffffffff_ffffc000`，高半本来就是全 1。
    pub fn bar_size(&self, index: u8) -> Option<(u64, bool)> {
        let register = BAR + index as u16 * 4;
        let original = self.read32(register);
        if original == 0 || original & 1 != 0 {
            return None;
        }
        let wide = is_wide(original);
        let upper = if wide {
            let upper = self.read32(register + 4);
            self.write32(register + 4, u32::MAX);
            upper
        } else {
            0
        };
        self.write32(register, u32::MAX);
        let low = self.read32(register);
        let high = if wide { self.read32(register + 4) } else { 0 };
        self.write32(register, original);
        if wide {
            self.write32(register + 4, upper);
        }
        let mask = ((high as u64) << 32) | (low as u64 & !0xF);
        if mask == 0 {
            return None;
        }
        let size = mask & mask.wrapping_neg();
        (size >= 0x10).then_some((size, wide))
    }

    /// 把 `index` 那一根内存 BAR 落到 `address`（64 位那一根要同时写高半）。
    pub fn place_bar(&self, index: u8, address: u64, wide: bool) {
        let register = BAR + index as u16 * 4;
        if wide {
            self.write32(register + 4, (address >> 32) as u32);
        }
        self.write32(register, address as u32 | if wide { 0x4 } else { 0 });
    }
}

/// 在 `base` 这一段 ECAM 上扫一个 `vendor` 的、设备号在 `devices` 里的功能，
/// 返它在 PCI 上的号。
///
/// 本景一条总线（QEMU `virt` 的端点就在 bus 0）、每个功能一格、按 devfn 升序取第一个命中。
/// 设备号给一组是因为同一个块设备有两个号：过渡态 `0x1001` 与纯 1.x 的 `0x1042`。
pub fn find(base: usize, vendor: u16, devices: &[u16]) -> Option<u16> {
    for dev in 0..32u16 {
        for func in 0..8u16 {
            let requester = dev << 3 | func;
            let scan = Ecam::new(base, requester);
            let id = scan.read16(DEVICE);
            if scan.read16(VENDOR) == vendor && devices.contains(&id) {
                return Some(requester);
            }
        }
    }
    None
}

/// 64 位那一根（地址位 2:1 = 10）。
fn is_wide(low: u32) -> bool {
    (low >> 1) & 3 == 2
}

/// 配置空间里那几格的偏移。
pub const VENDOR: u16 = 0x00;
pub const DEVICE: u16 = 0x02;
pub const COMMAND: u16 = 0x04;
pub const BAR: u16 = 0x10;
pub const CAPABILITY: u16 = 0x34;
/// 命令寄存器那两位：内存空间 / bus master。
pub const COMMAND_MEMORY: u16 = 1 << 1;
pub const COMMAND_MASTER: u16 = 1 << 2;
/// 能力表那一格：每一项的 `cap_vndr` 是它。
pub const CAP_VENDOR: u8 = 0x09;
