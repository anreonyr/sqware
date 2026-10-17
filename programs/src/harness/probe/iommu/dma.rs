//! 真实 PCI 端点在 IOMMU 翻译下的 DMA：ECAM 发现 → 精确 BAR → 真 DMA（正证）→
//! 服务死掉之后控制者**自己**收口（负证）→ 确认到位才归还保留。
//!
//! 四件事各有唯一条目：
//! - **发现**：`ecam` 扫配置空间、`virtio` 读能力表、`endpoint` 把 BAR 落位；
//! - **交付**：BAR 从 `Page::Region(aperture)` 那一枚门闩上派生**一段有界窗口**
//!   （`PieCall::AccordWindow`）。两枚：管理者自己那一枚（0 号位，它留作收口时的门铃通路），
//!   以及从它**嵌套收窄**出来、交给端点服务的那一枚（1 号位）——不放宽、不下放整段 aperture；
//! - **发布**：硬件到得了的每一页先经保留强持，再把**真实物理页起址**写进 DDT/DC/PT/队列；
//!   设备看到的是另一段 IOVA，故“按我们写下的表翻过去的”这件事有承重的证据；
//! - **收口**：先 IOTINVAL ＋ IOFENCE（可执行、要等到完成记录），再把 DDTP 落 Off 并回读确认；
//!   然后由管理者**自己那一枚窗口**打响门铃（服务已经不在了，这一下不经过它）；
//!   确认到位之后才 `close()` 归还保留。
//!
//! 角色分得开：**设备那一半**（配置空间之后的寄存器这一面）归服务，它手里只有一枚精确到一根 BAR 的
//! 窗口与几个地址；**内存那一半**（队列/描述符/数据页）与 IOMMU 寄存器归控制者，那是所有者的保留。
//! 服务在第二轮摆好之后**退场**（不撤队列、不复位设备），于是“服务死时端点还活着”是真的。
//!
//! 但这两台是**同一 Team（同一 AS）**：服务打响门铃走的是它自己那一枚 BAR，看完成记录却用
//! 控制者那一页的裸 VA（`HANDOFF.used_at`）——`used` 环不在它的窗口里。故这一步**不是**跨 AS 的
//! 独立服务：那需要它自己的授权映射，或经消息/中断拿完成（见 `docs/memory-flat-nest.md` 末节）。
//!
//! **这一段是线性代码，不是故障安全**：保留在发布之前取得、`close()` 在确认之后才调用，
//! 但本域被 abort / 杀任务时这里一行都不跑（Drop 也不跑）。产品那一侧要的是
//! System **独立**持保留、并在确认停住之后才释放（见 `docs/memory-flat-nest.md` 末节）。

use alloc::format;
use core::sync::atomic::Ordering;

use env::{Call, Mark, Name, PAGE_SIZE, Page, Permission, PieToken, Wait};
use resource::dock::Dock;

use crate::boot::Accounts;
use crate::driver::iommu::{
    context, order,
    pages::Pages,
    queue::{self, CommandQueue},
    regs, table,
};
use crate::driver::pci::ecam::{self, Ecam};
use crate::driver::pci::endpoint::Endpoint;
use crate::driver::pci::virtio::{self, Caps};

use super::handoff::HANDOFF;
use super::service;

/// 设备树里 `riscv,iommu` 那段 `reg` 的起址。
const IOMMU_BASE: u64 = 0x301_0000;
/// 设备树里 `pci@…` 的 `reg`：ECAM 那一段的起址。
const ECAM_BASE: u64 = 0x3000_0000;
/// 设备树 `ranges` 里那一段 64 位 PCI 窗口的父地址：精确 BAR 就从这段里取。
const APERTURE_BASE: u64 = 0x4_0000_0000;
const APERTURE_SIZE: u64 = 0x4_0000_0000;
/// 端点：virtio 的供应商号与块设备号（过渡态 / 纯 1.x 两个号都收）。
const VENDOR: u16 = 0x1AF4;
const BLOCK: [u16; 2] = [0x1001, 0x1042];
/// 交给驱动的那两枚分支各自的门牌号。
const BAR_MARK: Mark = Mark::of("iommu-dma-bar");
/// 管理者自己那一枚：读写、可再授出，形态位照源那一枚（`ONLY` 必须一致）。
const OWNER_SUBSET: Permission = Permission::FETCH
    .union(Permission::STORE)
    .union(Permission::VEST)
    .union(Permission::ONLY);
