use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::hart::{self, HartId};
use crate::lock::OnceLock;
use crate::putln;
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::ledger;
use sbi::ecall::SArgs;
use sbi::{self, fid};

static PUSHED: AtomicUsize = AtomicUsize::new(0);
static REAPED: AtomicUsize = AtomicUsize::new(0);
static ROOTED: AtomicBool = AtomicBool::new(false);

const BARRIER_REPORT_AT: usize = 20_000_000;

static HALTING: AtomicBool = AtomicBool::new(false);
static HALT_ARRIVED: AtomicUsize = AtomicUsize::new(0);

static WAITING: [AtomicUsize; WAITING_WORDS] = [const { AtomicUsize::new(0) }; WAITING_WORDS];

static PICK_CURSOR: AtomicUsize = AtomicUsize::new(0);

const WAITING_WORDS: usize = crate::layout::MAX_HART_SLOTS / usize::BITS as usize;

pub(crate) fn push() {
    PUSHED.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn exit() {
    REAPED.fetch_add(1, Ordering::Relaxed);
}

pub(super) fn done() -> bool {
    if !ROOTED.load(Ordering::Acquire) {
        return false;
    }
    let pushed = PUSHED.load(Ordering::Relaxed);
    pushed == 0 || REAPED.load(Ordering::Relaxed) == pushed
}

pub(crate) fn counts() -> (usize, usize) {
    (
        PUSHED.load(Ordering::Relaxed),
        REAPED.load(Ordering::Relaxed),
    )
}

pub(crate) fn barrier() -> (usize, usize) {
    (
        HALT_ARRIVED.load(Ordering::Acquire),
        crate::hart::hart_count(),
    )
}

pub(crate) fn rooted() {
    ROOTED.store(true, Ordering::Release);
}

pub(super) fn halt() -> ! {
    crate::memory::manager::asid::vacate();
    HALT_ARRIVED.fetch_add(1, Ordering::AcqRel);
    if !HALTING.swap(true, Ordering::AcqRel) {
        yell();
        let mut spins = 0usize;
        let mut reported = false;
        while HALT_ARRIVED.load(Ordering::Acquire) < hart::hart_count() {
            spins += 1;
            if !reported && spins == BARRIER_REPORT_AT {
                reported = true;
                let (arrived, total) = barrier();
                let (pushed, reaped) = counts();
                crate::putln!(
                    "[stop] halt 屏障等待：已达 {arrived}/{total} 核；任务 PUSHED={pushed}                      REAPED={reaped}（差 {}）—— 屏障等的是**核**，不是任务",
                    pushed.saturating_sub(reaped)
                );
            }
            core::hint::spin_loop();
        }
        putln!("task: all tasks exited, system halted");
        {
            let (late_n, late_max, late_sum) = timer::late_stats();
            let max_ms = clock::ticks_to_duration(late_max).as_millis();
            let avg_ms = if late_n == 0 {
                0
            } else {
                clock::ticks_to_duration(late_sum / late_n).as_millis()
            };
            let (tocks, mutes) = timer::tock_stats();
            putln!(
                "timer: late_n={late_n} late_max_ms={max_ms} late_avg_ms={avg_ms} late_max_tick={late_max} traps={} tocks={tocks} mutes={mutes}",
                timer::ticks()
            );
        }
        {
            let (held, starved, blocked, nudged) = crate::work::room::messenger::branch_stats();
            putln!("doom: held={held} starved={starved} blocked={blocked} nudged={nudged}");
            let (kicks, fallback) = kick_stats();
            putln!("sched: kicks={kicks} fallback={fallback}");
            let (ring, busy, idle_ring, idle_busy) = crate::runtime::switcher::trap::resources::stats();
            putln!("supervisor_external: ring={ring} busy={busy} idle_ring={idle_ring} idle_busy={idle_busy}");
            // 一只手有没有人取（`Push` 方向等超过 1 秒的账）——这件事由这一行接着看得见
            // （见 `work/mail/hole.rs` 的 `hold_line`）。
            crate::work::mail::hole::hold_line();
        }
        crate::runtime::diagnose::trace::note(crate::runtime::diagnose::trace::EventKind::Halt(
            crate::runtime::diagnose::trace::HaltEvent::Halt,
        ));
        hooked();
        if crate::testing() {
            if counts().0 == 0 {
                putln!("[verdict] 一台域都没起：世界没跑起来，这一景什么都没验");
                semihosting::process::exit(101)
            }
            // **判据 = 账里每一笔都是"干净的结局"**：`EXIT_OK`（自愿结束）与内核自己那两档收场码
            // （`EXIT_DOOM` / `EXIT_CASCADE`：机器已经在收场，连坐请走的那几台）算绿，其余一律算红
            // ——装配那一族的小整数（启动握手没走通）、`EXIT_PANIC`、`EXIT_FAULT` 都在内。
            // **旧口径只看 `EXIT_PANIC`，太松**：装配收场（小整数那一档）与故障收场（`EXIT_FAULT`）
            // 当时都判绿——实测过一例：`probe-rule` 干净退场、装配者随后判它"没就绪"⇒ 装配失败并
            // 收场，而用例报绿（`kernel/tests/embedded.rs` 的 `scene` 因此看得见机器起过、看不见
            // 装配没走完）。
            let mut blame: Option<ledger::Entry> = None;
            ledger::failures(|e| {
                if blame.is_none() && !clean_ending(e.reason) {
                    blame = Some(*e);
                }
            });
            let Some(e) = blame else {
                semihosting::process::exit(0)
            };
            ledger::each(|x| {
                putln!(
                    "[verdict] tid={} reason={:#x} observed={} note: {}",
                    x.task.get(),
                    x.reason,
                    x.observed,
                    x.note()
                );
            });
            putln!("[verdict] 塌在 tid={}: {}", e.task.get(), e.note());
            semihosting::process::exit(101)
        }
        let _ = sbi::SystemResetCall::new(fid::SystemReset::SystemReset).call();
    }
    loop {
        unsafe { core::arch::asm!("wfi") };
    }
}

type Hook = fn();

/// testing 的判据里"这一笔结局算干净吗"（见 [`halt`] 那一段）：自愿结束，或内核自己收场时
/// 连坐请走的那两档。**其它一律算红**——包括 `EXIT_PANIC` / `EXIT_FAULT` 与各域自己的启动编号。
fn clean_ending(reason: env::Reason) -> bool {
    reason == env::EXIT_OK
        || reason == crate::work::room::messenger::EXIT_DOOM
        || reason == crate::work::room::messenger::EXIT_CASCADE
}

static HOOKS: OnceLock<&'static [Hook]> = OnceLock::new();

pub(crate) fn hook(hooks: &'static [Hook]) {
    let _ = HOOKS.set(hooks);
}

fn hooked() {
    if let Some(hooks) = HOOKS.get() {
        for h in hooks.iter() {
            h();
        }
    }
}

pub(super) fn sleep(hart: HartId) {
    debug_assert!(
        hart.get() < crate::layout::MAX_HART_SLOTS,
        "sleep hart {hart} beyond MAX_HART_SLOTS"
    );
    let (word, bit) = hart.bit();
    WAITING[word].fetch_or(bit, Ordering::AcqRel);
}

pub(super) fn wake(hart: HartId) {
    debug_assert!(
        hart.get() < crate::layout::MAX_HART_SLOTS,
        "wake hart {hart} beyond MAX_HART_SLOTS"
    );
    let (word, bit) = hart.bit();
    WAITING[word].fetch_and(!bit, Ordering::AcqRel);
}

pub(crate) fn waiting(hart: HartId) -> bool {
    debug_assert!(
        hart.get() < crate::layout::MAX_HART_SLOTS,
        "waiting hart {hart} beyond MAX_HART_SLOTS"
    );
    let (word, bit) = hart.bit();
    WAITING[word].load(Ordering::Acquire) & bit != 0
}

pub(crate) fn pick() -> HartId {
    let seat = PICK_CURSOR.fetch_add(1, Ordering::Relaxed);
    let n = hart::hart_count();
    debug_assert!(n > 0, "pick with no hart");
    let sm = seat % (usize::BITS as usize);
    let mut best_rel = usize::MAX;
    let mut best_bit = 0usize;
    for (w, word) in WAITING.iter().enumerate() {
        let bits = word.load(Ordering::Acquire);
        if bits == 0 {
            continue;
        }
        let rot = bits.rotate_right(sm as u32);
        let rel = rot.trailing_zeros() as usize;
        let bit = w * (usize::BITS as usize) + (sm + rel) % (usize::BITS as usize);
        if rel < best_rel {
            best_rel = rel;
            best_bit = bit;
        }
    }

    if best_rel != usize::MAX {
        HartId::new(best_bit)
    } else {
        let me = hart::hart_id();
        let mut to = seat % n;
        if n > 1 && to == me.get() {
            to = (to + 1) % n;
        }
        HartId::new(to)
    }
}

static KICKS: AtomicUsize = AtomicUsize::new(0);
static FALLBACK: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn note_kick_ipi() {
    KICKS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn note_fallback() {
    FALLBACK.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn kick_stats() -> (usize, usize) {
    (
        KICKS.load(Ordering::Relaxed),
        FALLBACK.load(Ordering::Relaxed),
    )
}

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

pub(crate) fn nudge(hart: HartId) {
    let (word, bit) = hart.bit();
    let _ = sbi::IpiCall::new(fid::Ipi::SendIpi)
        .args(SArgs {
            a0: bit,
            a1: word * (usize::BITS as usize),
            ..Default::default()
        })
        .call();
}
