//! devices — boot 的设备供给：**一次设备树扫描**，把每台设备变成一枚门闩。
//!
//! 内核在这里出现**一次半**：一次扫描 + 一枚中断门铃（`raise_irq`，`trap` 那边只知道
//! "铃响了"）。线号、属主、驱动**全在域里**。本模块做的三件事：
//!
//! 1. 遍历设备树，对每个 (节点, `reg` 段) 造一枚 `Pole`（`Payload::Region`）；
//! 2. 给每枚配一个 `Pie`（原始自持、全权），落进 root 的权限表；
//! 3. 把「名字 + 该句柄」写成定长记录放进**配对块**，由 boot 只读借映进 root 空间。
//!
//! # 为什么名字是 basename
//!
//! 全路径最长 34 > `Name` 的 31（实测，PLIC），basename 最长 28 ⇒ 取 basename。
//! 同父下的 `@unit-address` 保证同父唯一；跨父不保证——**撞名不是内核的事**：
//! 内核不去重、不解释，按名取货的那一侧自己担。
//!
//! **名字在这张账里不是单值**：扫描单位是 (节点, `reg` 段) ⇒ 同一个节点的多段 `reg`
//! 会造出**两条同名记录**（实测 virt 上 `flash@20000000` 两段 ⇒ token 4 / 5，见 boot
//! 日志）。取货一侧（引导域 `Root::token`）按名取**第一枚**，其余同名的那几枚今天没有
//! 取名机制够得到——要够到得先有"第几段"这一格，那是取货侧的账（`Root::report_pairs`
//! 至少让"这台机器上有重名"这件事有读数）。
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

use env::{Name, PAIR_LEN, Pair, TaskId};

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::lock::OnceLock;
use crate::platform::machine;
use crate::work::mail;
use crate::work::mail::nole::NoleMeta;
use crate::work::unit::gate::{self, AnyPie, GateError, Permission};
use crate::work::unit::task::Task;

/// 中断门闩在配对块里的名字（root 据此把它交给 PLIC 驱动）。
const IRQ_NAME: &str = "irq";

/// 设备树本体在配对块里的名字（它不是设备树里的节点，故名字由内核定）。
///
/// 给它的门闩与设备同形（`Payload::Region`），用途也一样：**自描述要能被原样读到**。
/// 谁需要解释设备树（如 PLIC 驱动要数自己的 context），谁就自己去解释——内核不代劳。
const DTB_NAME: &str = "devicetree";

/// initrd 载荷区在配对块里的名字（它不是设备树节点，故名字由内核定）。
///
/// 与 [`DTB_NAME`] 并排：两者都是"boot 交出去的一枚门闩"，故在同一张账里——
/// 这张账记的是**交出了哪些门闩**，不是"有哪些设备"。
const INITRD_NAME: &str = "initrd";

/// 中断门铃**没有载荷**——内核只知道"有外部中断"这一件事，线号由 PLIC 驱动自己去
/// PLIC 里领。
///
/// 于是它是一枚 **Nole**（数据面为空）而不是"1 字节的孔、内容恒 0"：那个字节从来不是
/// 内容，是一个信号。用法（等 / 应 / 响）封装在 runtime 的 `Bell` 里。
///
/// 中断门铃的资源实体：**内核永久持源**。
///
/// 这是"资源寿命 = 能力寿命"的**有意例外**，理由在方向上：它不是谁的资源，是内核
/// 一件事实的出口。若它也随最后一份门闩消亡，域只要放下手里那份就能把中断面拆掉，
/// 而 `trap_handler` 还会继续响它（响在一具尸体上）。
static IRQ: OnceLock<Arc<NoleMeta>> = OnceLock::new();

