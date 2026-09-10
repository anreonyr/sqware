//! scene — 崩溃现场（可诊断的执行现场快照）+ 执行历史投影（backtrace）。
//!
//! 领域意象（单一隐喻贯穿）：**场景**。一次 [`Scene`] 是一次可诊断的现场快照，
//! 它的 [`Backtrace`] 是 Scene 对执行历史的一次投影。Scene 是「现场」，Backtrace
//! 是「投影」——两者是拥有关系，不是并列关系。
//!
//! 职责分离（核心/适配）：
//! - 核心（本模块下半部）：[`Backtrace`] / [`Frame`] / [`FrameResolver`]——
//!   回答「执行链是什么」与「这个地址是什么」，**零分配、不拥有 Space**。
//! - 适配（上半部）：[`Scene`] / `dump`——取本 hart/world 现场，转发核心回溯，
//!   组稿进 [`Report`]。可独立推理核心，不知道 report/panic 是什么。
//!
//! 现场语义：GPR 是处理器已压栈损坏的现场；真正可定位的是 CSR 的 sepc/scause/stval
//! （trap 进入后持续有效）与栈回溯。回溯 = 无帧指针启发式：`chain`（fp 链）+ `scan`
//! （无表时对断点附近扫描候选 ra，去重、深度封顶）。

use core::arch::asm;

use alloc::format;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;
use riscv::interrupt::{Exception, Interrupt, Trap};
use riscv::register::{satp, scause, sepc, sscratch, sstatus, stval, stvec};

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr;
use crate::runtime::diagnose::frame::{self, ResolveCfg, StackReader};
use crate::runtime::diagnose::report::Report;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::room::scheduler::core::ident;
use crate::work::unit::space::SpaceKind;

use super::backtrace::{backtrace_rows, symbol};

// 门的类型经 scene 转出，对外路径（`diagnose::scene::{Backtrace, FrameKind}`）不变。
#[allow(unused_imports)]
pub use super::backtrace::{Backtrace, FrameKind, FrameResolver};

/// 符号化已移除（Team 不再挂符号表）：统一渲染裸地址。
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

// ── 地址语义（FrameKind）─────────────────────────────────────────────

/// 地址语义分类 — 由 [`FrameResolver::classify`] 产出，**不挂在 [`Frame`] 上**。
///
/// `Frame` 只存裸地址（pc/sp/fp），不携带任何地址语义——语义是 resolve 的产物，
/// 属于独立通道。`Kind` 与 `Frame` 的解耦是「walk 与 resolve 分离」的类型化。
#[derive(Debug, Clone, Copy)]
pub struct Registers {
    pub pc: VirtAddr,
    pub sp: VirtAddr,
    pub fp: VirtAddr,
}

/// 一次可诊断的执行现场快照。
///
/// `Scene` 是「现场」，`Backtrace` 是现场对执行历史的一次投影。`Scene` 拥有
/// Backtrace（而非 Backtrace 去反推整个内核）；`space` 是 `SpaceKind`，不是
/// `&Space`——`Scene` 不拥有任何内存管理权。
#[derive(Debug)]
pub struct Scene {
    /// 现场所属 hart。
    pub hart: usize,
    /// 现场任务（idle/启动期无任务 → None，不 panic）。
    pub task: Option<usize>,
    /// 现场地址空间（按域归属分类，非 &Space）。
    pub space: SpaceKind,
    /// 现场寄存器（当前点 pc/sp/fp）。
    pub reg: Registers,
    /// 现场回溯投影（`pub(crate)`：`super::backtrace` 的行渲染要读它）。
    pub(crate) backtrace: Backtrace,
    /// trap 现场（内核态可由 CSR 重建；用户态由 TrapContext 采读）。
    cause: Option<Trap<Interrupt, Exception>>,
}

impl Scene {
    /// 内核现场采集：经 per-hart 帧（`machine::hart_frame()`）或归巢落盘值取 sp/fp。
    fn capture_kernel() -> Option<Scene> {
        // 内核现场起点：归巢落盘 [sp,fp]（`halt::scene()`；(0,0)=未归巢）。
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
        // 内核现场：根表 = 当前 satp；扫描上界 = per-hart trap 栈边钳制（sp 落 trap 栈内）。
        let ceiling = match crate::runtime::switcher::trap::trap_stack_hart(sp)
            .map(crate::runtime::switcher::trap::trap_stack_edge)
        {
            Some(edge) => sp.saturating_add(frame::SPAN).min(edge.as_usize()),
            None => sp.saturating_add(frame::SPAN),
        };
        let mut reader = StackReader::new(satp::read().bits() & ((1usize << 44) - 1));
        let cfg = ResolveCfg::kernel(ceiling);
        let code = |w: usize| VirtAddr::from_raw(w).is_kernel();
        let r = frame::walk(&mut reader, &cfg, sp, fp, Some(&code));
        let backtrace = Backtrace::from_walk(r);
        Some(Scene {
            hart: crate::machine::hart_id(),
            task: ident().map(|i| i.id()),
            space: SpaceKind::Supervisor,
            reg: Registers {
                pc: VirtAddr::from_raw(sepc::read()),
                sp: VirtAddr::from_raw(sp),
                fp: VirtAddr::from_raw(fp),
            },
            cause: scause::read().cause().try_into().ok(),
            backtrace,
        })
    }

