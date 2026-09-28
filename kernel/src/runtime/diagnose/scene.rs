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

#[allow(unused_imports)]
pub use super::backtrace::{Backtrace, FrameKind, FrameResolver};

fn hex(x: usize) -> String {
    format!("{x:#018x}")
}

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

#[derive(Debug, Clone, Copy)]
pub struct Registers {
    pub pc: VirtAddr,
    pub sp: VirtAddr,
    pub fp: VirtAddr,
}

#[derive(Debug)]
pub struct Scene {
    pub hart: crate::hart::HartId,
    pub task: Option<usize>,
    pub space: SpaceKind,
    pub reg: Registers,
    pub(crate) backtrace: Backtrace,
    cause: Option<Trap<Interrupt, Exception>>,
}

impl Scene {
    fn capture_kernel() -> Option<Scene> {
        let (sp, fp) = match crate::runtime::diagnose::halt::scene() {
            (0, 0) => {
                let (sp, fp): (usize, usize);
                // SAFETY: 只读本 hart 当前 sp/s0
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
            Some(edge) => sp.saturating_add(frame::SPAN).min(edge.as_usize()),
            None => sp.saturating_add(frame::SPAN),
        };
        let mut reader = StackReader::new(satp::read().bits() & ((1usize << 44) - 1));
        let cfg = ResolveCfg::kernel(ceiling);
        let code = |w: usize| VirtAddr::from_raw(w).is_kernel();
        let r = frame::walk(&mut reader, &cfg, sp, fp, Some(&code));
        let backtrace = Backtrace::from_walk(r);
        Some(Scene {
            hart: crate::hart::hart_id(),
            task: ident().map(|i| i.task_id()),
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

    fn capture_normal() -> Option<Scene> {
        let info = ident()?;
        let pa = info.trap()?;
        // SAFETY: Live 轴上本核任务帧未回收，PA 在用户 Frame 窗口（DRAM 恒等映射）
        let frame = unsafe { &*(pa.as_usize() as *const TrapContext) };
        let world = info
            .live()
            .map(|t| t.team.space.kind())
            .unwrap_or(SpaceKind::User);
        if world.is_supervisor() {
            return None;
        }
        let sp = frame.gpr.x(Gprs::SP);
        if sp == 0 {
            return None;
        }
        let fp = frame.gpr.x(Gprs::S0);
        let mut reader = StackReader::new(frame.user_satp.ppn());
        let cfg = ResolveCfg::normal(world, sp.saturating_add(frame::SPAN));
        let code = |w: usize| VirtAddr::from_raw(w).is_user();
        let r = frame::walk(&mut reader, &cfg, sp, fp, Some(&code));
        let backtrace = Backtrace::from_walk(r);
        Some(Scene {
            hart: crate::hart::hart_id(),
            task: Some(info.task_id()),
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

fn csr_rows() -> Vec<Vec<Option<String>>> {
    let mut rows: Vec<Vec<Option<String>>> = vec![
        if let Some(i) = ident() {
            vec![
                None,
                Some(format!("#{}", i.task_id())),
                Some(format!("task #{} @ team #{}", i.task_id(), i.team_id())),
            ]
        } else {
            vec![None, Some("failed to get task info".into()), None]
        },
        vec![None, Some("hex".into()), Some("note".into())],
    ];
    let sc = scause::read();
    let (int, code) = (sc.is_interrupt(), sc.code());
    rows.push(vec![
        Some("sepc".into()),
        Some(hex(sepc::read())),
        Some(symbol(VirtAddr::from_raw(sepc::read()))),
    ]);
    {
        let a = stval::read();
        let n = stval_note(int, code).to_string();
        rows.push(vec![Some("stval".into()), Some(hex(a)), Some(n)]);
    }
    {
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
        let scr = sscratch::read();
        let kfb = crate::layout::HART_FRAME_BASE.as_usize();
        let n = if scr == 0 {
            "Kernel frame".to_string()
        } else if scr >= kfb && scr < kfb + crate::layout::MAX_HART_SLOTS * PAGE_SIZE {
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

fn gpr_rows() -> Vec<Vec<Option<String>>> {
    const NAMES: [&str; 32] = [
        "x0", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4",
        "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4",
        "t5", "t6",
    ];
    let mut rows: Vec<Vec<Option<String>>> = vec![
        vec![None, Some("hex".into())],
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

pub fn dump(r: &mut Report) {
    let kernel_scene = Scene::capture_kernel();
    let hart = kernel_scene
        .as_ref()
        .map(|s| s.hart)
        .unwrap_or_else(crate::hart::hart_id);
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
    if let Some(scene) = kernel_scene.as_ref() {
        r.paragraph("scene", None).items.extend(scene_rows(scene));
    }
    r.paragraph("gpr", None).items.extend(gpr_rows());

    if let Some(scene) = kernel_scene.as_ref() {
        r.paragraph("kbt", None)
            .items
            .extend(backtrace_rows(scene, "kbt"));
    }
    if let Some(scene) = Scene::capture_normal() {
        r.paragraph("nbt", None)
            .items
            .extend(backtrace_rows(&scene, "nbt"));
    }

    crate::runtime::diagnose::trace::panic_dump(r);
}

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
        $crate::putln!($($arg)*);
        $crate::crash_scene!();
    }};
}