//! 一轮 virtio-blk 请求的页布局与读写（α 与控制器共用一处定义）。
//!
//! 控制器自己的那一枚 Pole 就是这几页；它把整窗交到 α 手里，α 再把同一窗交进 IOMMU。
//! 设备看到的是本绑定的 IOVA 窗：页序与来源页序一一对应（IOMMU 那一侧就是这么映的）。
//!
//! 两台各自把**自己那份映射的起址**交给 [`Block`]，于是读写只有这一处定义。

use env::PAGE_SIZE;

/// 页序：描述符表 / 可用环 / 已用环 / 数据 / 状态 / 头。
pub const DESC: usize = 0;
pub const AVAIL: usize = 1;
pub const USED: usize = 2;
pub const DATA: usize = 3;
pub const STATUS: usize = 4;
pub const HEADER: usize = 5;
/// 本场景一轮要几页。
pub const PAGES: usize = 6;
/// 一轮读一个扇区；头是设备只读的那 16 字节。
pub const SECTOR: usize = 512;
pub const HEADER_BYTES: usize = 16;
/// 设备**可写**那两段（数据 ＋ 状态）之和：硬件写回的 `len` 就是这个数。
pub const WRITTEN_BYTES: u32 = (SECTOR + 1) as u32;
/// 块设备请求类型与状态（`VIRTIO_BLK_T_IN` / `VIRTIO_BLK_S_OK`）。
const BLK_IN: u32 = 0;
pub const BLK_OK: u8 = 0;
/// 描述符标志：`NEXT` / `WRITE`（设备侧可写）。
const DESC_NEXT: u16 = 1;
const DESC_WRITE: u16 = 2;
/// 负证那一轮先铺在数据页里的哨兵。
pub const SENTINEL: u8 = 0x5A;

/// 第 `page` 页给设备看到的 IOVA。
pub fn iova(base: u64, page: usize) -> u64 {
    base + (page as u64) * PAGE_SIZE as u64
}

/// 宿主造那张盘时用的花纹：第 `at` 字节 = `at % 256`。
pub fn expected(at: usize) -> u8 {
    (at % 256) as u8
}

/// 一窗的读写：起址是本域那一份映射（控制器是 Pole 的 View，α 是交来的窗）。
pub struct Block {
    base: usize,
}

/// 本场景给设备看的 IOVA 窗长度（字节）。
pub const SPAN: u64 = PAGES as u64 * PAGE_SIZE as u64;


impl Block {
    pub const fn at(base: usize) -> Self {
        Self { base }
    }

    fn page(&self, page: usize) -> *mut u8 {
        (self.base + page * PAGE_SIZE) as *mut u8
    }

    pub fn put16(&self, page: usize, at: usize, value: u16) {
        // SAFETY: 整窗在本域已映射，页序与偏移由本文件钉住。
        unsafe { core::ptr::write_volatile(self.page(page).add(at) as *mut u16, value) };
    }

    pub fn put32(&self, page: usize, at: usize, value: u32) {
        // SAFETY: 同上，偏移按 4 字节格取。
        unsafe { core::ptr::write_volatile(self.page(page).add(at) as *mut u32, value) };
    }

    pub fn put64(&self, page: usize, at: usize, value: u64) {
        // SAFETY: 同上，偏移按 8 字节格取。
        unsafe { core::ptr::write_volatile(self.page(page).add(at) as *mut u64, value) };
    }

    pub fn get8(&self, page: usize, at: usize) -> u8 {
        // SAFETY: 同上。
        unsafe { core::ptr::read_volatile(self.page(page).add(at)) }
    }

    pub fn get16(&self, page: usize, at: usize) -> u16 {
        // SAFETY: 同上，偏移按 2 字节格取。
        unsafe { core::ptr::read_volatile(self.page(page).add(at) as *const u16) }
    }

    pub fn get32(&self, page: usize, at: usize) -> u32 {
        // SAFETY: 同上，偏移按 4 字节格取。
        unsafe { core::ptr::read_volatile(self.page(page).add(at) as *const u32) }
    }

    /// 把一整页铺成同一个字节。
    pub fn fill(&self, page: usize, byte: u8) {
        // SAFETY: 整页都在本域这一段可写映射里。
        unsafe { core::ptr::write_bytes(self.page(page), byte, PAGE_SIZE) };
    }

    /// 摆一轮请求：三段描述符（头 → 数据 → 状态）＋ 可用环那一格。
    ///
    /// 链头恒为 0（一轮一组三段），故可用环里那一格写 0；`avail.idx` 推进到 `round + 1`。
    pub fn arm(&self, iova_base: u64, round: u16) {
        // 头：类型 / 保留 / 扇区号。
        self.put32(HEADER, 0, BLK_IN);
        self.put32(HEADER, 4, 0);
        self.put64(HEADER, 8, 0);
        // 三段描述符：地址（8）＋ 长度（4）＋ 标志（2）＋ 下一段（2）。
        self.descriptor(0, iova(iova_base, HEADER), HEADER_BYTES as u32, DESC_NEXT, 1);
        self.descriptor(
            1,
            iova(iova_base, DATA),
            SECTOR as u32,
            DESC_NEXT | DESC_WRITE,
            2,
        );
        self.descriptor(2, iova(iova_base, STATUS), 1, DESC_WRITE, 0);
        // 可用环：那一格写 0，`idx` 推到 `round + 1`。
        self.put16(AVAIL, 4 + round as usize * 2, 0);
        self.put16(AVAIL, 2, round + 1);
    }

    fn descriptor(&self, index: usize, address: u64, len: u32, flags: u16, next: u16) {
        let at = index * 16;
        self.put64(DESC, at, address);
        self.put32(DESC, at + 8, len);
        self.put16(DESC, at + 12, flags);
        self.put16(DESC, at + 14, next);
    }

    /// 已用环的 `idx`。
    pub fn used_index(&self) -> u16 {
        self.get16(USED, 2)
    }

    /// 已用环最新那一项的 `(id, len)`。
    pub fn used_entry(&self) -> (u32, u32) {
        (self.get32(USED, 4), self.get32(USED, 8))
    }

    /// 状态那一格（设备写回后应当是 [`BLK_OK`]）。
    pub fn status(&self) -> u8 {
        self.get8(STATUS, 0)
    }

    /// 数据页第 `at` 字节。
    pub fn data(&self, at: usize) -> u8 {
        self.get8(DATA, at)
    }

    /// 一轮真读回来的花纹对不对。
    pub fn delivered(&self) -> bool {
        (0..SECTOR).all(|at| self.data(at) == expected(at))
    }
}
