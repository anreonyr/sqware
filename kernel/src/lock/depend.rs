// depend — 锁依赖（lock-order）校验：跨锁层级强制 + 不可重入锁的同锁重入检测。
//
// **仅 debug 构建编译本模块的校验机制**（POOL/held set/check/acquire/release/
// report/init 全体 `#[cfg(debug_assertions)]`）；`Level` 值类型保留（release 也
// 编译——锁的 `level` 字段与 `new_level` 调用点免 cfg，机制关闭即零开销）。
//
// 中心意象 depend「依赖」：回答「这把锁现在能不能取」。每 hart 维护自己当前
// 持有的锁集合（参与锁层级严格递增：新 level 必须 > 已持最大 level，同层/
// 下降即违规；exempt 锁记入但不计入层级）；取锁前的 check 在自旋之前执行
// ——抓得到 ABBA（死锁发生在自旋后，自旋前校验必先暴露）。
//
// 同锁重入的分界：**不可重入锁**（spin/bare）由 check 的 `contains` 判定为
// 违规；**可重入锁**（RelLock）在入口以 `owner == me` 跳过 check（重入计数
// 合法）；rw 共享读的递归读由 rw 的读路径保持自身检测（b 方案）。违规统一经
// [`report`] 报警（明细拼进 `[depend]` panic 消息，排版不是 depend 的职责）。
//
// 锁纪律：held set 仅本 hart + SIE 关时读写（lock 全程关中断）=> 无数据竞争、
// 无需原子与锁；acquire 在取到后记入（顺带记本次调用点）、release 在 guard
// Drop 移除。POOL 未装配（boot 早期）时各操作静默跳过。
//
// **exempt 锁（level=None）也记入持有集**（Held.level=None）：acquire/release
// 双侧都记账才平衡（否则 guard Drop 的 release 必然误报 unheld）；层级校验
// 只对 `Some(level)` 生效（max 只数参与锁），exempt 条目只作用于 `contains`
// ——block 池锁互不嵌套靠路由纪律、6→6 跨池合法不受层级误报，但同锁重入仍
// 必被 contains 揪出（单核自旋死锁的最后防线）。

use core::cell::UnsafeCell;

use alloc::boxed::Box;
use alloc::format;
use alloc::vec::Vec;

use super::OnceLock;
use crate::machine;
use crate::memory::manager::addr::VirtAddr;
use crate::work::unit::elftable;

/// 锁层级 — lock/mod.rs 层级契约的具名化（1 最低、9 最高）。
/// 参与锁才有 level；Option<Level>::None = exempt（不参与、不校验）。
/// 注册表的声明性条目（Block/Ledger/Tally/Spare 等当前无锁实际构造）：层级
/// 编号是契约的一部分，即使暂无用户也保留注册。
#[allow(dead_code)]
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
    /// block 的 `inner`/`pump`（每池实例锁，同 per-hart 调度锁）：互不嵌套靠
    /// 路由纪律（feed 持本池 pump 时可能经分配器锁他池 inner，同层 6→6 高发但
    /// 安全，因每池实例互不相扰）——保持 exempt，**勿加**本层级触发误报。
    Block = 6,
    /// allocator::fence::ledger::LEDGER（层级 7：只在无锁或低层级锁内获取；
    /// 持本锁**绝不分配**——容量 init 预留、运行期插入零分配。audit 只读块归属
    /// （pool_includes → tally，层级更高）不受此限，见 fence::audit）。
    Ledger = 7,
    /// block 簿记表（tally）：全部表访问（读/写/复合 RMW）自锁——own 单独持、
    /// 池内路径 inner → tally、审计 ledger → tally（审计只读）；绝无反向边
    /// （tally 是叶锁：锁内不再取任何锁）。portal 已无锁，此为簿记表现任闸门。
    Tally = 8,
    /// allocator::spare（后备仓）：崩溃打印 / trace 环形的分配源——常态显式调用、
    /// 崩溃经 portal 无锁切换（Backend::Spare）进入。恒居末尾（持 Ledger 不分配
    /// 的纪律之上再加一层保险）。
    Spare = 9,
}

/// 锁违规统一报警：明细拼进 `[depend]` panic 消息后 panic → halt（其余核由
/// halt::alarm 停核）。`what` = 违规说明；`lock` = 锁对象地址；`caller` =
/// 本次获取点（0 省略行）；held set 自取（check/acquire 语境必有）——逐行列
/// 出当前持有（含各锁的获取点 "acquired at"）。report / hazard 合一：重入、
/// 层级违规、溢出、计数失步全是同一函数的参数实例。
///
/// **第一件事：门户无锁切到后备仓（spare）**——`report` 的消息拼装
/// （format! / 符号化）要分配；此刻违规锁常仍被本核持着（guard Drop 内的
/// release 失配等），主堆分配会重入同一把锁自旋致死——切到 spare 后本函数及
/// 后续 panic_handler 的全部分配都进后备仓（其锁是独立实例、层级 9，正常持
/// 有链上无反向边），违规现场永不因"报告自己"而二次卡死。
#[cfg(debug_assertions)]
pub(crate) fn report(what: &'static str, lock: usize, caller: usize) -> ! {
    crate::memory::allocator::portal::switch(crate::memory::allocator::portal::Backend::Spare);
    let held = held().expect("depend: report outside collected set");
    let mut msg = format!(
        "[depend] {what}: {}",
        elftable::symbol(VirtAddr::from_raw(lock), None)
    );
    if caller != 0 {
        msg.push_str(&format!(
            "\n  caller: {}",
            elftable::symbol(VirtAddr::from_raw(caller), None)
        ));
    }
    if held.len == 0 {
        msg.push_str("\n  held: (none)");
    } else {
        let max = held.max_level();
        msg.push_str("\n  held:");
        for i in 0..held.len {
            let lv = held.slots[i]
                .level
                .map(|l| format!("{l:?}"))
                .unwrap_or_else(|| "exempt".into());
            msg.push_str(&format!(
                "\n    {} ({lv}){} acquired at {}",
                elftable::symbol(VirtAddr::from_raw(held.slots[i].addr), None),
                if held.slots[i].level == max {
                    "  <-- max held"
                } else {
                    ""
                },
                elftable::symbol(VirtAddr::from_raw(held.slots[i].caller), None)
            ));
        }
        msg.push_str("\n  rule: new level must exceed max(held); violation");
    }
    panic!("{msg}");
}

