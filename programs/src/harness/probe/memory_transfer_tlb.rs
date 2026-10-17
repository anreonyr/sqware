//! 热多 Hart 旧 TLB：源页在**远端 hart** 上用命过，撤页之后它第一次再访存必须缺页。
//!
//! 要证的独立效果：远端 hart 已经访过源页之后，`caller.pend` 横跨真实 `Transfer` 的
//! 撤页、远端 RFENCE 与结算仍然进展；移交**成功返回之后**、任何后续发布或别的同
//! ASID 刷新之前，远端第一次再 load 那个 VA 必须产生真实的 `LoadPageFault`，
//! 不能读到旧 Frame。
//!
//! 这一景只有两台，都在**引导镜像那一域**里（同一个 Team／同一张 ASID）：
//! **控制者**就是引导那一台（`acceptance`），它备来源、填花纹、写共享窗、再放行
//! **读者**——同一 Team 里一台 raw 起的真任务（`reader::spawn`）。
//!
//! 程序序就是证据：读者那一段窗口一进去就清 `SIE`、换上自己的向量，此后没有
//! ecall、没有 `satp`/`sfence.vma`、没有分配、也没有调度，只有直接 `rdtime` 与
//! 共享窗那两枚 atomic 门；控制者只在读到 `warmed` 之后做**那一次** `transfer`，
//! 并在它成功返回之后才开采样门。同一颗 hart 不可能平行做完这两件事（读者不放手），
//! 故读到 `warmed` 只可能是另一颗 hart 上那一段窗口还开着的时候。
//!
//! 不被别的刷新代替：从观察到 `warmed` 到开采样门之间，控制者只做那一次 `Transfer`
//! （`Source::shift`）——不 `format!`、不分配、不 `Attach`、不碰别的来源；判定用的
//! 那批读数也在读者落完结论（`status` 的 Acquire）之后才取，故没有第二次同 ASID
//! shootdown 能替这一次撤页背书。
//!
//! 时限全在窗口外先折好：读者那一段只能用直接 `rdtime`（`env::chrono` 是 ecall），
//! 故频率从引导账里那台机器的设备树读（与内核同一处事实），在放行读者之前折成 tick。
//! 读者自己到点就走出口，不会无限自旋；控制者两侧各有自己的总时限。
//!
//! 两端确实是“非内核 ASID 且非全局条目”：`Transfer` 的撤页对那两类当场拒
//! （`withdraw_pressings` → `NotTransferable`），而这里它成功了，故远端那一下失效
//! 只可能是按 ASID 定向的那一条（`RemoteSfenceVmaAsid`）。

mod hotpath;
mod reader;
mod window;

use alloc::format;

use env::{Name, Page};
use resource::dock::Dock;

use super::memory_transfer::console::Console;
use super::memory_transfer::fault;
use super::memory_transfer::source::Source;
use crate::boot::Accounts;

use window::Window;

/// 读者那一段热窗口的期限（纳秒），从它进窗口起算。
const WINDOW_LIMIT_NS: u64 = 200_000_000;
/// 控制者等 `warmed` 的总时限（纳秒）：恒长于上面那一格。
const WARM_LIMIT_NS: u64 = 5_000_000_000;
/// 控制者等读者结论的总时限（纳秒）。
const DONE_LIMIT_NS: u64 = 6_000_000_000;
/// `scause` 的 LoadPageFault 码。
const LOAD_PAGE_FAULT: usize = 13;
/// 纳秒换算的分母。
const NANOS_PER_SEC: u64 = 1_000_000_000;
/// `sstatus.SIE` 那一位。
const SIE: usize = 1 << 1;

/// 引导那一台：本景的控制者。
///
/// 它自己的失败判定在最后一步一口说完（读数先落串口，再判），
/// 故红的那一次现场里那排数仍然在。
pub fn acceptance() {
    let accounts = Accounts::take().expect("memory-transfer-tlb: boot accounts");
    let console = console(&accounts);
    let budget = budget(hertz(&accounts));
    let mut source = Source::acquire(2, 1);
    let va = source.page(0);
    // 懒物化与花纹都在放行读者之前做完：窗口里那两次访问都不该再缺页。
    fault::fill(0, va);
    window::WINDOW.prepare(va, fault::at(0, 0) as usize, budget);
    let reader = reader::spawn();
    note(
        &console,
        &format!(
            "reader tid={} va={:#x} view={} budget={}",
            reader.get(),
            va,
            source.id().get(),
            budget
        ),
    );
    env::unit::embark_task(reader).expect("memory-transfer-tlb: embark the reader");

    let warmed = await_warmed();
    if warmed {
        // 热窗口里唯一那一次被测动作：撤下源页、远端 RFENCE、结算。
        // 到开采样门之前不再写别的任何一格里（不 format!、不分配、不 Attach）；
        // 它失败就是本景失败——判死在 `Source::shift` 那一处（与 D1 共用同一入口）。
        source.shift(0, 1);
        window::WINDOW.open_sample();
    }
    let settled = await_status();

    let w = &window::WINDOW;
    note(
        &console,
        &format!(
            "reader tid={} warm=0x{:x} warm_at=0x{:x} sample=0x{:x} sepc=0x{:x} cause={} \
             stval=0x{:x} byte=0x{:x} status={} deadline=0x{:x} sstatus=0x{:x} stvec=0x{:x}",
            reader.get(),
            w.warm(),
            w.warm_at(),
            w.sample_at(),
            w.sepc(),
            w.cause(),
            w.stval(),
            w.byte(),
            w.status(),
            w.deadline(),
            w.sstatus(),
            w.stvec(),
        ),
    );
    if let Some(fail) = judge(warmed, settled) {
        note(&console, &format!("FAIL {fail}"));
        panic!("memory-transfer-tlb: {fail}");
    }
    note(&console, "fault caught at the sampled load");
    source.close();
    note(&console, "close done");
}

