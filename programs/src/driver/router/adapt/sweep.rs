//! alive 答不出的那几条线——拆线 + 空出格子。
//! 判定在 crate::core::lines（`vacate` 那一手，连它的两个后果）；探活是内核的一问
//! （mail::reserve），拆线是设备面的一手（`plic.unwire`）。
//! 时机是**每一次醒**（组那一次等待回来就扫一遍）：主人一没，它铸的那一枚孔就封印，而那一格
//! 故"收线"不靠板、也不靠一拍。
//! **这一跳有读数了**：`programs/src/harness/guest/lodger/main.rs`（房客）每次冷启动都占住 1 号线、然后一句话不说就走
//! :seal_owned → messenger::wipe → 组键

use crate::core::lines::Lines;
use crate::dev::plic::Plic;
use env::HoleDir;
use protocol::communication::session::establish::Endpoint;
use runtime::core::res::pile::Pile;
use runtime::env::mail::{self, HolePie};

/// 逐客：主人没了的那几条——拆线 + 空出格子
/// 两个后果缺一不可：不 `unwire` 则线还在本 context 里（电平挂着 ⇒ 白报），不 `detach` 则
/// 那一格永远留在组里（对端没了 ⇒ 每次都当场就绪）
pub fn run(lines: &mut Lines, plic: &Plic, pile: &Pile) {
    let held: alloc::vec::Vec<u32> = lines.held().collect();
    for line in held {
        let Some(lane) = lines.lane(line) else {
            continue;
        };
        if let Some(code) = alive(lane) {
            plic.unwire(line);
            let detached = pile
                .detach(&HolePie::from_token(lane.rx()), HoleDir::Pull)
                .is_ok();
            let _ = lines.vacate(line);
            // **（临时读数）真读数**（上面那一句 `debug!` 在 release 里是空的）：拆了哪一条、
            // 探活那一问**答的是什么码**（`-1` = 表里没这枚/不是孔，`-2` = 资源已封印）、
            // 以及"那一格从组里摘掉了没有"——`detach` 一成就把**转发**一起摘掉（见 `tole::detach`），
            // 此后客人往那一格上说什么，本端的组都听不见了。前 20 次打全。
            static N: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);
            if N.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) < 20 {
                protocol::debug::put(&alloc::format!(
                    "router: vacate line={line} reserve={code} detach={detached}"
                ));
            }
        }
    }
}

/// 客人还答得出来吗：**问它铸的那一枚**（mail::reserve 走存活闸：封印之后答不出）
/// 返 `None` = 还答得出来；`Some(码)` = 答不出（`码` 就是那一问的答复，见上）。
fn alive(lane: &Endpoint) -> Option<i32> {
    match lane.tx() {
        Some(at_peer) => mail::reserve(at_peer).err().map(|e| e.source as i32),
        None => Some(0),
    }
}
