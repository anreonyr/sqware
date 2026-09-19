// machine — **这台机器是什么**：一次设备树解析所得，注入一次、此后只读。
//
// 全是**纯值**（`Copy`、无引用、无 fdt 依赖）：解析完就把树放下，运行期读的人不碰树。
// 三组事实，同一个来路、同一时刻定型：
//   hart   — `/cpus`：核数 + 时基（两项一注，见 [`HartInfo`]）
//   内存   — `/memory`：dram 与 free
//   保留区 — `/chosen` 的 initrd 与设备树本体（boot 给的、终身的物理区，一张账两个读者）
//
// 不在本模块的两样：**每核的运行时上下文**在 `crate::hart`（tp 指向的块），
// **栈与镜像锚点**在 `crate::layout`。

use core::ops;

use crate::layout::root_stack_edge;
use crate::lock::OnceLock;
use crate::memory::PAGE_SIZE;

/// 半开物理区间 `[base, end)` — 内存池 / MMIO 设备区域通用。
///
/// 长度由 `base + size` 得到，不单独存 `end`。
#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub base: usize,
    pub size: usize,
}

impl Region {
    pub fn new(base: usize, size: usize) -> Self {
        Self { base, size }
    }
    pub fn range(&self) -> ops::Range<usize> {
        self.base..self.base + self.size
    }
}

/// CPU 侧的事实：**一次 `/cpus` 解析所得**的核数与时基频率。
///
/// 为什么这两个住一起：它们**同源、同时定型**，而
/// `Machine` 的其余字段是**内存侧**的事实（dram / free / reserved）。分组不是装饰
/// ——它让"这台机器的 CPU 侧长什么样"只有一处可读，读侧也少一次两字段配对的默记。
///
/// 与 [`PerHart`] 的分工：这个是**机器的值**（Copy、注入 once），那个是**每核的运行时
/// 上下文**（tp 指向、含帧 VA/调度器/租约，非 Copy）。核的**身份**（我是第几号核）由
/// [`hart_id()`] 动态读，不在这里——它是"谁在执行"，不是机器属性。
#[derive(Clone, Copy, Debug)]
pub struct HartInfo {
    /// CPU 核数（DTB `/cpus` 的 cpu 节点数）。
    pub count: usize,
    /// 时钟频率（DTB `/cpus` timebase-frequency，Hz）。
    pub hertz: usize,
}

/// 启动时从 DTB 解析出的机器设备信息（纯值，Copy，可安全存入 static）。
#[derive(Clone, Copy, Debug)]
pub struct Machine {
    /// CPU 侧的事实（核数 + 时基）：**两项一注**，同源于 `/cpus`。
    pub hart: HartInfo,
    /// 物理内存范围
    pub dram: Region,
    /// 物理内存空闲区
    pub free: Region,
    /// **持久保留区**（boot 给的、终身的物理区）：其物理页在 frame 分配器中永不
    /// 分配。槽号即语义（见 [`Machine::initrd`] / [`Machine::dtb`]）。
    ///
    /// 为什么是一张账而不是两个字段：initrd 与 DTB 的语义**逐字相同**（boot 给的、
    /// 终身的、不该被复用的物理区），差别只在谁来读——一张账两个读者，不是两份账。
    pub reserved: [Option<Region>; MAX_RESERVED],
}

/// 保留区槽数（initrd + DTB；再多一种来源就一起加在 [`Machine::init`] 里）。
pub const MAX_RESERVED: usize = 2;

/// `/chosen` 的 initrd 载荷区槽号。
const RESERVED_INITRD: usize = 0;
/// 设备树本体所在的槽号。
const RESERVED_DTB: usize = 1;

impl Machine {
    /// initrd 载荷区（`/chosen` 的 `linux,initrd-start/end`；QEMU `-initrd` 传递的
    /// 独立 payload）。无 initrd（未传参）→ None。作为**持久保留区**：其物理页在
    /// frame 分配器中永不分配（符号表 `&'static` 名字指向其 strtab，须终身存活）。
    pub fn initrd(&self) -> Option<Region> {
        self.reserved[RESERVED_INITRD]
    }

    /// 设备树本体所在物理区（始终存在——它在整个 boot 里被解析，且**原样搬运**
    /// 给域：节点自描述不解释、不转录，故它必须活到关机）。
    pub fn dtb(&self) -> Region {
        self.reserved[RESERVED_DTB].expect("machine::init always reserves the DTB")
    }
}

static MACHINE: OnceLock<Machine> = OnceLock::new();

