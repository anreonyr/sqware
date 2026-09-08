//! frame — 领域无关的「执行链投影」引擎（栈采样 + 链投影）。
//!
//! 领域意象：**投影**。给定一个可能属于任意地址空间的执行栈，安全地采样它，
//! 并把采样到的返回地址投影成一条 [`Frame`] 链。
//!
//! 分层（核心/适配）：
//! - **核心**（本模块）：[`StackReader`]（安全采样）+ [`walk`]（chain + scan 投影）+
//!   [`Frame`]（纯数据）。它**不知道自己属于 kernel 还是 user**——不掺和分域、
//!   不知道 scene/dump/Report 是什么。分域（world/gaps/ceiling）由调用方经
//!   [`ResolveCfg`] **注入**。
//! - **适配**（调用方）：`scene`（崩溃回溯）、`fence`（分配审计 site）、未来的
//!   backtrace envcall。各自构造自己的 `StackReader`（根表）与 `ResolveCfg`，调用
//!   `walk`。
//!
//! 安全契约：**采样绝不触发缺页**。`walk_raw` 只读页表（未映射/非 R → [`None`] →
//! 回溯提前终止），物理地址经 DRAM 值域守卫——用户构造的伪栈最多让链提前结束，
//! 不会让内核在回溯中途缺页侵入。**根表分域**由调用方保证（kernel 传 `satp` 根、
//! user 传 `user_satp` 根），本模块不持分域。

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::work::unit::space::SpaceKind;

/// 回溯扫描窗口（从当前 sp 向上）字节数。
pub const SPAN: usize = 4096;
/// 候选返回地址需 4 字节对齐（RISC-V 指令 2/4 字节）。
const ALIGN: usize = 4;
/// 帧链深度封顶。
pub const DEPTH: usize = 32;

// ── Frame — 纯数据投影（无地址语义）─────────────────────────────────

/// 一帧 — 纯裸地址，**不含任何地址语义**。
///
/// `sp` = 读该帧 fp 时的栈位置；`fp` = 下一帧 fp（caller）；`pc` = 返回地址。
/// `space` = 该帧所属地址空间（当次回溯的现场世界，非逐帧另查）。
/// `FrameKind` 不挂在本结构上——语义是 resolve 的产物，归 scene 的 `FrameResolver`。
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    pub pc: VirtAddr,
    pub sp: VirtAddr,
    pub fp: Option<VirtAddr>,
    pub space: SpaceKind,
}

// ── ResolveCfg — 分域参数（注入而非持有）────────────────────────────

/// 回溯策略配置 — 由调用方**注入**到 [`walk`]，`frame` 模块本身不持有分域。
///
/// 这是「入口独立 + 算法不重复」的枢纽：kernel/user/fence 三条路径各自构造自己
/// 的 `ResolveCfg`，`walk` 是共享的投影函数、不知道自己是哪条路径。
#[derive(Debug, Clone, Copy)]
pub struct ResolveCfg {
    /// 现场地址空间（回溯行/分类的域归属单一事实源）。
    pub world: SpaceKind,
    /// 扫描补偿是否在撞不可读页时跳页（用户栈允许跳页；内核栈遇缺口停扫）。
    pub gaps: bool,
    /// 回溯扫描上界（kernel trap 栈场景由 `trap_stack_edge` 钳制，其余 = sp+SPAN）。
    pub ceiling: usize,
}

impl ResolveCfg {
    /// 内核域配置：域 = 内核，遇缺口停扫。
    pub fn kernel(ceiling: usize) -> ResolveCfg {
        ResolveCfg {
            world: SpaceKind::Supervisor,
            gaps: false,
            ceiling,
        }
    }

    /// 用户域配置：域 = 该任务空间，遇缺口跳页。
    pub fn user(world: SpaceKind, ceiling: usize) -> ResolveCfg {
        ResolveCfg {
            world,
            gaps: true,
            ceiling,
        }
    }
}

// ── StackReader — 崩溃栈只读采样通道（绝不触发缺页）──────────────────

/// 崩溃栈只读采样通道 — 每页 `walk_raw` + R 校验后直读，绝不触发缺页。
///
/// 领域无关：不持分域（分域在上层 `ResolveCfg`）、不依赖 scene。回答的唯一问题是
/// 「这个 VA 序列能安全读出什么」。翻译走裸根表 PPN（调用方传入），物理地址过
/// DRAM 值域守卫。
#[derive(Debug, Clone, Copy)]
pub struct StackReader {
    root: PhysAddr,
    page: Option<(usize, PhysAddr)>,
}

impl StackReader {
    /// 以根表 PPN 构造（`kernel` 传 `satp` 根、`user` 传 `user_satp` 根——根表分域
    /// 即分域隔离的第一重保障）。
    pub fn new(root_ppn: usize) -> StackReader {
        StackReader {
            root: PhysAddr::from_raw(root_ppn << 12),
            page: None,
        }
    }

    /// 逐页翻译：VA 所在页 → 物理帧基址 + 标志（walk_raw + DRAM 守卫 + R 校验）。
    fn leaf(&mut self, page: usize) -> Option<PhysAddr> {
        if let Some((cached, base)) = self.page
            && cached == page
        {
            return Some(base);
        }
        let edge = crate::machine::dram_edge().unwrap_or(0x9000_0000);
        let (base, flags) = crate::memory::manager::table::TableNode::walk_raw(
            self.root,
            VirtAddr::from_raw(page),
            |pa| (0x8000_0000..edge).contains(&pa.as_usize()),
        )?;
        if !flags.contains(PteFlags::R) {
            return None;
        }
        self.page = Some((page, base));
        Some(base)
    }

