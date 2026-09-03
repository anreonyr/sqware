//! scene — 崩溃现场转储（定位错误的统一诊断）。
//!
//! 职责：回答「崩在哪个地址 / 哪条调用链」。本模块只产行
//! （`Vec<Vec<Option<String>>>`）投进 [`Report`] 的段落。
//!
//! 现场语义：GPR 是处理器已压栈损坏的现场；真正可定位的是 CSR 的 sepc/scause/stval
//! （trap 进入后持续有效）与栈回溯。
//!
//! 回溯 = 无帧指针启发式：扫描当前栈区间，收集可执行地址的候选返回地址（去重、
//! 深度封顶）。每条经 `elftable::resolve` 符号化，未命中打印裸 hex。

use core::arch::asm;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use riscv::interrupt::{Exception, Interrupt, Trap};
use riscv::register::{satp, scause, sepc, sscratch, sstatus, stval, stvec};

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::runtime::diagnose::report::Report;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::room::scheduler::core::ident;
use crate::work::unit::elftable::{self, ElfTable};
use crate::work::unit::team::kernel;

/// 按地址域取符号表：内核地址取内核团队表，用户地址取当前任务表。
fn table(va: VirtAddr) -> Option<Arc<ElfTable>> {
    if va.is_kernel() {
        kernel()?.elftable.clone()
    } else {
        ident()?.elftable()
    }
}

const DEPTH: usize = 32;
/// 栈扫描窗口（从当前 sp 向上）字节数。
const SPAN: usize = 4096;
/// 候选返回地址需 4 字节对齐（RISC-V 指令 2/4 字节）。
const ALIGN: usize = 4;

/// 定宽 hex 文本（{:#018x}）——值列的通用形态。
fn hex(x: usize) -> String {
    format!("{x:#018x}")
}

/// 读全部 31 个非零 GPR（x0 恒 0；ra/sp/gp/tp 首页）。
///
/// 注：tp 原值转储（内核态 = PerHart 指针，非裸 hartid；hart 号经
/// `machine::hart_id()` 读取）。
fn gprs() -> [usize; 32] {
    let mut r = [0usize; 32];
    unsafe {
        asm!("mv {0}, ra", out(reg) r[1]);
        asm!("mv {0}, sp", out(reg) r[2]);
        asm!("mv {0}, gp", out(reg) r[3]);
        asm!("mv {0}, tp", out(reg) r[4]);
        asm!("mv {0}, t0", out(reg) r[5]);
        asm!("mv {0}, t1", out(reg) r[6]);
        asm!("mv {0}, t2", out(reg) r[7]);
        asm!("mv {0}, s0", out(reg) r[8]);
        asm!("mv {0}, s1", out(reg) r[9]);
        asm!("mv {0}, a0", out(reg) r[10]);
        asm!("mv {0}, a1", out(reg) r[11]);
        asm!("mv {0}, a2", out(reg) r[12]);
        asm!("mv {0}, a3", out(reg) r[13]);
        asm!("mv {0}, a4", out(reg) r[14]);
        asm!("mv {0}, a5", out(reg) r[15]);
        asm!("mv {0}, a6", out(reg) r[16]);
        asm!("mv {0}, a7", out(reg) r[17]);
        asm!("mv {0}, s2", out(reg) r[18]);
        asm!("mv {0}, s3", out(reg) r[19]);
        asm!("mv {0}, s4", out(reg) r[20]);
        asm!("mv {0}, s5", out(reg) r[21]);
        asm!("mv {0}, s6", out(reg) r[22]);
        asm!("mv {0}, s7", out(reg) r[23]);
        asm!("mv {0}, s8", out(reg) r[24]);
        asm!("mv {0}, s9", out(reg) r[25]);
        asm!("mv {0}, s10", out(reg) r[26]);
        asm!("mv {0}, s11", out(reg) r[27]);
        asm!("mv {0}, t3", out(reg) r[28]);
        asm!("mv {0}, t4", out(reg) r[29]);
        asm!("mv {0}, t5", out(reg) r[30]);
        asm!("mv {0}, t6", out(reg) r[31]);
    }
    r
}