/// 交给服务那一枚：只能读写，**不能再授出**。
const DRIVER_SUBSET: Permission = Permission::FETCH
    .union(Permission::STORE)
    .union(Permission::ONLY);
/// 保留窗里的那些页。
const DDT: usize = 0;
const ROOT: usize = 1;
const MIDDLE: usize = 2;
const LEAF: usize = 3;
const COMMANDS: usize = 4;
const FAULTS: usize = 5;
const FENCE: usize = 6;
const DESC: usize = 7;
const AVAIL: usize = 8;
const USED: usize = 9;
const DATA: usize = 10;
const HEADER: usize = 11;
const STATUS: usize = 12;
const PAGES: usize = 13;
/// 布给设备的那一段：`DESC..=STATUS` 六页。
const DEVICE_PAGES: usize = STATUS - DESC + 1;
/// 设备看到的那一段 IOVA：与真实物理页不同，故“翻过去了”不是零距离。
const IOVA_BASE: usize = 0x2_0000_0000;
/// 一轮请求读一个扇区；头是设备只读的那 16 字节。
const SECTOR: usize = 512;
const HEADER_BYTES: usize = 16;
/// 一轮里设备**可写**那两段（数据 ＋ 状态）之和：硬件写回的 `len` 就是这个数，
/// 只读的那一段头不在内（`virtio_blk` 报的是写进可写缓冲的字节数）。
const WRITTEN_BYTES: u32 = (SECTOR + 1) as u32;
/// 块设备请求类型与状态（`VIRTIO_BLK_T_IN` / `VIRTIO_BLK_S_OK`）。
const BLK_IN: u32 = 0;
const BLK_OK: u8 = 0;
/// 描述符标志：`NEXT` / `WRITE`（设备侧可写）。
const DESC_NEXT: u16 = 1;
const DESC_WRITE: u16 = 2;
/// 负证那一轮先铺在数据页里的哨兵。
const SENTINEL: u8 = 0x5A;
/// 第一轮的号（可用环那一格写它、`avail.idx` 推到 `它 + 1`）。
const ROUND_FIRST: u16 = 0;
/// 等完成、等服务、等收工的上限（毫秒）。
const WAIT_MS: usize = 2000;
/// 描述符表的容量上限（一页 / 16 字节）——“本域那一页装得下吗”是布环这一侧的判据。
const CAPACITY: usize = PAGE_SIZE / 16;

