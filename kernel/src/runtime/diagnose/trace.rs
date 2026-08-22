//! trace — 内核事件环形缓冲（崩溃后反推的依据）。
//!
//! 分层（signature 即文档）：
//!   事件 = 按模块分组、字段自足的枚举（EventKind 聚合）
//!   核心 = Trace（per-hart 窗口）+ note / dump / reset / init
//!   适配 = panic_dump（把崩溃前最近窗口倒到控制台）+ 宿主镜像（共享同一事件流）
//!
//! 设计决策：
//!   - per-hart、静态池（.bss 按 POOL_HARTS×BUFFER_SIZE 备足），**不经分配器**——
//!     崩溃现场分配器不可信，缓冲必须仍可用。
//!   - 写 = 单生产者无锁：cursor 单调不回卷、读侧取模；读只在 panic，此时其余核
//!     已由 halt 停写（写读互斥由 halt 语义保证）。
//!   - 宿主镜像 = diagnose::export（feature semihosting，**默认开启**）：每条事件
//!     一条 JSON 记录写入宿主文件 sqware-diagnose.jsonl（qemu CWD 下；首行 '#' 头部
//!     溯源），runner 归档；依赖 QEMU -semihosting（runner 恒加）。
//!   - 事件写者 = host_note（live 流，经 export::line 串行）：panic 只经
//!     note(Halt(Panic)) 追加一条事件行，决不把环形窗口再 dump 一遍进文件；
//!     环形窗口文本由 scene::dump_crash 恒倒到控制台（人读上下文）。

use core::fmt;
use core::fmt::Write;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::console::Sink;
use crate::memory::manager::fault::FaultKind;
use crate::{machine, putln};
use table::{Cell, Fmt, Para, Table};

/// 每 hart 事件窗口容量。
pub const BUFFER_SIZE: usize = 512;
/// 崩溃转储每条 hart 倒出的事件数上限：现场只关心「崩前最近一段」，
/// 全量 512 对控制台过长且 Table 栈预算不可承载（ROWS 编译期常量）。
pub const TRACE_DUMP: usize = 64;
/// trace 可容纳的诊断核数上限（静态池按此备足）。
///
/// 内核核数已完全由 DTB 动态决定，无编译期上限；诊断环形按一个务实上限
/// （普通 SMP/仿真远低于此）在 .bss 静态备足，超限则 trace 静默停用（init 告警）。
pub const POOL_HARTS: usize = 64;

// ── 事件定义（按模块分组）────────────────────────────

/// 各模块事件的聚合。
#[derive(Clone, Copy)]
pub enum EventKind {
    Sched(SchedEvent),
    Env(EnvEvent),
    Mem(MemEvent),
    Halt(HaltEvent),
    Watch(WatchEvent),
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
    /// 完整性违例（IntegrityViolation 的 repr(u8) 编码；见 memory::integrity）。
    Integrity { code: u8, addr: usize },
}

/// runtime/halt 的系统级事件。
#[derive(Clone, Copy)]
pub enum HaltEvent {
    Halt,
    Panic,
}

/// runtime/watch 值班看护的报警事件。
#[derive(Clone, Copy)]
pub enum WatchEvent {
    Raised,
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
    buffer: [Event; BUFFER_SIZE],
}

impl Trace {
    const fn new() -> Trace {
        Trace {
            cursor: AtomicUsize::new(0),
            buffer: [Event::EMPTY; BUFFER_SIZE],
        }
    }

    fn note(&self, kind: EventKind, when: u64) {
        let i = self.cursor.fetch_add(1, Ordering::Relaxed) % BUFFER_SIZE;
        // SAFETY: i < BUFFER_SIZE；单生产者（本 hart 唯一写者），与跨核读者互斥由 halt 停写保证。
        unsafe { *(self.buffer.as_ptr() as *mut Event).add(i) = Event { when, kind } };
    }

    fn clear(&self) {
        self.cursor.store(0, Ordering::Relaxed);
    }
}

/// 静态池（.bss，不经分配器）。逻辑切分：hart h → POOL[h]（独立 512 条窗口）。
static POOL: [Trace; POOL_HARTS] = [const { Trace::new() }; POOL_HARTS];

