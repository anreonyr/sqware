//! 真实 PCI 端点在 IOMMU 翻译下的 DMA：ECAM 发现 → 精确 BAR → 真 DMA（正证）→
//! 隔离之后同一端点再也进不来（负证）→ 确认到位才归还保留。
//!
//! 四件事各有唯一条目：
//! - **发现**：`ecam` 扫配置空间、`virtio` 读能力表、`endpoint` 把 BAR 落位；
//! - **交付**：BAR 从 `Page::Region(aperture)` 那一枚门闩上派生**一段有界窗口**
//!   （`PieCall::AccordWindow`），只映那一根 BAR——不放宽、不下放整段 aperture；
//! - **发布**：硬件到得了的每一页先经保留强持，再把**真实物理页起址**写进 DDT/DC/PT/队列；
//!   设备看到的是另一段 IOVA，故“按我们写下的表翻过去的”这件事有承重的证据；
//! - **收口**：先 IOTINVAL ＋ IOFENCE（可执行、要等到完成记录），再停队列，
//!   再把 DDTP 落 Off 并回读确认；确认到位之后才 `close()` 归还保留。
//!
//! **这一段是线性代码，不是故障安全**：保留在发布之前取得、`close()` 在确认之后才调用，
//! 但本域被 abort / 杀任务时这里一行都不跑（Drop 也不跑）。产品那一侧要的是
//! System **独立**持保留、并在确认停住之后才释放（见 `docs/memory-flat-nest.md` 末节）。

use alloc::format;

use env::{Call, Mark, Name, PAGE_SIZE, Page, Permission, PieToken};
use resource::dock::Dock;

use crate::boot::Accounts;

use super::ecam::{self, Ecam};
use super::endpoint::Endpoint;
use super::pages::Pages;
use super::queue::{self, CommandQueue};
use super::virtio::{Caps, Virtio};
use super::{context, order, regs, table};

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
/// 交给驱动的那一段 BAR 的门牌号（这一枚的分支只给这一段）。
const BAR_MARK: Mark = Mark::of("iommu-dma-bar");
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
const DEVICE_FIRST: usize = DESC;
const DEVICE_PAGES: usize = STATUS - DESC + 1;
/// 设备看到的那一段 IOVA：与真实物理页不同，故“翻过去了”不是零距离。
const IOVA_BASE: usize = 0x2_0000_0000;
/// 一轮请求读一个扇区。
const SECTOR: usize = 512;
/// `virtio_blk_req` 头与块设备请求类型（`VIRTIO_BLK_T_IN`）。
const HEADER_BYTES: usize = 16;
const BLK_IN: u32 = 0;
const BLK_OK: u8 = 0;
/// 描述符标志：`NEXT` / `WRITE`（设备侧可写）。
const DESC_NEXT: u16 = 1;
const DESC_WRITE: u16 = 2;
/// 一轮里设备**可写**那两段（数据 ＋ 状态）之和：硬件写回的 `len` 就是这个数，
/// 只读的那一段头不在内（`virtio_blk` 报的是写进可写缓冲的字节数）。
const WRITTEN_BYTES: u32 = (SECTOR + 1) as u32;
/// 负证那一轮先铺在数据页里的哨兵。
const SENTINEL: u8 = 0x5A;
/// 等完成的上限（毫秒）。
const WAIT_MS: usize = 2000;
/// 队列描述符表的容量上限（一页 / 16 字节）。
const CAPACITY: u16 = (PAGE_SIZE / 16) as u16;
/// 一轮门铃之后等硬件写回的那一个数。
const FIRST_USED: u16 = 1;

