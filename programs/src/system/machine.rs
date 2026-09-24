//! machine — **编排域手里那台机器的自述**：设备树读一次，此后只读。
//!
//! # 它为什么住在这里
//!
//! 单子上那一格写的是**类**（`compatible` 串，收方的专业），而"这一类是哪一段区"是**树**说的
//! 事。两半合起来才是一条要得出去的坐标，于是"读树"必须发生在**造单子的那一域**：
//!
//! - **驱动自己不行**：单子不是它递的（子方只认得生我者，递单的是编排域）——它压根没有
//!   "开单"这一步。
//! - **引导域不行**：类只出现在单子上，而单子是编排域造的；放它那里等于让固件开始理解
//!   单子上的语义（它今天一个字不解释），且那块账里的记录与它自己再扫一遍树要对齐。
//!
//! 故本模块只做两件事：**类 → 那一段区**（[`Machine::site_of`]）与**读 `/chosen` 拿载荷区的
//! 坐标**（[`Machine::payload`]）。它不解释设备语义（类串是收方给的），也不持有任何设备
//! ——它只是把机器自己写的那份自述读出来。
//!
//! # 核心与适配
//!
//! 这里全是**核心**：视图进来、坐标出去，没有会话、没有门闩、没有失败策略。领树那一手
//! （递单 + `Dock::open`）在 `system::main` 里（[`Machine::of`] 之前那几行）。

use plan::Key;
use runtime::core::dock::View;

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

    /// 认设备：**类 → 那一段区**（`reg` **首段**的起点）。
    ///
    /// 契约：命中的多台里取 **`reg` 首址最小**的一台。**类不是单值**（实测 8 台
    /// `virtio,mmio`），而"取树的书写顺序第一条"会让读数跟着树怎么写走——地址才是机器的实情。
    /// 没有 `reg` 的节点**不参与**：内核就是按 `reg` 段造门闩的，那种节点根本没有门闩可取。
    /// 失败：`None` = 树里没有这一类（不是错误：单子要的东西这台机器上没有）。
    ///
    /// **照实记（两侧同一条规矩，写了两遍）**：零址 / 零长的 `reg` 不代表一段区——内核那侧就是
    /// 这么跳过它的（`devices.rs`），本函数取**第一段有效的**（同一对条件，写在这里）。不这么办
    /// 的话，"首址最小"那把尺会拿一段**内核没造过**的区去比，把真有门闩的那一台比下去。要收成
    /// 一处，得让"哪一段才算"这条定义只写一遍——那是"第几段"那一格的事（与"第几台"同一类）。
    ///
    /// **照实记**：多段 `reg` 的设备只认**首段**（单子上没有"第几段"这一格，与"第几台"同一类
    /// 问题）；今天 virt 上无人要那种设备。
    pub fn site_of(&self, class: &str) -> Option<Key> {
        let mut hit: Option<(usize, Key)> = None;
        for node in self.fdt.all_nodes() {
            if !node
                .compatible()
                .is_some_and(|c| c.all().any(|s| s == class))
            {
                continue;
            }
            let Some(base) = node.reg().and_then(|regs| {
                regs.filter(|r| r.size.is_some_and(|size| size != 0))
                    .map(|r| r.starting_address as usize)
                    .find(|base| *base != 0)
            }) else {
                continue;
            };
            if hit.is_none_or(|(best, _)| base < best) {
                hit = Some((base, Key::region(base as u64)));
            }
        }
        hit.map(|(_, key)| key)
    }

    /// 载荷区：`/chosen` 的 `linux,initrd-start` → 那一段区。
    ///
    /// 契约：**只读那一格属性，不做任何换算**（`end` 的页取整是内核那一侧的事，不进坐标——
    /// 坐标是键，键取直接读到的那一个数）。失败：`/chosen` 里没有那一格 ⇒ `None`。
    ///
    /// **照实记**：`end` 不读，内核那三条守卫（`start != 0` / `end > start` / 页对齐）也不重复
    /// ——它们决定的是**内核造不造那一条记录**。故在一棵写了坏值的树上，这里问到的坐标在块里
    /// 没有：装配当场停在 `system: payload ask`（**取不到，不是静默取错**）。
    pub fn payload(&self) -> Option<Key> {
        let chosen = self.fdt.find_node("/chosen")?;
        let start = chosen.property("linux,initrd-start")?.as_usize()?;
        Some(Key::region(start as u64))
    }
}
