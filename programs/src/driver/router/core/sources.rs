//! router::core::sources — **这台控制器自己的那两个事实**：它有几条线（线数）＋ 本域用哪个
//! context。
//!
//! 本文件**不碰寄存器、不碰内核**：不出现 `runtime::`、不出现 `View`——只吃**设备树那段字节**
//! （适配层把借映进来的那段切好交给它），故可独立推理。寄存器那一半住 `../plic.rs`（设备面）。
//!
//! # 它为什么还读设备树
//!
//! 两个数只有树里有：**这台控制器有几条线**（`riscv,ndev`）与**本域该用哪个 context**
//! （`interrupts-extended` 的项序，`cell == 9` 才是 S 模式外部中断）。内核不代劳——
//! 它只把设备树原样搬给域（`platform/devices.rs::supply_dtb`）。
//!
//! 认控制器用的那个类（`compatible`）与单子上那一格是**同一个常量**（`PLIC_CLASS`）。
//!
//! # 照实记（"区 ↔ 线号"那一半退场了：这一刀最大的一笔减法）
//!
//! 本文件原先还有**半张表**：扫一遍树，把"指到本控制器的中断源"收成 `Source { key, line, name }`
//! 一表，再答 `line_of(坐标)`——那是"**线 = 区的函数**"这条关系**在路由者这一侧的落点**，
//! 也是客户登记时唯一要带坐标的原因（客户只有区可报）。
//!
//! 那一整半随设备账那一台走了：**hub 读同一棵树时顺手把名 / 类 / 线一起算出来**
//! （`programs/src/system/machine.rs` 的 `devices`），认领那一答的契
//! （[`Deed`](protocol::driver::hub::Deed)）里就带着线号 ⇒ 客户报的是线号本身，路由者不再翻。
//! 跟着走的还有**五笔"没进来的账"**（`unparented` / `beyond` / `mapped` / `unparsed` /
//! `unregion`）：它们是**那一趟扫树**的读数，扫树搬走了，读数也搬走（今天在 hub 那一边：
//! 读树那一行报"有几台没有线"）。
//!
//! 留下的两句仍是它们的原义：`device_count` 是**账的容量**（按它拒越界的线号），`context` 是
//! **领哪一格**（`claim` / `complete` 是 per-context 的：线接在哪个 context 上，就从哪个
//! context 领——两件事同一个数，没有第二个数可以不一致）。

/// S 模式外部中断的中断号：`interrupts-extended` 里 `cell == 9` 的那一项。
///
/// **认 9 不认 11**（11 = M 模式）——认错就是把中断线交给固件。也**不硬算 `2h+1`**：
/// 项序是绑定的定义，算术不是。
const EXT_S: u32 = 9;

use programs::program::router::PLIC_CLASS;

/// 这台控制器的事实：**它有几条线 ＋ 本域用哪个 context**。
pub struct Sources {
    /// 本控制器有多少条线（`riscv,ndev`）——线账按它校验（越界不可表达），也按它拒线号。
    device_count: u32,
    /// 本域用的**那一个** context。
    ///
    /// `claim` / `complete` 是 **per-context** 的：一条线若在多个 context 上使能，
    /// 中断可能投给 A context，而本域去 B context 领——领回 0，源头却一直挂着，
    /// 下一次还会再报，于是空转。故**线接在哪个 context 上，就从哪个 context 领**：
    /// 两件事同一个数，没有第二个数可以不一致。
    ctx: u32,
}

impl Sources {
    /// 读设备树：认控制器、读线数、定下本域用的 context。
    ///
    /// 失败（`None`）：树里找不到那台控制器 / 没有 `interrupts-extended` / 里面没有
    /// S 模式那一项——三种都是"这台机器与本域要的东西对不上"，同一个下一步（本域起不来）。
    pub fn of(dtb: &[u8]) -> Option<Sources> {
        let fdt = fdt::Fdt::new(dtb).ok()?;
        let node = fdt.all_nodes().find(|n| {
            n.property("interrupt-controller").is_some()
                && n.compatible()
                    .is_some_and(|c| c.all().any(|s| s == PLIC_CLASS))
        })?;
        let device_count = node
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
        Some(Sources { device_count, ctx })
    }

    /// 本域用的那个 context 号（日志与判据用；它是本域自己的账，不是别人的）。
    pub fn context(&self) -> u32 {
        self.ctx
    }

    /// 本控制器自报的线数（`riscv,ndev`）。**账的容量按它校验**（见 `adapt/boot.rs`）。
    pub fn device_count(&self) -> u32 {
        self.device_count
    }
}
