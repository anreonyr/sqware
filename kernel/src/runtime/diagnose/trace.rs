//! trace — 内核事件环形缓冲（崩溃后反推的依据）。
//!
//! 分层（signature 即文档）：
//!   事件 = 按模块分组、字段自足的枚举（EventKind 聚合）
//!   核心 = Trace（per-hart 窗口）+ note / dump / reset / init
//!   适配 = panic_dump（把崩溃前最近窗口倒到控制台）+ 宿主镜像（共享同一事件流）
//!
//! 设计决策：
//!   - per-hart、环常驻后备仓（spare 内存，boot 早期一次分配：窗口表 + 事件槽，
//!     size_of 精确预算，见 trace::init / ring_bytes）——崩溃现场主堆不可信，缓冲
//!     仍可用（spare 是独立仓，budget 即契约，health 验收）。
//!   - 写 = 单生产者无锁：cursor 单调不回卷、读侧取模；读只在 panic，此时其余核
//!     已由 halt 停写（写读互斥由 halt 语义保证）。
//!   - 宿主镜像 = diagnose::export（feature semihosting，**默认开启**）：每条事件
//!     一条 JSON 记录写入宿主文件 sqware-diagnose.jsonl（qemu CWD 下；首行 '#' 头部
//!     溯源），runner 归档；依赖 QEMU -semihosting（runner 恒加）。
//!   - 事件写者 = host_note（live 流，经 export::line 串行）：panic 只经
//!     note(Halt(Panic)) 追加一条事件行，决不把环形窗口再 dump 一遍进文件；
//!     环形窗口文本由 scene::dump_crash 恒倒到控制台（人读上下文）。

use core::fmt;
use core::mem::size_of;
use core::sync::atomic::{AtomicUsize, Ordering};
#[cfg(feature = "semihosting")]
use core::fmt::Write;

use alloc::alloc::Layout;
use alloc::format;
use fack::prelude::Error;

use crate::lock::OnceLock;
use crate::memory::allocator::spare;
use crate::memory::manager::fault::FaultKind;
use crate::machine;
use crate::runtime::diagnose::{fmt::Fmt, render};

/// 每 hart 事件窗口容量。
pub const BUFFER_SIZE: usize = 512;
/// 崩溃转储每条 hart 倒出的事件数上限：现场只关心「崩前最近一段」，
/// 全量 512 对控制台过长且 Table 栈预算不可承载（ROWS 编译期常量）。
/// 多核下按 hart 平摊（hart_rows：总额恒 ≤ TRACE_DUMP，渲染预算与核数无关）。
pub const TRACE_DUMP: usize = 64;

// ── 事件定义（按模块分组）────────────────────────────

/// 各模块事件的聚合。
#[derive(Clone, Copy)]
pub enum EventKind {
    Sched(SchedEvent),
    Env(EnvEvent),
    Mem(MemEvent),
    Halt(HaltEvent),
    Boot(BootEvent),
}

/// work/scheduler 的任务生命周期。
#[derive(Clone, Copy)]
pub enum SchedEvent {
    Spawn { tid: usize },
    Switch { prev_tid: usize, next_tid: usize },
    Starve { tid: usize },
    Steal { tid: usize, src_hart: usize },
    Park { tid: usize, wake_at: usize },
    Wake { tid: usize },
    Exit { tid: usize },
    Reap { tid: usize },
    Idle,
}

/// work/envcall 的用户环境调用。
#[derive(Clone, Copy)]
pub enum EnvEvent {
    Call { call: usize, arg: usize },
}

/// memory/manager/fault 的缺页与 memory/integrity 的完整性违例。
#[derive(Clone, Copy)]
pub enum MemEvent {
    PageFault {
        va: usize,
        fault: FaultKind,
        resolved: bool,
    },
    /// 完整性违例（IntegrityViolation 的 repr(u8) 编码；见 allocator::fence）。
    Integrity { code: u8, addr: usize },
}

/// runtime/halt 的系统级事件。
#[derive(Clone, Copy)]
pub enum HaltEvent {
    Halt,
    Panic,
}

