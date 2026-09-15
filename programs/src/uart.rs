//! uart — NS16550A 驱动（virt 板上的第一台设备）的**设备侧**：视图 + 职业动作。
//!
//! 与 `plic.rs` 同形：收下一段视图 ＋ 本设备自己的动作。**开闩不在这里**（门闩 → 视图
//! 是 `Dock::open` 的事）；**不含**协议报文、不含线程、不含配给——那些在装配层
//! （`bin/supervisor/uart.rs`）。
//!
//! **不是设备类框架**（`docs/driver.md` §8 裁过）：没有 trait、没有注册表、没有模板
//! ——第二台设备（PLIC）出现时抽出来的只有"**视图 + 读写寄存器**"这两条（"开"那一步归
//! `runtime::core::dock` 的 `Dock`，见 `docs/dock.md`），而它连寄存器宽度
//! 都不同（u8 / u32），故两台各写各的，只保证**同形**。
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
//! `IER` 由**读线程**按 `mask → 排空 → unmask` 的节奏开合：它不只是"静音"，更是
//! **PLIC 那条线的重新武装**（见 [`Uart::mask_rx`] 的注——`complete` 不是重武装点，
//! 只认上跳沿）。§3.2.5 裁的"静音在设备侧"因此逐字成立，而且设备侧这一份是必需的。
//!
//! # `\n` 要自己译成 `\r\n`
//!
//! 在此之前，这条翻译是**固件做的**（OpenSBI 的 uart8250 驱动在写 `\n` 前补 `\r`）。
//! 接手设备就得接手这条翻译：不然行尾只剩换行、光标不回列首，终端上表现为阶梯。
//! 这是"驱动程序持设备"最容易被漏掉的一笔——它不是协议，是设备的脾气。

use runtime::core::dock::View;

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
    view: View,
}

impl Uart {
    /// 收下一段已映射的视图。
    ///
    /// VA 由内核按空间的用户段 lowest-first-fit 选定（`Open` 的返回值）——
    /// 域只拿到"从哪开始"与"有多长"，不关心映射落在哪。
    pub fn new(view: View) -> Self {
        Self { view }
    }

    // 旧 `at(base)`（由裸地址造视图）**已删**：能造视图的只有 `Dock::open` 的产物，
    // 一个裸 `usize` 不再是构造入口；同一个域的第二个线程要一份视图，直接拷那个 `Copy`
    // 的值即可，不需要第二个构造入口（零调用者的构造器不留，`f2e06d2` 的先例）。

    /// 写一段字节：逐字节等 `THRE`。
    ///
    /// **忙等就是背压**：与固件此前的同步块写语义相同（调用方被挡住，不丢字）。
    /// 环满/忙时丢掉字节是另一种语义（那是"丢弃式日志"），本仓不选它。
    ///
    /// 收 `&[u8]` 而不是 `&str`：它是**字节设备**。行尾的 `\r\n` 翻译在下面做，
    /// 与"这段字节是不是文本"无关。
    pub fn put(&self, bytes: &[u8]) {
        for &b in bytes {
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

    /// 关接收中断（`IER.RX`）：**进门先关**。
    ///
    /// 它不只是"静音"，**它是 PLIC 那条线的重新武装**（`docs/driver.md` §3.2.5/§7.2）：
    /// `complete` 不是重武装点，PLIC 只认**上跳沿**。若排空期间门开着，新到的字节只把线
    /// 保持在高端、不产生新边沿 ⇒ 没有新的 pending ⇒ 驱动再也醒不过来（实测：一个命令敲
    /// 到第三个字符就停住）。故次序是 `mask → 排空 → unmask`，排空完开门时若 FIFO 仍有
    /// 数据，线上跳 ⇒ PLIC 重新置 pending。
    ///
    /// 只动 `IER`：`LCR`/`FCR`/波特率是固件设的，碰它们等于把线格式从别人脚下抽走（§9.4）。
    pub fn mask_rx(&self) {
        self.write(IER, self.read(IER) & !IER_RX);
    }

    /// 开接收中断（`IER.RX`）：**出门再开**（见 [`Uart::mask_rx`] 的次序说明）。
    pub fn unmask_rx(&self) {
        self.write(IER, self.read(IER) | IER_RX);
    }

    #[inline]
    fn read(&self, off: usize) -> u8 {
        // SAFETY: `view` 是 `Dock::open` 的产物——那段设备页已借映进本域；偏移固定、只读。
        unsafe { core::ptr::read_volatile((self.view.base() + off) as *const u8) }
    }

    #[inline]
    fn write(&self, off: usize, v: u8) {
        // SAFETY: 同上，只写设备寄存器。
        unsafe { core::ptr::write_volatile((self.view.base() + off) as *mut u8, v) }
    }
}
