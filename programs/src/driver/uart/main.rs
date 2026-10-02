#![no_std]
#![no_main]

//! serial@10000000 的持有者，兼控制台服务（U 态，见
//! `driver/uart/mod.rs`）。

extern crate alloc;
extern crate programs;

/// 纯功能：交出去的那一批（非空不可表达）
mod core;

mod adapt;

mod dev;

use crate::core::batch::Batch;
use crate::dev::uart as device;
use env::{HoleDir, Wait};
use programs::driver::shared::fail::Fail;
use programs::unit::uart::E_UART;
use runtime::PAGE_SIZE;
use runtime::core::res::pile::Pile;
use runtime::env::mail::HolePie;

/// 它是一块 **Pane**：两枚门牌 `rx` / `tx` 在它下面
const ME: &str = "uart";

const MS: usize = 1000;

/// 一次排空最多搬走多少字节。FIFO 只有 16 字节，取四倍宽；满了剩下的还在设备里
const DRAIN_MAX: usize = 64;

/// **待交的那一列最多排几批**（满了就先不收新的——设备 FIFO 里的字节不会丢）。
const PENDING_MAX: usize = 16;

/// **"等这一批被取走"的期限**（毫秒）：等不到就回去**收写口/收线**，下一圈再来等。
/// 一拍足够（人那一圈就在毫秒级）；而**没有它就是一个死环**（见圈首那一节）。
const TAKEN_MS: usize = 1;