/// 崩溃现场栈的只读通道：逐页 walk_raw + R 校验后直读，绝不触发缺页。
struct Stack {
    root: PhysAddr,
    page: Option<(usize, PhysAddr)>,
}

impl Stack {
    fn kernel() -> Stack {
        let ppn = satp::read().bits() & ((1usize << 44) - 1);
        Stack {
            root: PhysAddr::from_raw(ppn << 12),
            page: None,
        }
    }

    fn user(root: usize) -> Stack {
        Stack {
            root: PhysAddr::from_raw(root << 12),
            page: None,
        }
    }

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

    fn word(&mut self, addr: usize) -> Option<usize> {
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

    /// RV64 psABI：序言 `sd ra, N-8(sp); sd s0, N-16(sp); s0 = sp+N`，
    /// 保存对在 fp 下方——caller 在 [frame-16]、ra 在 [frame-8]。
    fn pair(&mut self, frame: usize) -> Option<(usize, usize)> {
        Some((self.word(frame - 16)?, self.word(frame - 8)?))
    }
}

/// 回溯轨迹：对齐、去重、封顶三条纪律在 push 内闭合。
struct Trail {
    frames: [usize; DEPTH],
    count: usize,
    last: usize,
}

impl Trail {
    fn new() -> Trail {
        Trail {
            frames: [0; DEPTH],
            count: 0,
            last: 0,
        }
    }

    fn push(&mut self, addr: usize) -> bool {
        if self.full() || addr == 0 || addr & (ALIGN - 1) != 0 || addr == self.last {
            return false;
        }
        self.frames[self.count] = addr;
        self.count += 1;
        self.last = addr;
        true
    }

    fn full(&self) -> bool {
        self.count == DEPTH
    }

    fn frames(&self) -> &[usize] {
        &self.frames[..self.count]
    }
}

/// 扫描期筛法：候选是否属本域代码，以及撞不可读页时跳页还是停扫。
struct Sift<'a> {
    code: &'a dyn Fn(usize) -> bool,
    gaps: bool,
}

/// 一次勘探：轨迹与解释它所需的符号表同生共死。
struct Trace {
    trail: Trail,
    table: Arc<ElfTable>,
}

impl Trace {
    fn rows(&self, head: &str) -> Vec<Vec<Option<String>>> {
        let mut rows: Vec<Vec<Option<String>>> = vec![vec![
            Some(head.into()),
            Some("hex".into()),
            Some("sym".into()),
        ]];
        for (i, a) in self.trail.frames().iter().enumerate() {
            rows.push(vec![
                Some(format!("#{i}")),
                Some(hex(*a)),
                Some(elftable::symbol(VirtAddr::from_raw(*a), Some(&self.table))),
            ]);
        }
        rows
    }
}

/// 沿帧指针链收 ra，返回断链处帧地址（一帧未走则返回入参）。
fn chain(
    stack: &mut Stack,
    trail: &mut Trail,
    frame: usize,
    floor: usize,
    ceiling: usize,
) -> usize {
    let mut f = frame;
    let mut broke = frame;
    while !trail.full() && f >= floor && f <= ceiling {
        broke = f;
        let Some((caller, ra)) = stack.pair(f) else {
            break;
        };
        trail.push(ra);
        if caller == 0 || caller <= f {
            break;
        }
        f = caller;
    }
    broke
}

/// 区间内按字步进，收筛法认可的候选。
fn scan(stack: &mut Stack, trail: &mut Trail, sift: &Sift, from: usize, to: usize) {
    let mut a = from;
    while a < to && !trail.full() {
        match stack.word(a) {
            Some(w) => {
                if (sift.code)(w) {
                    trail.push(w);
                }
                a += 8;
            }
            None if sift.gaps => a = (a & !(PAGE_SIZE - 1)) + PAGE_SIZE,
            None => break,
        }
    }
}