/// 这枚铃的读数：**摇了几次 / 其中几次"还响着"**（`Busy`），以及其中的**空闲核补摇**那一支
/// （`scheduler::core::fetch` 的空闲循环）。
///
/// 账落在本模块：`raise_irq` 是内核唯一的摇铃点，故"摇了几次"归它。`idle_*` 是补摇那一支
/// 的读数——那一支正是"没人可调"窗口的补丁（见 `fetch` 那一段注释），**没有读数就落不下**：
/// 它多半只在"控制器挂着而核在空闲"时才有非零值，故它为零本身也是读数（那一段没发生）。
/// 只读、`Relaxed`：收尾印那一行时全部核已过 halt 屏障，计数器不再变。
static IRQ_RING: AtomicUsize = AtomicUsize::new(0);
static IRQ_BUSY: AtomicUsize = AtomicUsize::new(0);
static IRQ_IDLE_RING: AtomicUsize = AtomicUsize::new(0);
static IRQ_IDLE_BUSY: AtomicUsize = AtomicUsize::new(0);

/// 响铃：把"有外部中断"记进门铃（**trap 上下文**：不分配、不阻塞、不搬字节）。
///
/// `Err(Busy)` = 铃还响着（上一件没人应）⇒ 调用方（trap 分支）据此关本 hart 的闸门。
/// 闸门的另一半在 `envcall/mail.rs::hush`：用户应铃时立刻重开。
pub(crate) fn raise_irq() -> Result<(), GateError> {
    IRQ_RING.fetch_add(1, Ordering::Relaxed);
    let meta = IRQ.get().expect("irq bell not built (devices::scan)");
    let r = mail::nole::ring(meta);
    if r.is_err() {
        IRQ_BUSY.fetch_add(1, Ordering::Relaxed);
    }
    r
}

/// 空闲核那一支的振铃点：与 [`raise_irq`] 同一件事，另记一格"这一次是空闲补摇的"。
///
/// 调用点只有一处（`scheduler::core::fetch` 的空闲循环，且只在 `sip.SEIP` 挂着时走）
/// ⇒ `idle_ring` 同时就是"空闲核见到 `SEIP` 挂着"的轮数，也就是照实记里那个**有界自旋**
/// 的长度（`idle_busy` = 其中消费者还没应、下一轮还要再看的）。
pub(crate) fn raise_irq_idle() -> Result<(), GateError> {
    let r = raise_irq();
    IRQ_IDLE_RING.fetch_add(1, Ordering::Relaxed);
    if r.is_err() {
        IRQ_IDLE_BUSY.fetch_add(1, Ordering::Relaxed);
    }
    r
}

/// 那四格读数（收尾时印一行）。
pub(crate) fn irq_stats() -> (usize, usize, usize, usize) {
    (
        IRQ_RING.load(Ordering::Relaxed),
        IRQ_BUSY.load(Ordering::Relaxed),
        IRQ_IDLE_RING.load(Ordering::Relaxed),
        IRQ_IDLE_BUSY.load(Ordering::Relaxed),
    )
}

/// initrd 载荷区：**与设备树逐字同一条路**（终身的、boot 给的保留区；这里额外给它
/// 一枚门闩）。区别只有权：只读（`FETCH`），因为它是**要装的字节**，不是要写的东西。
///
/// 为什么它也要成为一枚门闩：域侧要"把这块账转手出去"（引导域 → 编排域）时，
/// **裸映射转不了手**——能转的只有句柄。做成门闩之后，引导域一转手，编排域就能按
/// 自己的 VA 借映同一批物理页去解析清单、读镜像：**零拷贝**。
fn supply_initrd() -> Option<(Name, AnyPie)> {
    let initrd = machine::info().initrd()?;
    let meta = mail::pole::region(initrd.base, initrd.size, TaskId::new(0)).ok()?;
    let pie = gate::new_pie(
        meta,
        // 多读者（清单与每一颗镜像都在里头，收方各自借映）⇒ 共享、授出即复制。
        Permission::FETCH | Permission::VEST,
        None,
    );
    Some((
        Name::new(INITRD_NAME).expect("initrd name fits"),
        AnyPie::Pole(pie),
    ))
}

/// 设备树本体：一段终身的、boot 给的物理区，与 initrd 走同一条保留区
/// 机制；这里额外给它一枚门闩，好让需要读它的域自己去读。
fn supply_dtb() -> (Name, AnyPie) {
    let dtb = machine::info().dtb();
    let meta = mail::pole::region(dtb.base, dtb.size, TaskId::new(0)).expect("devicetree region");
    let pie = gate::new_pie(
        meta,
        // 自描述**天然多读者**：共享（不带 `ONLY`），授出即复制。
        Permission::FETCH | Permission::VEST,
        None,
    );
    (
        Name::new(DTB_NAME).expect("devicetree name fits"),
        AnyPie::Pole(pie),
    )
}

