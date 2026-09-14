//! plic — PLIC（中断控制器）的**设备侧**：寄存器视图 + 职业动作。
//!
//! 与 `uart.rs` 同形：`open`（门闩 → 视图）＋ 本设备自己的动作。**不含**协议报文、
//! 不含服务循环、不含配给——那些在装配层（`bin/supervisor/plic.rs`）。
//!
//! # 它认识什么、不认识什么
//!
//! 它认识 PLIC 的寄存器布局与设备树的绑定（**名字 → 线号**）；它**不认识任何设备**
//! ——不知道 `serial@10000000` 后面是串口还是网卡。线表由同一棵树建出来
//! （`from_tree` 在 `crate::lines`），本文件只负责"这台控制器有几条线、有哪些
//! context"这件**设备事实**。
//!
//! # 它是"中断源"的另一端：控制器
//!
//! 设备侧的中断面分两种角色：**源**（UART：`drain` / `IER`）与**控制器**（本文件：
//! `claim` / `complete` / `enable` / `disable`）。两者的动作不交集，故不抽公共词——
//! 把它们连起来的只有一件东西：`protocol::irq`（投线号）。

use alloc::vec::Vec;

use env::TaskId;
use protocol::irq::Lines;
use runtime::env::mail::PolePie;

use crate::lines::from_tree;

/// S 模式外部中断的中断号（`interrupts-extended` 里的 cell 值）。
const EXT_S: u32 = 9;

/// 一条线的优先级：恒 1（0 = 静音，见 [`Plic::disable`]）。它不再是客户端的字段——
/// 这个量没有第二个取值，问客户端等于让它替本域做设备侧的决定。
pub const LINE_PRIORITY: u32 = 1;

/// PLIC 寄存器偏移（SiFive PLIC 布局；`reg` 给的是整块 0x600000）。
const PRIORITY: usize = 0x0000_0000;
const ENABLE: usize = 0x0000_2000;
const ENABLE_STRIDE: usize = 0x80;
const CONTEXT: usize = 0x0020_0000;
const CONTEXT_STRIDE: usize = 0x1000;
const THRESHOLD: usize = 0x00;
const CLAIM: usize = 0x04;

/// PLIC 的寄存器视图（一段有主的、可映射的内存——`docs/driver.md` §1）。
pub struct Plic {
    base: usize,
    /// 本控制器有多少条线（设备树 `riscv,ndev`）——按它拒绝越界的注册。
    pub ndev: u32,
    /// 所有 **S 外部** context 序号（每颗 hart 一个）。
    pub contexts: Vec<u32>,
}

impl Plic {
    /// 开闩 + 读设备树：把"我是谁、我有哪些 context、这台机器上有哪些中断源"问清楚。
    ///
    /// 判据 = `interrupt-controller` 且 `compatible` 里含 `plic` 的节点。**解释设备
    /// 树是驱动的事**（内核只原样搬运自描述，§3.1.3），故这里可以按 compatible 认。
    ///
    /// 返的第二件是**线表**：它由同一棵树建出来（名字 → 线号），本域此后只认名字。
    pub fn open(pole: PolePie, dtb: PolePie, sire: TaskId) -> Option<(Self, Lines)> {
        let base = pole.open().ok()?;
        let dtb_va = dtb.open().ok()?;
        // SAFETY: DTB 门闩把设备树本体只读借映进了本域；`fdt` 只读它。
        let fdt = unsafe { fdt::Fdt::from_ptr(dtb_va as *const u8) }.ok()?;
        let node = fdt.all_nodes().find(|n| {
            n.property("interrupt-controller").is_some()
                && n.compatible()
                    .is_some_and(|c| c.all().any(|s| s.contains("plic")))
        })?;
        let ndev = node
            .property("riscv,ndev")
            .and_then(|p| p.as_usize())
            .unwrap_or(0) as u32;
        // 每项 = <目标 phandle, 中断号…>；中断号的字节数由 `#interrupt-cells` 定。
        let cells = node
            .property("#interrupt-cells")
            .and_then(|p| p.as_usize())
            .unwrap_or(1);
        let mut contexts = Vec::new();
        let prop = node.property("interrupts-extended")?;
        let stride = 4 + 4 * cells;
        for (i, entry) in prop.value.chunks_exact(stride).enumerate() {
            let cell = u32::from_be_bytes(entry[4..8].try_into().ok()?);
            if cell == EXT_S {
                contexts.push(i as u32);
            }
        }
        // 线表与控制器同源：同一个 `ndev` 只读一次（它就是"这台控制器有几条线"）。
        let lines = from_tree(&fdt, &node, ndev, sire);
        Some((
            Self {
                base,
                ndev,
                contexts,
            },
            lines,
        ))
    }

    /// 使能一条线：`priority` 一条、**每个 S context** 各一份 enable。
    ///
    /// 为什么每个 context：中断要能在**任意一颗 hart** 上被取到（不引入任务亲和性，
    /// §3.2.7）。阈值恒 0（不卡仲裁）。
    pub fn enable(&self, line: u32, priority: u32) {
        self.write(PRIORITY + 4 * line as usize, priority);
        for &ctx in &self.contexts {
            let at = CONTEXT + CONTEXT_STRIDE * ctx as usize;
            self.write(at + THRESHOLD, 0);
            let e = ENABLE + ENABLE_STRIDE * ctx as usize + 4 * (line / 32) as usize;
            let bits = self.read(e) | 1 << (line % 32);
            self.write(e, bits);
        }
    }

    /// 关一条线（**收线**用，§12 ②）：`priority = 0`。
    ///
    /// 这是**可逆静音**，不是把线拆掉（§7.2 量到的语义）：`pending` 照旧置位，但
    /// `claim` 恒 0 ⇒ 本域不再投递它；重新登记时 [`Plic::enable`] 把优先级写回
    /// [`LINE_PRIORITY`]。enable 位留着——线号与设备的绑定没变，变的只是"现在有没有
    /// 人接"。
    pub fn disable(&self, line: u32) {
        self.write(PRIORITY + 4 * line as usize, 0);
    }

    /// 领一条线号；0 = 没有可领的（**不是错误**：另一颗 hart 的 context 可能已经
    /// 把它领走了——`claim` 原子清挂起，故第二个 `claim` 拿到 0）。
    pub fn claim(&self) -> u32 {
        let ctx = match self.contexts.first() {
            Some(&c) => c as usize,
            None => return 0,
        };
        self.read(CONTEXT + CONTEXT_STRIDE * ctx + CLAIM)
    }

    /// 结一条线（**不是重武装点**：`complete` 不会把中断带回来，§7.2）。
    pub fn complete(&self, line: u32) {
        let ctx = match self.contexts.first() {
            Some(&c) => c as usize,
            None => return,
        };
        self.write(CONTEXT + CONTEXT_STRIDE * ctx + CLAIM, line);
    }

    fn read(&self, off: usize) -> u32 {
        // SAFETY: `base` 是本域已映射的 PLIC 页；偏移落在 `reg` 声明的区间内。
        unsafe { core::ptr::read_volatile((self.base + off) as *const u32) }
    }

    fn write(&self, off: usize, v: u32) {
        // SAFETY: 同上，只写控制器寄存器。
        unsafe { core::ptr::write_volatile((self.base + off) as *mut u32, v) }
    }
}