/// 内核现场：起点取归巢落盘的原始 sp/fp，未归巢（crash_scene! 直调）读当前。
fn ktrace() -> Option<Trace> {
    let table = kernel()?.elftable.clone()?;
    let (sp, fp) = match crate::runtime::diagnose::halt::scene() {
        (0, 0) => {
            let (sp, fp): (usize, usize);
            // SAFETY: 只读本 hart 当前 sp/s0，无副作用。
            unsafe {
                asm!("mv {0}, sp", out(reg) sp);
                asm!("mv {0}, s0", out(reg) fp);
            }
            (sp, fp)
        }
        s => s,
    };
    let ceiling = match crate::runtime::switcher::trap::trap_stack_hart(sp)
        .map(crate::runtime::switcher::trap::trap_stack_edge)
    {
        Some(edge) => sp.saturating_add(SPAN).min(edge.as_usize()),
        None => sp.saturating_add(SPAN),
    };
    let mut stack = Stack::kernel();
    let mut trail = Trail::new();
    let broke = chain(&mut stack, &mut trail, fp, sp + 16, ceiling);
    let sift = Sift {
        code: &|w| table.lookup(VirtAddr::from_raw(w)).is_some(),
        gaps: false,
    };
    scan(&mut stack, &mut trail, &sift, broke + 8, ceiling);
    Some(Trace { trail, table })
}

/// 用户现场：running 任务的用户 trap 帧存着最近一次用户态 sp/fp。
fn utrace() -> Option<Trace> {
    let info = ident()?;
    let table = info.elftable()?;
    let pa = info.trap()?;
    // SAFETY: Live 轴 = 本核在跑任务，帧未回收；帧 PA 在用户 Frame 窗口（DRAM
    // 恒等映射）；崩溃现场只读，其余核已冻结。
    let frame = unsafe { &*(pa.as_usize() as *const TrapContext) };
    if frame.sepc.is_kernel() {
        return None;
    }
    let sp = frame.gpr.x(Gprs::SP);
    if sp == 0 {
        return None;
    }
    let fp = frame.gpr.x(Gprs::S0);
    let ceiling = sp.saturating_add(SPAN);
    let mut stack = Stack::user(frame.user_satp.ppn());
    let mut trail = Trail::new();
    let broke = chain(&mut stack, &mut trail, fp, sp + 16, ceiling);
    let sift = Sift {
        code: &|w| table.lookup(VirtAddr::from_raw(w)).is_some(),
        gaps: true,
    };
    scan(&mut stack, &mut trail, &sift, broke + 8, ceiling);
    Some(Trace { trail, table })
}

/// stval 解码：按 scause 的语义注解（fault 地址 / 指令位 / 断点地址）；
/// 无有价值语义时输出 Unknown（中断 / ecall / 保留码 stval 均无定义）。
fn stval_note(int: bool, code: usize) -> &'static str {
    if int {
        return "Unknown";
    }
    match code {
        0 | 1 | 4 | 5 | 6 | 7 | 12 | 13 | 15 => "faulting addr",
        2 => "Illegal instruction bits",
        3 => "Breakpoint",
        _ => "Unknown",
    }
}