/// 注入机器信息
pub fn init(dtp: usize) {
    let fdt = unsafe { fdt::Fdt::from_ptr(dtp as *const u8) }.expect("invalid device tree blob");

    let count = fdt.cpus().count();

    let mem = fdt
        .memory()
        .regions()
        .next()
        .expect("device tree has no /memory node");
    let dram_base = mem.starting_address.addr();
    let dram_size = mem.size.unwrap_or(0);
    let hertz = hertz(&fdt);
    // 两项同源（都在 `/cpus`）⇒ 一处构造、一处注入（见 [`HartInfo`]）。
    let hart = HartInfo { count, hertz };

    let free_base = root_stack_edge();
    // ROOT 栈位于镜像内（guard + 栈区），整个空闲区
    // （free_base = 栈顶 .. dram_end）均可分配。
    let free_end = dram_base + dram_size;
    let free_size = free_end - free_base;

    let initrd = initrd_region(&fdt);
    // 设备树本体也是一段**终身的、boot 给的物理区**：它与 initrd 走同一条机制
    // （保留区），语义逐字相同。长度取 blob 自述的
    // totalsize（`fdt` 头里的字段），向上取整到页。
    let dtb = Region::new(dtp, dtb_size(dtp as *const u8));

    let mut reserved = [None; MAX_RESERVED];
    reserved[RESERVED_INITRD] = initrd;
    reserved[RESERVED_DTB] = Some(dtb);

    MACHINE
        .set(Machine {
            dram: Region::new(dram_base, dram_size),
            free: Region::new(free_base, free_size),
            hart,
            reserved,
        })
        .unwrap()
}

/// 设备树本体的字节数（向上取整到页）——扁平设备树的头里自述 `totalsize`
/// （大端 u32，偏移 4）。
///
/// 从**原始头**读而不经 `fdt::Fdt`：本值要在任何分配之前定下来（frame 分配器的
/// 保留区账就按它算），故它不能依赖被封装过的东西——只认规范里那 8 个字节。
fn dtb_size(dtp: *const u8) -> usize {
    // SAFETY: dtp 是 boot 交上来的设备树首址（`main` 的 a1），前 8 字节是
    // magic(u32) + totalsize(u32)；此处只读 4 字节，无副作用。
    let total = unsafe { core::ptr::read_unaligned(dtp.add(4).cast::<u32>()) };
    (u32::from_be(total) as usize).next_multiple_of(PAGE_SIZE)
}

/// 读取注入的机器信息（驱动按需调用）。
pub fn info() -> &'static Machine {
    MACHINE.get().expect("machine not initialized")
}

/// DRAM 物理上界（exclusive，恒等区可直读区间的上界）。
/// 机器信息未注入（`machine::init` 前）→ None，调用方自行退回保守值。
/// 取 None 而非 panic：崩溃现场绝不能再 panic。
pub(crate) fn dram_edge() -> Option<usize> {
    MACHINE.get().map(|m| m.dram.range().end)
}

/// 读取 DTB `/cpus` 的 timebase-frequency（Hz）；缺失/非法长度返回 0。
fn hertz(fdt: &fdt::Fdt) -> usize {
    fdt.find_node("/cpus")
        .and_then(|n| n.property("timebase-frequency"))
        .map(|p| match p.value.len() {
            4 => u32::from_be_bytes([p.value[0], p.value[1], p.value[2], p.value[3]]) as usize,
            8 => {
                let b = p.value;
                u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as usize
            }
            _ => 0,
        })
        .unwrap_or(0)
}

/// 读取 `/chosen` 的 initrd 载荷区（`linux,initrd-start/end`，QEMU `-initrd` 填）。
///
/// 无配置（FDT 无该对属性）→ None（不启动 initrd 装载路径）。属性值为 64 位
/// 大端物理地址。`start` 须页对齐（QEMU 保证）；`end` 常非页对齐（实为字节长
/// 边界），向上取整到页界以覆盖完整区间。`end <= start` 按无配置处理（不 panic）。
fn initrd_region(fdt: &fdt::Fdt) -> Option<Region> {
    let chosen = fdt.find_node("/chosen")?;
    let start = chosen.property("linux,initrd-start")?.as_usize()?;
    let end = chosen.property("linux,initrd-end")?.as_usize()?;
    if start == 0 || end <= start || !start.is_multiple_of(PAGE_SIZE) {
        return None;
    }
    let end = end.next_multiple_of(PAGE_SIZE);
    Some(Region::new(start, end - start))
}
