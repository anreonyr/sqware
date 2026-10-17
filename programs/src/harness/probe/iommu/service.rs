//! 端点服务：一台**真任务**，只领那一段 BAR 与那三个 IOVA。
//!
//! 它按设备寄存器那一套把设备立起来、打响一次门铃、等一次完成（这就是驱动那半边），
//! 然后**退场**——不撤队列、不复位设备：端点还是活的、队列里还压着控制者刚摆好的那一轮，
//! 这正是“服务死掉时半边配置还在”那一种局面。
//!
//! 它不碰内存那一侧：队列、描述符、数据页都在控制者手里（那是所有者按保留发布的那些页，
//! 硬件地址也是它发布的）。故服务手里只有一枚窗口（精确到一根 BAR）与几个数字。

use core::sync::atomic::Ordering;

use env::{EXIT_OK, PAGE_SIZE, PieToken, TaskId, TeamId};
use resource::dock::Dock;

use super::dma::await_used_at;
use super::handoff::HANDOFF;
use super::virtio::{Caps, Virtio};

/// 服务自己的人读那几格（放开之前写死，故不取 Acquire 那一套）。
const READY: Ordering = Ordering::Relaxed;
/// 服务自己写下那两格的次序（控制者等的是另一枚门，不是这里）。
const WROTE: Ordering = Ordering::Relaxed;
/// 服务等一次完成记录的期限（毫秒），与等收工许可同一个量级。
const WAIT_MS: usize = 2000;

/// 起一台端点服务（**held**：控制者把坐标写死、翻译发布之后才放它走）。
pub(super) fn spawn() -> TaskId {
    execution::unit::spawn(TeamId::new(0), entry as *const () as usize, &[], 0)
        .unwrap_or_else(|error| panic!("iommu-dma: spawn endpoint service: {:?}", error.source))
}

/// 那条 raw 入口（`a0` = 启动参数区；本景不给参数）。
extern "C" fn entry(_arg: usize) -> ! {
    run();
    execution::room::reap(EXIT_OK, None)
}

fn run() {
    let token = PieToken::mint(HANDOFF.bar.load(READY));
    let pages = HANDOFF.pages.load(READY);
    let dock = Dock::open(token).expect("iommu-dma: service bar mapping");
    let bar = dock.view().base();
    // 领到的就只有那一段：长度不多不少正是那一根 BAR（多一页都不行）。
    assert_eq!(
        dock.view().size(),
        pages * PAGE_SIZE,
        "the service is handed exactly the bar window"
    );
    let caps = Caps::from_parts(
        HANDOFF.common.load(READY) as u32,
        HANDOFF.notify.load(READY) as u32,
        HANDOFF.multiplier.load(READY) as u32,
        HANDOFF.device.load(READY) as u32,
        dock.view().size(),
    )
    .expect("iommu-dma: service caps");

    let mut virtio = Virtio::open(bar, &caps).unwrap_or_else(|fail| panic!("{fail}"));
    let (desc, avail, used) = (
        HANDOFF.desc.load(READY) as u64,
        HANDOFF.avail.load(READY) as u64,
        HANDOFF.used.load(READY) as u64,
    );
    let size = virtio
        .setup_queue(desc, avail, used)
        .unwrap_or_else(|fail| panic!("{fail}"));
    HANDOFF.queue_size.store(size as usize, WROTE);
    virtio.notify();
    let observed = await_used_at(HANDOFF.used_at.load(READY) as *const u16, 1, WAIT_MS);
    HANDOFF.observed.store(observed as usize, WROTE);
    crate::debug::put(&alloc::format!(
        "iommu-dma: service queue={size} used={observed} status={}",
        virtio.status(),
    ));

    // 等控制者说“第二轮已摆好”再收工：这样它死的时候队列里压着没打铃的那一轮。
    let until = env::chrono::clock().saturating_add(WAIT_MS as u64 * 1_000_000);
    while !HANDOFF.armed() && env::chrono::clock() < until {
        let _ = execution::room::park(core::time::Duration::from_millis(1));
    }
    // 只撤自己那一段图；门闩与它的分支随任务退场收尾。
    let _ = dock.shut();
}