/// 端点那一景：本域就是控制者（也是保留与 IOMMU 那一侧的持有者）。
pub fn acceptance() {
    let accounts = Accounts::take().expect("iommu-dma: boot accounts");
    let retention = token(&accounts, Name::Call(Call::Retain), "retention authority");
    let iommu = token(
        &accounts,
        Name::Page(Page::Region(IOMMU_BASE)),
        "iommu region",
    );
    let ecam = token(
        &accounts,
        Name::Page(Page::Region(ECAM_BASE)),
        "ecam region",
    );
    let aperture = token(
        &accounts,
        Name::Page(Page::Region(APERTURE_BASE)),
        "pci aperture",
    );

    let iommu_dock = Dock::open(iommu).expect("iommu-dma: iommu mapping");
    let base = iommu_dock.view().base();
    let ecam_dock = Dock::open(ecam).expect("iommu-dma: ecam mapping");
    let config = ecam_dock.view().base();

    // ① 复位态的真实读数（隔离态由后面的显式编程与回读确认，不拿复位默认值当安全前提）。
    let capability = regs::read64(base, regs::CAP);
    let reset = regs::read64(base, regs::DDTP);
    note(&format!(
        "iommu-dma: base={base:#x} cap={capability:#x} version={} pas={} ddtp_reset={reset:#x} mode={} busy={}",
        capability & 0xFF,
        (capability >> 32) & 0x3F,
        reset & 0xF,
        (reset & regs::DDTP_BUSY) != 0,
    ));

    // ② 发现端点：能力表说四个区在哪一根 BAR，端点把那一段落到 aperture 窗口里。
    let requester = ecam::find(config, VENDOR, &BLOCK).expect("iommu-dma: endpoint absent");
    let scan = Ecam::new(config, requester);
    let (bar_index, caps) = Caps::read(&scan).expect("iommu-dma: virtio caps");
    // 量 BAR 那一手的读数先落串口：探尺本身红的时候，现场里要有掩码那几个数。
    let probe = scan.bar_size(bar_index);
    note(&format!(
        "iommu-dma: endpoint requester={requester} bar={bar_index} probe={probe:?}"
    ));
    let endpoint = Endpoint::claim(&scan, bar_index, (APERTURE_BASE, APERTURE_SIZE))
        .unwrap_or_else(|fail| panic!("{fail}"));
    note(&format!(
        "iommu-dma: endpoint address={:#x} size={:#x} window={:#x}..{:#x}",
        endpoint.address(),
        endpoint.size(),
        APERTURE_BASE,
        APERTURE_BASE + APERTURE_SIZE,
    ));

    // ③ 精确 BAR 的两枚窗口：管理者自己那一枚（先开图，再把它交出去），
    //    以及从它嵌套收窄出来交给服务的那一枚（坐标都是相对来源起点的绝对页偏移）。
    let first = ((endpoint.address() - APERTURE_BASE) / PAGE_SIZE as u64) as usize;
    let count = (endpoint.size() as usize).div_ceil(PAGE_SIZE);
    let mine = env::pie::accord_window(
        aperture,
        env::unit::self_id(),
        OWNER_SUBSET,
        BAR_MARK,
        first,
        count,
    )
    .expect("iommu-dma: bar window");
    let bar = Dock::open(mine).expect("iommu-dma: bar mapping");
    let bar_base = bar.view().base();
    let service = service::spawn();
    let theirs = env::pie::accord_window(mine, service, DRIVER_SUBSET, BAR_MARK, first, count)
        .expect("iommu-dma: bar window for the service");

    // ④ 先把设备那一侧的坐标交出去（放开服务之前写死；那时还没有第二个写者）。
    let (common, notify, multiplier, device) = caps.parts();
    HANDOFF.bar.store(theirs.get(), Ordering::Relaxed);
    HANDOFF.pages.store(count, Ordering::Relaxed);
    HANDOFF.common.store(common as usize, Ordering::Relaxed);
    HANDOFF.notify.store(notify as usize, Ordering::Relaxed);
    HANDOFF
        .multiplier
        .store(multiplier as usize, Ordering::Relaxed);
    HANDOFF.device.store(device as usize, Ordering::Relaxed);
    HANDOFF.desc.store(iova(DESC) as usize, Ordering::Relaxed);
    HANDOFF.avail.store(iova(AVAIL) as usize, Ordering::Relaxed);
    HANDOFF.used.store(iova(USED) as usize, Ordering::Relaxed);

    // ⑤ 保留，再发布：这一段窗是本域对每一页的强持，设备地址从 `traverse` 出口取。
    let pages = Pages::take(retention, PAGES).expect("iommu-dma: retained pages");
    HANDOFF
        .used_at
        .store(pages.at(USED, 2) as usize, Ordering::Relaxed);
    let context = context::DeviceContext::first_stage(pages.physical(ROOT));
    for (index, word) in context.words().iter().enumerate() {
        put64(
            &pages,
            DDT,
            endpoint.requester() as usize * 64 + index * 8,
            *word,
        );
    }
    table::map_pages(&pages, ROOT, MIDDLE, LEAF, IOVA_BASE, DESC, DEVICE_PAGES)
        .unwrap_or_else(|fail| panic!("{fail}"));

    let mut queues = CommandQueue::new(base, &pages, COMMANDS, FAULTS, FENCE);
    queues
        .enable()
        .unwrap_or_else(|fail| panic!("iommu-dma: {fail}"));
    // ⑥ 发布之前先失效＋IOFENCE：表与翻译缓存都到位了才接上设备目录表。
    queues.invalidate_all();
    let fenced = queues.fence();
    order::sync();
    let programmed = regs::ddtp_set(base, regs::DDTP_MODE_1LVL, Some(pages.physical(DDT)))
        .unwrap_or_else(|(name, seen)| panic!("{name}: ddtp={seen:#x}"));

    // ⑦ 第一轮：控制者摆好（头 / 描述符 / 可用环），再放开服务——设备那一半由它做。
    fill(&pages, DATA, 0);
    put32(&pages, HEADER, 0, BLK_IN);
    put32(&pages, HEADER, 4, 0);
    put64(&pages, HEADER, 8, 0);
    put8(&pages, STATUS, 0, 0xFF);
    arm(&pages, ROUND_FIRST);
    order::sync();
    env::unit::embark_task(service).expect("iommu-dma: embark the service");
    let served = await_observed();
    let used = get16(&pages, USED, 2);
    let queue_size = HANDOFF.queue_size.load(Ordering::Relaxed);
    let status = get8(&pages, STATUS, 0);
    let delivered = (0..SECTOR).all(|at| get8(&pages, DATA, at) == expected(at));
    let (faults, cause, did, ttype) = queues.faults();
    let control = regs::translate(
        base,
        endpoint.requester() as u32,
        iova(DATA) as usize,
        false,
    );
    let neighbour = regs::translate(
        base,
        endpoint.requester() as u32 + 1,
        iova(DATA) as usize,
        false,
    );
    note(&format!(
        "iommu-dma: ddtp={programmed:#x} queue={queue_size} served={served} used={used} id={} len={} status={status} faults={faults} cause={cause} did={did} ttype={ttype} translate={control:#x} neighbour={neighbour:#x}",
        get32(&pages, USED, 4),
        get32(&pages, USED, 8),
    ));
    let mapped = control & regs::TR_FAULT == 0
        && control & regs::TR_PPN_FIELD
            == regs::page_number(pages.physical(DATA)) << regs::PPN_SHIFT;
    let split = neighbour & regs::TR_FAULT != 0;
    let delivered_through = used >= ROUND_FIRST + 1
        && get32(&pages, USED, 4) == 0
        && get32(&pages, USED, 8) == WRITTEN_BYTES;
    let served_ok = served >= ROUND_FIRST + 1 && queue_size != 0 && queue_size <= CAPACITY;

    // ⑧ 第二轮先摆好（哨兵），再让服务收工：这样它**死的时候**端点是活的、队列里压着没打铃的一轮。
    fill(&pages, DATA, SENTINEL);
    put8(&pages, STATUS, 0, 0xFF);
    arm(&pages, ROUND_FIRST + 1);
    order::sync();
    HANDOFF.arm_exit();
    let stopped_service = env::unit::join_task(service, Wait::AtMost(WAIT_MS)).unwrap_or(false);

    // ⑨ 服务没了之后，控制者**自己**收口：失效＋IOFENCE、DDTP 落 Off 并回读确认，
    //    再由自己那一枚窗口打响门铃——这一下不经过任何服务。
    queues.invalidate_all();
    let fenced_before_off = queues.fence();
    order::sync();
    let stopped = regs::ddtp_set(base, regs::DDTP_MODE_OFF, None)
        .unwrap_or_else(|(name, seen)| panic!("{name}: ddtp={seen:#x}"));
    let isolated = stopped & 0xF == regs::DDTP_MODE_OFF && stopped & regs::DDTP_BUSY == 0;
    virtio::ring(bar_base, &caps, 0);
    let idle = await_used(&pages, ROUND_FIRST + 2);
    let untouched = (0..SECTOR).all(|at| get8(&pages, DATA, at) == SENTINEL);
    let unmoved = get8(&pages, STATUS, 0) == 0xFF;
    // 设备那一次尝试被 IOMMU 拒的那条记录：`ctx` 拉不出来时由 `riscv_iommu_ctx` 自己写
    // （与控制器自己的调试口读是两条路），故这一读必须排在 `translate` 之前。
    let (refusals, refusal_cause, refusal_did, refusal_ttype) = queues.faults();
    note(&format!(
        "iommu-dma: ddtp_off={stopped:#x} fenced_before_off={fenced_before_off} idle={idle} refusals={refusals} refusal_cause={refusal_cause} refusal_did={refusal_did} refusal_ttype={refusal_ttype}",
    ));
    let refused_device =
        refusal_cause == regs::DMA_DISABLED as u32 && refusal_did == endpoint.requester() as u32;
    let refused = regs::translate(
        base,
        endpoint.requester() as u32,
        iova(DATA) as usize,
        false,
    );
    let disabled = refused & regs::TR_FAULT != 0 && regs::cause(refused) == regs::DMA_DISABLED;

    // ⑩ 收口：IOFENCE 先可执行并完成，再停两台队列；确认到位之后才归还保留与撤图。
    let fenced_late = queues.fence();
    let (command_csr, fault_csr) = queues.disable();
    let page_csr = regs::read32(base, regs::PQCSR);
    let commands_off = queue::quiet(command_csr, regs::QUEUE_ON);
    let faults_off = queue::quiet(fault_csr, regs::QUEUE_ON);
    let page_unused = page_csr & (regs::QUEUE_ENABLE | regs::QUEUE_ON) == 0;
    let queue_errors = queue::command_error(command_csr)
        || queue::queue_error(fault_csr)
        || queue::queue_error(page_csr);

    let checks: [(&str, bool); 18] = [
        (
            "the owner window is exactly one bar",
            bar.view().size() == count * PAGE_SIZE,
        ),
        (
            "ddtp programmed to 1LVL",
            programmed & 0xF == regs::DDTP_MODE_1LVL,
        ),
        ("command fence completed before publishing", fenced),
        ("debug port maps the retained data page", mapped),
        ("another requester is not translated", split),
        (
            "the service observed the completion it rang for",
            served_ok,
        ),
        ("device consumed the descriptor chain", delivered_through),
        ("device wrote the image sector through IOVA", delivered),
        ("blk status is ok", status == BLK_OK),
        ("no fault recorded while the device read", faults == 0),
        ("the service stopped", stopped_service),
        ("isolation confirmed by readback", isolated),
        (
            "the device attempt after the death was refused",
            refused_device,
        ),
        (
            "the same endpoint could not reach memory",
            idle < ROUND_FIRST + 2,
        ),
        ("the target page is unchanged", untouched && unmoved),
        ("same request refused as dma-disabled", disabled),
        ("command fence completed before stopping", fenced_late),
        (
            "queues stopped and page queue never enabled",
            commands_off && faults_off && page_unused && !queue_errors,
        ),
    ];
    if let Some((name, _)) = checks.iter().find(|(_, ok)| !ok) {
        panic!("iommu-dma: {name}");
    }

    pages
        .close()
        .unwrap_or_else(|fail| panic!("iommu-dma: {fail}"));
    bar.shut()
        .unwrap_or_else(|_| panic!("iommu-dma: bar unmap"));
    ecam_dock
        .shut()
        .unwrap_or_else(|_| panic!("iommu-dma: ecam unmap"));
    iommu_dock
        .shut()
        .unwrap_or_else(|_| panic!("iommu-dma: iommu unmap"));
    note(&format!(
        "iommu-dma: requester={} bar={bar_base:#x} iova={:#x} delivered={SECTOR} bytes",
        endpoint.requester(),
        iova(DATA),
    ));
}