/// 本景要证的那一件事发生了没有：`None` = 发生了，`Some(理由)` = 没发生。
///
/// 要证的是：采样那一次访存（`sample_at` 那一条）在移交成功结算、而别的什么都没做
/// 的那一段之后，仍然按“`sepc` 指着它自己、`stval` 指着公告 VA、`cause` 是取数缺页”
/// 的方式缺页；而它之前那次热身已经把这一页的翻译放进那颗 hart 的 TLB 了。
fn judge(warmed: bool, settled: bool) -> Option<&'static str> {
    let w = &window::WINDOW;
    if !warmed {
        return Some(match w.status() {
            Window::TRAPPED => "the warm access faulted before the reader published warmed",
            _ => "no verdict and no warmed: the reader never entered its window",
        });
    }
    if !settled {
        return Some("the reader never left its window");
    }
    match w.status() {
        Window::STALE => return Some("the sampled load read a byte: the hot TLB was not revoked"),
        Window::TIMEOUT => return Some("the reader timed out waiting for the sampled gate"),
        Window::TRAPPED => {}
        _ => return Some("the reader left no verdict at all"),
    }
    // 四格地址必须都是真地址：窗口一处也没发布时，`sepc == sample_at` 会以 0 == 0 成立，
    // 那样“cause/stval 都对”就能把一次**没发生过的采样**判成通过。
    if w.va() == 0 || w.warm_at() == 0 || w.sample_at() == 0 || w.sepc() == 0 {
        return Some("the window never published its load addresses");
    }
    if w.sample_at() == w.warm_at() {
        return Some("the warm access and the sampled load are the same instruction");
    }
    if w.sepc() != w.sample_at() {
        return Some("the trap is not at the sampled load");
    }
    if w.cause() != LOAD_PAGE_FAULT {
        return Some("the trap is not a load page fault");
    }
    if w.stval() != w.va() {
        return Some("the trap's stval is not the announced page");
    }
    if w.warm() != w.want() {
        return Some("the warm access did not read the announced pattern");
    }
    if w.sstatus() & SIE == 0 {
        return Some("the window did not start with interrupts on");
    }
    None
}

/// 等 `warmed`：到点没等到就当场说不，也不往下做那一次移交。
fn await_warmed() -> bool {
    let until = env::chrono::clock() + WARM_LIMIT_NS;
    while !window::WINDOW.warmed() {
        if env::chrono::clock() >= until {
            return false;
        }
    }
    true
}

/// 等读者那一段结论：到点还是空的就是本景失败（读者自己那一份期限另有它的）。
fn await_status() -> bool {
    let until = env::chrono::clock() + DONE_LIMIT_NS;
    while window::WINDOW.status() == Window::PENDING {
        if env::chrono::clock() >= until {
            return false;
        }
    }
    true
}

/// 本景输出那一面：串口那一页的门闩由**引导账**直接给（控制者自己就是引导镜像），
/// 不经任何装载者转手。本景只出不进，故没有宿主门。
fn console(accounts: &Accounts) -> Console {
    let region = Console::region(accounts);
    let token = accounts
        .token(region)
        .expect("memory-transfer-tlb: the console lock");
    Console::open(token)
}

/// 报一行本景的话（串口那一面，宿主按前缀认）。
///
/// 本景那一面**只出不进**，宿主不开任何门，故不走 D1 那具带宿主门的 `Gate`：
/// 一行就是一段现推的字节。
fn note(console: &Console, text: &str) {
    console.put(format!("memory-transfer-tlb: {text}\n").as_bytes());
}

/// 热窗口那一段期限折成 `rdtime` 的 tick 数。
fn budget(hertz: u64) -> usize {
    (hertz.saturating_mul(WINDOW_LIMIT_NS) / NANOS_PER_SEC) as usize
}

/// 这台机器的时基频率（Hz）：与内核同一处事实——引导账里那棵设备树的 `/cpus`。
fn hertz(accounts: &Accounts) -> u64 {
    const PROPERTY: &str = "timebase-frequency";
    let token = accounts
        .token(Name::Page(Page::Dtb))
        .expect("memory-transfer-tlb: boot device tree");
    let dock = Dock::open(token).expect("memory-transfer-tlb: device tree view");
    // SAFETY: `Page::Dtb` 那一支门闩借映的就是一棵终身存活的 FDT。
    let tree = unsafe { fdt::Fdt::from_ptr(dock.view().base() as *const u8) }
        .expect("memory-transfer-tlb: device tree parse");
    tree.find_node("/cpus")
        .and_then(|cpus| cpus.property(PROPERTY))
        .and_then(|property| property.as_usize())
        .expect("memory-transfer-tlb: the machine timebase") as u64
}
