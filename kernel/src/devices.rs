//! devices — boot 的设备供给：**一次设备树扫描**，把每台设备变成一枚门闩。
//!
//! 内核在这里出现**一次**，之后就零设备概念（`docs/driver.md` §1）。本模块做的三件事：
//!
//! 1. 遍历设备树，对每个 (节点, `reg` 段) 造一枚 `Pole`（`Payload::Region`）；
//! 2. 给每枚配一个 `Pie`（原始自持、全权），落进 root 的权限表；
//! 3. 把「名字 + 该句柄」写成定长记录放进**配对块**，由 boot 只读借映进 root 空间。
//!
//! # 为什么名字是 basename
//!
//! 全路径最长 34 > `Name` 的 31（实测，PLIC），basename 最长 28 ⇒ 取 basename。
//! 同父下的 `@unit-address` 保证同父唯一；跨父不保证——**撞名不是内核的事**：
//! 内核不去重、不解释，域在目录里注册时撞了就是 `Register` 拒绝（§9.2）。
//!
//! # 为什么内核不留一份强引用
//!
//! `Pie` 是资源实体的**唯一强引用**（资源寿命 = 能力寿命）。内核若把造好的门闩留在
//! 静态里，设备就永远死不了——那正好是"资源寿命 ≠ 能力寿命"。故本模块**只造一次、
//! 交出去、不留底**：配对块里留下的只是 `(名字, token)`，是**供给清单**，不是第二
//! 份设备账。
//!
//! # 为什么块是静态区
//!
//! 块的内容是 boot 的供给清单，寿命就是镜像的寿命（它不回收），用门闩去管它只会多
//! 一次分配、多一个失败模式。内核恒等装载 ⇒ 静态区的地址即物理地址，借映不需要翻译。

use alloc::sync::Arc;
use alloc::vec::Vec;

use env::{Name, PAIR_LEN, Pair, PieToken};

use crate::lock::OnceLock;
use crate::machine;
use crate::work::mail;
use crate::work::mail::HoleMeta;
use crate::work::unit::gate::{self, AnyPie, GateError, Permission};
use crate::work::unit::task::Task;

/// 中断门闩在配对块里的名字（root 据此把它交给 PLIC 驱动）。
const IRQ_NAME: &str = "irq";

/// 设备树本体在配对块里的名字（它不是设备树里的节点，故名字由内核定）。
///
/// 给它的门闩与设备同形（`Payload::Region`），用途也一样：**自描述要能被原样读到**。
/// 谁需要解释设备树（如 PLIC 驱动要数自己的 context），谁就自己去解释——内核不代劳
/// （`docs/driver.md` §3.1.5）。
const DTB_NAME: &str = "devicetree";

/// 中断门闩的载荷上限——**1 字节，内容恒为零**：内核只知道"有外部中断"这一件事，
/// 线号由 PLIC 驱动自己去 PLIC 里领（`docs/driver.md` §3.2.3）。
const IRQ_MTU: usize = 1;

/// 中断门闩的资源实体：**内核永久持源**（§3.2.3 明文裁决）。
///
/// 这是"资源寿命 = 能力寿命"的**有意例外**，理由在方向上：它不是谁的资源，是内核
/// 一件事实的出口。若它也随最后一份门闩消亡，域只要放下手里那份就能把中断面拆掉，
/// 而 `trap_handler` 还会继续往里推（推到一具尸体上）。
static IRQ: OnceLock<Arc<HoleMeta>> = OnceLock::new();

/// 把"有外部中断"推进 `irq` 门闩（**trap 上下文**：不分配、不阻塞）。
///
/// `Err(Busy)` = 槽里还压着上一枚 ⇒ 调用方（trap 分支）据此关本 hart 的闸门。
pub(crate) fn raise_irq() -> Result<(), GateError> {
    let meta = IRQ.get().expect("irq hole not built (devices::scan)");
    // `from = 0`：推者是内核，不是哪个域（`HoleMeta.from` 的既有约定）。
    mail::hole::try_push(meta, &[0u8], 0)
}

