//! 真实 RISC-V IOMMU（`riscv-iommu-sys`）那两景共用的一片：寄存器格、设备上下文、
//! Sv39 表、命令/故障队列，以及把一台 PCI 端点接上来的那三个模块。
//!
//! `iommu-probe`：本域取设备区 → 建 DDT / DC / Sv39 表 / 队列 → 编程 → 硬件按我们写下的表
//! 翻译一次（正证）→ 拆除时 **DDTP Off 与队列 on/busy 分别确认** → 同一 requester 与同一 IOVA
//! 再译一次必须被拒（隔离负证）。
//!
//! `iommu-dma`：同一片寄存器路径 ＋ 一台**真实 PCI 端点**（`virtio-blk-pci`）——
//! ECAM 发现、BAR 落位、精确 BAR 交给驱动，再让端点按 IOVA 真写内存。

mod context;
pub mod dma;
mod ecam;
mod endpoint;
mod order;
mod pages;
mod queue;
mod regs;
mod table;
mod virtio;

use alloc::format;

use env::{Call, Name, Page};
use resource::dock::Dock;

use context::DeviceContext;
use pages::Pages;
use queue::CommandQueue;

use crate::boot::Accounts;

/// 这台机器的 `riscv,iommu` 节点坐标（设备树 `reg` 里的那一段）。
const IOMMU_BASE: u64 = 0x301_0000;
/// 选一个 requester id（一级 DDT + 扩展格式下必须 < 64）。
const DEVICE_ID: u32 = 0;
/// 被翻译的那一段 IOVA：Sv39 规范地址，且与真实物理页不同。
const IOVA: usize = 0x2_0000_0000;

/// 页窗口里的八页。
const DDT: usize = 0;
const ROOT: usize = 1;
const MIDDLE: usize = 2;
const LEAF: usize = 3;
const COMMANDS: usize = 4;
const FAULTS: usize = 5;
const FENCE: usize = 6;
const DATA: usize = 7;
const PAGES: usize = 8;

