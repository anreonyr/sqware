//! 真实 RISC-V IOMMU（`riscv-iommu-sys`）的寄存器段：本域取设备区 → 建 DDT / DC / Sv39 表 / 队列
//! → 编程 → 硬件按我们写下的表翻译一次（正证）→ 拆除时 **DDTP Off 与三个队列 on/busy 分别确认**
//! → 同一 requester 与同一 IOVA 再译一次必须被拒（隔离负证）。
//!
//! 证据都是寄存器读数：CAP 与复位 DDTP 的原值、编程后 DDTP 的回读、队列 CSR 的活动/忙位、
//! IOFENCE.C 写下的完成记录、以及调试翻译口报回的物理页号。
//! 端点真 DMA、产品 System 的 `Platform` 安装与跨主体 BAR 窗口不在本景。

mod context;
mod pages;
mod queue;
mod regs;
mod table;

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
    table::map(&pages, ROOT, MIDDLE, LEAF, IOVA, data);
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
    queues.enable().unwrap_or_else(|fail| panic!("iommu-probe: {fail}"));

    // ③ 把设备目录表接上：一级 DDT + 回读确认。
    let ddtp = regs::DDTP_MODE_1LVL | (regs::page_number(pages.physical(DDT)) << regs::PPN_SHIFT);
    regs::write64(base, regs::DDTP, ddtp);
    let programmed = regs::read64(base, regs::DDTP);
    let ddtp_on = programmed & 0xF == regs::DDTP_MODE_1LVL && programmed & regs::DDTP_BUSY == 0;

    // ④ 队列仍开着时才能等到完成记录：先失效旧翻译，再让硬件写下完成数。
    queues.invalidate_all();
    let fenced = queues.fence();
    let (command_head, fault_head) = queues.heads();
    let (command_csr_after, fault_csr_after) = queues.status();
    let (command_base, fault_base) = queues.bases();

    // ⑤ 正证：让硬件按我们写下的 DDT / DC / Sv39 表走一遍，报回物理页号。
    let translated = translate(base);
    let (faults, cause) = queues.faults();
    note(&format!(
        "iommu-probe: ddtp_on={ddtp_on} ddtp={programmed:#x} fenced={fenced} cqh={command_head} fqh={fault_head} cqcsr={command_csr_after:#x} fqcsr={fault_csr_after:#x} cqb={command_base:#x} fqb={fault_base:#x} response={translated:#x} faults={faults} cause={cause}",
    ));
    let mapped = translated & regs::TR_FAULT == 0
        && (translated & regs::TR_PPN_FIELD)
            == (regs::page_number(data) << regs::PPN_SHIFT);
    let published = command_base == regs::page_number(pages.physical(COMMANDS))
        && fault_base == regs::page_number(pages.physical(FAULTS));

    // ⑥ 拆除：DDTP Off 与三个队列各自确认（「停」与「忙已清」是两件事）。
    regs::write64(base, regs::DDTP, regs::DDTP_MODE_OFF);
    let stopped = regs::read64(base, regs::DDTP);
    let (command_csr, fault_csr) = queues.disable();
    drop(queues);
    regs::write32(base, regs::PQCSR, 0);
    let page_csr = regs::read32(base, regs::PQCSR);
    let isolated = stopped & 0xF == regs::DDTP_MODE_OFF && stopped & regs::DDTP_BUSY == 0;
    let commands_off = queue::quiet(command_csr, regs::QUEUE_ON);
    let faults_off = queue::quiet(fault_csr, regs::QUEUE_ON);
    let page_off = queue::quiet(page_csr, regs::QUEUE_ON);
    let queue_errors = queue::queue_error(command_csr)
        || queue::queue_error(fault_csr)
        || queue::queue_error(page_csr);

    // ⑦ 负证：同一 requester、同一 IOVA 再译一次必须被拒（不是拿复位默认值充数）。
    let denied = translate(base);
    note(&format!(
        "iommu-probe: ddtp_off={stopped:#x} cqcsr={command_csr:#x} fqcsr={fault_csr:#x} pqcsr={page_csr:#x} isolated_response={denied:#x}",
    ));
    let refused = denied & regs::TR_FAULT != 0;

    pages.close().unwrap_or_else(|fail| panic!("iommu-probe: {fail}"));

    let checks: [(&str, bool); 12] = [
        ("ddtp programmed to 1LVL", ddtp_on),
        ("queue bases hold the retained pages", published),
        ("command fence completed", fenced),
        ("hardware consumed both commands", command_head == 2 && fault_head == 0),
        ("translation returned the retained page", mapped),
        ("no fault recorded while translating", faults == 0),
        ("ddtp off", isolated),
        ("command queue off", commands_off),
        ("fault queue off", faults_off),
        ("page request queue off", page_off),
        ("no queue error bit set", !queue_errors),
        ("same request refused after isolation", refused),
    ];
    if let Some((name, _)) = checks.iter().find(|(_, ok)| !ok) {
        panic!("iommu-probe: {name}");
    }
    note(&format!("iommu-probe: translated {IOVA:#x} -> {data:#x}"));
}

/// 让硬件按当前 DDT / DC / 页表翻一次 `IOVA`，返回响应寄存器的原值。
fn translate(base: usize) -> u64 {
    regs::write64(base, regs::TR_REQ_IOVA, IOVA as u64);
    let control = regs::TR_GO | regs::TR_NW | ((DEVICE_ID as u64) << regs::TR_DID_SHIFT);
    regs::write64(base, regs::TR_REQ_CTL, control);
    regs::read64(base, regs::TR_RESPONSE)
}

fn note(message: &str) {
    crate::debug::put(message);
}
