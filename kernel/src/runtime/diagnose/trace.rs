//! trace — 内核事件环形缓冲（崩溃后反推的依据）。
//!
//! 事件 = 按模块分组、字段自足的枚举（EventKind 聚合）；核心 = Trace（per-hart
//! 窗口）+ note / dump / reset / init；适配 = panic_dump + 宿主镜像。

use core::fmt;
use core::mem::size_of;
use core::sync::atomic::{AtomicUsize, Ordering};
use serde::Serialize;

use alloc::alloc::{Allocator, Layout};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use fack::prelude::Error;

use crate::lock::OnceLock;
use crate::machine;
use crate::memory::allocator::spare;
use crate::memory::manager::fault::FaultKind;
use crate::runtime::diagnose::report::Report;

/// 每 hart 事件窗口容量。
pub const BUFFER_SIZE: usize = 512;
/// 崩溃转储每条 hart 倒出的事件数上限（多核按 hart 平摊）。
pub const TRACE_DUMP: usize = 64;

// ── 事件定义（按模块分组）────────────────────────────

/// 各模块事件的聚合。
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Room(RoomEvent),
    Env(EnvEvent),
    Memory(MemoryEvent),
    Halt(HaltEvent),
    Boot(BootEvent),
}

/// 调度事件。
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomEvent {
    Spawn { tid: usize },
    Switch { prev_tid: usize, next_tid: usize },
    Starve { tid: usize },
    Steal { tid: usize, src_hart: usize },
    Park { tid: usize, wake_at: usize },
    Wait { tid: usize, key: usize },
    Wake { tid: usize },
    Exit { tid: usize },
    Reap { tid: usize },
    Idle,
}

/// 环境调用事件。
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvEvent {
    Call { call: usize, arg: usize },
}

/// 内存事件（缺页 + 完整性违例）。
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryEvent {
    PageFault {
        va: usize,
        fault: FaultKind,
        resolved: bool,
    },
    /// 完整性违例（repr(u8) 编码）。
    Integrity { code: u8, addr: usize },
}

/// 停机事件。
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HaltEvent {
    Halt,
    Panic,
}

/// boot 初始化消息（各 hart 启动时一次）。
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BootEvent {
    /// 主核 call 该副核（HSM start）。
    Launch { hart: usize },
    /// 副核 trap/调度初始化完成。
    Done { hart: usize },
}

/// 一条事件：时间戳 + 聚合事件。仅标量、Copy，可 const 初始化。
#[derive(Clone, Copy, Serialize)]
pub struct Event {
    when: u64,
    kind: EventKind,
}

impl Event {
    const EMPTY: Event = Event {
        when: 0,
        kind: EventKind::Room(RoomEvent::Idle),
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

    #[allow(unused)]
    fn clear(&self) {
        self.cursor.store(0, Ordering::Relaxed);
    }
}

/// 事件池（spare 仓内）：写入一次、读多次。分配于 `init`，环在分配后只读结构。
static POOL: OnceLock<&'static [Trace]> = OnceLock::new();

/// 环形常驻字节数（h 个窗口表 + 事件槽，size_of 精确）。
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
    // 宿主镜像（semihosting）：每条结构化事件送宿主。
    #[cfg(feature = "semihosting")]
    host_note(kind, hart, when);
}

// ── 宿主镜像适配（feature gate: semihosting）──────────────────────────

/// 宿主镜像：把一条事件序列化成 JSON 记录写入宿主文件。序列化全交 serde_json；
/// 失败静默（诊断路径永不 panic）。
#[cfg(feature = "semihosting")]
fn host_note(kind: EventKind, hart: usize, when: u64) {
    use crate::runtime::diagnose::export::push;
    let e = Event { when, kind };
    if let Ok(json) = serde_json::to_vec(&HostEvent { h: hart, e: &e }) {
        push(&json);
    }
}

/// 事件导出行：`{"h":…,"when":…,"kind":…}`——hart 由包装补，when/kind 经
/// flatten 从 [`Event`] 展开（内存环不含 hart，per-hart 窗口只是隐式的）。
#[cfg(feature = "semihosting")]
#[derive(Serialize)]
struct HostEvent<'a> {
    h: usize,
    #[serde(flatten)]
    e: &'a Event,
}

