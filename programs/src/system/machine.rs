//! machine — **编排域手里那台机器的自述**：设备树读一次，此后只读。
//!
//! 判据：单子上那一格写的是**类**（`compatible` 串，收方的专业），而"这一类是哪一段区"是**树**说的
//! 事。两半合起来才是一条要得出去的坐标，于是"读树"必须发生在**造单子的那一域**。
//!
//! 故本模块只做三件事：**枚举全机的可领之物**（[`Machine::devices`]——名 / 类 / 区 / 线四格一次
//! 说齐）、**读 `/chosen` 拿载荷区的坐标**（[`Machine::payload`]）、以及按**已知坐标**要那两件
//! （它不解释设备语义：类串是收方给的；也不持有任何设备——它只是把机器自己写的那份自述读出来）。
//!
//! **照实记（`site_of` 退场：这一格的权威搬去了设备账）**：本模块原先只答两件事——"类 → 那一段
//! 区"（`site_of`：命中多台里取 `reg` 首址最小的那台）与 `/chosen`。它长在**装配者**身上，因为
//! 递单的人要沿类把坐标定下来。这一刀把"谁回答'这台是哪一段区 / 哪一条线'"整个搬进了设备账
//! 那一台（`system/hub`）⇒ 枚举那一手从此长在**读树的地方**，而 [`Machine::devices`] 是它的
//! **唯一实现**：hub 用全部四格（名 / 类 / 线 / 区），装配者只用它的 `key`（照单取源）。
//! 旧那一格（第一个 `compatible` 命中的最小首址）恰好就是**新表按区升序的头一台**，故四台驱动
//! 认到的是同一台设备——读数逐字不变。
//!
//! 这里全是**核心**：视图进来、坐标出去，没有会话、没有门闩、没有失败策略。

use alloc::vec::Vec;

use env::{Key, Tag};
use runtime::core::dock::View;

use crate::program::router::PLIC_CLASS;

/// **树上一台可领的设备**：那一段区 ＋ 它叫什么 ＋ 它属哪一类 ＋ **它是哪条线**。
///
/// **四格各有各的消费者**：`key` → 装配者向引导域领那一枚门闩（只有它认坐标）；
/// `name` → hub 落 `/dev/<类>/<名>` 的那一段、驱动那行读数；`class` → 那一格 `/dev/<类>` 那块
/// Pane 与认领的盟；`line` → 契里那一格（路由者按它接线、客户按它登记）。
///
/// **`line == 0` 是一句诚实的答话**（不是"没算出来"）：这台设备不是本控制器的中断源
/// （控制器自己、以及那些没写 `interrupt-parent` 的节点）。要占线的驱动拿到 0 就知道"这台没有线"。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Device {
    pub key: Key,
    pub name: Tag,
    pub class: Tag,
    pub line: u32,
}

/// **引导期那两件不按类认的东西**的坐标由它们自己说（`Key::dtb()` / `Key::irq()`）——
/// 它们不进 [`Machine::devices`] 那张表（树里没有"哪一类"可判），由装配者按**已知坐标**要
/// （见 `system/control/assemble.rs` 的入册那一趟）。
/// 本域手里那台机器的自述。
pub struct Machine {
    fdt: fdt::Fdt<'static>,
}

