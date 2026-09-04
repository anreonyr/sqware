// 系统级生命周期：跨核共享的原子状态与编排。
//
// 两件事：
//   全退出停机 — PUSHED/REAPED 任务计数 + BOOT_DONE 守门；BOOT_DONE=true 且
//              （PUSHED==0 或 REAPED==PUSHED）= 全部退出 → 发 SBI srst 复位；
//              HALTING 做一次性互斥，防多核同时发复位。
//   休眠唤醒   — WAITING 位图（bit h = hart h 正 WFI 等待）；入队后 kick
//              单点唤醒（消雷鸣群），halt 屏障由 yell 广播喊全员归队。
//
// 命名：动词（spawn/exit/done/halt/sleep/wake/**kick**/**yell**/wfi/boot_done）+
// 计数名词（PUSHED/REAPED/WAITING/HALTING/BOOT_DONE）。

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::lock::OnceLock;
use crate::machine;
use crate::putln;
use sbi::scall::SArgs;
use sbi::{self, fid};

/// 已入队（创建）任务计数（全退出检测：REAPED == PUSHED → 停机）。
static PUSHED: AtomicUsize = AtomicUsize::new(0);
static REAPED: AtomicUsize = AtomicUsize::new(0);
/// Boot 装配完成标记（push 通道关门）。一旦置位，`done()` 才允许 true——
/// 防 PUSHED 永久为 0 时被误判"全部结束"。一次性：`boot::init` 在
/// `spawn_demos()` 返回后立即置位（之后不再有 spawn）。
static BOOT_DONE: AtomicBool = AtomicBool::new(false);
/// 停机互斥：第一个触发 srst 的核胜出，其余 wfi（避免双 srst）。
static HALTING: AtomicBool = AtomicBool::new(false);
/// 已到达 halt 的核数 — 关机屏障：胜出核须等**全部**核到达后再断言帧基线。
static HALT_ARRIVED: AtomicUsize = AtomicUsize::new(0);
/// WFI 等待唤醒的 hart 位图（字 w 的 bit b = hart `w·64 + b` 正阻塞）。
/// 多字：字宽 = SBI 单次 `sbi_send_ipi` 的寻址窗口（XLEN = 64 位），按
/// MAX_HART_SLOTS 分字。位位置 = hartid。
static WAITING: [AtomicUsize; WAITING_WORDS] = [const { AtomicUsize::new(0) }; WAITING_WORDS];
/// 全局旋转游标：`kick` 选位起点用。单点选最低 set bit 会长期偏向同一
/// hart（最低位 hart 因抢失败一直留位时被反复踢——其他 hart 永不被踢），
/// 用游标取模 64bit 字宽作为扫描起点，每次踢推到下一个等待 hart，4 hart
/// 时每 4 次循环一次——保证公平。Relaxed fetch_add 即可（不必严格跨核
/// 同步：每次 kick 拿不同起点即可）。
static YELL_CURSOR: AtomicUsize = AtomicUsize::new(0);

/// WAITING 位图字数：每字 64 位（= 协议单次 IPI 掩码窗口）。
const WAITING_WORDS: usize = crate::machine::MAX_HART_SLOTS / usize::BITS as usize;

/// 任务入队计数 +1（PUSHED）。Relaxed 够用：计数只用于相等比较，且自增发生在持调度锁时。
pub(super) fn push() {
    PUSHED.fetch_add(1, Ordering::Relaxed);
}

/// 任务回收计数 +1（REAPED）。
pub(super) fn exit() {
    REAPED.fetch_add(1, Ordering::Relaxed);
}

/// 全部任务是否已退出。
///
/// 守门 `BOOT_DONE == true`（防 boot 早期 PUSHED==0 误判"全部结束"——
/// 当时 PUSHED 尚未增长就被 read，会永久返 false → 无任务场景无法停机）。
///
/// 守门后：`PUSHED == 0`（boot 没 spawn，过期 PUSHED==0 仍可停机）或
/// `REAPED == PUSHED`（全部回收）。
pub(super) fn done() -> bool {
    if !BOOT_DONE.load(Ordering::Acquire) {
        return false;
    }
    let pushed = PUSHED.load(Ordering::Relaxed);
    pushed == 0 || REAPED.load(Ordering::Relaxed) == pushed
}

/// 标记 boot 装配完成（push 通道关门）。由 `boot::init` 在 `spawn_demos()`
/// 返回后立即置位（一次性）。守门 `done()` 必须见位才认 true。
pub(crate) fn boot_done() {
    BOOT_DONE.store(true, Ordering::Release);
}

/// 全部任务已退出：显式停机（srst；AtomicBool 防双核同时触发——后到者 wfi）。
///
/// 关机屏障：胜出核等全部核到齐后跑注册关机钩子、最后复位。钩子按注册顺序：
/// dock/ring shutdown → 调度器槽清空 → block flush → audit 基线。
/// 钩子由 `boot::init` 一次性注册，conductor 不直接命名任何子系统。
pub(super) fn halt() -> ! {
    // 退租：本核即将卧倒，永不再应答清退——必须先从名册消失，否则关机钩子
    // （`rip` 拆任务 → 拆空间 → 清退）会死等本核。
    crate::memory::manager::evict::vacate();
    HALT_ARRIVED.fetch_add(1, Ordering::AcqRel);
    if !HALTING.swap(true, Ordering::AcqRel) {
        // 喊醒所有 WFI 睡核：它们醒来后同样会走 done → halt → 登记到达。
        // **必须用广播 `yell`**——屏障要求全员到齐，单点 `kick` 会让部分
        // hart 留 WFI 不归队、屏障永远释放不了。
        yell();
        while HALT_ARRIVED.load(Ordering::Acquire) < machine::hart_count() {
            core::hint::spin_loop();
        }
        putln!("task: all tasks exited, system halted");
        crate::runtime::diagnose::trace::note(crate::runtime::diagnose::trace::EventKind::Halt(
            crate::runtime::diagnose::trace::HaltEvent::Halt,
        ));
        run_shutdown_hooks();
        let _ = sbi::SystemResetCall::new(fid::SystemReset::SystemReset).call();
    }
    loop {
        unsafe { core::arch::asm!("wfi") };
    }
}

