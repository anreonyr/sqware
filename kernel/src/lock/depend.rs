// depend — 锁依赖（lock-order）校验：单 hart 重入检测 + 跨锁层级强制（debug）。
//
// 中心意象 depend「依赖」：回答「这把锁现在能不能取」。已有的单 hart 能力
// （read_ra / report）沿用；新增 per-hart「已持集合」+ 取锁前层级校验（hazard 报警），
// 把 lock/mod.rs 的 1-6 层级契约从注释变成运行时强制。
//
// 语义：每 hart 维护自己当前持有的参与锁集合，层级严格递增（新 level 必须
// > 已持最大 level，同层/下降即违规）。取锁前的 check 在自旋之前执行——抓得到
// ABBA（死锁发生在自旋后，自旋前校验必先暴露）。
//
// 锁纪律：held set 仅本 hart + SIE 关时读写（lock 全程关中断）=> 无数据竞争、
// 无需原子与锁；acquire 在取到后记入、release 在 guard Drop 移除。POOL 未装配
// （boot 早期）时各操作静默跳过——装配点（depend::init）前无需校验。
//
// 仅 DEBUG：整个 held-set/层级强制部分包在 #[cfg(debug_assertions)]；release 只留
// read_ra/report（单 hart 重入检测，维持现行为）。
//
// 表格输出：复用 crate table（papergrid 无堆渲染），本模块只负责把报警现场
// 组装成 Table 并写到控制台 sink。符号器由 boot 注入 table::set_symbolizer。

use core::cell::UnsafeCell;
use core::fmt::Write;

use alloc::boxed::Box;
use alloc::vec::Vec;

use super::OnceLock;
use crate::console;
use crate::machine;
use crate::putln;
use table::{Table, render_addr};

/// 锁层级 — lock/mod.rs 层级契约的具名化（1 最低、6 最高）。
/// 参与锁才有 level；Option<Level>::None = exempt（不参与、不校验）。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[repr(u8)]
pub enum Level {
    /// SCHEDULERS[hart]
    Scheduler = 1,
    /// Space.inner (RelLock)
    Space = 2,
    /// Team.tasks / TIMER_DEADLINES / blocked / reaped
    L3 = 3,
    /// ASID_ALLOCATOR
    Asid = 4,
    /// FRAME_ALLOCATOR
    Frame = 5,
    /// portal / block
    ///
    /// 注意：block 的 `inner`/`pump` 是**每池实例**锁（同 per-hart 调度锁），
    /// 其互不嵌套靠路由纪律（feed 持本池 pump 时可能经分配器锁他池 inner，
    /// 同层 6→6 高发但安全，因每池实例互不相扰）——故保持 exempt，**勿加**
    /// 本层级触发误报。
    Block = 6,
    /// memory::integrity::LEDGER（层级末尾：只在无锁或低层级锁内获取，且绝不在
    /// 持本锁时触碰分配器——容量 init 预留、运行期插入零分配）。
    Ledger = 7,
}

// ── 单 hart 重入检测（沿用；release 亦生效）──────────────────────────

/// 读取调用者返回地址（ra）。#[inline(never)] 锁入口保证 ra 有效。
#[inline(always)]
pub(crate) fn ra() -> usize {
    let ra: usize;
    // SAFETY: 读 ra 无副作用；asm 未声明 ra 视为 clobber，编译器不假设它保持。
    unsafe { core::arch::asm!("mv {}, ra", out(reg) ra) };
    ra
}

/// 单 hart 重入/升级现场报告后 panic。
///
/// 用 table 渲染两列表（label / addr），整块写控制台。
pub(crate) fn report(
    kind: &'static str,
    what: &'static str,
    lock: usize,
    holder: usize,
    caller: usize,
) -> ! {
    putln!("[depend] {kind}: {what} (single-hart lock-order violation)");
    // hart 首行并入 Table（值列写数字，非地址）；表格第 0 列顶格，无行首空格。
    let mut t = Table::<4, 2, 64>::new();
    t.set_col_width(0, 10);
    {
        let row = t.open_row();
        row[0].push_str("hart");
        let _ = write!(&mut row[1], "{}", machine::hart_id());
    }
    for (label, addr) in [("lock", lock), ("holder", holder), ("caller", caller)] {
        let row = t.open_row();
        row[0].push_str(label);
        let _ = render_addr(&mut row[1], addr);
    }
    write_table(t);
    panic!("{kind} lock-order violation: {what}");
}

