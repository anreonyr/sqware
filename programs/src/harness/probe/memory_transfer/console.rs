//! 本景的控制台面：串口设备寄存器那一页。
//!
//! 产品景里 `serial@10000000` 归 `driver/uart` 那一台（它把设备面藏在 bin 里，客人只经
//! 服务那一面收字节）；本景不装服务，故那一页由控制单元自己拿着：按设备树在那台机器的
//! 自述里认出第一台 NS16550 兼容串口，取它那一格区门闩，映射进来直接收发寄存器。
//! 只借三格：`RBR`/`THR`（读/写一个字节）与 `LSR`（`DR` 有没有字节、`THRE` 发得出去没有）
//! ——与 `driver/uart/dev/uart.rs` 读的是同一台设备的同三格。
//!
//! **为什么不走 `DebugCall::Get`**：那一格借的是内核自己那支 DBCN，而本板子的
//! `Platform Console Device` 是 **semihosting**（QEMU 的 stdin；测试路把它钉成 null），
//! 不是这台串口。实量过的读数：`qtest --scene memory-transfer` 那一景里五道门
//! 每一次轮询都同时问一下 `DebugGet`，全部答 0；而同一枚字节从这页寄存器读得到。

use crate::boot::Accounts;
use crate::support::machine::Machine;
use ::resource::dock::{Dock, View};
use env::{Mark, Name, Page, PieToken};

/// 装载者把这一枚区门闩种在控制单元表里用的标号。
pub(super) const UART_MARK: Mark = Mark::of("memory-transfer-console");

/// 这台板子上串口在设备树里的类名（`driver/uart` 认领设备时问的也是这一格）。
const SERIAL_CLASS: &str = "ns16550a";

/// NS16550 的三格。
const RBR: usize = 0;
const THR: usize = 0;
const LSR: usize = 5;
/// `LSR.DR`：收一个字节到了。
const LSR_DR: u8 = 0x01;
/// `LSR.THRE`：发送保持寄存器空了。
const LSR_THRE: u8 = 0x20;

pub struct Console {
    /// 映射归这一格：`Dock` 不随视图放下而撤图，它活到本域走完。
    _dock: Dock,
    view: View,
}

impl Console {
    /// 这台板子的串口在设备树里的那一格区（`Name::Page(Page::Region(base))`）。
    pub fn region(accounts: &Accounts) -> Name {
        let dtb = accounts
            .token(Name::Page(Page::Dtb))
            .expect("memory-transfer: boot device tree");
        let dock = Dock::open(dtb).expect("memory-transfer: device tree view");
        let machine = Machine::of(dock.view()).expect("memory-transfer: device tree parse");
        machine
            .devices()
            .expect("memory-transfer: device inventory")
            .into_iter()
            .find(|device| device.class == SERIAL_CLASS)
            .expect("memory-transfer: board serial")
            .resource
    }

    /// 把那一页映进本域。
    pub fn open(token: PieToken) -> Console {
        let dock = Dock::open(token).expect("memory-transfer: console view");
        let view = dock.view();
        Console { _dock: dock, view }
    }

    /// 推一条字出去：`THRE` 置着才塞 `THR`（与产品那一台同一条等法）。
    pub fn put(&self, bytes: &[u8]) {
        let at = self.view.base() as *const u8;
        for &byte in bytes {
            // SAFETY: 设备那一页由本域映射着；只在 `THRE` 置着时写 `THR`。
            unsafe {
                while core::ptr::read_volatile(at.add(LSR)) & LSR_THRE == 0 {
                    core::hint::spin_loop();
                }
                core::ptr::write_volatile(at.add(THR) as *mut u8, byte);
            }
        }
    }

    /// 收一个字节：没有就答 `None`——不阻塞、不等固定时长。
    pub fn get(&self) -> Option<u8> {
        let at = self.view.base() as *const u8;
        // SAFETY: 同上；`DR` 置着才有字节，读 `RBR` 即取走一个。
        unsafe {
            if core::ptr::read_volatile(at.add(LSR)) & LSR_DR == 0 {
                return None;
            }
            Some(core::ptr::read_volatile(at.add(RBR)))
        }
    }
}