/// 对窗口内最近 ≤k 条事件从旧到新调 f。
///
/// 用回调而非返回 &[Event]：环形窗口可能跨缝（cursor 为绝对下标、读按取模），
/// 给不出一段连续切片。
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

/// 初始化：从 spare 仓取 ring_bytes 的常驻环（窗口表 + 事件槽）。失败 = 预算错误，
/// 返回 Err。须在 clock 就绪后、任何 note 之前调用。
///
/// # Errors
///
/// spare 仓余量不足 → [`TraceInitError::OutOfMemory`]；重复初始化 → [`TraceInitError::AlreadyInit`]。
pub fn init() -> Result<(), TraceInitError> {
    let h = machine::hart_count();
    let total = ring_bytes(h);
    // 16 为 2 的幂硬对齐，from_size_align 不可失败（不变量）。
    let layout = Layout::from_size_align(total, 16).expect("trace: ring layout");
    let chunk = spare::spare()
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
    /// spare 仓余量不足。
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
        EventKind::Room(RoomEvent::Spawn { tid }) => write!(w, "spawn tid={tid}"),
        EventKind::Room(RoomEvent::Switch { prev_tid, next_tid }) => {
            write!(w, "switch {prev_tid}->{next_tid}")
        }
        EventKind::Room(RoomEvent::Starve { tid }) => write!(w, "starve tid={tid}"),
        EventKind::Room(RoomEvent::Steal { tid, src_hart }) => {
            write!(w, "steal tid={tid} from hart {src_hart}")
        }
        EventKind::Room(RoomEvent::Park { tid, wake_at }) => {
            write!(w, "park tid={tid} @{wake_at:#x}")
        }
        EventKind::Room(RoomEvent::Wait { tid, key }) => {
            write!(w, "wait tid={tid} key={key:#x}")
        }
        EventKind::Room(RoomEvent::Wake { tid }) => write!(w, "wake tid={tid}"),
        EventKind::Room(RoomEvent::Exit { tid }) => write!(w, "exit tid={tid}"),
        EventKind::Room(RoomEvent::Reap { tid }) => write!(w, "reap tid={tid}"),
        EventKind::Room(RoomEvent::Idle) => write!(w, "idle"),
        EventKind::Env(EnvEvent::Call { call, arg }) => write!(w, "envcall #{call} arg={arg:#x}"),
        EventKind::Memory(MemoryEvent::PageFault {
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
        EventKind::Memory(MemoryEvent::Integrity { code, addr }) => {
            write!(w, "integrity code={code} addr={addr:#x}")
        }
        EventKind::Halt(HaltEvent::Halt) => write!(w, "halt"),
        EventKind::Halt(HaltEvent::Panic) => write!(w, "panic"),
        EventKind::Boot(BootEvent::Launch { hart }) => write!(w, "launch hart {hart}"),
        EventKind::Boot(BootEvent::Done { hart }) => write!(w, "boot done hart {hart}"),
    }
}

/// 每 hart 倒出行数（总量平摊）：总额恒 ≤ TRACE_DUMP。
pub fn hart_rows() -> usize {
    (TRACE_DUMP / machine::hart_count()).max(1)
}

/// 崩溃转储：遍历已启动各 hart 的最近窗口，每人开一段（标题 + 两列表 t/描述）
/// 投进报告（表中首行恒为表头）。只倒最近 hart_rows 条（总量平摊）。
pub fn panic_dump(r: &mut Report) {
    for h in 0..machine::hart_count() {
        let mut rows: Vec<Vec<Option<String>>> = vec![
            vec![Some("t".into()), Some("event".into())], // 首行表头
        ];
        dump(h, hart_rows(), |e| {
            let mut d = String::new();
            let _ = fmt_description(e, &mut d);
            rows.push(vec![Some(format!("{:#018x}", e.when)), Some(d)]);
        });
        r.paragraph("trace", Some(format!("[trace] hart {h}:")))
            .items
            .extend(rows);
    }
}
