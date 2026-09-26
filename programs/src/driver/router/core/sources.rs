//! router::core::sources — **这台控制器自己的事实 ＋ 它那几条线**：设备树 → （线数 / context / 区 ↔ 线号）。
//!
//! 本文件**不碰寄存器、不碰内核**：不出现 `runtime::`、不出现 `View`——只吃**设备树那段字节**
//! （适配层把借映进来的那段切好交给它），故可独立推理。寄存器那一半住 `../plic.rs`（设备面）。
//!
//! # 它为什么要读设备树
//!
//! 两件事只有树里有：**这台控制器有几条线**（`riscv,ndev`）与**本域该用哪个 context**
//! （`interrupts-extended` 的项序，`cell == 9` 才是 S 模式外部中断）。内核不代劳——
//! 它只把设备树原样搬给域（`platform/devices.rs::supply_dtb`）。
//!
//! 认控制器用的那个类（`compatible`）与单子上那一格是**同一个常量**
//! （[`PLIC_CLASS`]，住 `plan::assembly` 那张单子旁边）——"我是哪台控制器"这个断言只有一处。
//!
//! # 线集合怎么来的（以及哪两类源**不**进来）
//!
//! 线号 = 中断说明符的**第一个单元**：`#interrupt-cells` 数 1 或 2 的控制器都成立
//! （第一格是中断号，第二格是触发类型）。两类源不进这张表：
//!
//! - 带 `interrupt-map` 的节点（PCIe 那一台）——中断由 map 描述，本域**不解析** map；
//! - `#interrupt-cells` 取别的值的控制器——本域**拒解**，不猜那一格的含义。
//!
//! 「指到本控制器、但本节点没写 `interrupt-parent`」（绑定允许沿父链继承，本域今天不追）
//! 与「线号越出控制器自报的 `device_count`」也各记一笔。**没进来的每一笔都是一条读数**
//! （[`Sources`]）：线集合因此可复核，不靠注释声称。

use alloc::vec::Vec;

use env::Name;
use plan::{Key, assembly::PLIC_CLASS};

/// S 模式外部中断的中断号：`interrupts-extended` 里 `cell == 9` 的那一项。
///
/// **认 9 不认 11**（11 = M 模式）——认错就是把中断线交给固件。也**不硬算 `2h+1`**：
/// 项序是绑定的定义，算术不是。
const EXT_S: u32 = 9;

/// 树里指到本控制器的中断源（**那一段区 + 线号**）+ **没进来的那几笔账** + 这台控制器的事实。
///
/// 每一项都对应一条"没进 `lines` 的理由"，都是读数不是判断——起域时打一行（见 `adapt/boot.rs`），
/// 让这台机器上的线集合可复核，不靠注释声称。
///
/// **区是这一格的关键**：「线 = 区的函数」那条关系就落在这里——客户登记时报的是**它手里那件
/// 东西的区**（内核就是按 `reg` 段造门闩的），解树只发生这一处（内核不代劳，它连线号都不知道）。
/// 名字只是**日志**：当场从树里读出来，装不下就不打（不影响这条件成立）。
pub struct Sources {
    /// 要接的线，落在 `[1, device_count]` 内（线数的权威是控制器自己）。
    pub lines: Vec<Source>,
    /// 有 `interrupts`、但**本节点自己没写** `interrupt-parent` 的节点数
    /// （绑定允许沿父链继承；本域今天不追父链）。
    pub unparented: usize,
    /// 指到本控制器、但线号越出 `[1, device_count]` 的节点数。
    pub beyond: usize,
    /// 带 `interrupt-map` 的节点数（PCIe 那一类）——它的中断由 map 描述，本域不解析。
    pub mapped: usize,
    /// 指到本控制器、但 `#interrupt-cells` 不是 1 / 2 的节点数——本域**拒解**那一格。
    pub unparsed: usize,
    /// 指到本控制器、但**报不出区**的节点数（没有 `reg`，或 `reg` 都不是一段区：零址 / 零长）
    /// ——客户只有区可报，故这台报不出来。
    pub unregion: usize,
    /// 本控制器有多少条线（`riscv,ndev`）——按它拒绝越界的线号，也按它备账。
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
    /// 读设备树：认控制器、读线数、定下本域用的 context，并把**要接的线**连同
    /// **没进来的那几笔账**一起交出去（见模块头）。
    ///
    /// 返的第二件是**源账**（每条带区与线号，另加那几笔没进来的账）：登记那一趟按
    /// [`Sources::line_of`] 解"区 → 线号"——**那条权威只在这一处**。
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
        Some(sources(&fdt, &node, device_count, cells, ctx))
    }

    /// 本域用的那个 context 号（日志与判据用；它是本域自己的账，不是别人的）。
    pub fn context(&self) -> u32 {
        self.ctx
    }

    /// 本控制器自报的线数（`riscv,ndev`）。**账的容量按它校验**（见 `adapt/boot.rs`）。
    pub fn device_count(&self) -> u32 {
        self.device_count
    }

    /// 坐标 → 那条线（**权威只在这一处**）；名字随那条一起给出来，好打日志。
    /// 查不到 ⇒ 树里没这条线（那个坐标不是中断源）。
    pub fn line_of(&self, key: Key) -> Option<&Source> {
        self.lines.iter().find(|s| s.key == key)
    }
}