/// boot / 副核启动的初始化消息（直打控制台会扰乱 panic 现场，改写进 trace）。
/// 各 hart 启动时一次；crash 后由报警源统一 dump，既可查又不打断现场。
#[derive(Clone, Copy)]
pub enum BootEvent {
    /// 主核 call 该副核（HSM start）。
    Launch { hart: usize },
    /// 副核 trap/调度初始化完成（原 console 的 "trap init done"）。
    Done { hart: usize },
    /// 该核 trap 栈峰值水位（原 console 的 "high-water"）。
    Stack { hart: usize, used: usize },
}

/// 一条事件：时间戳 + 聚合事件。仅标量、Copy，可 const 初始化。
#[derive(Clone, Copy)]
pub struct Event {
    when: u64,
    kind: EventKind,
}

impl Event {
    const EMPTY: Event = Event {
        when: 0,
        kind: EventKind::Sched(SchedEvent::Idle),
    };
}

// ── 核心：per-hart 窗口 ─────────────────────────────

pub struct Trace {
    /// 写游标：单调不回卷；读侧取模落位。
    cursor: AtomicUsize,
    /// 本 hart 事件槽（后备仓分配，容量 = BUFFER_SIZE；写经裸指针，读见 dump）。
    buffer: &'static [Event],
}

impl Trace {
    fn note(&self, kind: EventKind, when: u64) {
        let i = self.cursor.fetch_add(1, Ordering::Relaxed) % BUFFER_SIZE;
        // SAFETY: i < BUFFER_SIZE；单生产者（本 hart 唯一写者），与跨核读者互斥由 halt 停写保证。
        unsafe { *(self.buffer.as_ptr() as *mut Event).add(i) = Event { when, kind } };
    }

    fn clear(&self) {
        self.cursor.store(0, Ordering::Relaxed);
    }
}

/// 事件池（spare 仓内）：写入一次、读多次。分配于 `init`，环在分配后只读结构。
static POOL: OnceLock<&'static [Trace]> = OnceLock::new();

/// 环形常驻字节数（h 个窗口表 + 事件槽，size_of 精确）——spare 预算公式同源。
pub fn ring_bytes(h: usize) -> usize {
    h * (size_of::<Trace>() + BUFFER_SIZE * size_of::<Event>())
}

// ── 核心原语 ────────────────────────────────────────

/// 记一条事件到**本 hart** 窗口（盖 when 时间戳）。
///
/// 尽力而为、不失败：hart 越界即丢弃；诊断路径永不允许失败或 panic。
/// 只进环形（release/panic 转储），不做实时控制台输出——不扰动时序。
pub fn note(kind: EventKind) {
    let Some(pool) = POOL.get() else {
        return; // 未初始化（init 前）静默跳过——诊断路径不失败
    };
    let hart = machine::hart_id();
    let Some(t) = pool.get(hart) else {
        return;
    };
    let when = crate::runtime::chrono::clock::now().as_ticks();
    t.note(kind, when);
    // 宿主全量（feature semihosting，默认开启）：每条结构化事件经 semihosting 送宿主，
    // 带自描述长度 "len"，便于 tail/离线解析可靠分帧（try_lock 原子成行、尽力而为）。
    #[cfg(feature = "semihosting")]
    host_note(kind, hart, when);
}

// ── 宿主镜像适配（feature gate: semihosting）──────────────────────────
// 事件 JSON 序列化（json_payload）在此；人读文本（fmt_description）同源平行。
// 文件/分帧/锁/闩均归 diagnose::export（JSON Lines 单文件 sqware-diagnose.jsonl）。

/// 宿主镜像：把一条事件序列化成 JSON 记录，经 `export::line` 写入宿主文件
/// sqware-diagnose.jsonl。依赖 QEMU -semihosting（feature 默认开启，runner
/// 恒加该标志）。打开失败/写失败尽力而为（诊断路径永不 panic）；跨核写由
/// export 以 try_lock 串行。
///
/// 事件写者：panic 只追加自身一行（note(Halt(Panic))），环形窗口 dump 不写文件。
#[cfg(feature = "semihosting")]
fn host_note(kind: EventKind, hart: usize, when: u64) {
    use crate::runtime::diagnose::export::line;
    let e = Event { when, kind };
    line(|w| {
        let _ = write!(w, "\"h\":{hart},\"t\":{}", e.when);
        let _ = json_payload(&e, w);
    });
}

