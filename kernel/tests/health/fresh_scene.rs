//! Fresh 失败回退那一景的内核夹具：在**真移交**的在途窗口里放开读者、注入一次失败的
//! 撤页确认，再原样交回。
//!
//! 占槽之后、撤页之前，core 锁已经放开而 `caller.pend` 仍持着：这时按握手页里的请求
//! 放开那一颗读者、等它真登记在目标关系上，最后装一次“下一次撤页确认失败”就立即返回。
//! 这一刻之后不再走表或访问握手页，避免提前消耗这一次失败。
//!
//! 不是这一景时（握手页没标记）静默返回：同一个入口是所有景唯一的移交路径，
//! 这个夹具不许把它弄红。等待位的那对坐标也按**真正压着源页**那段关系取：
//! ASID 是它的，关系身份是控制者报过来的那一条。

use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;

use crate::core::memory::address::{PageIndex, VirtAddr};
use crate::core::memory::flat::FlatRange;
use crate::core::memory::nest::identity::fail_next_shootdown;
use crate::core::memory::nest::source_views;
use crate::core::memory::transfer::arm_in_flight;
use crate::core::work::room::conductor::current;
use crate::core::work::room::messenger::{self, WakeKey};
use crate::machine::clock;

#[allow(dead_code)]
#[path = "../../../programs/src/harness/probe/memory_transfer_fresh/handshake.rs"]
mod handshake;

/// 等读者登记的时限（毫秒）：到点就不注入，把结论写回握手页交回现场。
const AWAIT_MS: u64 = 2_000;

/// 装起本景那一个在途探针：由 `embedded` 那一例在 `boot::start` 之前显式调一次。
pub fn arm() {
    assert!(
        crate::testing(),
        "the fresh fixture arms only in a test run"
    );
    arm_in_flight(Some(probe));
}

/// 在途窗口那一步：把这一次的请求做完就返回。
fn probe(source: &FlatRange, source_page: PageIndex) {
    let Some(task) = current().running_task() else {
        return;
    };
    let space = task.ident.team.space.clone();
    let Some((page, _)) = space.nest.translate(VirtAddr::wrap(handshake::VA)) else {
        return;
    };
    // SAFETY: 本景在独立普通 RAM 页中建立这些对齐原子字；控制者直到本次
    // Transfer 返回后才可关闭该 View，且握手页不参与迁移。没有借用在途源帧。
    let words = unsafe { &*(page.as_usize() as *const [AtomicU64; handshake::WORDS]) };
    if word(words, handshake::off::TAG) != handshake::TAG {
        return;
    }
    let mode = word(words, handshake::off::MODE);
    if mode != handshake::mode::AWAIT && mode != handshake::mode::LATE {
        return;
    }
    if word(words, handshake::off::STATE) != handshake::state::PREPARED {
        return;
    }
    let view = word(words, handshake::off::VIEW) as usize;
    let asid = {
        let pressings = source_views::pressings_over(source, source_page)
            .expect("fresh fixture: enumerate the relations pressing the source page");
        assert_eq!(
            pressings.len(),
            1,
            "fresh fixture: the source page presses exactly one relation"
        );
        let pressing = &pressings[0];
        assert_ne!(
            pressing.view.identity(),
            view,
            "fresh fixture: source and target are different relations"
        );
        let nest = pressing
            .nest
            .as_ref()
            .expect("fresh fixture: the pressing's nest is live");
        nest.asid()
    };
    let key = WakeKey::View { space: asid, view };
    if mode == handshake::mode::AWAIT {
        // 放开读者，再等**真实登记**：`queued` 只看这条等待位的队列长。
        put(words, handshake::off::START, handshake::RELEASE);
        let until =
            clock::now().as_ticks() + clock::duration_to_ticks(Duration::from_millis(AWAIT_MS));
        while messenger::queued(key) == 0 {
            if clock::now().as_ticks() >= until {
                put(words, handshake::off::STATE, handshake::state::TIMEOUT);
                return;
            }
            core::hint::spin_loop();
        }
    } else {
        assert_eq!(
            messenger::queued(key),
            0,
            "fresh fixture: the late phase has no waiter yet"
        );
    }
    put(words, handshake::off::STATE, handshake::state::ARMED);
    fail_next_shootdown(asid);
}

/// 握手页里第 `at` 个字节处那一个字。
fn word(words: &[AtomicU64; handshake::WORDS], at: usize) -> u64 {
    words[at / size_of::<u64>()].load(Ordering::Acquire)
}

fn put(words: &[AtomicU64; handshake::WORDS], at: usize, value: u64) {
    words[at / size_of::<u64>()].store(value, Ordering::Release);
}