/// 引导那一台：本景的控制者。
///
/// 失败时先把读数落串口再判，故红的那一次现场里那排数仍然在。
pub fn acceptance() {
    let accounts = Accounts::take().expect("iommu-probe: boot accounts");
    let retention = accounts
        .token(Name::Call(Call::Retain))
        .expect("iommu-probe: retention authority");
    let iommu = accounts
        .token(Name::Page(Page::Region(IOMMU_BASE)))
        .expect("iommu-probe: iommu region");
    let dock = Dock::open(iommu).expect("iommu-probe: iommu mapping");
    let base = dock.view().base();

    // ① 复位后的**真实读数**：隔离态由显式编程与回读确认，不拿复位默认值当安全前提。
    let capability = regs::read64(base, regs::CAP);
    let reset = regs::read64(base, regs::DDTP);
    note(&format!(
        "iommu-probe: base={base:#x} cap={capability:#x} version={} pas={} ddtp_reset={reset:#x} mode={} busy={}",
        capability & 0xFF,
        (capability >> 32) & 0x3F,
        reset & 0xF,
        (reset & regs::DDTP_BUSY) != 0,
    ));

    // ② 发布硬件地址之前先把这些页 retain 好，并拿到它们真实的物理页起址。
    let pages = Pages::take(retention, PAGES).expect("iommu-probe: retained pages");
    let data = pages.physical(DATA);
    let context = DeviceContext::first_stage(pages.physical(ROOT));
    for (index, word) in context.words().iter().enumerate() {
        let at = pages.at(DDT, DEVICE_ID as usize * 64 + index * 8) as *mut u64;
        // SAFETY: DDT 页是本域刚取的可写页，DC 项按 64 字节排。
        unsafe { core::ptr::write_volatile(at, *word) };
    }
    table::map_pages(&pages, ROOT, MIDDLE, LEAF, IOVA, DATA, 1)
        .unwrap_or_else(|fail| panic!("iommu-probe: {fail}"));
    note(&format!(
        "iommu-probe: ddt={:#x} root={:#x} commands={:#x} faults={:#x} fence={:#x} data={:#x} iova={IOVA:#x}",
        pages.physical(DDT),
        pages.physical(ROOT),
        pages.physical(COMMANDS),
        pages.physical(FAULTS),
        pages.physical(FENCE),
        data,
    ));

    let mut queues = CommandQueue::new(base, &pages, COMMANDS, FAULTS, FENCE);
    queues
        .enable()
        .unwrap_or_else(|fail| panic!("iommu-probe: {fail}"));

    // ③ 把设备目录表接上：一级 DDT ＋ 规范那一手（写 → 轮询 busy 落 → 回读校验 mode）。
    let ddtp = match regs::ddtp_set(base, regs::DDTP_MODE_1LVL, Some(pages.physical(DDT))) {
        Ok(seen) => (seen, true),
        Err((_, seen)) => (seen, false),
    };
    let (programmed, ddtp_on) = ddtp;

    // ④ 队列仍开着时才能等到完成记录：先失效旧翻译，再让硬件写下完成数。
    queues.invalidate_all();
    let fenced = queues.fence();
    let (command_head, fault_head) = queues.heads();
    let (command_csr_after, fault_csr_after) = queues.status();
    let (command_base, fault_base) = queues.bases();

    // ⑤ 正证：让硬件按我们写下的 DDT / DC / Sv39 表走一遍，报回物理页号。
    let translated = translate(base);
    let (faults, cause, did, ttype) = queues.faults();
    note(&format!(
        "iommu-probe: ddtp_on={ddtp_on} ddtp={programmed:#x} fenced={fenced} cqh={command_head} fqh={fault_head} cqcsr={command_csr_after:#x} fqcsr={fault_csr_after:#x} cqb={command_base:#x} fqb={fault_base:#x} response={translated:#x} faults={faults} cause={cause} did={did} ttype={ttype}",
    ));
    let mapped = translated & regs::TR_FAULT == 0
        && (translated & regs::TR_PPN_FIELD) == (regs::page_number(data) << regs::PPN_SHIFT);
    let published = command_base == regs::page_number(pages.physical(COMMANDS))
        && fault_base == regs::page_number(pages.physical(FAULTS));

    // ⑥ 拆除：DDTP Off 与两个**已启用**的队列各自确认。
    // 页请求队列本景从未启用（没有 ATS 请求），故不写它、也不把它当“停过”的证。
    let stopped = match regs::ddtp_set(base, regs::DDTP_MODE_OFF, None) {
        Ok(seen) => seen,
        Err((_, seen)) => seen,
    };
    let (command_csr, fault_csr) = queues.disable();
    drop(queues);
    let page_csr = regs::read32(base, regs::PQCSR);
    let isolated = stopped & 0xF == regs::DDTP_MODE_OFF && stopped & regs::DDTP_BUSY == 0;
    let commands_off = queue::quiet(command_csr, regs::QUEUE_ON);
    let faults_off = queue::quiet(fault_csr, regs::QUEUE_ON);
    let page_unused = page_csr & (regs::QUEUE_ENABLE | regs::QUEUE_ON) == 0;
    let queue_errors = queue::command_error(command_csr)
        || queue::queue_error(fault_csr)
        || queue::queue_error(page_csr);

    // ⑦ 负证：同一 requester、同一 IOVA 再译一次必须被拒，且**原因码就是 DMA_DISABLED**
    // （只看故障位不够：页表坏掉也是故障，实测那种情形报 13 = RD_FAULT_S）。
    let denied = translate(base);
    note(&format!(
        "iommu-probe: ddtp_off={stopped:#x} cqcsr={command_csr:#x} fqcsr={fault_csr:#x} pqcsr={page_csr:#x} isolated_response={denied:#x}",
    ));
    let refused = denied & regs::TR_FAULT != 0 && regs::cause(denied) == regs::DMA_DISABLED;

    pages
        .close()
        .unwrap_or_else(|fail| panic!("iommu-probe: {fail}"));

    let checks: [(&str, bool); 12] = [
        ("ddtp programmed to 1LVL", ddtp_on),
        ("queue bases hold the retained pages", published),
        ("command fence completed", fenced),
        (
            "hardware consumed both commands",
            command_head == 2 && fault_head == 0,
        ),
        ("translation returned the retained page", mapped),
        ("no fault recorded while translating", faults == 0),
        ("ddtp off", isolated),
        ("command queue off", commands_off),
        ("fault queue off", faults_off),
        ("page request queue never enabled", page_unused),
        ("no queue error bit set", !queue_errors),
        ("same request refused as dma-disabled", refused),
    ];
    if let Some((name, _)) = checks.iter().find(|(_, ok)| !ok) {
        panic!("iommu-probe: {name}");
    }
    note(&format!("iommu-probe: translated {IOVA:#x} -> {data:#x}"));
}

/// 让硬件按当前 DDT / DC / 页表翻一次 `IOVA`，返回响应寄存器的原值。
fn translate(base: usize) -> u64 {
    regs::translate(base, DEVICE_ID, IOVA, false)
}

fn note(message: &str) {
    crate::debug::put(message);
}