/// 取一枚引导账里的门闩。
fn token(accounts: &Accounts, name: Name, what: &'static str) -> PieToken {
    accounts
        .token(name)
        .unwrap_or_else(|| panic!("iommu-dma: {what}"))
}

/// 第 `page` 页在设备那一侧的 IOVA。
const fn iova(page: usize) -> u64 {
    (IOVA_BASE + (page - DESC) * PAGE_SIZE) as u64
}

/// 宿主造那张盘时用的花纹：第 `at` 字节 = `at % 256`。
fn expected(at: usize) -> u8 {
    (at % 256) as u8
}

/// 摆一轮请求：三段描述符（头 → 数据 → 状态）＋可用环那一格。
///
/// 链头恒为 0（一轮一组三段），故可用环里那一格写 0；`avail.idx` 推进到 `round + 1`。
fn arm(pages: &Pages, round: u16) {
    descriptor(pages, 0, iova(HEADER), HEADER_BYTES as u32, DESC_NEXT, 1);
    descriptor(
        pages,
        1,
        iova(DATA),
        SECTOR as u32,
        DESC_NEXT | DESC_WRITE,
        2,
    );
    descriptor(pages, 2, iova(STATUS), 1, DESC_WRITE, 0);
    put16(pages, AVAIL, 4 + round as usize * 2, 0);
    put16(pages, AVAIL, 2, round + 1);
}