    /// 安全读一个字：页已翻译且带 R；越界/未映射/非 R → [`None`]（不触发缺页）。
    pub fn word(&mut self, addr: usize) -> Option<usize> {
        let page = addr & !(PAGE_SIZE - 1);
        let base = self.leaf(page)?;
        // SAFETY: 该页已 walk 命中且带 R；偏移恒在页内，S 态直读。脱链后地址
        // 继承任意 callee-saved 保存值，无对齐保证 → read_unaligned。
        Some(unsafe {
            (base.as_usize() as *const u8)
                .add(addr - page)
                .cast::<usize>()
                .read_unaligned()
        })
    }

    /// 标准 RV64 帧对读取（`word` 的两页组合）：caller 在 [fp-16]、ra 在 [fp-8]。
    pub fn pair(&mut self, frame: usize) -> Option<(usize, usize)> {
        Some((self.word(frame - 16)?, self.word(frame - 8)?))
    }
}

/// 扫描期筛法：候选是否属本域代码，以及撞不可读页时跳页还是停扫。
struct Sift<'a> {
    code: &'a dyn Fn(usize) -> bool,
    gaps: bool,
}

// ── walk — 执行链投影（chain + scan）────────────────────────────────

/// 执行链投影器（有状态内部）：`chain`（确定性 fp 链）+ `scan`（启发式补偿）。
struct Walk<'a> {
    reader: &'a mut StackReader,
    fp: usize,
    frames: [Frame; DEPTH],
    count: usize,
    last: usize,
}

impl<'a> Walk<'a> {
    fn new(reader: &'a mut StackReader, fp: usize, world: SpaceKind) -> Walk<'a> {
        Walk {
            reader,
            fp,
            frames: [Frame {
                pc: VirtAddr::from_raw(0),
                sp: VirtAddr::from_raw(0),
                fp: None,
                space: world,
            }; DEPTH],
            count: 0,
            last: 0,
        }
    }

    /// 沿帧指针链收 ra；返回断链处帧地址（一帧未走则返回入参）。
    fn chain(&mut self, world: SpaceKind, floor: usize, ceiling: usize) -> usize {
        let mut f = self.fp;
        let mut broke = self.fp;
        while !self.full() && f >= floor && f <= ceiling {
            broke = f;
            let Some((caller, ra)) = self.reader.pair(f) else {
                break;
            };
            self.push(Frame {
                pc: VirtAddr::from_raw(ra),
                sp: VirtAddr::from_raw(f),
                fp: (caller != 0 && caller > f).then(|| VirtAddr::from_raw(caller)),
                space: world,
            });
            if caller == 0 || caller <= f {
                break;
            }
            f = caller;
        }
        broke
    }

    /// 区间内按字步进，收筛法认可的候选。
    fn scan(&mut self, sift: &Sift, from: usize, to: usize, world: SpaceKind) {
        let mut a = from;
        while a < to && !self.full() {
            match self.reader.word(a) {
                Some(w) => {
                    if (sift.code)(w) {
                        self.push(Frame {
                            pc: VirtAddr::from_raw(w),
                            sp: VirtAddr::from_raw(a),
                            fp: None,
                            space: world,
                        });
                    }
                    a += 8;
                }
                None if sift.gaps => a = (a & !(PAGE_SIZE - 1)) + PAGE_SIZE,
                None => break,
            }
        }
    }

    fn push(&mut self, f: Frame) -> bool {
        if self.full() || f.pc.as_usize() == 0 || f.pc.as_usize() & (ALIGN - 1) != 0
            || f.pc.as_usize() == self.last
        {
            return false;
        }
        self.frames[self.count] = f;
        self.count += 1;
        self.last = f.pc.as_usize();
        true
    }

    fn full(&self) -> bool {
        self.count == DEPTH
    }
}

/// 执行链投影 — 由采样通道沿 `.walk(chain + scan)` 收帧。
///
/// 纯函数形态：入采样通道 + 分域配置 + 起点 sp/fp，出 [`Frame`] 链与有效帧数。
/// `frame` 模块不持分域——分域由 `cfg` 注入。链在 core 预编译库处断（无 FP）后，
/// 从断点帧顶向上扫描候选 ra（域筛 + 去重 + 4 对齐 + 深度封顶）。
///
/// `code` 为域筛回调（候选是否属本域代码）：None = 无表仅 hex，仅 chain 收 ra 即出
/// 轨迹；Some = 有符号表/区间时扫描按它筛。
///
/// 返回 `(frames, count)`：`frames[..count]` 为有效帧（`count` 由 `[Frame; DEPTH]`
/// 的 `frames[0..count]` 承载，调用方按 `count` 切片）。
pub fn walk(
    reader: &mut StackReader,
    cfg: &ResolveCfg,
    sp: usize,
    fp: usize,
    code: Option<&dyn Fn(usize) -> bool>,
) -> ([Frame; DEPTH], usize) {
    let world = cfg.world;
    let mut w = Walk::new(reader, fp, world);
    let ceiling = cfg.ceiling;
    let broke = w.chain(world, sp + 16, ceiling);
    // 无符号表 = 仅 hex：chain 收 ra 即出轨迹；扫描依赖域筛法跳，无表跳过。
    if let Some(code) = code {
        let sift = Sift {
            code,
            gaps: cfg.gaps,
        };
        w.scan(&sift, broke + 8, ceiling, world);
    }
    (w.frames, w.count)
}