// ── 跨锁层级强制（仅 debug）──────────────────────────────────────────

/// 每 hart 持有集最大深度（真实嵌套 <=4；8 为防御线，溢出即违规）。
#[cfg(debug_assertions)]
const MAX_HELD: usize = 8;

/// 持有集一个槽位。
#[derive(Clone, Copy)]
#[cfg(debug_assertions)]
struct Held {
    addr: usize,
    level: Level,
}

/// 本 hart 当前持有的参与锁（有界栈；仅本 hart + SIE 关访问）。
#[cfg(debug_assertions)]
struct HeldSet {
    len: usize,
    slots: [Held; MAX_HELD],
}

#[cfg(debug_assertions)]
impl HeldSet {
    const fn new() -> HeldSet {
        HeldSet {
            len: 0,
            slots: [Held {
                addr: 0,
                level: Level::Scheduler,
            }; MAX_HELD],
        }
    }

    /// 已持最大层级（空集 → None）。
    fn max_level(&self) -> Option<Level> {
        self.slots[..self.len].iter().map(|h| h.level).max()
    }

    /// 是否已持 addr。
    fn contains(&self, addr: usize) -> bool {
        self.slots[..self.len].iter().any(|h| h.addr == addr)
    }

    /// 记入；满 → Err（溢出违规，由调用方报警）。
    fn push(&mut self, addr: usize, level: Level) -> Result<(), ()> {
        if self.len >= MAX_HELD {
            return Err(());
        }
        self.slots[self.len] = Held { addr, level };
        self.len += 1;
        Ok(())
    }

    /// 移除 addr；缺失 → Err（计数失步违规）。
    fn remove(&mut self, addr: usize) -> Result<(), ()> {
        for i in 0..self.len {
            if self.slots[i].addr == addr {
                self.slots.copy_within(i + 1..self.len, i);
                self.len -= 1;
                return Ok(());
            }
        }
        Err(())
    }
}

/// 每 hart 持有集单元（UnsafeCell：仅本核 + SIE 关可写；panic 现场只读，其余核
/// 已被 halt 停核冻结——跨核 best-effort 读取不在此版，见 hazard 注释）。
#[cfg(debug_assertions)]
struct HeldCell(UnsafeCell<HeldSet>);

// SAFETY: 每核只写自己那份，唯一 &mut 由 held_mut（本核 + SIE 关）保证。
#[cfg(debug_assertions)]
unsafe impl Send for HeldCell {}
#[cfg(debug_assertions)]
unsafe impl Sync for HeldCell {}

/// per-hart 持有集池（depend::init 装配后只读索引）。
#[cfg(debug_assertions)]
static POOL: OnceLock<&'static [HeldCell]> = OnceLock::new();

/// lockdep 装配错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(debug_assertions)]
pub enum DepInitError {
    /// 分配失败（OOM）。
    OutOfMemory,
    /// 重复装配。
    AlreadyInit,
}

/// 装配 per-hart 持有集；boot 一次，须在分配器就绪后、首个参与锁使用前。
#[cfg(debug_assertions)]
pub(crate) fn init(hart_count: usize) -> Result<(), DepInitError> {
    if POOL.get().is_some() {
        return Err(DepInitError::AlreadyInit);
    }
    let n = hart_count.clamp(1, crate::machine::MAX_HART_SLOTS);
    let cells: Vec<HeldCell> = (0..n)
        .map(|_| HeldCell(UnsafeCell::new(HeldSet::new())))
        .collect();
    let pool: &'static [HeldCell] = Box::leak(cells.into_boxed_slice());
    POOL.set(pool).map_err(|_| DepInitError::AlreadyInit)
}

/// 本 hart 持有集（POOL 未装配 → None）。前置：SIE 关 + 本核单流。
#[cfg(debug_assertions)]
fn held_mut() -> Option<&'static mut HeldSet> {
    let pool = POOL.get()?;
    let h = machine::hart_id();
    if h >= pool.len() {
        panic!("depend: hart {h} out of pool ({} slots)", pool.len());
    }
    // SAFETY: 持有集仅本核 + SIE 关访问；panic 路径只读，其余核已被 halt 冻结。
    Some(unsafe { &mut *pool[h].0.get() })
}