impl Machine {
    /// 把一段**已借映的只读区**解释成设备树。
    ///
    /// 前置：`view` 指向终身存活、只读、形状合法的 FDT（`Key::dtb()` 那一枚门闩的视图）。
    /// 失败：`Err` = 头读不懂（那条路在 `main` 里印出来，本模块不印）。
    pub fn of(view: View) -> Result<Machine, &'static str> {
        // SAFETY: `view` 是设备树本体那一枚门闩借映进来的整段保留区——它在**本域存活期间**
        // 一直有效（门闩在本域表里，本域到收场才退出），只读（授权不含 `STORE`），
        // 故借出来的树和它里面的字节活一样久；本域只解析、不写。
        let fdt = unsafe { fdt::Fdt::from_ptr(view.base() as *const u8) }
            .map_err(|_| "system: tree parse")?;
        Ok(Machine { fdt })
    }

    /// **这台机器上每一台可领的设备**（按**区升序**）——读一次树，四格一起给。
    ///
    /// # 三条判据（一条不落地对着内核那一侧写）
    ///
    /// 1. **有 `compatible`**：类串是这一轴的钥匙（没有它的节点——`/memory`、`/chosen`、
    ///    `/cpus`——不是"某一类设备"，它们要么另有坐标（`Key::dtb()` 那两件），要么压根不发）。
    /// 2. **`exempt` 那两族不要**（`memory` / `clint`）：[`exempt`] 抄的是**内核那一侧**的同一条
    ///    规矩（`kernel/src/platform/devices.rs::exempt`）——那里跳过它们、**不给门闩**。抄漏了
    ///    的后果是**两头对不上**：装配者以为能领，引导域答 `Unknown`（整段入册当场失败）；
    ///    更要紧的是**定时器那一台**（`clint@2000000`：它有 `compatible`、内核不给门闩）会以
    ///    `/dev/sifive,clint0/…` 的身份落进设备账——**那一段是内核的滴答**。
    /// 3. **取首段有效的 `reg`**（零址 / 零长不算一段区）：内核就是按 `reg` 段造门闩的，
    ///    多段 `reg` 的设备只认首段（今天 virt 上无人要那种设备）。
    ///
    /// 类取**第一个** `compatible`（绑定里写得最具体的那一个）：一台设备只落一格 `/dev/<类>/<名>`
    /// 才说得通"名在册里唯一"。今天三台驱动的类都写在节点头一位。
    ///
    /// 线号：指到**本控制器**（`interrupt-controller` ＋ [`PLIC_CLASS`]）的那些，取
    /// `interrupts` 首格、且落在 `[1, riscv,ndev]`；其余一律 `0`（见 [`Device::line`]）。
    /// **只认节点自己写的 `interrupt-parent`**（不沿父链继承）、`#interrupt-cells` 不是 1 / 2
    /// 的不猜、带 `interrupt-map` 的（PCIe）不解析——三条都与从前路由者那一份读数同口径。
    ///
    /// 失败：`None` = **备不下那张表**（调用方按"这一台机器读不出来"处置——两个调用点都是起手
    /// 那一步，答案相同：这一台起不来）。
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
                .and_then(|c| Tag::new(c))
            else {
                continue;
            };
            // 名字装不下（> 31 字节）⇒ 这一台**落不了格**（`/dev/<类>/<名>` 那一段就是它）。
            // 不猜一个截短的名字：那会让两台不同的设备撞成同一格。
            let Some(name) = Tag::new(node.name) else {
                continue;
            };
            let Some(base) = first_region(node) else {
                continue;
            };
            out.try_reserve(1).ok()?;
            out.push(Device {
                key: Key::region(base as u64),
                name,
                class,
                line: plic.as_ref().and_then(|p| p.line_of(node)).unwrap_or(0),
            });
        }
        // **区升序**：读数是"哪一台"（"取首址最小的一台"那条旧判据仍在，只是它现在由
        // 这张表自己说了算；`list` 那一条路的第一条因此仍是同一台）。
        out.sort_by_key(|d| d.key.parts().1);
        Some(out)
    }

    /// 载荷区：`/chosen` 的 `linux,initrd-start` → 那一段区。
    ///
    /// 契约：**只读那一格属性，不做任何换算**（`end` 的页取整是内核那一侧的事，不进坐标——
    /// 坐标是键，键取直接读到的那一个数）。失败：`/chosen` 里没有那一格 ⇒ `None`。
    pub fn payload(&self) -> Option<Key> {
        let chosen = self.fdt.find_node("/chosen")?;
        let start = chosen.property("linux,initrd-start")?.as_usize()?;
        Some(Key::region(start as u64))
    }
}

/// **内核不给门闩的那两族**（照抄 `kernel/src/platform/devices.rs::exempt`）。
///
/// 比的是 `@` 之前那一段（`memory@80000000` → `memory`）：两台设备的 `reg` 不同，可"是哪一族"
/// 只看词根。**这一条是安全相关的**：`clint` 是内核的滴答、`memory` 是内存本身。
fn exempt(name: &str) -> bool {
    let stem = name.split('@').next().unwrap_or(name);
    matches!(stem, "memory" | "clint")
}

/// 节点的**首段有效 `reg`** 的起址（零址 / 零长不算一段区）。
///
/// 与内核那一侧同一条规矩（`devices.rs`：`base == 0 || size == 0` 的段跳过），也与旧
/// `site_of` 那一格同一条——**两侧读同一棵树，各写了一遍**（内核在引导期、本层在运行期）。
fn first_region(node: fdt::node::FdtNode) -> Option<usize> {
    node.reg().and_then(|regs| {
        regs.filter(|r| r.size.is_some_and(|size| size != 0))
            .map(|r| r.starting_address as usize)
            .find(|base| *base != 0)
    })
}

/// **这台控制器那两个数 ＋ 线号那一问**：树里找控制器，读它自报的线数，再把"这一台指哪条线"
/// 答出来。没这台控制器 ⇒ 没有 `Plic`，所有线号是 `0`。
struct Plic {
    phandle: usize,
    device_count: u32,
    /// 一个中断说明符几单元（`#interrupt-cells`）。不是 1 / 2 ⇒ **拒解**（不猜第一格的含义）。
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

    /// 这一台指不指到本控制器（以及是哪条线）。**答 `0` 的情形与写法**见 [`Machine::devices`]。
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