/// 一条中断源：**那一段区 + 线号**（`name` 只为日志；装不下 ⇒ `None`）。
///
/// 区取节点的**第一段有效的** `reg`：内核就是按 (节点, `reg` 段) 造门闩的，且跳过零址 / 零长
/// 的那种写法（`devices.rs`）——本表跟着跳过同一批，故**两侧报出来的基址逐字相同**（两侧读
/// 同一棵树，同一条"哪一段才算"的规矩各写了一遍）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Source {
    pub key: Key,
    pub line: u32,
    pub name: Option<Name>,
}

/// 扫一遍树：收线号，顺手把"没进来的"分成五笔。
///
/// 判据只有一条，且只问树：`interrupt-parent` 等于本控制器的 phandle（**只认节点自己写的**，
/// 不沿父链继承——没写的那一类记进 `unparented`）。线号取中断说明符的**第一个单元**
/// （见模块头）；越出 `device_count` 的不接。
fn sources(
    fdt: &fdt::Fdt,
    controller: &fdt::node::FdtNode,
    device_count: u32,
    cells: usize,
    ctx: u32,
) -> Sources {
    let mut out = Sources {
        lines: Vec::new(),
        unparented: 0,
        beyond: 0,
        mapped: 0,
        unparsed: 0,
        unregion: 0,
        device_count,
        ctx,
    };
    // 控制器自己没有 phandle ⇒ 树里没有任何节点指得到它 ⇒ 空表（这不是错误：
    // 那棵树在说"没有指向它的中断源"）。
    let Some(phandle) = controller.property("phandle").and_then(|p| p.as_usize()) else {
        return out;
    };
    for node in fdt.all_nodes() {
        if node.property("interrupt-map").is_some() {
            // 走 map 的那一类：它的中断挂在 map 里，不在 `interrupts` 里。
            out.mapped += 1;
            continue;
        }
        match node.property("interrupt-parent").and_then(|p| p.as_usize()) {
            Some(parent) if parent == phandle => {}
            Some(_) => continue,
            None => {
                if node.property("interrupts").is_some() {
                    out.unparented += 1;
                }
                continue;
            }
        }
        if !matches!(cells, 1 | 2) {
            // 说明符的单元数不是 1 / 2 ⇒ 第一格未必是中断号：**不猜**。
            if node.property("interrupts").is_some() {
                out.unparsed += 1;
            }
            continue;
        }
        let Some(line) = first_cell(node) else {
            // 指到本控制器、但没有可解的 `interrupts`：它不是中断源，不入账。
            continue;
        };
        if line < 1 || line > device_count {
            out.beyond += 1;
            continue;
        }
        // 客户的坐标是内核按**有效的** `reg` 段造的 ⇒ 本表也取第一段有效的（零址 / 零长不算
        // 一段区，内核那侧同样跳过——两侧同一条规矩，各写了一遍，见 `platform/devices.rs`）。
        let Some(base) = node.reg().and_then(|regs| {
            regs.filter(|r| r.size.is_some_and(|size| size != 0))
                .map(|r| r.starting_address as usize)
                .find(|base| *base != 0)
        }) else {
            // 没有**有效**的 `reg` 段（或压根没有 `reg`）：客户只有区可报，故这台报不出来。
            out.unregion += 1;
            continue;
        };
        // 名字**只为日志**：装不下就不打（坐标是区，这条线照收）。
        let name = Name::new(node.name).ok();
        out.lines.push(Source {
            key: Key::region(base as u64),
            line,
            name,
        });
    }
    out
}

/// 节点的第一个中断号（`interrupts = <10>` ⇒ 10）。
///
/// 认**第一个单元**：`#interrupt-cells` 数 1 或 2 时它都是中断号（第二格是触发类型）。
/// 别的取值由调用方拒解（见 [`sources`] 的 `unparsed`）。
fn first_cell(node: fdt::node::FdtNode) -> Option<u32> {
    let value = node.property("interrupts")?.value;
    Some(u32::from_be_bytes(value.get(..4)?.try_into().ok()?))
}