    /// 用户现场采集：running 任务的用户 trap 帧（`ident().trap()`）。
    fn capture_user() -> Option<Scene> {
        let info = ident()?;
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
        let world = info
            .live()
            .map(|t| t.team.space.kind())
            .unwrap_or(SpaceKind::User);
        // 根表 = 用户根表（user_satp）；域 = 该任务空间；上界 = sp+SPAN。
        let mut reader = StackReader::new(frame.user_satp.ppn());
        let cfg = ResolveCfg::user(world, sp.saturating_add(frame::SPAN));
        let code = |w: usize| VirtAddr::from_raw(w).is_user();
        let r = frame::walk(&mut reader, &cfg, sp, fp, Some(&code));
        let backtrace = Backtrace::from_walk(r);
        Some(Scene {
            hart: crate::machine::hart_id(),
            task: Some(info.id()),
            space: world,
            reg: Registers {
                pc: frame.sepc,
                sp: VirtAddr::from_raw(sp),
                fp: VirtAddr::from_raw(fp),
            },
            cause: None,
            backtrace,
        })
    }
}

// ── 组稿（适配层）────────────────────────────────────────────────────

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
                Some(format!("'{}' @ team '{}'", i.name(), i.team_name())),
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
        Some(symbol(VirtAddr::from_raw(sepc::read()))),
    ]);
    {
        // 符号命中 → 「sym note」单空格衔接；未命中 → 仅 stval 语义。
        let a = stval::read();
        let n = stval_note(int, code).to_string();
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
        Some(symbol(VirtAddr::from_raw(stvec::read().address()))),
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

/// Scene 快照行集（首行表头）：hart / task / pc / sp / fp / cause。
///
/// 让 `Scene` 的快照字段（`hart`/`task`/`reg`/`cause`）真正落列——`Scene` 是
/// 「现场」，此处就是现场身份与当前点的渲染（`csr` 段首行的现场戳源头）。
fn scene_rows(scene: &Scene) -> Vec<Vec<Option<String>>> {
    let mut rows: Vec<Vec<Option<String>>> = vec![vec![
        Some("scene".into()),
        Some("hart".into()),
        Some("task".into()),
        Some("pc".into()),
        Some("sp".into()),
        Some("fp".into()),
        Some("cause".into()),
    ]];
    let cause = scene
        .cause
        .map(|c| format!("{c:?}"))
        .unwrap_or_else(|| "-".into());
    rows.push(vec![
        Some("edge".into()),
        Some(scene.hart.to_string()),
        Some(
            scene
                .task
                .map(|t| t.to_string())
                .unwrap_or_else(|| "-".into()),
        ),
        Some(hex(scene.reg.pc.as_usize())),
        Some(hex(scene.reg.sp.as_usize())),
        Some(hex(scene.reg.fp.as_usize())),
        Some(cause),
    ]);
    rows
}

/// 末尾倒出每 hart 最近事件窗口。
pub fn dump(r: &mut Report) {
    // canary 现场清查依赖 ledger 模块（audit-feature-gated）；非 audit 构建
    // 下 ledger 整体未编译，本调用也必须 gate 同步，否则 E0433。
    #[cfg(feature = "audit")]
    {
        let _ = crate::memory::allocator::fence::ledger::LEDGER.sweep_canaries();
    }
    // 投稿：CSR/GPR/回溯段入报告（[scene] 标题挂首段，其余段空标题同段落）。
    let kernel_scene = Scene::capture_kernel();
    let hart = kernel_scene
        .as_ref()
        .map(|s| s.hart)
        .unwrap_or_else(crate::machine::hart_id);
    let scene_head = kernel_scene
        .as_ref()
        .and_then(|s| {
            s.task
                .map(|t| format!("[scene] crash scene, hart {hart}, task #{t}"))
        })
        .unwrap_or_else(|| format!("[scene] crash scene, hart {hart}"));
    r.paragraph("csr", Some(scene_head))
        .items
        .extend(csr_rows());
    // Scene 快照行（hart/task/pc/sp/fp/cause）并入 csr 段最前。
    if let Some(scene) = kernel_scene.as_ref() {
        r.paragraph("scene", None).items.extend(scene_rows(scene));
    }
    r.paragraph("gpr", None).items.extend(gpr_rows());

    if let Some(scene) = kernel_scene.as_ref() {
        r.paragraph("kbt", None)
            .items
            .extend(backtrace_rows(scene, "kbt"));
    }
    if let Some(scene) = Scene::capture_user() {
        r.paragraph("ubt", None)
            .items
            .extend(backtrace_rows(&scene, "ubt"));
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
        $crate::runtime::diagnose::scene::dump(&mut __r);
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
