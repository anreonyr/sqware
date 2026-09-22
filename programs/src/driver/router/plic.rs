//! plic — PLIC（中断控制器）的**设备侧**：寄存器视图 + 这台控制器自己的事实。
//!
//! "设备侧"的意思是它只认识**这台控制器**：寄存器布局、有几条线、有几个 context。
//! 它**不认识任何设备**——不知道 `serial@10000000` 后面是串口还是网卡；也不含服务循环、
//! 不含配给、不含投递（那些在 `src/plic/`，装配与适配）。
//!
//! # 它为什么要读设备树
//!
//! 两件事只有树里有：**这台控制器有几条线**（`riscv,ndev`）与**本域该用哪个 context**
//! （`interrupts-extended` 的项序，`cell == 9` 才是 S 模式外部中断）。内核不代劳——
//! 它只把设备树原样搬给域（`platform/devices.rs::supply_dtb`）。
//!
//! # 本域要哪三样
//!
//! 要哪几样、多少权、以什么形态出去，写在**本域自己开的**需求单里
//! （[`crate::driver::router::needs`]）：本域收到记录后**按位次**认领自己那几格——**控制器
//! 按类**（`compatible`，编排域读树把类定成那一段区），**设备树本体与门铃按坐标本身**
//! （`Key::dtb()` / `Key::irq()`：它们不是树里的节点）。名字不在本文件里第二遍。
//! 本模块只管这台控制器自己——寄存器布局、几条线、哪个 context。
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

use env::{Key, Name};
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
    device_count: u32,
    /// 本域用的**那一个** context。
    ///
    /// `claim` / `complete` 是 **per-context** 的：一条线若在多个 context 上使能，
    /// 中断可能投给 A context，而本域去 B context 领——领回 0，源头却一直挂着，
    /// 下一次还会再报，于是空转。故**线接在哪个 context 上，就从哪个 context 领**：
    /// 两件事同一个数，没有第二个数可以不一致。
    ctx: u32,
}