/// 一条事件的 JSON 载荷：`,"kind":"…"` + 模块字段（数字裸写、字符串经 k/v）。
/// 与 fmt_description（人读文本）平行——事件只有机器/人读两个形状，各一个 match。
#[cfg(feature = "semihosting")]
fn json_payload(e: &Event, w: &mut crate::runtime::diagnose::export::Buf) -> fmt::Result {
    use crate::runtime::diagnose::export::{k, v};
    match e.kind {
        EventKind::Sched(SchedEvent::Spawn { tid }) => {
            write!(w, ",\"kind\":\"spawn\"")?;
            k(w, "tid")?;
            write!(w, "{tid}")
        }
        EventKind::Sched(SchedEvent::Switch { prev_tid, next_tid }) => {
            write!(w, ",\"kind\":\"switch\"")?;
            k(w, "prev")?;
            write!(w, "{prev_tid}")?;
            k(w, "next")?;
            write!(w, "{next_tid}")
        }
        EventKind::Sched(SchedEvent::Starve { tid }) => {
            write!(w, ",\"kind\":\"starve\"")?;
            k(w, "tid")?;
            write!(w, "{tid}")
        }
        EventKind::Sched(SchedEvent::Steal { tid, src_hart }) => {
            write!(w, ",\"kind\":\"steal\"")?;
            k(w, "tid")?;
            write!(w, "{tid}")?;
            k(w, "src_hart")?;
            write!(w, "{src_hart}")
        }
        EventKind::Sched(SchedEvent::Park { tid, wake_at }) => {
            write!(w, ",\"kind\":\"park\"")?;
            k(w, "tid")?;
            write!(w, "{tid}")?;
            k(w, "wake_at")?;
            write!(w, "{wake_at}")
        }
        EventKind::Sched(SchedEvent::Wake { tid }) => {
            write!(w, ",\"kind\":\"wake\"")?;
            k(w, "tid")?;
            write!(w, "{tid}")
        }
        EventKind::Sched(SchedEvent::Exit { tid }) => {
            write!(w, ",\"kind\":\"exit\"")?;
            k(w, "tid")?;
            write!(w, "{tid}")
        }
        EventKind::Sched(SchedEvent::Reap { tid }) => {
            write!(w, ",\"kind\":\"reap\"")?;
            k(w, "tid")?;
            write!(w, "{tid}")
        }
        EventKind::Sched(SchedEvent::Idle) => write!(w, ",\"kind\":\"idle\""),
        EventKind::Env(EnvEvent::Call { call, arg }) => {
            write!(w, ",\"kind\":\"envcall\"")?;
            k(w, "call")?;
            write!(w, "{call}")?;
            k(w, "arg")?;
            write!(w, "{arg:#x}")
        }
        EventKind::Mem(MemEvent::PageFault {
            va,
            fault,
            resolved,
        }) => {
            write!(w, ",\"kind\":\"pagefault\"")?;
            k(w, "va")?;
            write!(w, "{va:#x}")?;
            k(w, "fault")?;
            let mut f = Fmt::<32>::new();
            let _ = write!(f, "{fault:?}");
            v(w, f.as_str())?;
            k(w, "resolved")?;
            write!(w, "{resolved}")
        }
        EventKind::Mem(MemEvent::Integrity { code, addr }) => {
            write!(w, ",\"kind\":\"integrity\"")?;
            k(w, "code")?;
            write!(w, "{code}")?;
            k(w, "addr")?;
            write!(w, "{addr:#x}")
        }
        EventKind::Halt(HaltEvent::Halt) => write!(w, ",\"kind\":\"halt\""),
        EventKind::Halt(HaltEvent::Panic) => write!(w, ",\"kind\":\"panic\""),
        EventKind::Boot(BootEvent::Launch { hart }) => {
            write!(w, ",\"kind\":\"launch\"")?;
            k(w, "hart")?;
            write!(w, "{hart}")
        }
        EventKind::Boot(BootEvent::Done { hart }) => {
            write!(w, ",\"kind\":\"bootdone\"")?;
            k(w, "hart")?;
            write!(w, "{hart}")
        }
        EventKind::Boot(BootEvent::Stack { hart, used }) => {
            write!(w, ",\"kind\":\"trpstack\"")?;
            k(w, "hart")?;
            write!(w, "{hart}")?;
            k(w, "used")?;
            write!(w, "{used}")
        }
    }
}

