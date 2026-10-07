//! 读取设备树中的设备名称、区域坐标与中断线路。

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use env::{Name, Page};
use ::resource::dock::View;

use crate::unit::router::PLIC_CLASS;

/// 可派发的设备区域及其驱动信息；line 为零表示没有本控制器的中断线路。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Device {
    pub resource: Name,
    pub name: String,
    pub class: String,
    pub line: u32,
}

/// **引导期那两件不按类认的东西**的坐标由它们自己说（`Name::Page(Page::Dtb)` / `Name::Trap(env::Trap::SupervisorExternal)`）——
/// 它们不进 Machine::devices 那张表（树里没有"哪一类"可判），由装配者按**已知坐标**要
#[derive(Clone, Copy)]
pub struct Machine {
    fdt: fdt::Fdt<'static>,
}

impl Machine {
    /// 把一段**已借映的只读区**解释成设备树
    /// 前置：`view` 指向终身存活、只读、形状合法的 FDT（`Name::Page(Page::Dtb)` 那一枚门闩的视图）
    pub fn of(view: View) -> Result<Machine, &'static str> {
        let fdt = unsafe { fdt::Fdt::from_ptr(view.base() as *const u8) }
            .map_err(|_| "system: tree parse")?;
        Ok(Machine { fdt })
    }

    /// 按区域起址排列，选择每个设备的首段有效区域。
    pub fn devices(&self) -> Option<Vec<Device>> {
        let plic = Plic::of(&self.fdt);
        let mut out: Vec<Device> = Vec::new();
        for node in self.fdt.all_nodes() {
            if exempt(node.name) {
                continue;
            }
            let Some(class) = node
                .compatible()
                .and_then(|c| c.all().next())
                .map(|c| c.to_string())
            else {
                continue;
            };
            // 名字装不下（长度那一字节放不下 / 那一帧的缓冲装不下）⇒ 这一台**落不了格**
            // 不猜一个截短的名字：那会让两台不同的设备撞成同一格。
            let name = node.name.to_string();
            let Some(base) = first_region(node) else {
                continue;
            };
            out.try_reserve(1).ok()?;
            out.push(Device {
                resource: Name::Page(Page::Region(base as u64)),
                name,
                class,
                line: plic.as_ref().and_then(|p| p.line_of(node)).unwrap_or(0),
            });
        }
        // **区升序**：读数是"哪一台"（"取首址最小的一台"那条旧判据仍在，只是它现在由
        // 这张表自己说了算；`list` 那一条路的第一条因此仍是同一台）。
        out.sort_by_key(|d| d.resource.base().unwrap_or(0));
        Some(out)
    }

    /// 设备树声明了启动载荷时，返回其固定资源名。
    pub fn payload(&self) -> Option<Name> {
        let chosen = self.fdt.find_node("/chosen")?;
        let start = chosen.property("linux,initrd-start")?.as_usize()?;
        if start == 0 { return None; }
        Some(Name::Page(Page::Initrd))
    }
}

/// **内核不给门闩的那两族**（照抄 `kernel/src/platform/devices.rs::exempt`）
fn exempt(name: &str) -> bool {
    let stem = name.split('@').next().unwrap_or(name);
    matches!(stem, "memory" | "clint")
}

/// 节点的**首段有效 `reg`** 的起址（零址 / 零长不算一段区）
/// 与内核那一侧同一条规矩（`devices.rs`：`base == 0 || size == 0` 的段跳过），也与旧
/// `site_of` 那一格同一条——**两侧读同一棵树，各写了一遍**（内核在引导期、本层在运行期）
fn first_region(node: fdt::node::FdtNode) -> Option<usize> {
    node.reg().and_then(|regs| {
        regs.filter(|r| r.size.is_some_and(|size| size != 0))
            .map(|r| r.starting_address as usize)
            .find(|base| *base != 0)
    })
}

/// **这台控制器那两个数 ＋ 线号那一问**：树里找控制器，读它自报的线数，再把"这一台指哪条线"
/// 答出来。没这台控制器 ⇒ 没有 `Plic`，所有线号是 `0`
struct Plic {
    phandle: usize,
    device_count: u32,
    /// 一个中断说明符几单元（`#interrupt-cells`）。不是 1 / 2 ⇒ **拒解**（不猜第一格的含义）
    cells: u32,
}

impl Plic {
    fn of(fdt: &fdt::Fdt) -> Option<Plic> {
        let node = fdt.all_nodes().find(|n| {
            n.property("interrupt-controller").is_some()
                && n.compatible()
                    .is_some_and(|c| c.all().any(|s| s == PLIC_CLASS))
        })?;
        Some(Plic {
            phandle: node.property("phandle").and_then(|p| p.as_usize())?,
            device_count: node
                .property("riscv,ndev")
                .and_then(|p| p.as_usize())
                .unwrap_or(0) as u32,
            cells: node
                .property("#interrupt-cells")
                .and_then(|p| p.as_usize())
                .unwrap_or(1) as u32,
        })
    }

    /// 这一台指不指到本控制器（以及是哪条线）。**答 `0` 的情形与写法**见 Machine::devices
    fn line_of(&self, node: fdt::node::FdtNode) -> Option<u32> {
        if node.property("interrupt-map").is_some() {
            return None;
        }
        if !matches!(self.cells, 1 | 2) {
            return None;
        }
        if node.property("interrupt-parent").and_then(|p| p.as_usize()) != Some(self.phandle) {
            return None;
        }
        let value = node.property("interrupts")?.value;
        let line = u32::from_be_bytes(value.get(..4)?.try_into().ok()?);
        (line >= 1 && line <= self.device_count).then_some(line)
    }
}