/// `Ok(())` 是"跑完了"（常驻域走不到那一格）。**一族口径**在 programs::driver::shared::fail
#[programs::entry]
fn main() -> Result<(), Fail> {
    // 也不写死地址："哪一台是串口"由设备账回答（类 `ns16550a`）。
    let desk = adapt::desk::start(Wait::AtMost(MS))?;

    // **两个源**：写口上有客人交来的一条字、线上有"设备收来了字节"——组等任意一格
    // （与 `rtc` 那一台同一条判据）。
    let pile = Pile::unseal(false).map_err(|_| Fail::at(E_UART, "desk"))?;
    let lane = desk.line.hole().map_err(|_| Fail::at(E_UART, "line"))?;
    if pile.attach(&desk.tx, HoleDir::Pull).is_err()
        || pile
            .attach(&HolePie::from_token(lane), HoleDir::Pull)
            .is_err()
    {
        return Err(Fail::at(E_UART, "desk"));
    }
    let view = desk.dev.view();
    // 写口那一页：**余量**——本族一条消息远小于它（与 `rtc` 备缓冲同一手）。
    let mut word: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    if word.try_reserve_exact(PAGE_SIZE).is_err() {
        return Err(Fail::at(E_UART, "desk"));
    }
    word.resize(PAGE_SIZE, 0);
    let mut raw = [0u8; DRAIN_MAX];
    // **待交的那几批**：字节住在**自有的** `Vec` 里（递给孔之后不必当场等它被取走）。
    // **有界**：满了就不再往下排——设备 FIFO 里的字节不会丢（这一圈不再 `receive`/排空，
    // 线那一侧因此不会接着报，等这一列腾出空来再收）。
    let mut pending: alloc::vec::Vec<alloc::vec::Vec<u8>> = alloc::vec::Vec::new();
    // 头一批**已经递出去**了没有：递出去之后只等"被取走"，不再递（递两次就是两条）。
    let mut handed = false;

    loop {
        // 0：**先把没交完的那一批再走一趟**（两半各自**有期**）。
        //
        // **为什么不能像从前那样在这里死等**：本驱动与"人"（`canonical`）是两个方向的
        // "一次一格 ＋ 无期限"握手，而两边"收"那一手在各自圈里都排在"等"那一半**之后**：
        //   本驱动的一圈：等组 → **收写口** → 收线 → 排空 → **等本批被取走**；
        //   人的一圈：**收读口** → 行规程 → **写回显（等本驱动收）**。
        // 两边各压着一只手、各等对方先收 ⇒ 环闭上，谁也动不了（量到的原文：`console: publish
        // wait begin … seq=1` 之后**没有** `done`）。
        // 现在这里"等不到就往下走"——**走到圈首去收写口**，那正是对方在等的那件事 ⇒ 环打开。
        if let Some(b) = pending.first() {
            if !handed {
                handed = desk.ctx.offer(b);
            }
            if handed {
                match desk.ctx.taken(Wait::AtMost(TAKEN_MS)) {
                    Some(true) => {
                        pending.remove(0);
                        handed = false;
                    }
                    Some(false) => {}
                    // 那一格没了（人退场）：这一批再也送不到，就地收掉（下一批接着走）。
                    None => {
                        pending.remove(0);
                        handed = false;
                    }
                }
            }
        }
        // **（临时读数）谁把我叫醒的**：这一格是"漏唤醒（该叫没叫）"与"没人叫（这一格根本
        // 没人投递）"的分界。前 20 次打全。
        match pile.await_(Wait::Forever) {
            Ok(woke) => {
                static N: ::core::sync::atomic::AtomicUsize =
                    ::core::sync::atomic::AtomicUsize::new(0);
                if N.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) < 20 {
                    protocol::debug::put(&alloc::format!(
                        "uart: woke {:?}",
                        woke.map(|(tok, dir)| (tok.get(), dir))
                    ));
                }
            }
            Err(_) => return Err(Fail::at(E_UART, "line gone")),
        }
        // FIFO 里排着，多等这一瞬不丢。
        let mut wrote = 0usize;
        while let Ok((len, _)) = desk.tx.pull(&mut word, Wait::POLL) {
            device::put(view, &word[..len]);
            wrote += 1;
        }
        // **（临时读数）我往设备里写了几条**：`wrote>0` = 人（`canonical`）推来的字到了设备
        // （回显/USAGE 都在这一条路上）——"人推了、设备没吐出来"与"人根本没推"由此分开。
        if wrote > 0 {
            static N: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);
            if N.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) < 20 {
                protocol::debug::put(&alloc::format!("uart: tx took={wrote}"));
            }
        }
        // **（临时读数）每一圈看见了几条**：每 500 圈打一行（封顶 20 行）。
        // `woke` 那一行只说"组把我叫醒了"，这一行说"我叫醒之后到底取到了什么"——
        // 两者合起来才能把"没叫"与"叫了却没东西"分开。
        {
            static PASS: ::core::sync::atomic::AtomicUsize =
                ::core::sync::atomic::AtomicUsize::new(0);
            let n = PASS.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) + 1;
            if n % 500 == 0 {
                static N: ::core::sync::atomic::AtomicUsize =
                    ::core::sync::atomic::AtomicUsize::new(0);
                if N.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) < 20 {
                    protocol::debug::put(&alloc::format!("uart: pass={n} tx={wrote}"));
                }
            }
        }
        while pending.len() < PENDING_MAX && desk.line.receive(Wait::POLL).is_ok() {
            let n = device::drain(view, &mut raw);
            // 人（`canonical`）总会回到"取一行"那一格，故等它是有界的。
            if let Some(batch) = Batch::of(&raw, n) {
                // **这一批进"待交"那一列**：字节**抄进自有的那一格**（`Vec`），于是递出去之后
                // **不必**当场等它被取走——圈首那一手（见上）会一趟一趟来等。这就是本驱动
                // **不再有无限期的等**的地方（0 字节不排：`Batch::of` 那一句已把空批挡掉）。
                pending.push(batch.bytes().to_vec());
                static N: ::core::sync::atomic::AtomicUsize =
                    ::core::sync::atomic::AtomicUsize::new(0);
                if N.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) < 20 {
                    protocol::debug::put(&alloc::format!(
                        "uart: pending q={} n={n}",
                        pending.len()
                    ));
                }
            }
            // 排空的**通知**照旧发：0 字节也算"这一条我处理完了"——那一格回闲 + 把线放回去。
            desk.line.exhaust().unwrap();
        }
    }
}
