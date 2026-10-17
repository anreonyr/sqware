//! 端点服务那一半：**只碰设备**，不碰内存。
//!
//! 它从控制者那一枚孔上收下坐标（一枚精确到一根 BAR 的窗口种子、页数与几个 IOVA），
//! 按设备寄存器那一套把设备立起来、打响一次门铃，然后就停在那里——控制者自己看
//! 完成记录，并在合适的时候收它。
//!
//! **它不读控制者的内存**：设备写回的 `used` 环在控制者保留的页里，那是控制者自己的
//! 事；同一 Team 里靠裸 VA 等完成记录那一手这里没有。故它手里只有一枚窗口与几个数。
//!
//! 它自己那一枚窗口就是**唯一事实**：`Open` 出来的长度必须正好是那一根 BAR——
//! 多一页（拿到整段 aperture）就是交付路径把窗口丢了，当场拒。

use env::{PAGE_SIZE, PieToken, Wait};
use resource::dock::Dock;
use resource::raw::Hole;

use crate::driver::pci::virtio::{Caps, Virtio};

/// 控制者把坐标推给本服务的那一枚孔的门牌（与 `system::device::dma` 同一枚）。
pub const COORD_MARK: env::Mark = env::Mark::of("system-dma-coord");
/// 等坐标、等一次完成记录的期限（毫秒）。
const WAIT_MS: usize = 2000;
/// 那一帧坐标的字数：种子 / 页数 / common / notify / multiplier / device / desc / avail / used。
const WORDS: usize = 9;

/// 本服务那一台的身子。
pub fn run() -> Result<(), &'static str> {
    let sire = env::unit::sire();
    let entry = ipc::session::establish::claim(sire, COORD_MARK, Wait::AtMost(WAIT_MS))
        .map_err(|_| "system-dma: coordinate channel")?;
    let mut bytes = [0u8; WORDS * 8];
    let (len, _) = Hole::from_raw(entry)
        .pull(&mut bytes, Wait::AtMost(WAIT_MS))
        .map_err(|_| "system-dma: coordinates")?;
    if len != bytes.len() {
        return Err("system-dma: coordinate frame");
    }
    let mut words = [0usize; WORDS];
    for (at, word) in words.iter_mut().enumerate() {
        *word = usize::from_le_bytes(
            bytes[at * 8..at * 8 + 8]
                .try_into()
                .map_err(|_| "system-dma: coordinate word")?,
        );
    }
    let seed = words[0];
    let bar_pages = words[1];
    let (desc, avail, used) = (words[6] as u64, words[7] as u64, words[8] as u64);

    let bar = Dock::open(PieToken::mint(seed)).map_err(|_| "system-dma: bar mapping")?;
    // 承重判据：领到的就只有那一段——整段 aperture 或任何别的东西都当场拒。
    if bar.view().size() != bar_pages * PAGE_SIZE {
        return Err("system-dma: bar window is not the exact bar");
    }
    let caps = Caps::from_parts(
        words[2] as u32,
        words[3] as u32,
        words[4] as u32,
        words[5] as u32,
        bar.view().size(),
    )
    .ok_or("system-dma: virtio caps")?;
    let mut virtio = Virtio::open(bar.view().base(), &caps)?;
    let queue = virtio.setup_queue(desc, avail, used)?;
    crate::debug::put(&alloc::format!(
        "system-dma: service bar={:#x} pages={bar_pages} queue={queue} status={}",
        bar.view().base(),
        virtio.status(),
    ));
    virtio.notify();

    // 铃已打：等一会儿（控制者在这段时间里看完成记录），然后收工退场。
    // 控制者若先一步收它（`room::doom`/`slay_team`），这一圈就走不到。
    let until = env::chrono::clock().saturating_add(WAIT_MS as u64 * 1_000_000);
    while env::chrono::clock() < until {
        let _ = execution::room::park(core::time::Duration::from_millis(5));
    }
    Ok(())
}