/// CSR 段行集（首行表头）：sepc/stval/scause = 崩点；stvec/sscratch = 陷阱
/// 入口/暂存；sstatus/satp = 特权/地址空间域。task 行 = 运行中任务（若有；
/// try_lock 拿不到则跳过）。注解列 = 符号化 + 解码。
fn csr_rows() -> Vec<Vec<Option<String>>> {
    let mut rows: Vec<Vec<Option<String>>> = vec![
        if let Some(i) = ident() {
            vec![
                None,
                Some(format!("#{}", i.id())),
                Some(format!("'{}'", i.name())),
            ]
        } else {
            vec![None, Some("failed to get task info".into()), None]
        },
        vec![None, Some("hex".into()), Some("note".into())], // 首行表头
    ];
    let sc = scause::read();
    let (int, code) = (sc.is_interrupt(), sc.code());
    rows.push(vec![
        Some("sepc".into()),
        Some(hex(sepc::read())),
        Some({
            let va = VirtAddr::from_raw(sepc::read());
            elftable::symbol(va, table(va).as_deref())
        }),
    ]);
    {
        // 符号命中 → 「sym note」单空格衔接；未命中 → 仅 stval 语义。
        let a = stval::read();
        let va = VirtAddr::from_raw(a);
        let n = if let Some((name, off)) = table(va).and_then(|t| t.lookup(va)) {
            format!("{name}+{off:#x} {}", stval_note(int, code))
        } else {
            stval_note(int, code).to_string()
        };
        rows.push(vec![Some("stval".into()), Some(hex(a)), Some(n)]);
    }
    {
        // 类型化枚举：变体名自解释；非法码回退 Unknown。
        let trap: Option<Trap<Interrupt, Exception>> = sc.cause().try_into().ok();
        let note = match trap {
            Some(Trap::Interrupt(i)) => format!("{i:?}"),
            Some(Trap::Exception(e)) => format!("{e:?}"),
            None => "Unknown".to_string(),
        };
        rows.push(vec![
            Some("scause".into()),
            Some(hex(sc.bits())),
            Some(note),
        ]);
    }
    rows.push(vec![
        Some("stvec".into()),
        Some(hex(stvec::read().address())),
        Some({
            let va = VirtAddr::from_raw(stvec::read().address());
            elftable::symbol(va, table(va).as_deref())
        }),
    ]);
    {
        // sscratch 约定：内核态 = 本 hart trap 帧 VA（HART_FRAME_BASE +
        // hart·PAGE，可反推 hart）；用户态 = 当前线程帧 self_va（team 帧区）。
        // 值域判定：hart 帧区 → 内核态帧（可推 hart）；team 帧区 → 用户帧。
        let scr = sscratch::read();
        let kfb = crate::layout::HART_FRAME_BASE.as_usize();
        let n = if scr == 0 {
            "Kernel frame".to_string()
        } else if scr >= kfb && scr < kfb + crate::machine::MAX_HART_SLOTS * PAGE_SIZE {
            format!("Kernel frame @ {}", (scr - kfb) / PAGE_SIZE)
        } else if scr >= crate::layout::TEAM_FRAME_BASE.as_usize()
            && scr < crate::layout::HART_FRAME_BASE.as_usize()
        {
            "User frame".into()
        } else {
            "other".into()
        };
        rows.push(vec![Some("sscratch".into()), Some(hex(scr)), Some(n)]);
    }
    {
        // 注解只列非默认态：前特权模式恒打；布尔位置位才打缩写（SIE/SPIE/
        // SUM/MXR/SD）；FS/VS/XS 非 Off 才打短码。
        let ss = sstatus::read();
        let mut note = format!("{:?}", ss.spp());
        if ss.sie() {
            note.push_str(" SIE");
        }
        if ss.spie() {
            note.push_str(" SPIE");
        }
        if ss.fs() != riscv::register::mstatus::FS::Off {
            note.push_str(match ss.fs() {
                riscv::register::mstatus::FS::Initial => " FS FI",
                riscv::register::mstatus::FS::Clean => " FS FC",
                riscv::register::mstatus::FS::Dirty => " FS FD",
                riscv::register::mstatus::FS::Off => " FS FO",
            });
        }
        if ss.vs() != riscv::register::mstatus::VS::Off {
            note.push_str(match ss.vs() {
                riscv::register::mstatus::VS::Initial => " VS VI",
                riscv::register::mstatus::VS::Clean => " VS VC",
                riscv::register::mstatus::VS::Dirty => " VS VD",
                riscv::register::mstatus::VS::Off => " VS VO",
            });
        }
        if ss.xs() != riscv::register::mstatus::XS::AllOff {
            note.push_str(match ss.xs() {
                riscv::register::mstatus::XS::NoneDirtyOrClean => " XS XI",
                riscv::register::mstatus::XS::NoneDirtySomeClean => " XS XC",
                riscv::register::mstatus::XS::SomeDirty => " XS XD",
                riscv::register::mstatus::XS::AllOff => " XS XA",
            });
        }
        if ss.sum() {
            note.push_str(" SUM");
        }
        if ss.mxr() {
            note.push_str(" MXR");
        }
        if ss.sd() {
            note.push_str(" SD");
        }
        rows.push(vec![
            Some("sstatus".into()),
            Some(hex(ss.bits())),
            Some(note),
        ]);
    }
    {
        let s = satp::read();
        let note = format!("{:?} {:#06x} {:#013x}", s.mode(), s.asid(), s.ppn());
        rows.push(vec![Some("satp".into()), Some(hex(s.bits())), Some(note)]);
    }
    rows
}