// ── 关机钩子注册面 ──
//
// 子系统（mail::dock / mail::ring / scheduler / allocator）在 `boot::init` 把
// 自己的关机函数挂到这里——conductor 不再硬编码子系统名。每条钩子调一次，
// 顺序 = 注册顺序（mail → scheduler → block::flush → audit），由 boot::init
// 装配时定。
type ShutdownHook = fn();

static SHUTDOWN_HOOKS: OnceLock<&'static [ShutdownHook]> = OnceLock::new();

/// 注册关机钩子（一次性；由 `boot::init` 调用）。
pub(crate) fn register_shutdown_hooks(hooks: &'static [ShutdownHook]) {
    let _ = SHUTDOWN_HOOKS.set(hooks);
}

/// 跑注册关的（halt 屏障之后调）。
fn run_shutdown_hooks() {
    if let Some(hooks) = SHUTDOWN_HOOKS.get() {
        for hook in hooks.iter() {
            hook();
        }
    }
}

/// 标记 hart 进入 WFI 等待。调用方须在置位后**复查队列**再睡。
pub(super) fn sleep(hart: usize) {
    debug_assert!(
        hart < crate::machine::MAX_HART_SLOTS,
        "sleep hart {hart} beyond MAX_HART_SLOTS"
    );
    WAITING[hart / (usize::BITS as usize)]
        .fetch_or(1usize << (hart % (usize::BITS as usize)), Ordering::AcqRel);
}

/// 清除 hart 的等待标记（WFI 唤醒后 / 复查发现任务时调用）。
pub(super) fn wake(hart: usize) {
    debug_assert!(
        hart < crate::machine::MAX_HART_SLOTS,
        "wake hart {hart} beyond MAX_HART_SLOTS"
    );
    WAITING[hart / (usize::BITS as usize)].fetch_and(
        !(1usize << (hart % (usize::BITS as usize))),
        Ordering::AcqRel,
    );
}

/// 单点唤醒 1 个 WFI 等待 hart（push / wake / drain 热路径）。从 WAITING 位
/// 图用全局旋转游标 `YELL_CURSOR` 选起点扫一个 word 找第一个 set bit——
/// 唤醒单 hart，**避免雷鸣群**（多 hart 同醒 → 同时抢源 hart 的 L1 → cache
/// 行乒乓）。
///
/// 公平性：游标单调推进 → 每个 hart 轮流被踢。4 hart 全等待时周期 0/1/2/3，
/// 每个 hart 每 4 次循环踢一次；避免最低位 hart 长期被偏爱（之前裸选最低
/// set bit 时存在的问题：低位 hart 因抢失败留位 → 反复踢同 hart）。
///
/// 失败兜底：被踢醒 hart 抢失败 → 哑睡壳（保留 sleep 位，等下次事件）；work
/// 仍留在源 hart 的 starved queue——源 hart 下次 yield 自取（≤ 1 个时间片）。
pub(super) fn kick() {
    // 起点游标：fetch_add(1) % 64bit 字宽。下次 call 拿到新起点，跨核亦然。
    let cursor_mod = YELL_CURSOR.fetch_add(1, Ordering::Relaxed) % (usize::BITS as usize);
    'word: for (w, word) in WAITING.iter().enumerate() {
        let waiting = word.load(Ordering::Acquire);
        if waiting == 0 {
            continue;
        }
        // 从 cursor 旋转扫描整个 word 找第一个 set bit——最坏 64 次 bit
        // test ≈ ~10 cycles，远低 SBI ecall 开销。命中即发 1-bit IPI 后返回。
        for off in 0..(usize::BITS as usize) {
            let bit_pos = (cursor_mod + off) % (usize::BITS as usize);
            let bit = 1usize << bit_pos;
            if waiting & bit != 0 {
                let _ = sbi::IpiCall::new(fid::Ipi::SendIpi)
                    .args(SArgs {
                        a0: bit,
                        a1: w * (usize::BITS as usize),
                        ..Default::default()
                    })
                    .call();
                break 'word;
            }
        }
    }
}

/// 广播唤醒所有 WFI 等待 hart（**halt 屏障专用**）。mask = waiting 字保
/// 留 XLEN 位全部 set——保证全员到达 `halt()` 登记屏障。
///
/// **不可用于 push / wake / drain**——单次 push 多 hart 醒 = 雷鸣群 = 同
/// 时抢同 L1 锁 = cache 行乒乓，与 `kick()` 单点化目的背道而驰。
///
/// 命名：动词（kick = 单点轻踢；yell = 喊全员；语义对仗）。
pub(super) fn yell() {
    for (w, word) in WAITING.iter().enumerate() {
        let waiting = word.load(Ordering::Acquire);
        if waiting == 0 {
            continue;
        }
        let _ = sbi::IpiCall::new(fid::Ipi::SendIpi)
            .args(SArgs {
                a0: waiting,
                a1: w * (usize::BITS as usize),
                ..Default::default()
            })
            .call();
    }
}