/// 中断门铃：一枚、**空载荷**、`owner = 0`、内核永久持源。
///
/// 它进配对块，与设备同列——**它是"设备"吗**？不是：它没有一段内存、没有 `reg`。
/// 它是**一件内核侧事实的出口**（外部中断的入口在 `trap_handler`，那是内核的领地，
/// 故必须有一小块内核结构）。同一张账里放两种东西并不冲突：账记的是"boot 交出了
/// 哪些门闩"，不是"有哪些设备"。
///
/// 权限只给 `FETCH | VEST`：听与应都在"取"这一侧，而**谁也 `Ring` 不动它**——响它的是
/// 内核（持源实体，不走门闩）。`VEST` 是给 root 把它授给 PLIC 驱动用的。
/// **共享**（不带 `ONLY`）：root 留一份、驱动得一份。
fn supply_irq() -> (Name, AnyPie) {
    let meta = NoleMeta::new(TaskId::new(0));
    assert!(IRQ.set(meta.clone()).is_ok(), "irq bell built twice");
    let pie = gate::new_pie(meta, Permission::FETCH | Permission::VEST, None);
    (
        Name::new(IRQ_NAME).expect("irq name fits"),
        AnyPie::Nole(pie),
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
/// 豁免两类：
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
            let Ok(meta) = mail::pole::region(base, size, TaskId::new(0)) else {
                continue;
            };
            let pie = gate::new_pie(
                meta,
                // `ONLY` = **同一时刻只该有一个使用者**（寄存器页）：授出即移交，
                // 复制不出来。这条判断住在造门闩这一处——内核知道谁是 MMIO。
                Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY,
                None,
            );
            out.push((name, AnyPie::Pole(pie)));
        }
    }
    // 设备树本体与中断门闩也与设备同列（见 [`supply_dtb`] / [`supply_irq`]）：
    // 它们不是"设备"，但都是 boot 交出去的门闩——这张账记的是后者。
    out.push(supply_dtb());
    out.push(supply_irq());
    // initrd 载荷区同列：引导域只借映了它，**手上没有能转手的句柄**——给它一枚，
    // 它才能把这批字节交给编排域（零拷贝，见 [`supply_initrd`]）。
    if let Some(pie) = supply_initrd() {
        out.push(pie);
    }
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
    let n = items.len();
    // 供给清单打进 boot 日志：这是**这台机器上有什么**的唯一一次陈述（此后内核
    // 零设备概念，要问只能问域）。内核打印走 SBI，不碰设备。
    crate::putln!("devices: {n} handed to root");
    for (i, (name, pie)) in items.into_iter().enumerate() {
        let token = pie.token();
        // 内核这一侧写的是**字节**（[`Pair::bytes`]）：它持的是自己表里的号（裸值），
        // 而 `PieToken` 是"收号的人"才该有的类型（见 `env::wire::handle`）——
        // 内核根本不经手句柄类型，只写记录。
        let record = Pair::bytes(name, token);
        // SAFETY: 记录与块同长（`PAIR_LEN` 是步长，编译期断言锁死）；写偏移恒 < 块长
        // （上面查过条数上限）。用 `write_unaligned` 是因为 `Block` 只保证页对齐，
        // 而记录步长 40 字节——记录自身不要求对齐。
        unsafe {
            core::ptr::write_unaligned(
                (pa as *mut u8).add(i * PAIR_LEN).cast::<[u8; PAIR_LEN]>(),
                record,
            );
        }
        task.pies.lock().push(pie);
        crate::putln!("  {} -> token {}", name.as_str(), token);
    }
    n
}

/// 该节点是否豁免（见 [`scan`]）——按名字的**词干**（`@` 之前）判。
fn exempt(name: &str) -> bool {
    let stem = name.split('@').next().unwrap_or(name);
    matches!(stem, "memory" | "clint")
}
