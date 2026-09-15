//! plic — PLIC（中断控制器）的**设备侧**：寄存器视图 + 这台控制器自己的事实。
//!
//! "设备侧"的意思是它只认识**这台控制器**：寄存器布局、有几条线、有几个 context。
//! 它**不认识任何设备**——不知道 `serial@10000000` 后面是串口还是网卡；也不含服务循环、
//! 不含配给、不含投递（那些在 `bin/supervisor/plic`，装配与适配）。
//!
//! # 它为什么要读设备树
//!
//! 两件事只有树里有：**这台控制器有几条线**（`riscv,ndev`）与**本域该用哪个 context**
//! （`interrupts-extended` 的项序，`cell == 9` 才是 S 模式外部中断）。内核不代劳——
//! 它只把设备树原样搬给域（`platform/devices.rs::supply_dtb`）。
//!
//! # 本域要哪四样
//!
//! 要哪几样、多少权、以什么形态出去，写在**两端共用的需求单**里
//! （`programs::needs::PLIC`）：本域按 `Slot` 认领自己的那几格，名字不在本文件里第二遍。
//! 名字本身是 **boot 给的**（设备树节点 basename；`devicetree` / `irq` 两条由内核定）。
//! 本模块只管这台控制器自己——寄存器布局、几条线、哪个 context。

use alloc::vec::Vec;

use runtime::core::dock::View;

/// S 模式外部中断的中断号：`interrupts-extended` 里 `cell == 9` 的那一项。
///
/// **认 9 不认 11**（11 = M 模式）——认错就是把中断线交给固件。也**不硬算 `2h+1`**：
/// 项序是绑定的定义，算术不是。
const EXT_S: u32 = 9;

/// 一条线的优先级：恒 1。**0 是"静音"**（见 [`Plic::disable`]），故本值不能是 0。
pub const LINE_PRIORITY: u32 = 1;

// PLIC 寄存器偏移（SiFive 布局；`reg` 给的是整块）。
const PRIORITY: usize = 0x0000_0000;
const ENABLE: usize = 0x0000_2000;
const ENABLE_STRIDE: usize = 0x80;
const CONTEXT: usize = 0x0020_0000;
const CONTEXT_STRIDE: usize = 0x1000;
const THRESHOLD: usize = 0x00;
const CLAIM: usize = 0x04;

/// PLIC 的寄存器视图 + 这台控制器的事实。
pub struct Plic {
    view: View,
    /// 本控制器有多少条线（`riscv,ndev`）——按它拒绝越界的线号。
    ndev: u32,
    /// 本域用的**那一个** context。
    ///
    /// `claim` / `complete` 是 **per-context** 的：一条线若在多个 context 上使能，
    /// 中断可能投给 A context，而本域去 B context 领——领回 0，源头却一直挂着，
    /// 下一次还会再报，于是空转。故**线接在哪个 context 上，就从哪个 context 领**：
    /// 两件事同一个数，没有第二个数可以不一致。
    ctx: u32,
}

impl Plic {
    /// 读设备树：认控制器、读线数、定下本域用的 context，并列出**要接的线**。
    ///
    /// 返的第二件是线号而不是名字：第一刀还没有客户端，故不需要"名字 → 线号"那张表。
    pub fn new(view: View, dtb: View) -> Option<(Self, Vec<u32>)> {
        // SAFETY: `dtb` 是内核只读借映进本域的整棵设备树（保留区，终身存活）；只读。
        let fdt = unsafe { fdt::Fdt::from_ptr(dtb.base() as *const u8) }.ok()?;
        let node = fdt.all_nodes().find(|n| {
            n.property("interrupt-controller").is_some()
                && n.compatible()
                    .is_some_and(|c| c.all().any(|s| s.contains("plic")))
        })?;
        let ndev = node
            .property("riscv,ndev")
            .and_then(|p| p.as_usize())
            .unwrap_or(0) as u32;
        // 一单元的字节数由 `#interrupt-cells` 定；virt 上的 PLIC 实测为 1。
        let cells = node
            .property("#interrupt-cells")
            .and_then(|p| p.as_usize())
            .unwrap_or(1);
        let prop = node.property("interrupts-extended")?;
        let stride = 4 + 4 * cells;
        // **项序即 context 号**：第一个 `cell == EXT_S` 的项，就是本域要用的那一个。
        let ctx = prop
            .value
            .chunks_exact(stride)
            .position(|e| u32::from_be_bytes([e[4], e[5], e[6], e[7]]) == EXT_S)?
            as u32;
        let this = Self { view, ndev, ctx };
        let lines = sources(&fdt, &node, ndev);
        Some((this, lines))
    }