/// GPR 段行集（首行表头，其后只打非零）：label/hex 两槽。
fn gpr_rows() -> Vec<Vec<Option<String>>> {
    const NAMES: [&str; 32] = [
        "x0", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4",
        "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4",
        "t5", "t6",
    ];
    let mut rows: Vec<Vec<Option<String>>> = vec![
        vec![None, Some("hex".into())], // 首行表头
    ];
    let r = gprs();
    for (i, name) in NAMES.iter().enumerate().skip(1) {
        if r[i] == 0 {
            continue;
        }
        rows.push(vec![Some((*name).into()), Some(hex(r[i]))]);
    }
    rows
}

/// 统一崩溃现场组稿：CSR 三列表 + GPR 两列表 + 回溯表（内核栈 kbt + 用户栈 ubt）。
/// 末尾倒出每 hart 最近事件窗口。
pub fn dump_crash(r: &mut Report) {
    // 探针：panic 现场 drop-in 完整性体检——越界写破坏用户符号表/相邻活块
    // 时自报。两者均纯读零分配、只经 putln! 直写控制台——panic 现场安全，
    // 且不截断本次转储。
    #[cfg(debug_assertions)]
    {
        if let Some(et) = ident().as_ref().and_then(|i| i.elftable()).as_ref() {
            et.check_integrity();
        }
    }
    // canary 现场清查依赖 ledger 模块（audit-feature-gated）；非 audit 构建
    // 下 ledger 整体未编译，本调用也必须 gate 同步，否则 E0433。
    #[cfg(feature = "audit")]
    {
        let _ = crate::memory::allocator::fence::ledger::LEDGER.sweep_canaries();
    }
    // 投稿：CSR/GPR/回溯段入报告（[scene] 标题挂首段，其余段空标题同段落）。
    r.paragraph(
        "csr",
        Some(format!(
            "[scene] crash scene, hart {}",
            crate::machine::hart_id()
        )),
    )
    .items
    .extend(csr_rows());
    r.paragraph("gpr", None).items.extend(gpr_rows());

    if let Some(t) = ktrace() {
        r.paragraph("kbt", None).items.extend(t.rows("kbt"));
    }
    if let Some(t) = utrace() {
        r.paragraph("ubt", None).items.extend(t.rows("ubt"));
    }

    // 每 hart 最近事件窗口（人读对照）。
    crate::runtime::diagnose::trace::panic_dump(r);
}

/// 统一崩溃现场宏：空调用即完整转储（自建报告、成册、印发——可在任意点
/// drop-in 调试）；带参则先写一行消息再转储。
#[macro_export]
macro_rules! crash_scene {
    () => {{
        let mut __r = $crate::runtime::diagnose::report::Report::default();
        $crate::runtime::diagnose::scene::dump_crash(&mut __r);
        let __sealed = __r.seal();
        let mut __sink = $crate::console::Sink;
        $crate::runtime::diagnose::render::render(__sealed, &mut __sink, 2);
        #[cfg(feature = "semihosting")]
        $crate::runtime::diagnose::export::export(__sealed);
    }};
    ($($arg:tt)*) => {{
        $crate::console::_write(format_args!($($arg)*));
        $crate::put!("\n"); // 消息后换行，[scene] 标题不与消息同段紧贴
        $crate::crash_scene!();
    }};
}