/// 取锁前校验：允许取 level 当且仅当 level > max(held) 且本核未持同 addr。
/// 通过 → 不动状态（适配层随后 acquire）；违规 → hazard（不返回）。
/// POOL 未装配 → 静默跳过（boot 早期）。
#[cfg(debug_assertions)]
pub(crate) fn check(addr: usize, level: Level, caller: usize) {
    let Some(held) = held_mut() else {
        return;
    };
    let (contains, max) = (held.contains(addr), held.max_level());
    if contains || max.is_some_and(|m| level <= m) {
        hazard(addr, level, caller);
    }
}

/// 取到后记入本核持有集。前置：已过 check；本核未持该锁。
#[cfg(debug_assertions)]
pub(crate) fn acquire(addr: usize, level: Level) {
    let Some(held) = held_mut() else {
        return;
    };
    if held.push(addr, level).is_err() {
        hazard(addr, level, 0);
    }
}

/// 释放时移除本核持有集条目。前置：对应 acquire 已发生。
#[cfg(debug_assertions)]
pub(crate) fn release(addr: usize, level: Level) {
    let Some(held) = held_mut() else {
        return;
    };
    if held.remove(addr).is_err() {
        putln!("[depend] release of unheld lock {addr:#x} (level {level:?})");
        panic!("depend: release of unheld lock {addr:#x}");
    }
}

/// 锁序违规报警：打印本核持有集后 panic → halt（其余核由 halt::alarm 停核）。
/// 跨核持有集 best-effort 读取留待后续（需 raw usize 拷贝避免 &mut 别名 UB）。
#[cfg(debug_assertions)]
pub(crate) fn hazard(addr: usize, level: Level, caller: usize) -> ! {
    putln!("[depend] lock-order hazard");
    let Some(held) = held_mut() else {
        panic!("depend: lock-order hazard taking {addr:#x} @ {level:?}");
    };
    // 行 = label / addr / 说明。hart 首行并入 Table（值列写数字），taking + caller 两行，中间夹 held 各行。
    let mut t = Table::<{ MAX_HELD + 3 }, 3, 96>::new();
    t.set_col_width(0, 10);
    {
        let row = t.open_row();
        row[0].push_str("hart");
        let _ = write!(&mut row[1], "{}", machine::hart_id());
    }
    {
        let row = t.open_row();
        row[0].push_str("taking");
        let _ = render_addr(&mut row[1], addr);
        let _ = write!(&mut row[2], "({:?})", level);
    }
    if held.len == 0 {
        let row = t.open_row();
        row[0].push_str("held");
        let _ = write!(&mut row[2], "(none)");
    } else {
        let max = held.max_level().unwrap_or(level);
        for i in 0..held.len {
            let row = t.open_row();
            row[0].push_str("held");
            let _ = render_addr(&mut row[1], held.slots[i].addr);
            let _ = write!(
                &mut row[2],
                "({:?}){}",
                held.slots[i].level,
                if held.slots[i].level == max {
                    "  <-- max held"
                } else {
                    ""
                }
            );
        }
    }
    {
        let row = t.open_row();
        row[0].push_str("caller");
        let _ = render_addr(&mut row[1], caller);
    }
    write_table(t);
    putln!(
        "  rule       new level must exceed max(held);  {:?} <= max => violation",
        level
    );
    panic!("depend: lock-order hazard taking {addr:#x} @ {level:?}");
}

/// 把整表渲染进一个栈缓冲，再整块写控制台（无堆；一次 SBI 调用）。
/// 无 cfg：`depend::report`（单 hart 重入检测）release 亦生效，须可渲染现场表。
/// Table::render 末行不补尾换行，此处补——表格块须独立结束，避免紧接下行输出。
fn write_table<const R: usize, const C: usize, const CAP: usize>(t: Table<R, C, CAP>) {
    let mut buf: table::Line<512> = table::Line::new();
    let _ = t.render(&mut buf);
    console::_write(format_args!("{buf}\n"));
}
