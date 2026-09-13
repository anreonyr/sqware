//! lines — 设备树 → 行：**名字 → 线号**（驱动侧）。
//!
//! 表本身的语义（属主 / 委托 / 实例 / 收线）在 `protocol::irq::server`：判据是协议的事；
//! 而"这台控制器上有哪些中断源、各是几号"是**设备的活**——`docs/driver.md` §3.1.3：
//! 解释设备树是驱动的事。故本文件只把树收成行，交 [`Lines::new`] 建表；协议层因此
//! 不必背 `fdt` 这个依赖，而"这条线是哪台设备的"那笔账仍只有一个出处。
//!
//! 三个判据都在设备树这一侧，一条都不问客户端：
//! - `interrupt-parent` 指向本控制器的 phandle（**只认本节点自己写的**，不沿父链继承）；
//! - 有 `interrupts`（本节点就是中断源）；
//! - 线号落在控制器自报的 `riscv,ndev` 之内（线数的权威是控制器自己）。

use alloc::vec::Vec;

use env::{Name, TaskId};
use fdt::Fdt;
use fdt::node::FdtNode;
use protocol::irq::Lines;

/// 建表：把设备树里**指到本控制器**的中断源收成行（名字 → 线号）。
///
/// `ndev` 由调用方传入、不在这里再读一遍：同一个事实（`riscv,ndev`）只留一份账，
/// 适配层已经为"我的中断面接没接上"读过它了。
pub fn from_tree(fdt: &Fdt, controller: &FdtNode, ndev: u32, sire: TaskId) -> Lines {
    let mut rows = Vec::new();
    // 控制器自己没有 phandle ⇒ 树里没有任何节点指得到它 ⇒ 空表（这不是错误：
    // 那棵树在说"没有指向它的中断源"）。
    let Some(phandle) = controller.property("phandle").and_then(|p| p.as_usize()) else {
        return Lines::new(rows, sire);
    };
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
        // 名字装不下 = 这台设备没有可表达的身份（同 `devices.rs` 的裁决：**不截断**，
        // 截断会把两台设备指成同一个名字），跳过它。
        let Ok(name) = Name::new(node.name) else {
            continue;
        };
        rows.push((name, line));
    }
    Lines::new(rows, sire)
}

/// 节点的第一个中断号（`interrupts = <10>` ⇒ 10）。
///
/// 单元数由**父控制器**的 `#interrupt-cells` 定；这里只认 1 单元（virt 上的 PLIC 实测
/// `#interrupt-cells = <1>`）。多单元的中断控制器要另写解码——见 §9 的已知边界。
fn first_cell(node: FdtNode) -> Option<u32> {
    let value = node.property("interrupts")?.value;
    Some(u32::from_be_bytes(value.get(..4)?.try_into().ok()?))
}