/// 第 h 个 hart 的窗口（调用方保证 h < POOL_HARTS）。
fn trace(hart: usize) -> &'static Trace {
    &POOL[hart]
}

// ── 核心原语 ────────────────────────────────────────

/// 记一条事件到**本 hart** 窗口（盖 when 时间戳）。
///
/// 尽力而为、不失败：hart 越界即丢弃；诊断路径永不允许失败或 panic。
/// 只进环形（release/panic 转储），不做实时控制台输出——不扰动时序。
pub fn note(kind: EventKind) {
    let hart = machine::hart_id();
    if hart >= POOL_HARTS {
        return;
    }
    let when = crate::runtime::chrono::clock::now().as_ticks();
    trace(hart).note(kind, when);
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
        EventKind::Watch(WatchEvent::Raised) => write!(w, ",\"kind\":\"watch\""),
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
    if hart >= POOL_HARTS {
        return;
    }
    let t = trace(hart);
    let w = t.cursor.load(Ordering::Relaxed);
    let start = w.saturating_sub(k);
    for i in start..w {
        // SAFETY: i % BUFFER_SIZE < BUFFER_SIZE；读侧此时无写者（panic 停写 / 单读）。
        f(unsafe { &*t.buffer.as_ptr().add(i % BUFFER_SIZE) });
    }
}

/// 清空某 hart 窗口（boot / 复现）。
pub fn reset(hart: usize) {
    if hart < POOL_HARTS {
        trace(hart).clear();
    }
}

/// 初始化（boot 恰好一次）。
///
/// 静态池编译期已备足，本函数只做防御断言（核数 ≤ 静态上限），并作为未来
/// semihost 镜像 / 构建开关的接线点。须在 clock 就绪后、任何 note 之前调用。
pub fn init() {
    // 静态池为 POOL_HARTS 核备足。核数超限不 panic（避免破坏大 SMP 启动），
    // 而是告警并静默停用：note 对越限 hart 直接丢弃。
    if machine::hart_count() > POOL_HARTS {
        crate::putln!(
            "trace: hart_count {} exceeds diagnostic cap {POOL_HARTS}; trace disabled",
            machine::hart_count()
        );
    }
    // 清空全部窗口：boot 建立干净基线（支持复现/重入）。静态池初始即空，此处确立契约。
    for h in 0..POOL_HARTS {
        reset(h);
    }
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
        EventKind::Watch(WatchEvent::Raised) => write!(w, "watch raised"),
        EventKind::Boot(BootEvent::Launch { hart }) => write!(w, "launch hart {hart}"),
        EventKind::Boot(BootEvent::Done { hart }) => write!(w, "boot done hart {hart}"),
        EventKind::Boot(BootEvent::Stack { hart, used }) => {
            write!(w, "trap stack high-water hart {hart}: {used} B")
        }
    }
}

/// 崩溃转储：遍历已启动各 hart 的最近窗口，按段落（标题 + 两列表 t/描述）倒出。
///
/// 必须在 halt 已让其它核停写后调用（panic_handler 的报警核）。无分配、无锁。
/// 每 hart 一个段落（标题后空行、表格经 Para 缩进 2 空格——与 scene/depend 同
/// 格式）。只倒最近 TRACE_DUMP 条。
pub fn panic_dump() {
    for h in 0..machine::hart_count() {
        let mut p = Para::new(Sink);
        p.title(format_args!("[trace] hart {h}:"));
        let mut tab = Table::<2, { TRACE_DUMP }, 96>::new();
        tab.set_total_width(64); // 统一诊断表宽预算（同 scene，最长行同宽）。
        {
            let mut it = tab.rows_mut();
            dump(h, TRACE_DUMP, |e| {
                let Some(row) = it.next() else {
                    return;
                };
                let mut w = Fmt::<40>::new();
                w.hexw(e.when as usize);
                let mut d = Fmt::<96>::new();
                let _ = fmt_description(e, &mut d);
                row[0] = Cell::new(w.as_str());
                row[1] = Cell::new(d.as_str());
            });
        }
        p.table(&tab);
    }
}
