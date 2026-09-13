//! irq·server — 线表：**名字是权威的轴，线号是设备的轴**（`docs/driver.md` §12 甲）。
//!
//! 一张表、四个动作；核心不碰寄存器（开关线是适配层 `prog-plic` 的事）：
//!
//! ```text
//! new         建表：把驱动从设备树解出来的 (名字, 线号) 收成行
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
//!
//! # 设备事实由驱动交进来
//!
//! 建表要的"名字 → 线号"是**设备树**的事（`docs/driver.md` §3.1.3：解释设备树是驱动
//! 的事），故 [`Lines::new`] 收的是驱动解好的行，本模块不认识 `fdt`——协议层因此不必
//! 多背一个依赖，而"这条线是哪台设备的"那笔账仍只有一个出处。

use alloc::vec::Vec;

use env::{Name, TaskId};

use super::wire::Refused;

/// 一行：一台中断源设备。
struct Line {
    name: Name,
    /// 本控制器上的线号——**建表时定，此后不变**（它是设备的属性，不是谁的资源）。
    line: u32,
    /// 属主（`TaskId(0)` = 还没人认领）。只有 [`Lines::refer`] 写它。
    who: TaskId,
    /// 属主**唯一的原始写者**：本域的 `sire`（= root 的主线程）。**建表时定死**
    /// （它不是运行时判断，是"谁生的我"）。
    sire: TaskId,
    /// 被 root **委托**的第二个写者（`Lines::delegate` 写它）：root 的服务重启落在它自己
    /// 域里的监护线程上，而线程不是域——这条委托就是"root 亲口把这份写权借给它"。
    /// 只有一个：今天只有一个消费者（console 的重发），第二个出现时再抽表。
    writer: Option<TaskId>,
    /// 活实例：正挂着这条线的会话门闩（**驱动侧**的 token）。只有
    /// [`Lines::register`]/[`Lines::retire`] 动它。
    holder: Option<usize>,
}

/// 线表。
pub struct Lines {
    rows: Vec<Line>,
}

impl Lines {
    /// 建表：把驱动从设备树收出来的 `(名字, 线号)` 收成行。
    ///
    /// 判据全在设备树那一侧、一条都不问客户端（`docs/driver.md` §12 甲）：
    /// - `interrupt-parent` 指向本控制器的 phandle；
    /// - 有 `interrupts`（本节点就是中断源）；
    /// - 线号落在控制器自报的 `riscv,ndev` 之内（线数的权威是控制器自己）。
    ///
    /// 那三条由驱动在读树时落地（`programs/src/lines.rs`）——本函数只收结果。
    pub fn new(entries: Vec<(Name, u32)>, sire: TaskId) -> Lines {
        let rows = entries
            .into_iter()
            .map(|(name, line)| Line {
                name,
                line,
                who: TaskId(0),
                sire,
                writer: None,
                holder: None,
            })
            .collect();
        Lines { rows }
    }

    /// 表里有多少行（适配层在启动时用它判"这棵树里到底有没有中断源"）。
    pub fn count(&self) -> usize {
        self.rows.len()
    }

    /// 写属主：这个名字归 `who`。`from` = 发起者——**只有本行的写者能写**
    /// （本域的 `sire`，或 root 为这个名字委托过的那个线程，见 [`Lines::delegate`]）。
    ///
    /// **只写属主，不动实例**：旧实例摘不摘是"收线"那条路的事——没摘，新实例拿的就是
    /// [`Refused::Taken`]（这正是 §12 的判据：不必读 PLIC 寄存器就知道线收没收到）。
    pub fn refer(&mut self, name: &Name, from: TaskId, who: TaskId) -> Result<(), Refused> {
        let row = self.row_mut(name).ok_or(Refused::Unknown)?;
        if !row.writable_by(from) {
            return Err(Refused::NotYours);
        }
        row.who = who;
        Ok(())
    }

    /// 委托写权：这个名字的属主，从此也可以由 `who` 写（**只有 `sire` 能委托**）。
    pub fn delegate(&mut self, name: &Name, from: TaskId, who: TaskId) -> Result<(), Refused> {
        let row = self.row_mut(name).ok_or(Refused::Unknown)?;
        if from != row.sire {
            return Err(Refused::NotYours);
        }
        row.writer = Some(who);
        Ok(())
    }

    /// 填实例：**只能填自己那一行**（`row.who == from`）。成则返线号（调用方拿去使能）。
    ///
    /// 判据的次序是有意的：先问"这个名字认不认识"，再问"是不是你的"，最后才问"有没有人
    /// 占着"——**权威在属主，不在占用**（别人占没占着，不是请求者该知道的事）。
    ///
    /// `alive` = "这枚会话门闩还在不在"（探能力链，由适配层注入——核心不碰门闩）。
    /// **它让 `Taken` 只表示"有一个活实例占着"**：持有者已经没了（客户端死了、内核沿派生链
    /// 把本域手里那枚摘掉）⇒ 这一行其实空着，实例**当场自愈**（行保留、实例摘空——与
    /// [`Lines::retire`] 是同一笔账，只是触发者从"投递失败"换成"新人来登记"）。
    ///
    /// 为什么非有它不可：收线那条路的触发点是**一次投递失败**（§12 ②），而"客户端死了、
    /// 在新实例登记之前**没有过任何投递**"是常态（§9.14）——没有这一步，root 重发出来的
    /// 新实例会拿到 `Taken` 而永远登不上记。
    pub fn register(
        &mut self,
        name: &Name,
        from: TaskId,
        session: usize,
        alive: impl Fn(usize) -> bool,
    ) -> Result<u32, Refused> {
        let row = self.row_mut(name).ok_or(Refused::Unknown)?;
        if row.who.get() == 0 {
            return Err(Refused::Unclaimed);
        }
        if row.who != from {
            return Err(Refused::NotYours);
        }
        if let Some(held) = row.holder {
            if alive(held) {
                return Err(Refused::Taken);
            }
            row.holder = None;
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

impl Line {
    /// 本行的写者 = 本域的 `sire`，或 root 委托过的那个（一个）。
    fn writable_by(&self, task: TaskId) -> bool {
        task == self.sire || self.writer == Some(task)
    }
}
