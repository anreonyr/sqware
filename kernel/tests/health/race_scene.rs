//! 真实多 Hart 交错那一景的内核夹具：在**真移交**的在途窗口里放开对端，等它
//! 真登记在**别人空间**那条关系上，再原样交回。
//!
//! 占槽之后、撤页之前，core 锁已经放开而 `caller.pend` 仍持着、`SIE` 是关的：
//! 这一段里这颗 hart 出不去，所以“对端登记了”只可能是**另一颗 hart** 上发生的事。
//! 夹具只做三件事实：核出压着源页的两条关系（自己那条 ＋ 别人空间那条）、
//! 放开对端、等那条关系上真的排上了一个等待者（`messenger::queued`，有界自旋）。
//! 它不睡固定时长、不替对端举牌子、不注入任何失败，也不改任何生产路径。
//!
//! 不是这一景时（共享控制页没标记）静默返回：同一个入口是所有景唯一的移交路径，
//! 这个夹具不许把它弄红。

use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;

use crate::core::memory::address::{PageIndex, VirtAddr};
use crate::core::memory::flat::FlatRange;
use crate::core::memory::nest::source_views;
use crate::core::memory::transfer::arm_in_flight;
use crate::core::work::room::conductor::current;
use crate::core::work::room::messenger::{self, WakeKey};
use crate::machine::clock;

#[allow(dead_code)]
#[path = "../../../programs/src/harness/probe/memory_transfer_race/handshake.rs"]
mod handshake;

/// 等对端登记的时限（毫秒）：到点就把结论写成 `TIMEOUT` 交回现场，不冒充登记过。
///
/// 它也把 `QEMU_SMP=1` 那一档变成一次**有理由的**失败：只有一颗 hart 时，
/// 管理端驻留在窗口里，对端根本没机会跑，这一格必然到点。
const AWAIT_MS: u64 = 2_000;

/// 装起本景那一个在途探针：由 `embedded` 那一例在 `boot::start` 之前显式调一次。
pub fn arm() {
    assert!(crate::testing(), "the race fixture arms only in a test run");
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
    // SAFETY: 本景把共享控制页挂在自己空间里，两侧只按同一份线格式读写那几个对齐字；
    // 这一页不参与迁移，且控制端直到本次 Transfer 返回之后才可关它。
    let words = unsafe { &*(page.as_usize() as *const [AtomicU64; handshake::WORDS]) };
    if word(words, handshake::off::TAG) != handshake::TAG {
        return;
    }
    if word(words, handshake::off::MODE) != handshake::mode::FAULT {
        return;
    }
    if word(words, handshake::off::STATE) != handshake::state::PREPARED {
        return;
    }
    // 对端必须先真读过第 0 页（那一手会真物化、真发布一条 PTE），否则“这条关系
    // 压着源页”只是范围上的事，这次移交也没真从别人空间里撕下什么。
    assert!(
        word(words, handshake::off::READY) != handshake::NONE
            && word(words, handshake::off::READY) != 0,
        "race fixture: the peer never reported its page-0 read"
    );
    let pressings = source_views::pressings_over(source, source_page)
        .expect("race fixture: enumerate the relations pressing the source page");
    assert_eq!(
        pressings.len(),
        2,
        "race fixture: the source page presses exactly two relations"
    );
    let mine = space.nest.asid();
    let mut foreign = None;
    for pressing in &pressings {
        let nest = pressing
            .nest
            .as_ref()
            .expect("race fixture: the pressing's nest is live")
            .clone();
        if nest.asid() != mine {
            foreign = Some((nest, pressing.view.identity()));
        }
    }
    let Some((nest, view)) = foreign else {
        panic!("race fixture: no relation in another space presses the source page")
    };
    let asid = nest.asid();
    // 那条关系在源页上真握着一条翻译：本景要证的“移交从别人空间里撕下一格”
    // 才不是范围上的空话。对端已经读过那一页（上面核过），故这一格必定成立；
    // 这里当场把它读出来，就是那一手真实发生过的现场。
    let va0 = word(words, handshake::off::VA0) as usize;
    let held = nest.translate(VirtAddr::wrap(va0)).is_some();
    put(words, handshake::off::FOREIGN, held as u64);
    // 放开对端，再等**真实登记**：`queued` 只看这条关系那一个等待位的队列长。
    put(words, handshake::off::START, handshake::RELEASE);
    let until = clock::now().as_ticks() + clock::duration_to_ticks(Duration::from_millis(AWAIT_MS));
    while messenger::queued(WakeKey::View { space: asid, view }) == 0 {
        if clock::now().as_ticks() >= until {
            put(words, handshake::off::STATE, handshake::state::TIMEOUT);
            return;
        }
        core::hint::spin_loop();
    }
    put(words, handshake::off::STATE, handshake::state::ARMED);
}

/// 共享控制页里第 `at` 个字节处那一个字。
fn word(words: &[AtomicU64; handshake::WORDS], at: usize) -> u64 {
    words[at / size_of::<u64>()].load(Ordering::Acquire)
}

fn put(words: &[AtomicU64; handshake::WORDS], at: usize, value: u64) {
    words[at / size_of::<u64>()].store(value, Ordering::Release);
}