/// 对窗口内最近 ≤k 条事件从旧到新调 f。
///
/// 用回调而非返回 &[Event]：环形窗口可能跨缝（cursor 为绝对下标、读按取模），
/// 给不出一段连续切片。panic_dump / 未来 semihost 都经此消费同一事件流。
pub fn dump<F: FnMut(&Event)>(hart: usize, k: usize, mut f: F) {
    let Some(t) = POOL.get().and_then(|p| p.get(hart)) else {
        return;
    };
    let w = t.cursor.load(Ordering::Relaxed);
    let start = w.saturating_sub(k);
    for i in start..w {
        // SAFETY: i % BUFFER_SIZE < BUFFER_SIZE；读侧此时无写者（panic 停写 / 单读）。
        f(unsafe { &*t.buffer.as_ptr().add(i % BUFFER_SIZE) });
    }
}

/// 清空某 hart 窗口（boot / 复现）。
pub fn reset(hart: usize) {
    if let Some(t) = POOL.get().and_then(|p| p.get(hart)) {
        t.clear();
    }
}

/// 初始化（boot 恰好一次）：从 spare 仓取 ring_bytes 的常驻环（窗口表 + 事件槽）。
/// 失败 = 预算错误，返回 Err 由 main 统一 fail-fast（panic → halt）。须在 clock
/// 就绪后、任何 note 之前调用。
///
/// # Errors
///
/// spare 仓余量不足（预算与 ring 失配）→ [`TraceInitError::OutOfMemory`]；
/// 重复初始化 → [`TraceInitError::AlreadyInit`]。
pub fn init() -> Result<(), TraceInitError> {
    let h = machine::hart_count();
    let total = ring_bytes(h);
    // 16 为 2 的幂硬对齐，from_size_align 不可失败（不变量）。
    let layout = Layout::from_size_align(total, 16).expect("trace: ring layout");
    let chunk = spare::allocator()
        .allocate(layout)
        .map_err(|_| TraceInitError::OutOfMemory)?;
    // SAFETY: chunk 为 spare 仓内块（16B 对齐）；下分窗口表区 + 事件槽区，互不重叠。
    let base = chunk.as_ptr() as *mut u8 as usize;
    let traces = base as *mut Trace;
    let events = (base + h * size_of::<Trace>()) as *mut Event;
    for i in 0..h {
        // SAFETY: 槽区总长 = h × BUFFER_SIZE × size_of<Event>，本窗口切片在界内。
        let buf =
            unsafe { core::slice::from_raw_parts_mut(events.add(i * BUFFER_SIZE), BUFFER_SIZE) };
        for slot in buf.iter_mut() {
            *slot = Event::EMPTY;
        }
        // SAFETY: traces 区内第 i 个 Trace 未初始化；boot 单核写入，无并发。
        unsafe {
            traces.add(i).write(Trace {
                cursor: AtomicUsize::new(0),
                buffer: buf,
            });
        }
    }
    // SAFETY: 全部 h 个 Trace 已初始化；此后只读结构（环写经裸指针 + 原子游标）。
    let pool = unsafe { core::slice::from_raw_parts(traces, h) };
    POOL.set(pool).map_err(|_| TraceInitError::AlreadyInit)?;
    Ok(())
}

/// trace 初始化错误。
#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceInitError {
    /// spare 仓余量不足（容量预算与 ring 需求失配）。
    #[error("spare ring allocation failed")]
    OutOfMemory,
    /// 重复初始化。
    #[error("trace already initialized")]
    AlreadyInit,
}