/// 每 hart 持有集最大深度（真实嵌套 <=4；8 为防御线，溢出即违规）。
#[cfg(debug_assertions)]
const MAX_HELD: usize = 8;

/// 持有集一个槽位（caller = 该锁获取点，供报告 "acquired at"）。
/// level=None = exempt 锁（不参与层级校验，但参与 contains 重入检测）。
#[cfg(debug_assertions)]
#[derive(Clone, Copy)]
struct Held {
    addr: usize,
    level: Option<Level>,
    caller: usize,
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
                level: None,
                caller: 0,
            }; MAX_HELD],
        }
    }

    /// 已持最大**参与**层级（空集或全 exempt → None；exempt 条目不计入）。
    fn max_level(&self) -> Option<Level> {
        self.slots[..self.len].iter().filter_map(|h| h.level).max()
    }

    /// 是否已持 addr（exempt 与参与锁一视同仁）。
    fn contains(&self, addr: usize) -> bool {
        self.slots[..self.len].iter().any(|h| h.addr == addr)
    }

    /// 记入（含获取点；exempt 锁 level=None）；满 → Err（溢出违规，由调用方报警）。
    fn push(&mut self, addr: usize, level: Option<Level>, caller: usize) -> Result<(), ()> {
        if self.len >= MAX_HELD {
            return Err(());
        }
        self.slots[self.len] = Held {
            addr,
            level,
            caller,
        };
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
/// 已被 halt 停核冻结——跨核 best-effort 读取不在此版）。
#[cfg(debug_assertions)]
struct HeldCell(UnsafeCell<HeldSet>);

// SAFETY: 每核只写自己那份，唯一 &mut 由 `as_mut`（本核 + SIE 关）保证。
#[cfg(debug_assertions)]
unsafe impl Send for HeldCell {}
#[cfg(debug_assertions)]
unsafe impl Sync for HeldCell {}

/// per-hart 持有集池（depend::init 装配后只读索引）。
#[cfg(debug_assertions)]
static POOL: OnceLock<&'static [HeldCell]> = OnceLock::new();

/// lockdep 装配错误。
#[cfg(debug_assertions)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepInitError {
    /// 分配失败（OOM）——当前容量下不触发，预留错误路径。
    #[allow(dead_code)]
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
fn held() -> Option<&'static mut HeldSet> {
    let pool = POOL.get()?;
    let h = machine::hart_id();
    if h >= pool.len() {
        panic!("[depend] hart {h} out of pool ({} slots)", pool.len());
    }
    Some({
        let this = &pool[h];
        unsafe { &mut *this.0.get() }
    })
}

/// 取锁前校验（不可重入锁）：**同锁重入与 level 解耦**——`contains(addr)` 对
/// 所有锁生效（exempt 锁也查：exempt 锁同样记入持有集，单核重入是悬死的行
/// 列）；层级校验仅对参与锁（`Some(level)` 须 > max(held)，严格递增；exempt
/// 不计入 max）。通过 → 不动状态（适配层随后 acquire）；违规 → [`report`]。
/// POOL 未装配 → 静默跳过（boot 早期）。可重入锁（RelLock）入口以
/// `owner == me` 跳过本校验（重入计数合法）。
#[cfg(debug_assertions)]
pub(crate) fn check(addr: usize, level: Option<Level>, caller: usize) {
    let Some(held) = held() else {
        return;
    };
    if held.contains(addr) {
        report("recursive acquisition", addr, caller);
    }
    if let Some(lv) = level
        && held.max_level().is_some_and(|m| lv <= m)
    {
        report("lock-order level violation", addr, caller);
    }
}

/// 取到后记入本核持有集（含本次调用点）。前置：已过 check；本核未持该锁
/// （不可重入锁）/ 首获（可重入锁）。exempt 锁（level=None）也记入——与
/// release 平衡（防误报 unheld），并让 contains 对它们生效。
#[cfg(debug_assertions)]
pub(crate) fn acquire(addr: usize, level: Option<Level>, caller: usize) {
    let Some(held) = held() else {
        return;
    };
    if held.push(addr, level, caller).is_err() {
        report("held set overflow", addr, caller);
    }
}

/// 释放时移除本核持有集条目。前置：对应 acquire 已发生（不可重入锁每次 /
/// 可重入锁末次递减后）。level 不参与匹配（同锁地址即平衡）。
#[cfg(debug_assertions)]
pub(crate) fn release(addr: usize) {
    let Some(held) = held() else {
        return;
    };
    if held.remove(addr).is_err() {
        report("release of unheld lock", addr, 0);
    }
}

