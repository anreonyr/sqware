//! lines — 线表：**名字是权威的轴，线号是设备的轴**（`docs/driver.md` §12 甲）。
//!
//! 一张表、四个动作；核心不碰寄存器（开关线是适配层 `prog-plic` 的事）：
//!
//! ```text
//! from_tree   建表：名字 → 线号（设备树：`interrupts` × `interrupt-parent` 指向本控制器）
//! refer       写属主：这个名字归谁——**只有 root 会调它**（判据在适配层：推者 == 本域的 sire）
//! register    填实例：只能填**自己那一行**，成则回答"这条线归你了"（线号）
//! retire      摘实例：线号与属主都不动（**行保留**，等 root 重发）
//! ```
//!
//! # 为什么线号是名字的函数
//!
//! "这条线是不是你的"没法由客户端自证：门闩证得了"是同一枚设备"，证不了"是哪一台"
//! （用户态拿不到物理基址）。故权威落在**名字**上，而名字 → 线号只能有一个出处——boot
//! 给的那棵设备树。客户端因此不报线号（报文里也没有这个字段），驱动也不信任何自述：
//! 它自己去树里解。
//!
//! # 为什么属主与实例是两件事
//!
//! 属主是**政策**（root 写的：这个名字归谁），实例是**事实**（谁的门闩正挂着这条线）。
//! 两者分开才有"收线"：客户端死了，实例没了，但行还在——root 把名字交给新实例时，
//! 表里已经有它的位置（§12 ②）。**判据也因此在属主一侧**：`Taken` 只可能发生在
//! "名字是你的、但旧实例还没走"这一种情形。

use alloc::vec::Vec;

use env::{Name, TaskId};
use fdt::Fdt;
use fdt::node::FdtNode;
use protocol::irq::Refused;

/// 一行：一台中断源设备。
struct Line {
    name: Name,
    /// 本控制器上的线号——**建表时定，此后不变**（它是设备的属性，不是谁的资源）。
    line: u32,
    /// 属主（`TaskId(0)` = 还没人认领）。只有 [`Lines::refer`] 写它。
    who: TaskId,
    /// 活实例：正挂着这条线的会话门闩（**驱动侧**的 token）。只有
    /// [`Lines::register`]/[`Lines::retire`] 动它。
    holder: Option<usize>,
}

/// 线表。
pub struct Lines {
    rows: Vec<Line>,
}

impl Lines {
    /// 建表：把设备树里**指到本控制器**的中断源收成行（名字 → 线号）。
    ///
    /// 三个判据都在设备树这一侧，一条都不问客户端：
    /// - `interrupt-parent` 指向本控制器的 phandle（**只认本节点自己写的**，不沿父链继承）；
    /// - 有 `interrupts`（本节点就是中断源）；
    /// - 线号落在控制器自报的 `riscv,ndev` 之内（线数的权威是控制器自己）。
    ///
    /// `ndev` 由调用方传入、不在这里再读一遍：同一个事实（`riscv,ndev`）只留一份账,
    /// 适配层已经为"我的中断面接没接上"读过它了。
    pub fn from_tree(fdt: &Fdt, controller: &FdtNode, ndev: u32) -> Lines {
        let mut rows = Vec::new();
        // 控制器自己没有 phandle ⇒ 树里没有任何节点指得到它 ⇒ 空表（这不是错误：
        // 那棵树在说"没有指向它的中断源"）。
        let Some(phandle) = controller.property("phandle").and_then(|p| p.as_usize()) else {
            return Lines { rows };
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
            rows.push(Line {
                name,
                line,
                who: TaskId(0),
                holder: None,
            });
        }
        Lines { rows }
    }

    /// 表里有多少行（适配层在启动时用它判"这棵树里到底有没有中断源"）。
    pub fn count(&self) -> usize {
        self.rows.len()
    }

    /// 写属主：这个名字归 `who`。
    ///
    /// **只写属主，不动实例**：旧实例摘不摘是"收线"那条路的事——没摘，新实例拿的就是
    /// [`Refused::Taken`]（这正是 §12 的判据：不必读 PLIC 寄存器就知道线收没收到）。
    pub fn refer(&mut self, name: &Name, who: TaskId) -> Result<(), Refused> {
        let row = self.row_mut(name).ok_or(Refused::Unknown)?;
        row.who = who;
        Ok(())
    }

    /// 填实例：**只能填自己那一行**（`row.who == from`）。成则返线号（调用方拿去使能）。
    ///
    /// 判据的次序是有意的：先问"这个名字认不认识"，再问"是不是你的"，最后才问"有没有人
    /// 占着"——**权威在属主，不在占用**（别人占没占着，不是请求者该知道的事）。
    pub fn register(&mut self, name: &Name, from: TaskId, session: usize) -> Result<u32, Refused> {
        let row = self.row_mut(name).ok_or(Refused::Unknown)?;
        if row.who.get() == 0 {
            return Err(Refused::Unclaimed);
        }
        if row.who != from {
            return Err(Refused::NotYours);
        }
        if row.holder.is_some() {
            return Err(Refused::Taken);
        }
        row.holder = Some(session);
        Ok(row.line)
    }

    /// 活得实例的会话门闩。投递路径按**线号**查它（领到的是线号，不是名字——这就是
    /// 两条轴各自的用法）。
    pub fn holder(&self, line: u32) -> Option<usize> {
        self.rows
            .iter()
            .find(|r| r.line == line)
            .and_then(|r| r.holder)
    }

    /// 摘实例（**行保留**：名字、线号、属主都不动），返该放下的会话门闩。
    pub fn retire(&mut self, line: u32) -> Option<usize> {
        self.rows
            .iter_mut()
            .find(|r| r.line == line)
            .and_then(|r| r.holder.take())
    }

    fn row_mut(&mut self, name: &Name) -> Option<&mut Line> {
        self.rows.iter_mut().find(|r| r.name == *name)
    }
}

/// 节点的第一个中断号（`interrupts = <10>` ⇒ 10）。
///
/// 单元数由**父控制器**的 `#interrupt-cells` 定；这里只认 1 单元（virt 上的 PLIC 实测
/// `#interrupt-cells = <1>`）。多单元的中断控制器要另写解码——见 §9 的已知边界。
fn first_cell(node: FdtNode) -> Option<u32> {
    let value = node.property("interrupts")?.value;
    Some(u32::from_be_bytes(value.get(..4)?.try_into().ok()?))
}