// ── 适配：panic_dump ────────────────────────────────

/// 事件描述文本（无时间前缀）——供表格列 1 使用。
fn fmt_description(e: &Event, w: &mut impl fmt::Write) -> fmt::Result {
    match e.kind {
        EventKind::Sched(SchedEvent::Spawn { tid }) => write!(w, "spawn tid={tid}"),
        EventKind::Sched(SchedEvent::Switch { prev_tid, next_tid }) => {
            write!(w, "switch {prev_tid}->{next_tid}")
        }
        EventKind::Sched(SchedEvent::Starve { tid }) => write!(w, "starve tid={tid}"),
        EventKind::Sched(SchedEvent::Steal { tid, src_hart }) => {
            write!(w, "steal tid={tid} from hart {src_hart}")
        }
        EventKind::Sched(SchedEvent::Park { tid, wake_at }) => {
            write!(w, "park tid={tid} @{wake_at:#x}")
        }
        EventKind::Sched(SchedEvent::Wake { tid }) => write!(w, "wake tid={tid}"),
        EventKind::Sched(SchedEvent::Exit { tid }) => write!(w, "exit tid={tid}"),
        EventKind::Sched(SchedEvent::Reap { tid }) => write!(w, "reap tid={tid}"),
        EventKind::Sched(SchedEvent::Idle) => write!(w, "idle"),
        EventKind::Env(EnvEvent::Call { call, arg }) => write!(w, "envcall #{call} arg={arg:#x}"),
        EventKind::Mem(MemEvent::PageFault {
            va,
            fault,
            resolved,
        }) => {
            write!(
                w,
                "pagefault va={va:#x} kind={:?} resolved={resolved}",
                fault
            )
        }
        EventKind::Mem(MemEvent::Integrity { code, addr }) => {
            write!(w, "integrity code={code} addr={addr:#x}")
        }
        EventKind::Halt(HaltEvent::Halt) => write!(w, "halt"),
        EventKind::Halt(HaltEvent::Panic) => write!(w, "panic"),
        EventKind::Boot(BootEvent::Launch { hart }) => write!(w, "launch hart {hart}"),
        EventKind::Boot(BootEvent::Done { hart }) => write!(w, "boot done hart {hart}"),
        EventKind::Boot(BootEvent::Stack { hart, used }) => {
            write!(w, "trap stack high-water hart {hart}: {used} B")
        }
    }
}

/// 每 hart 倒出行数（总量平摊）：总额恒 ≤ TRACE_DUMP——渲染预算与核数无关，
/// 64 hart 极端配置也不超 spare 的打印预算。
pub fn hart_rows() -> usize {
    (TRACE_DUMP / machine::hart_count()).max(1)
}

/// 崩溃转储：遍历已启动各 hart 的最近窗口，按段落（标题 + 两列表 t/描述）倒出。
///
/// 必须在 halt 已让其它核停写后调用（panic_handler 的报警核）。表入全局收集器
/// （render::push，stanza 定宽截断建格；控制台静默），由 scene::dump_crash 末尾
/// render_all 统一打印（「收集完所有信息后再打印」）。只倒最近 hart_rows 条
/// （总量平摊）。
pub fn panic_dump() {
    const TRACE_W: [usize; 2] = [18, 45]; // t = {:#018x} 18 字符；ΣW + 1 = 64
    for h in 0..machine::hart_count() {
        let mut tab = render::fixed_table(&TRACE_W);
        dump(h, hart_rows(), |e| {
            let mut w = Fmt::<40>::new();
            w.hexw(e.when as usize);
            let mut d = Fmt::<96>::new();
            let _ = fmt_description(e, &mut d);
            // with_row 消耗 self：FnMut 捕获内不能 move，经 mem::take 换出再链。
            tab = core::mem::take(&mut tab).with_row(render::row(&TRACE_W, [w.as_str(), d.as_str()]));
        });
        render::push(format!("[trace] hart {h}:"), &tab);
    }
}
