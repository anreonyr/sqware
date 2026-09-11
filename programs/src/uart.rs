//! uart — NS16550A 驱动（virt 板上的第一台设备）。
//!
//! **不是设备类框架**（`docs/driver.md` §8 裁过）：没有 trait、没有注册表、没有模板
//! ——只有这一台、只有这一种。第二台设备出现时该抽什么，那时候才知道。
//!
//! # 它凭什么能直接读写寄存器
//!
//! 设备 = 一段有主的、可映射的内存（§1）。`Open` 一枚设备门闩就把那段物理区借映进
//! 本域的空间，拿到 VA 之后 load/store 就是设备访问——不需要任何 syscall，也不需要
//! 内核认识"串口"二字。
//!
//! # 只动哪些寄存器
//!
//! `THR`/`RBR`（数据）与 `LSR`（状态）、`IER`（中断掩码）。**绝不碰 `LCR`/`FCR`/
//! 波特率**——那是固件（OpenSBI）设的，改它等于把控制台的线格式从别人脚下抽走
//! （§9.4）。
//!
//! # `\n` 要自己译成 `\r\n`
//!
//! 在此之前，这条翻译是**固件做的**（OpenSBI 的 uart8250 驱动在写 `\n` 前补 `\r`）。
//! 接手设备就得接手这条翻译：不然行尾只剩换行、光标不回列首，终端上表现为阶梯。
//! 这是"服务自己持设备"最容易被漏掉的一笔——它不是协议，是设备的脾气。

use env::EnvResult;
use runtime::env::mail::PolePie;

/// 寄存器偏移（16550 的经典布局，`reg` 只暴露前 0x100 字节）。
const RBR_THR: usize = 0; // 读 = RBR，写 = THR
const IER: usize = 1;
const LSR: usize = 5;

/// `LSR`：数据就绪（读走一个字节）。
const LSR_DR: u8 = 0x01;
/// `LSR`：发送保持寄存器空（可以写下一个字节）。
const LSR_THRE: u8 = 0x20;

/// `IER`：收到数据可用（**只开这一位**——TX 侧走 `LSR.THRE` 轮询，不开中断）。
const IER_RX: u8 = 0x01;

/// 一枚设备门闩打开后的视图。
#[derive(Clone, Copy)]
pub struct Uart {
    base: usize,
}

impl Uart {
    /// 开闩：把设备门闩映射进本域空间，返回设备视图。
    ///
    /// VA 由内核按空间的用户段 lowest-first-fit 选定（`Open` 的返回值）——
    /// 域只拿到"从哪开始"，不关心映射落在哪。
    pub fn open(pole: PolePie) -> EnvResult<Self> {
        Ok(Self { base: pole.open()? })
    }

    /// 由已经映射好的地址造视图（同一台设备在**另一张页表**里也可以有视图）。
    pub const fn at(base: usize) -> Self {
        Self { base }
    }

    /// 写一段字符串：逐字节等 `THRE`。
    ///
    /// **忙等就是背压**：与固件此前的同步块写语义相同（调用方被挡住，不丢字）。
    /// 环满/忙时丢掉字节是另一种语义（那是"丢弃式日志"），本仓不选它。
    pub fn put(&self, s: &str) {
        for &b in s.as_bytes() {
            if b == b'\n' {
                self.put_byte(b'\r');
            }
            self.put_byte(b);
        }
    }

    fn put_byte(&self, b: u8) {
        // LSR.THRE 是**持续状态不是事件**（§7.3）：写之前查它、写之后不管它，
        // 下一步会再查。故这里没有"清标志"这回事。
        while self.read(LSR) & LSR_THRE == 0 {
            core::hint::spin_loop();
        }
        self.write(RBR_THR, b);
    }

    /// 非阻塞取一个字节（无输入 → `None`）。
    pub fn try_get(&self) -> Option<u8> {
        if self.read(LSR) & LSR_DR == 0 {
            return None;
        }
        Some(self.read(RBR_THR))
    }

    /// 排空接收侧：读到 `DR = 0` 为止（中断路径进门先做这件事——16550 的 FIFO 里
    /// 可能已经攒了好几个字节，只读一个会让线一直拉着）。
    pub fn drain(&self, mut sink: impl FnMut(u8)) {
        while let Some(b) = self.try_get() {
            sink(b);
        }
    }

    /// 开/关接收中断（`IER.RX`）——**静音归设备侧**（`docs/driver.md` §3.2.5）：
    /// 客户端进门先关自己的门、出门再开，PLIC 侧因此不需要静音表、deadline 或扫描。
    ///
    /// 只动 `IER`：`LCR`/`FCR`/波特率是固件设的，碰它们等于把线格式从别人脚下抽走
    /// （§9.4）。
    pub fn mask_rx(&self) {
        self.write(IER, self.read(IER) & !IER_RX);
    }

    pub fn unmask_rx(&self) {
        self.write(IER, self.read(IER) | IER_RX);
    }

    #[inline]
    fn read(&self, off: usize) -> u8 {
        // SAFETY: `base` 是本域已映射的设备页（`Open` 的产物），偏移固定、只读。
        unsafe { core::ptr::read_volatile((self.base + off) as *const u8) }
    }

    #[inline]
    fn write(&self, off: usize, v: u8) {
        // SAFETY: 同上，只写设备寄存器。
        unsafe { core::ptr::write_volatile((self.base + off) as *mut u8, v) }
    }
}