/// 设备树本体（§3.1.5）：一段终身的、boot 给的物理区，与 initrd 走同一条保留区
/// 机制；这里额外给它一枚门闩，好让需要读它的域自己去读。
fn supply_dtb() -> (Name, AnyPie) {
    let dtb = machine::info().dtb();
    let meta = mail::pole::region(dtb.base, dtb.size, 0).expect("devicetree region");
    let pie = gate::new_pie(
        meta,
        Permission::READ | Permission::VEST | Permission::BACK,
        None,
    );
    (
        Name::new(DTB_NAME).expect("devicetree name fits"),
        AnyPie::Pole(pie),
    )
}

/// 中断门闩（§3.2.3）：一枚、`mtu = 1`、空载荷、`owner = 0`、内核永久持源。
///
/// 它进配对块，与设备同列——**它是"设备"吗**？不是：它没有一段内存、没有 `reg`。
/// 它是**一件内核侧事实的出口**（外部中断的入口在 `trap_handler`，那是内核的领地，
/// 故必须有一小块内核结构）。同一张账里放两种东西并不冲突：账记的是"boot 交出了
/// 哪些门闩"，不是"有哪些设备"。
fn supply_irq() -> (Name, AnyPie) {
    let meta = mail::hole::meta(IRQ_MTU, 0).expect("irq hole");
    assert!(IRQ.set(meta.clone()).is_ok(), "irq hole built twice");
    let pie = gate::new_pie(
        meta,
        Permission::READ | Permission::WRITE | Permission::VEST | Permission::BACK,
        None,
    );
    (
        Name::new(IRQ_NAME).expect("irq name fits"),
        AnyPie::Hole(pie),
    )
}

/// 配对块容量（条）。实测 virt 带 `reg` 的节点 17 个（含 `flash` 的两段），
/// 上限给足一页（64 条 × 40 B = 2560 B）。
pub(crate) const MAX_PAIRS: usize = 64;

/// 配对块字节数——**一整页**（`borrow` 的页对齐义务；`repr(align)` 见下）。
pub(crate) const BLOCK_BYTES: usize = crate::memory::PAGE_SIZE;

/// 记录数组装得下、且块对齐即页对齐（两个都是借映的前置）。
const _: () = assert!(MAX_PAIRS * PAIR_LEN <= BLOCK_BYTES && BLOCK_BYTES == 4096);

/// 借映进 root 空间的配对块（页对齐，故可直接 `borrow`）。
///
/// 写入只发生在 boot 单核期（`spawn_root` 早于 `boot_harts`），此后只读。
#[repr(C, align(4096))]
struct Block(core::cell::UnsafeCell<[u8; BLOCK_BYTES]>);

// SAFETY: 写入限于 boot 单核期、此后只读（见上）；读者（root）读到的是写完的内容
// ——借映的 PTE 在写完之后才建立。
unsafe impl Sync for Block {}

static BLOCK: Block = Block(core::cell::UnsafeCell::new([0u8; BLOCK_BYTES]));

/// 配对块的（物理首址，字节数）——boot 借映它、把这两个数经启动参数交给 root。
pub(crate) fn block() -> (usize, usize) {
    (core::ptr::addr_of!(BLOCK) as usize, BLOCK_BYTES)
}