fn descriptor(pages: &Pages, index: usize, address: u64, len: u32, flags: u16, next: u16) {
    let at = index * 16;
    put64(pages, DESC, at, address);
    put32(pages, DESC, at + 8, len);
    put16(pages, DESC, at + 12, flags);
    put16(pages, DESC, at + 14, next);
}

/// 等服务把它看到的完成记录报回来（有界）。
fn await_observed() -> u16 {
    let until = env::chrono::clock().saturating_add(WAIT_MS as u64 * 1_000_000);
    loop {
        let seen = HANDOFF.observed.load(Ordering::Relaxed);
        if seen != 0 || env::chrono::clock() >= until {
            return seen as u16;
        }
        let _ = execution::room::park(core::time::Duration::from_millis(1));
    }
}

/// 有界等硬件把 `used.idx` 推到 `want`（本域那一页在保留窗里）。
fn await_used(pages: &Pages, want: u16) -> u16 {
    await_used_at(pages.at(USED, 2) as *const u16, want, WAIT_MS)
}

/// 有界等 `used.idx` 到 `want`，返最后看到的那个数（服务那一侧也走这一条）。
///
/// 每一次等待都过一次 `park`：完成的载荷要主循环转起来才送得到本域，
/// 而 vCPU 一直不退到主循环就没人推它。故这不是兜底，是这条路上必须走的一步。
pub(super) fn await_used_at(used: *const u16, want: u16, within_ms: usize) -> u16 {
    let until = env::chrono::clock().saturating_add(within_ms as u64 * 1_000_000);
    loop {
        // SAFETY: `used` 指向保留窗里已用环那一页的 `idx` 格（本域已映射）。
        let seen = unsafe { core::ptr::read_volatile(used) };
        if seen >= want || env::chrono::clock() >= until {
            return seen;
        }
        let _ = execution::room::park(core::time::Duration::from_millis(1));
    }
}

