use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use riscv::register::time;
use sbi::ecall::SArgs;
use sbi::{self, fid};

use crate::hart;

const SLOTS: usize = 8;
const ROUNDS: usize = 16;
const WAIT_TICKS: u64 = 1_000_000;
const BUDGET_TICKS: u64 = 100_000;
const LOADED_TICKS: u64 = 1_000_000;

static IN_WFI: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
static EXIT: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
static SSIP_EXIT: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
static SELF_ADDR: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];

pub(crate) fn wfi_entry(hart: crate::hart::HartId) {
    if hart.get() < SLOTS {
        IN_WFI[hart.get()].store(1, Ordering::Relaxed);
        let me = crate::work::room::scheduler::core::current() as *const _ as usize;
        SELF_ADDR[hart.get()].store(me, Ordering::Relaxed);
    }
}

pub(crate) fn wfi_exit(hart: crate::hart::HartId, ssip: bool) {
    if hart.get() < SLOTS {
        IN_WFI[hart.get()].store(0, Ordering::Relaxed);
        EXIT[hart.get()].fetch_add(1, Ordering::Relaxed);
        if ssip {
            SSIP_EXIT[hart.get()].fetch_add(1, Ordering::Relaxed);
        }
    }
}

static BOOT_TIME: AtomicU64 = AtomicU64::new(0);
static NEXT_SAMPLE: AtomicU64 = AtomicU64::new(0);
static SAMPLES_LEFT: AtomicUsize = AtomicUsize::new(0);

const SAMPLE_GAP: u64 = 5_000_000;
const SAMPLES: usize = 6;

pub(crate) fn start_delayed() {
    let now = time::read() as u64;
    BOOT_TIME.store(now, Ordering::Relaxed);
    NEXT_SAMPLE.store(now + SAMPLE_GAP, Ordering::Relaxed);
    SAMPLES_LEFT.store(SAMPLES, Ordering::Relaxed);
}

pub(crate) fn idle_hook() {
    let now = time::read() as u64;
    let next = NEXT_SAMPLE.load(Ordering::Relaxed);
    if next == 0 || now < next {
        return;
    }
    if NEXT_SAMPLE
        .compare_exchange(next, now + SAMPLE_GAP, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    if SAMPLES_LEFT
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
        .is_err()
    {
        NEXT_SAMPLE.store(0, Ordering::Relaxed);
        return;
    }
    crate::putln!(
        "ipi: [loaded] t_ms={} left={}",
        (now - BOOT_TIME.load(Ordering::Relaxed)) / 10_000,
        SAMPLES_LEFT.load(Ordering::Relaxed)
    );
    run("loaded", Some(now + LOADED_TICKS));
}

pub(crate) fn run(tag: &str, due: Option<u64>) -> (usize, usize, usize) {
    let due = due.unwrap_or(u64::MAX);
    let me = hart::hart_id();
    let n = hart::hart_count();
    if n < 2 {
        crate::putln!("ipi: {tag} only {n} hart — 自检需要至少两颗核，跳过");
        return (0, 0, 0);
    }
    let mut left_passes = (n.min(SLOTS) - 1) * 2;
    let mut tested = 0;
    let mut d_total = 0;
    let mut b_total = 0;
    for target in (0..n.min(SLOTS)).map(crate::hart::HartId::new) {
        if target == me {
            continue;
        }
        let d_due = share(due, left_passes);
        left_passes -= 1;
        let (d_woke, d_ssip, d_rounds) = rounds(target, false, d_due);
        let b_due = share(due, left_passes);
        left_passes -= 1;
        let (b_woke, b_ssip, b_rounds) = rounds(target, true, b_due);
        let self_addr = SELF_ADDR[target.get()].load(Ordering::Relaxed);
        let expect = crate::work::room::scheduler::core::scheduler_addr(target);
        crate::putln!(
            "ipi: {tag} target={target} directed={d_woke}/{d_rounds} ssip={d_ssip} broadcast={b_woke}/{b_rounds} ssip={b_ssip} match={}",
            self_addr == expect
        );
        tested += 1;
        d_total += d_woke;
        b_total += b_woke;
    }
    crate::putln!(
        "ipi: {tag} me={me} n={n} tested={tested} directed_woke={d_total} broadcast_woke={b_total}"
    );
    (tested, d_total, b_total)
}

fn rounds(target: crate::hart::HartId, broadcast: bool, due: u64) -> (usize, usize, usize) {
    let mut valid = 0;
    let mut woke = 0;
    let mut ssip = 0;
    for _ in 0..ROUNDS {
        let Some(left) = budget_left(due) else { break };
        if !wait_until(WAIT_TICKS.min(left), || {
            IN_WFI[target.get()].load(Ordering::Relaxed) != 0
        }) {
            continue;
        }
        let Some(left) = budget_left(due) else { break };
        let before = EXIT[target.get()].load(Ordering::Relaxed);
        let before_ssip = SSIP_EXIT[target.get()].load(Ordering::Relaxed);
        send(target, broadcast);
        if wait_until(BUDGET_TICKS.min(left), || {
            EXIT[target.get()].load(Ordering::Relaxed) > before
        }) {
            woke += 1;
            if SSIP_EXIT[target.get()].load(Ordering::Relaxed) > before_ssip {
                ssip += 1;
            }
        }
        valid += 1;
    }
    (woke, ssip, valid)
}

fn send(target: crate::hart::HartId, broadcast: bool) {
    let (word, bit) = target.bit();
    let n = hart::hart_count().min(usize::BITS as usize);
    let mask = if broadcast { (1usize << n) - 1 } else { bit };
    let _ = sbi::IpiCall::new(fid::Ipi::SendIpi)
        .args(SArgs {
            a0: mask,
            a1: word * (usize::BITS as usize),
            ..Default::default()
        })
        .call();
}

fn wait_until(budget: u64, pred: impl Fn() -> bool) -> bool {
    let deadline = time::read() as u64 + budget;
    loop {
        // SAFETY fence: relaxed 原子读必须不被 LLVM 提出自旋环
        core::sync::atomic::compiler_fence(Ordering::Acquire);
        if pred() {
            return true;
        }
        if time::read() as u64 >= deadline {
            return false;
        }
        core::hint::spin_loop();
    }
}

fn budget_left(due: u64) -> Option<u64> {
    let left = due.saturating_sub(time::read() as u64);
    (left > 0).then_some(left)
}

fn share(due: u64, passes: usize) -> u64 {
    if due == u64::MAX {
        return due;
    }
    let now = time::read() as u64;
    now + due.saturating_sub(now) / passes.max(1) as u64
}