    /// 本域用的那个 context 号（日志与判据用；它是本域自己的账，不是别人的）。
    pub fn context(&self) -> u32 {
        self.ctx
    }

    /// 接上一条线：写优先级 + 本 context 的阈值与 enable。
    ///
    /// 阈值恒 0（不卡仲裁）。线上限由控制器自报的 `ndev` 把握——越界不是错误，是"这条线
    /// 不在这台控制器上"，接了也没用。
    pub fn enable(&self, line: u32, priority: u32) {
        if line < 1 || line > self.ndev {
            return;
        }
        self.write(PRIORITY + 4 * line as usize, priority);
        let at = CONTEXT + CONTEXT_STRIDE * self.ctx as usize;
        self.write(at + THRESHOLD, 0);
        let e = ENABLE + ENABLE_STRIDE * self.ctx as usize + 4 * (line / 32) as usize;
        let bits = self.read(e) | 1 << (line % 32);
        self.write(e, bits);
    }

    /// 静音一条线：**`priority = 0`**。
    ///
    /// 这是**可逆静音**，不是把线拆掉：`pending` 照旧置位，但 `claim` 恒 0 ⇒ 本域不再接它；
    /// 把优先级写回 [`LINE_PRIORITY`] 即复原。enable 位留着——线号与设备的绑定没变，
    /// 变的只是"现在有没有人接"。
    ///
    /// 第一刀靠它当刹车：本域**不读走设备里的字节**（那是 console 的输入），于是源头一直
    /// 挂着电平；不静音就会 claim → complete → 立刻又报，空转成风暴。
    pub fn disable(&self, line: u32) {
        if line < 1 || line > self.ndev {
            return;
        }
        self.write(PRIORITY + 4 * line as usize, 0);
    }

    /// 领一条线号；**0 = 没有可领的**（不是错误：别的 context 可能已经领走了）。
    pub fn claim(&self) -> u32 {
        self.read(CONTEXT + CONTEXT_STRIDE * self.ctx as usize + CLAIM)
    }

    /// 结一条线（把线号写回去）。**不是重武装点**：`complete` 不会把中断带回来。
    pub fn complete(&self, line: u32) {
        self.write(CONTEXT + CONTEXT_STRIDE * self.ctx as usize + CLAIM, line);
    }

    fn read(&self, off: usize) -> u32 {
        // SAFETY: `view` 是 `Dock::open` 的产物——这段已借映进本域；偏移落在 `reg` 区间内。
        unsafe { core::ptr::read_volatile((self.view.base() + off) as *const u32) }
    }

    fn write(&self, off: usize, v: u32) {
        // SAFETY: 同上，只写控制器寄存器。
        unsafe { core::ptr::write_volatile((self.view.base() + off) as *mut u32, v) }
    }
}

/// 树里所有**指到本控制器**的中断源（线号）。
///
/// 判据只有一条，且只问树：`interrupt-parent` 等于本控制器的 phandle（**只认节点自己写的**，
/// 不沿父链继承）。线号落在 `ndev` 之内——线数的权威是控制器自己。
fn sources(fdt: &fdt::Fdt, controller: &fdt::node::FdtNode, ndev: u32) -> Vec<u32> {
    // 控制器自己没有 phandle ⇒ 树里没有任何节点指得到它 ⇒ 空表（这不是错误：
    // 那棵树在说"没有指向它的中断源"）。
    let Some(phandle) = controller.property("phandle").and_then(|p| p.as_usize()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for node in fdt.all_nodes() {
        if node.property("interrupt-parent").and_then(|p| p.as_usize()) != Some(phandle) {
            continue;
        }
        let Some(line) = first_cell(node) else {
            continue;
        };
        if line < 1 || line > ndev {
            continue;
        }
        out.push(line);
    }
    out
}

/// 节点的第一个中断号（`interrupts = <10>` ⇒ 10）。
///
/// 单元数由**父控制器**的 `#interrupt-cells` 定；这里只认 1 单元（virt 上的 PLIC 实测
/// 如此）。多单元的中断控制器要另写解码。
fn first_cell(node: fdt::node::FdtNode) -> Option<u32> {
    let value = node.property("interrupts")?.value;
    Some(u32::from_be_bytes(value.get(..4)?.try_into().ok()?))
}