/// 端点那一景：本域就是控制者。
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
    let caps = Caps::read(&scan).expect("iommu-dma: virtio caps");
    // 量 BAR 那一手的读数先落串口：探尺本身红的时候，现场里要有掩码那几个数。
    let probe = scan.bar_size(caps.bar());
    note(&format!(
        "iommu-dma: endpoint requester={requester} bar={} probe={probe:?}",
        caps.bar(),
    ));
    let endpoint = Endpoint::claim(&scan, caps.bar(), (APERTURE_BASE, APERTURE_SIZE))
        .unwrap_or_else(|fail| panic!("{fail}"));
    note(&format!(
        "iommu-dma: endpoint address={:#x} size={:#x} window={:#x}..{:#x}",
        endpoint.address(),
        endpoint.size(),
        APERTURE_BASE,
        APERTURE_BASE + APERTURE_SIZE,
    ));

    // ③ 精确 BAR：从 aperture 那一枚门闩上派生**正好这一根 BAR** 的有界窗口。
    // 窗口坐标是**相对来源起点的绝对页偏移**，故嵌套派生不会叠加两层偏移；
    // 权限只到 RW ＋ 可再授出（不能执行；来源的 `ONLY` 形态位照旧）。
    let first = ((endpoint.address() - APERTURE_BASE) / PAGE_SIZE as u64) as usize;
    let count = (endpoint.size() as usize).div_ceil(PAGE_SIZE);
    let mine = env::pie::accord_window(
        aperture,
        env::unit::self_id(),
        Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY,
        BAR_MARK,
        first,
        count,
    )
    .expect("iommu-dma: bar window");
    let bar = Dock::open(mine).expect("iommu-dma: bar mapping");
    let bar_base = bar.view().base();
    let bar_size = bar.view().size();

    // ④ 先保留，再发布：这一段窗是本域对每一页的强持，设备地址从 `traverse` 出口取。
    let pages = Pages::take(retention, PAGES).expect("iommu-dma: retained pages");
    let context = context::DeviceContext::first_stage(pages.physical(ROOT));
    for (index, word) in context.words().iter().enumerate() {
        put64(
            &pages,
            DDT,
            endpoint.requester() as usize * 64 + index * 8,
            *word,
        );
    }
    table::map_pages(
        &pages,
        ROOT,
        MIDDLE,
        LEAF,
        IOVA_BASE,
        DEVICE_FIRST,
        DEVICE_PAGES,
    )
    .unwrap_or_else(|fail| panic!("{fail}"));

    let mut queues = CommandQueue::new(base, &pages, COMMANDS, FAULTS, FENCE);
    queues
        .enable()
        .unwrap_or_else(|fail| panic!("iommu-dma: {fail}"));
    // ⑤ 发布之前先失效＋IOFENCE：表与翻译缓存都到位了才接上设备目录表。
    queues.invalidate_all();
    let fenced = queues.fence();
    order::sync();
    let programmed = regs::ddtp_set(base, regs::DDTP_MODE_1LVL, Some(pages.physical(DDT)))
        .unwrap_or_else(|(name, seen)| panic!("{name}: ddtp={seen:#x}"));

    // ⑥ 驱动：把设备立起来，队列三段指向**本域挑的 IOVA**（不是物理地址）。
    let mut virtio = Virtio::open(bar_base, &caps).unwrap_or_else(|fail| panic!("{fail}"));
    let capacity = virtio
        .setup_queue(iova(DESC), iova(AVAIL), iova(USED), CAPACITY)
        .unwrap_or_else(|fail| panic!("{fail}"));

    // ⑦ 正证：一轮真读。头、数据、状态三页都只经 IOVA 到得了。
    fill(&pages, DATA, 0);
    put32(&pages, HEADER, 0, BLK_IN);
    put32(&pages, HEADER, 4, 0);
    put64(&pages, HEADER, 8, 0);
    put8(&pages, STATUS, 0, 0xFF);
    arm(&pages, 0);
    virtio.notify();
    let used = await_used(&pages, FIRST_USED);
    let status = get8(&pages, STATUS, 0);
    let delivered = (0..SECTOR).all(|at| get8(&pages, DATA, at) == expected(at));
    let (faults, cause, did, ttype) = queues.faults();
    let control = regs::translate(
        base,
        endpoint.requester() as u32,
        IOVA_BASE + (DATA - DESC) * PAGE_SIZE,
        false,
    );
    let neighbour = regs::translate(
        base,
        endpoint.requester() as u32 + 1,
        IOVA_BASE + (DATA - DESC) * PAGE_SIZE,
        false,
    );
    note(&format!(
        "iommu-dma: ddtp={programmed:#x} fenced={fenced} capacity={capacity} used={used} id={} len={} status={status} faults={faults} cause={cause} did={did} ttype={ttype} translate={control:#x} neighbour={neighbour:#x}",
        get32(&pages, USED, 4),
        get32(&pages, USED, 8),
    ));
    let mapped = control & regs::TR_FAULT == 0
        && control & regs::TR_PPN_FIELD
            == regs::page_number(pages.physical(DATA)) << regs::PPN_SHIFT;
    let split = neighbour & regs::TR_FAULT != 0;
    let delivered_through = used >= FIRST_USED
        && get16(&pages, USED, 2) >= FIRST_USED
        && get32(&pages, USED, 4) == 0
        && get32(&pages, USED, 8) == WRITTEN_BYTES;

    // ⑧ 负证：先失效＋IOFENCE（**可执行并完成**），再把 DDTP 落 Off 并回读确认。
    // 然后同一个端点、同一段数据再来一轮：门铃响了，但翻译已经不在了。
    queues.invalidate_all();
    let fenced_before_off = queues.fence();
    order::sync();
    let stopped = regs::ddtp_set(base, regs::DDTP_MODE_OFF, None)
        .unwrap_or_else(|(name, seen)| panic!("{name}: ddtp={seen:#x}"));
    let isolated = stopped & 0xF == regs::DDTP_MODE_OFF && stopped & regs::DDTP_BUSY == 0;
    fill(&pages, DATA, SENTINEL);
    put8(&pages, STATUS, 0, 0xFF);
    arm(&pages, 1);
    virtio.notify();
    let idle = await_used(&pages, FIRST_USED + 1);
    let untouched = (0..SECTOR).all(|at| get8(&pages, DATA, at) == SENTINEL);
    let unmoved = get8(&pages, STATUS, 0) == 0xFF;
    let refused = regs::translate(
        base,
        endpoint.requester() as u32,
        IOVA_BASE + (DATA - DESC) * PAGE_SIZE,
        false,
    );
    note(&format!(
        "iommu-dma: ddtp_off={stopped:#x} fenced_before_off={fenced_before_off} idle={idle} refused={refused:#x} refused_cause={}",
        regs::cause(refused),
    ));
    let disabled = refused & regs::TR_FAULT != 0 && regs::cause(refused) == regs::DMA_DISABLED;

    // ⑨ 收口：IOFENCE 先可执行并完成，再停两台队列；然后才归还保留与撤图。
    let fenced_late = queues.fence();
    let (command_csr, fault_csr) = queues.disable();
    let page_csr = regs::read32(base, regs::PQCSR);
    let commands_off = queue::quiet(command_csr, regs::QUEUE_ON);
    let faults_off = queue::quiet(fault_csr, regs::QUEUE_ON);
    let page_unused = page_csr & (regs::QUEUE_ENABLE | regs::QUEUE_ON) == 0;
    let queue_errors = queue::command_error(command_csr)
        || queue::queue_error(fault_csr)
        || queue::queue_error(page_csr);

    let checks: [(&str, bool); 15] = [
        (
            "bar window is exactly one bar",
            bar_size == count * PAGE_SIZE,
        ),
        (
            "ddtp programmed to 1LVL",
            programmed & 0xF == regs::DDTP_MODE_1LVL,
        ),
        ("command fence completed before publishing", fenced),
        ("debug port maps the retained data page", mapped),
        ("another requester is not translated", split),
        ("device consumed the descriptor chain", delivered_through),
        ("device wrote the image sector through IOVA", delivered),
        ("blk status is ok", status == BLK_OK),
        ("no fault recorded while the device read", faults == 0),
        ("isolation confirmed by readback", isolated),
        (
            "the same endpoint could not reach memory",
            idle < FIRST_USED + 1,
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
fn token(accounts: &Accounts, name: Name, what: &str) -> PieToken {
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

/// 有界等硬件把 `used.idx` 推到 `want`，返最后看到的那个数。
///
/// 每一次等待都过一次 `park`：完成的载荷要主循环转起来才送得到本域，
/// 而 vCPU 一直不退到主循环就没人推它。故这不是兜底，是这条路上必须走的一步。
fn await_used(pages: &Pages, want: u16) -> u16 {
    let start = env::chrono::clock();
    loop {
        let seen = get16(pages, USED, 2);
        if seen >= want || env::chrono::clock().saturating_sub(start) > WAIT_MS as u64 * 1_000_000 {
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
