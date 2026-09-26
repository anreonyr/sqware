//! router::adapt::sweep — **逐客（适配）**：`alive` 答不出的那几条线——**拆线 + 空出格子**。
//!
//! 判定在 `contract::driver::line::core`（`vacate` 那一手，连它的两个后果）；探活是内核的一问
//! （`mail::reserve`），拆线是设备面的一手（`plic.unwire`）。
//!
//! 时机是**每一次醒**（组那一次等待回来就扫一遍）：主人一没，它铸的那一枚孔就封印，而那一格
//! 正挂在本域这只组上（`seal` 走 `wipe` 敲到组键）⇒ 那一次敲键就是把本域叫起来的那一件事。
//! 故"收线"不靠板、也不靠一拍。
//!
//! **这一跳有读数了**：`harness/src/lodger`（房客）每次冷启动都占住 1 号线、然后一句话不说就走
//! ⇒ 本域被叫醒、`alive` 答不出 ⇒ `router: vacate line=1`。链条本身是
//! `cull::seal_owned` → `messenger::wipe` → 组键。

use crate::plic::Plic;
use env::HoleDir;
use protocol::debug;
use protocol::driver::line::core::Lines;
use protocol::session::Pier;
use runtime::core::pile::Pile;
use runtime::env::mail::{self, HolePie};

/// 逐客：主人没了的那几条——拆线 + 空出格子。
///
/// 两个后果缺一不可：不 `unwire` 则线还在本 context 里（电平挂着 ⇒ 白报），不 `detach` 则
/// 那一格永远留在组里（对端没了 ⇒ 每次都当场就绪）。
pub fn run(lines: &mut Lines, plic: &Plic, pile: &Pile) {
    let held: alloc::vec::Vec<u32> = lines.held().collect();
    for line in held {
        let Some(lane) = lines.lane(line) else {
            continue;
        };
        if alive(&lane) {
            continue;
        }
        plic.unwire(line);
        let _ = pile.detach(&HolePie::from_token(lane.hole()), HoleDir::Pull);
        let _ = lines.vacate(line);
        debug!("router: vacate line={line}");
    }
}

/// 客人还答得出来吗：**问它铸的那一枚**（`mail::reserve` 走存活闸：封印之后答不出）。
///
/// 问的是对端的写端（`at_peer`）而不是本端读的那一枚：本端那一枚的活命随本域，问它恒活。
fn alive(lane: &Pier) -> bool {
    match lane.at_peer() {
        Some(at_peer) => mail::reserve(at_peer).is_ok(),
        None => false,
    }
}