impl Plic {
    /// 读设备树：认控制器、读线数、定下本域用的 context，并把**要接的线**连同
    /// **没进来的那几笔账**一起交出去（见模块头）。
    ///
    /// 认控制器用的那个类（`compatible`）与单子上那一格是**同一个常量**
    /// （[`super::needs::PLIC`]）——"我是哪台控制器"这个断言只有一处。
    ///
    /// 返的第二件是**源账**（每条带区与线号，另加那几笔没进来的账）：登记那一趟按
    /// [`Sources::line_of`] 解"区 → 线号"——**那条权威只在这一处**。
    pub fn new(view: View, dtb: View) -> Option<(Self, Sources)> {
        // SAFETY: `dtb` 是内核只读借映进本域的整棵设备树（保留区，终身存活）；只读。
        let fdt = unsafe { fdt::Fdt::from_ptr(dtb.base() as *const u8) }.ok()?;
        let node = fdt.all_nodes().find(|n| {
            n.property("interrupt-controller").is_some()
                && n.compatible()
                    .is_some_and(|c| c.all().any(|s| s == super::needs::PLIC))
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
        let this = Self {
            view,
            device_count,
            ctx,
        };
        let sources = sources(&fdt, &node, device_count, cells);
        Some((this, sources))
    }

    /// 本域用的那个 context 号（日志与判据用；它是本域自己的账，不是别人的）。
    pub fn context(&self) -> u32 {
        self.ctx
    }

    /// 本控制器自报的线数（`riscv,ndev`）。**静音账的容量按它校验**（见 `main` 那一步）。
    pub fn device_count(&self) -> u32 {
        self.device_count
    }

    /// 接上一条线：写优先级 + 本 context 的阈值与 enable。
    ///
    /// 阈值恒 0（不卡仲裁）。线上限由控制器自报的 `device_count` 把握——越界不是错误，是"这条线
    /// 不在这台控制器上"，接了也没用。
    pub fn enable(&self, line: u32, priority: u32) {
        if line < 1 || line > self.device_count {
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
    /// 它当刹车的作用是**不白叫醒客户**（实测：临时去掉这一手，短跑里就多出两次
    /// `uart: rang n=0`——设备那一格已经空了，客户被叫起来排到 0 字节）。**"空转成风暴"那
    /// 句话的来源是另一格**：设备里那一格没清（`rtc` 实测：把 `CLEAR_INTERRUPT` 那一手去掉，
    /// 同一段运行里投递从 5 次变 3093 次）。
    ///
    /// **为什么不是"压着不结"**（`deliver` 之后不 `complete`、押到客户排空）：它同样防得住
    /// 白叫醒，但**结是那一格的再武装**——见 [`Plic::complete`]。实测：故意不结 line 10 ⇒
    /// 只投递一次，之后第二次输入再也进不来（回显与停机都没了）。那一格裁在
    /// `protocol::driver::line` 的"静音还是压着不结"一节里。
    pub fn disable(&self, line: u32) {
        if line < 1 || line > self.device_count {
            return;
        }
        self.write(PRIORITY + 4 * line as usize, 0);
    }

    /// 拆线：**把这一条从本 context 摘出去**——优先级归零 + 清掉 enable 位（[`Plic::enable`]
    /// 的反面）。
    ///
    /// 与 [`Plic::disable`] 不是一回事：静音留着 enable 位（复原走 `enable`，那一格的账没动），
    /// 拆线是"这条线不归本域管了"——主人没了才做，要再接上只能重新登记一次（`occupy`）。
    pub fn unwire(&self, line: u32) {
        if line < 1 || line > self.device_count {
            return;
        }
        self.write(PRIORITY + 4 * line as usize, 0);
        let e = ENABLE + ENABLE_STRIDE * self.ctx as usize + 4 * (line / 32) as usize;
        let bits = self.read(e) & !(1 << (line % 32));
        self.write(e, bits);
    }

    /// 领一条线号；**0 = 没有可领的**（不是错误：别的 context 可能已经领走了）。
    pub fn claim(&self) -> u32 {
        self.read(CONTEXT + CONTEXT_STRIDE * self.ctx as usize + CLAIM)
    }

    /// 结一条线（把线号写回去）。
    ///
    /// **它是那一格的再武装，不是礼貌**（QEMU `hw/intc/sifive_plic.c`）：领走那一步（读
    /// `claim`）当场置 `claimed`，而仲裁只看 `pending & ~claimed & enable` ⇒ **结没写回去，
    /// 那一条线就再也不前送**。`claimed` 在那份实现里还是**整台控制器一份**（不像 `enable`
    /// 按 context 各一份）⇒ 一笔没付出去的结把这条线钉死到重启。
    ///
    /// 实测：故意不结 line 10 ⇒ 只投递一次，之后第二次输入再也进不来（回显与停机都没了）。
    ///
    /// 故本域**当场结清**（`disable` 挡在它之前——那一手是"这一条现在有没有人接"），
    /// 而不是把它押到客户排空那一刻。旧注写的是"`complete` 不会把中断带回来"——**照实记**：
    /// 那句话与 `disable` 那一注互相矛盾，且两句都只说了一半；今天两句都按读数重写了。
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

/// 树里指到本控制器的中断源（**那一段区 + 线号**）+ **没进来的那几笔账**。
///
/// 每一项都对应一条"没进 `lines` 的理由"，都是读数不是判断——起域时打一行（见 `main`），
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

impl Sources {
    /// 坐标 → 那条线（**权威只在这一处**）；名字随那条一起给出来，好打日志。
    /// 查不到 ⇒ 树里没这条线（那个坐标不是中断源）。
    pub fn line_of(&self, key: Key) -> Option<&Source> {
        self.lines.iter().find(|s| s.key == key)
    }
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
) -> Sources {
    let mut out = Sources {
        lines: Vec::new(),
        unparented: 0,
        beyond: 0,
        mapped: 0,
        unparsed: 0,
        unregion: 0,
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