/// 把一整页铺成同一个字节。
fn fill(pages: &Pages, page: usize, byte: u8) {
    // SAFETY: 保留窗里的页是本域刚取的可写页，整页都在这一段里。
    unsafe { core::ptr::write_bytes(pages.at(page, 0), byte, PAGE_SIZE) };
}

fn put8(pages: &Pages, page: usize, at: usize, value: u8) {
    // SAFETY: 保留窗里的页是本域刚取的可写页，偏移在页内。
    unsafe { core::ptr::write_volatile(pages.at(page, at), value) };
}

fn put16(pages: &Pages, page: usize, at: usize, value: u16) {
    // SAFETY: 同 `put8`，偏移按 2 字节格取。
    unsafe { core::ptr::write_volatile(pages.at(page, at) as *mut u16, value) };
}

fn put32(pages: &Pages, page: usize, at: usize, value: u32) {
    // SAFETY: 同 `put8`，偏移按 4 字节格取。
    unsafe { core::ptr::write_volatile(pages.at(page, at) as *mut u32, value) };
}

fn put64(pages: &Pages, page: usize, at: usize, value: u64) {
    // SAFETY: 同 `put8`，偏移按 8 字节格取。
    unsafe { core::ptr::write_volatile(pages.at(page, at) as *mut u64, value) };
}

fn get8(pages: &Pages, page: usize, at: usize) -> u8 {
    // SAFETY: 保留窗里的页是本域刚取的页，偏移在页内。
    unsafe { core::ptr::read_volatile(pages.at(page, at)) }
}

fn get16(pages: &Pages, page: usize, at: usize) -> u16 {
    // SAFETY: 同 `get8`，偏移按 2 字节格取。
    unsafe { core::ptr::read_volatile(pages.at(page, at) as *const u16) }
}

fn get32(pages: &Pages, page: usize, at: usize) -> u32 {
    // SAFETY: 同 `get8`，偏移按 4 字节格取。
    unsafe { core::ptr::read_volatile(pages.at(page, at) as *const u32) }
}

fn note(message: &str) {
    crate::debug::put(message);
}
