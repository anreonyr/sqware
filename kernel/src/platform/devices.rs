use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use env::{Key, Mark, PAIR_LEN, Pair};
use env::{MailFail, TaskId};

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::console::Sink;
use crate::lock::OnceLock;
use crate::platform::machine;
use crate::runtime::diagnose::render::render;
use crate::runtime::diagnose::report::Report;
use crate::work::mail;
use crate::work::mail::nole::NoleMeta;
use crate::work::unit::gate::{self, AnyPie, Permission};
use crate::work::unit::task::Task;

static IRQ: OnceLock<Arc<NoleMeta>> = OnceLock::new();

static IRQ_RING: AtomicUsize = AtomicUsize::new(0);
static IRQ_BUSY: AtomicUsize = AtomicUsize::new(0);
static IRQ_IDLE_RING: AtomicUsize = AtomicUsize::new(0);
static IRQ_IDLE_BUSY: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn raise_irq() -> Result<(), MailFail> {
    IRQ_RING.fetch_add(1, Ordering::Relaxed);
    let meta = IRQ.get().expect("irq bell not built (devices::scan)");
    let r = mail::nole::ring(meta);
    if r.is_err() {
        IRQ_BUSY.fetch_add(1, Ordering::Relaxed);
    }
    r
}

pub(crate) fn raise_irq_idle() -> Result<(), MailFail> {
    let r = raise_irq();
    IRQ_IDLE_RING.fetch_add(1, Ordering::Relaxed);
    if r.is_err() {
        IRQ_IDLE_BUSY.fetch_add(1, Ordering::Relaxed);
    }
    r
}

pub(crate) fn irq_stats() -> (usize, usize, usize, usize) {
    (
        IRQ_RING.load(Ordering::Relaxed),
        IRQ_BUSY.load(Ordering::Relaxed),
        IRQ_IDLE_RING.load(Ordering::Relaxed),
        IRQ_IDLE_BUSY.load(Ordering::Relaxed),
    )
}

fn supply_initrd() -> Option<(Key, AnyPie)> {
    let initrd = machine::info().initrd()?;
    let meta = mail::pole::region(initrd.base, initrd.size, TaskId::new(0)).ok()?;
    let pie = gate::new_pie(meta, Mark::NONE, Permission::FETCH | Permission::VEST, None);
    Some((Key::region(initrd.base as u64), AnyPie::Pole(pie)))
}

fn supply_dtb() -> (Key, AnyPie) {
    let dtb = machine::info().dtb();
    let meta = mail::pole::region(dtb.base, dtb.size, TaskId::new(0)).expect("devicetree region");
    let pie = gate::new_pie(meta, Mark::NONE, Permission::FETCH | Permission::VEST, None);
    (Key::dtb(), AnyPie::Pole(pie))
}

fn supply_irq() -> (Key, AnyPie) {
    let meta = NoleMeta::new(TaskId::new(0));
    assert!(IRQ.set(meta.clone()).is_ok(), "irq bell built twice");
    let pie = gate::new_pie(meta, Mark::NONE, Permission::FETCH | Permission::VEST, None);
    (Key::irq(), AnyPie::Nole(pie))
}

pub(crate) const MAX_PAIRS: usize = 64;

pub(crate) const BLOCK_BYTES: usize = crate::memory::PAGE_SIZE;

const _: () = assert!(MAX_PAIRS * PAIR_LEN <= BLOCK_BYTES && BLOCK_BYTES == 4096);

#[repr(C, align(4096))]
struct Block(core::cell::UnsafeCell<[u8; BLOCK_BYTES]>);

// SAFETY: boot 单核期写，此后只读
unsafe impl Sync for Block {}

static BLOCK: Block = Block(core::cell::UnsafeCell::new([0u8; BLOCK_BYTES]));

pub(crate) fn block() -> (usize, usize) {
    (core::ptr::addr_of!(BLOCK) as usize, BLOCK_BYTES)
}

pub(crate) fn scan() -> Vec<(Key, AnyPie)> {
    let dtb = machine::info().dtb();
    // SAFETY: dtb 是 boot 交上来的 DTB 区（终身存活），此处只读
    let fdt = unsafe { fdt::Fdt::from_ptr(dtb.base as *const u8) }.expect("device tree blob");
    let mut out = Vec::new();
    let mut log = Report::default();
    let log_p = log.paragraph("supplies", None);
    for node in fdt.all_nodes() {
        if exempt(node.name) {
            continue;
        }
        let Some(reg) = node.reg() else {
            continue;
        };
        for r in reg {
            let base = r.starting_address.addr();
            let Some(size) = r.size else {
                continue;
            };
            if base == 0 || size == 0 {
                continue;
            }
            let Ok(meta) = mail::pole::region(base, size, TaskId::new(0)) else {
                continue;
            };
            let pie = gate::new_pie(
                meta,
                Mark::NONE,
                Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY,
                None,
            );
            let pie = AnyPie::Pole(pie);
            log_p.items.push(vec![
                Some(String::from(node.name)),
                Some(format!("token {}", pie.token().get())),
            ]);
            out.push((Key::region(base as u64), pie));
        }
    }
    let (key, pie) = supply_dtb();
    log_p.items.push(vec![
        Some(String::from("devicetree")),
        Some(format!("token {}", pie.token().get())),
    ]);
    out.push((key, pie));
    let (key, pie) = supply_irq();
    log_p.items.push(vec![
        Some(String::from("irq")),
        Some(format!("token {}", pie.token().get())),
    ]);
    out.push((key, pie));
    if let Some((key, pie)) = supply_initrd() {
        log_p.items.push(vec![
            Some(String::from("initrd")),
            Some(format!("token {}", pie.token().get())),
        ]);
        out.push((key, pie));
    }
    log_p.items.push(vec![
        Some(String::from("handed to root")),
        Some(format!("{} entries", out.len())),
    ]);
    let sealed = log.seal();
    render(sealed, &mut Sink, 0);
    out
}

pub(crate) fn install(task: &Task, items: Vec<(Key, AnyPie)>) -> usize {
    if items.len() > MAX_PAIRS {
        panic!(
            "device tree has {} devices, pairing block holds {MAX_PAIRS}",
            items.len()
        );
    }
    let (pa, _bytes) = block();
    let n = items.len();
    for (i, (key, pie)) in items.into_iter().enumerate() {
        let token = pie.token();
        let record = Pair::bytes(key, token.get());
        // SAFETY: 记录步长 = PAIR_LEN 编译期锁定；写偏移 < 块长（条数上限检查过）
        unsafe {
            core::ptr::write_unaligned(
                (pa as *mut u8).add(i * PAIR_LEN).cast::<[u8; PAIR_LEN]>(),
                record,
            );
        }
        task.pies.lock().push(pie);
    }
    n
}

fn exempt(name: &str) -> bool {
    let stem = name.split('@').next().unwrap_or(name);
    matches!(stem, "memory" | "clint")
}