/// 扫描设备树，为每台设备造一枚门闩（**不落表、不留底**——见模块头）。
///
/// 豁免两类（§3.1.6）：
/// - `memory`：内核已把它解析成 `dram`，再交出去就是第二份账；
/// - `clint`：内核的时钟与 IPI 经 SBI 走它，交出去等于交出节拍。
pub(crate) fn scan() -> Vec<(Name, AnyPie)> {
    let dtb = machine::info().dtb();
    // SAFETY: dtb 是 boot 交上来的设备树区（已进保留区，终身存活），此处只读。
    let fdt = unsafe { fdt::Fdt::from_ptr(dtb.base as *const u8) }.expect("device tree blob");
    let mut out = Vec::new();
    for node in fdt.all_nodes() {
        if exempt(node.name) {
            continue;
        }
        let Some(reg) = node.reg() else {
            continue;
        };
        for r in reg {
            let base = r.starting_address.addr();
            let Some(size) = r.size else {
                continue;
            };
            // 零长 / 零址的 `reg` 段不是一段区（设备树的合法写法），跳过即正确。
            if base == 0 || size == 0 {
                continue;
            }
            let Ok(name) = Name::new(node.name) else {
                // 名字装不下 = 这台设备没有可表达的身份。**不截断**（截断会把两台
                // 设备指成同一个名字），也不拖垮整机：报出来、跳过它。
                crate::putln!(
                    "devices: node name longer than {} bytes, skipped: {}",
                    env::NAME_LEN - 1,
                    node.name
                );
                continue;
            };
            // owner = 0：**内核给的**（`PoleMeta.owner` 的既有约定）。故没有域能
            // `Seal` 一台设备（`Seal` 要求 owner == 自己）——设备无生死可判。
            let Ok(meta) = mail::pole::region(base, size, 0) else {
                continue;
            };
            let pie = gate::new_pie(
                meta,
                Permission::READ | Permission::WRITE | Permission::VEST | Permission::BACK,
                None,
            );
            out.push((name, AnyPie::Pole(pie)));
        }
    }
    // 设备树本体与中断门闩也与设备同列（见 [`supply_dtb`] / [`supply_irq`]）：
    // 它们不是"设备"，但都是 boot 交出去的门闩——这张账记的是后者。
    out.push(supply_dtb());
    out.push(supply_irq());
    out
}

/// 把扫出来的设备交给 `task`：落它的权限表 + 写进配对块。返回块里的条数。
///
/// 所有权**移动**（不是克隆）——门闩交出去之后内核就不再有强引用（见模块头）。
pub(crate) fn install(task: &Task, items: Vec<(Name, AnyPie)>) -> usize {
    if items.len() > MAX_PAIRS {
        // 装不下就是"这台机器的设备表比内核的配对块还大"——内核不能只交一半：
        // 少掉的那台在域侧表现为"设备不存在"，而它其实存在。boot 当场停。
        panic!(
            "device tree has {} devices, pairing block holds {MAX_PAIRS}",
            items.len()
        );
    }
    let (pa, _bytes) = block();
    let mut n = 0;
    for (i, (name, pie)) in items.into_iter().enumerate() {
        let record = Pair::new(name, PieToken::new(pie.token()));
        // SAFETY: 记录数组与块同源（`Pair` 的尺寸由编译期断言锁定为 `PAIR_LEN`）；
        // 写偏移恒 < 块长（上面查过条数上限）。用 `write_unaligned` 是因为 `Block`
        // 只保证页对齐，而记录步长 40 字节——记录自身不要求对齐。
        unsafe {
            core::ptr::write_unaligned((pa as *mut u8).add(i * PAIR_LEN).cast::<Pair>(), record);
        }
        task.pies.lock().push(pie);
        n += 1;
    }
    // 供给清单打进 boot 日志：这是**这台机器上有什么**的唯一一次陈述（此后内核
    // 零设备概念，要问只能问域）。内核打印走 SBI，不碰设备（`docs/driver.md` §7.3）。
    crate::putln!("devices: {n} handed to root");
    for i in 0..n {
        let (pa, _) = block();
        let at = (pa as *const u8).wrapping_add(i * PAIR_LEN);
        // SAFETY: 上一条循环刚写完这些记录；只读。
        let record = unsafe { core::ptr::read_unaligned(at.cast::<Pair>()) };
        let name = record.name();
        crate::putln!(
            "  {} -> token {}",
            name.as_ref().map(|v| v.as_str()).unwrap_or("?"),
            record.token().get()
        );
    }
    n
}

/// 该节点是否豁免（见 [`scan`]）——按名字的**词干**（`@` 之前）判。
fn exempt(name: &str) -> bool {
    let stem = name.split('@').next().unwrap_or(name);
    matches!(stem, "memory" | "clint")
}
