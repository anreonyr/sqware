use crate::core::work::record::{ledger, trace};
use crate::core::work::room::{deadline, halt, interrupt, messenger, scheduler};
use crate::core::work::unit::tally;
use crate::machine::clock;
use crate::putln;
use core::sync::atomic::{AtomicUsize, Ordering};

static FRAME_BASELINE: AtomicUsize = AtomicUsize::new(usize::MAX);

pub(crate) fn arm() {
    if crate::testing() {
        // The ASID bitmap is permanent kernel metadata, allocated on its first use.
        let asid = crate::core::memory::nest::identity::Asid::try_allocate()
            .expect("frame baseline: ASID allocator");
        crate::core::memory::nest::identity::Asid::release_unpublished(asid);
        FRAME_BASELINE.store(frame_allocations(), Ordering::Release);
        #[cfg(debug_assertions)]
        putln!(
            "memory: baseline kinds {}",
            crate::core::memory::flat::allocator::statistics::kinds()
        );
    }
}

fn frame_allocations() -> usize {
    use crate::core::memory::flat::allocator::statistics;
    statistics::frame_occupied() - statistics::block_occupied()
}

pub(crate) fn waiting() {
    let (arrived, total) = halt::barrier();
    let (pushed, reaped) = tally::counts();
    putln!(
        "[stop] halt 屏障等待：已达 {arrived}/{total} 核；任务 PUSHED={pushed}                      REAPED={reaped}（差 {}）—— 屏障等的是**核**，不是任务",
        pushed.saturating_sub(reaped)
    );
}

pub(crate) fn report() {
    putln!("task: all tasks exited, system halted");
    let (late_n, late_max, late_sum) = deadline::late_stats();
    let max_ms = clock::ticks_to_duration(late_max).as_millis();
    let avg_ms = if late_n == 0 {
        0
    } else {
        clock::ticks_to_duration(late_sum / late_n).as_millis()
    };
    let (tocks, mutes) = deadline::tock_stats();
    putln!(
        "timer: late_n={late_n} late_max_ms={max_ms} late_avg_ms={avg_ms} late_max_tick={late_max} traps={} tocks={tocks} mutes={mutes}",
        deadline::ticks()
    );
    let (held, starved, blocked, nudged) = messenger::branch_stats();
    putln!("doom: held={held} starved={starved} blocked={blocked} nudged={nudged}");
    let (kicks, fallback) = scheduler::kick_stats();
    putln!("sched: kicks={kicks} fallback={fallback}");
    let (ring, busy, idle_ring, idle_busy) = interrupt::stats();
    putln!(
        "supervisor_external: ring={ring} busy={busy} idle_ring={idle_ring} idle_busy={idle_busy}"
    );
    crate::core::work::mail::hole::hold_line();
    trace::note(trace::EventKind::Halt(trace::HaltEvent::Halt));
}

pub(crate) fn verdict() {
    if !crate::testing() {
        return;
    }
    let before = FRAME_BASELINE.load(Ordering::Acquire);
    let after = frame_allocations();
    putln!("memory: frame_allocations before={before} after={after}");
    #[cfg(debug_assertions)]
    putln!(
        "memory: final kinds {}",
        crate::core::memory::flat::allocator::statistics::kinds()
    );
    if before == usize::MAX || before != after {
        putln!("[verdict] physical frame allocations did not return to baseline");
        semihosting::process::exit(101)
    }
    #[cfg(feature = "semihosting")]
    if !super::export::complete() {
        putln!("[verdict] structured export incomplete; scene evidence rejected");
        semihosting::process::exit(101)
    }
    if tally::counts().0 == 0 {
        putln!("[verdict] 一台域都没起：世界没跑起来，这一景什么都没验");
        semihosting::process::exit(101)
    }
    let mut blame: Option<ledger::Entry> = None;
    ledger::each(|e| {
        if blame.is_none() && !clean_ending(e.reason) {
            blame = Some(*e);
        }
    });
    let Some(e) = blame else {
        semihosting::process::exit(0)
    };
    ledger::each(|x| {
        putln!(
            "[verdict] tid={} reason={:#x} note: {}",
            x.task.get(),
            x.reason,
            x.note()
        );
    });
    putln!("[verdict] 塌在 tid={}: {}", e.task.get(), e.note());
    semihosting::process::exit(101)
}

fn clean_ending(reason: env::Reason) -> bool {
    reason == env::EXIT_OK || reason == messenger::EXIT_DOOM || reason == messenger::EXIT_CASCADE
}
