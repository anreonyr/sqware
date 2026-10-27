//! 真实多 Hart 交错那一景的内核夹具：在**真移交**的在途窗口里放开对端，等它
//! 真登记在**别人空间**那条关系上，再原样交回；第二段在同一个窗口里与对端
//! 真拆图（`Cleanup::finish`）重叠。
//!
//! 占槽之后、撤页之前，core 锁已经放开而 `caller.pend` 仍持着、`SIE` 是关的：
//! 这一段里这颗 hart 出不去，所以“对端登记了”只可能是**另一颗 hart** 上发生的事。
//! 夹具只做三件事实：核出压着源页的两条关系（自己那条 ＋ 别人空间那条）、
//! 放开对端、等那条关系上真的排上了一个等待者（`messenger::queued`，有界自旋）。
//! 它不睡固定时长、不替对端举牌子、不注入任何失败，也不改任何生产路径；
//!
//! 第二段（`mode=teardown`）走另一条入口：对端在**另一颗 hart** 上真释放它那枚
//! 来源子分支，`Cleanup::finish` 在它那一边的 `pause` 停住（清理已取得、所有 pend
//! 与锁都已放开），而本端在这颗 hart 的**在途窗口**里当场把“放行之前两条关系、
//! 别人空间那枚翻译还在”与“放行之后只剩一条、那枚翻译已经没了”读出来——两面
//! 都在内核侧读真对象，不是程序举的牌子。
//!
//! 不是这一景时（共享控制页没标记）静默返回：同一个入口是所有景唯一的移交路径，
//! 这个夹具不许把它弄红。

use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;

use crate::core::memory::address::{PageIndex, VirtAddr};
use crate::core::memory::flat::FlatRange;
use crate::core::memory::nest::source_views;
use crate::core::memory::transfer::arm_in_flight;
use crate::core::work::pend::Pie;
use crate::core::work::room::conductor::current;
use crate::core::work::room::messenger::{self, WakeKey};
use crate::core::work::unit::space::Space;
use crate::machine::clock;

use alloc::sync::Arc;

use kernel_test_api::memory_transfer_race as handshake;

/// 等对端登记的时限（毫秒）：到点就把结论写成 `TIMEOUT` 交回现场，不冒充登记过。
///
/// 它也把 `QEMU_SMP=1` 那一档变成一次**有理由的**失败：只有一颗 hart 时，
/// 管理端驻留在窗口里，对端根本没机会跑，这一格必然到点。
const AWAIT_MS: u64 = 2_000;

/// 装起本景那两个探针：由 `embedded` 那一例在 `boot::start` 之前显式调一次。
pub fn arm() {
    assert!(crate::testing(), "the race fixture arms only in a test run");
    arm_in_flight(Some(probe));
    crate::core::work::pend::arm_finish_pause(Some(pause));
}

/// 拆图入口那一步（`mode=teardown`）：只认那一枚被交回的**来源子分支**。
///
/// 匹配依据是可核的：`Cleanup` 里恰好一条、是 Pole 上的子分支（`sire` 在）、
/// **它的号就是控制页里对端报回来的那一枚**；调用者就是控制页里那一位对端、
/// 当前模式是第二段、且一次性请求还在。夹具读走请求（`PAUSE` 归零），
/// 故之后的拆图（比如退出那一条）不再停；号对不上的时候也**不**动那个请求。
fn pause(removed: &[Pie]) {
    if removed.len() != 1
        || removed[0].sire().is_none()
        || removed[0].mail().kind() != env::PieKind::Pole
    {
        return;
    }
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
    if word(words, handshake::off::MODE) != handshake::mode::TEARDOWN {
        return;
    }
    if word(words, handshake::off::PEER) != task.ident.id.get() as u64 {
        return;
    }
    if removed[0].token().get() as u64 != word(words, handshake::off::SOURCE_TOKEN) {
        return;
    }
    if take(words, handshake::off::PAUSE) != 1 {
        return;
    }
    put(words, handshake::off::STATE, handshake::state::PAUSED);
    let until = clock::now().as_ticks() + clock::duration_to_ticks(Duration::from_millis(AWAIT_MS));
    while word(words, handshake::off::GO) != handshake::RELEASE {
        if clock::now().as_ticks() >= until {
            put(words, handshake::off::STATE, handshake::state::TIMEOUT);
            return;
        }
        core::hint::spin_loop();
    }
    put(words, handshake::off::RESUMED, 1);
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
    // SAFETY: 同 `pause`。
    let words = unsafe { &*(page.as_usize() as *const [AtomicU64; handshake::WORDS]) };
    if word(words, handshake::off::TAG) != handshake::TAG {
        return;
    }
    match word(words, handshake::off::MODE) {
        handshake::mode::FAULT => fault_phase(words, &space, source, source_page),
        handshake::mode::TEARDOWN => teardown_phase(words, &space, source, source_page),
        _ => {}
    }
}

/// 第一段：放开对端、等它真登记在**别人空间**那条关系上。
fn fault_phase(
    words: &[AtomicU64; handshake::WORDS],
    space: &Arc<Space>,
    source: &FlatRange,
    source_page: PageIndex,
) {
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

/// 第二段：对端的拆图已经停在 `finish` 入口，在本窗口里放行并核它落地。
fn teardown_phase(
    words: &[AtomicU64; handshake::WORDS],
    space: &Arc<Space>,
    source: &FlatRange,
    source_page: PageIndex,
) {
    if word(words, handshake::off::STATE) != handshake::state::PAUSED {
        return;
    }
    let va = word(words, handshake::off::VA1) as usize;
    // 快照只留一件事实：那两条关系里哪一条是**别人空间**的。随后立刻放掉快照：
    // 本夹具自己强持着那条关系的话，它退场时就摘不掉登记链上那一条，
    // 于是同一个窗口里永远数出两条——那是夹具把现场钉住了，不是对端没拆。
    let (before, foreign) = {
        let pressings = source_views::pressings_over(source, source_page)
            .expect("race fixture: enumerate the relations pressing the source page");
        let mine = space.nest.asid();
        let foreign = pressings.iter().find_map(|pressing| {
            pressing
                .nest
                .as_ref()
                .filter(|nest| nest.asid() != mine)
                .cloned()
        });
        (pressings.len(), foreign)
    };
    let held = foreign
        .as_ref()
        .is_some_and(|nest| nest.translate(VirtAddr::wrap(va)).is_some());
    put(words, handshake::off::PRESS_BEFORE, before as u64);
    put(words, handshake::off::PEER_BEFORE, held as u64);
    let Some(nest) = foreign else {
        put(words, handshake::off::GO, handshake::RELEASE);
        put(words, handshake::off::STATE, handshake::state::TIMEOUT);
        return;
    };
    if before != 2 || !held {
        // 窗口里没看到“清理已取得、还没落地”的现场：放掉对端，按超时交回现场。
        put(words, handshake::off::GO, handshake::RELEASE);
        put(words, handshake::off::STATE, handshake::state::TIMEOUT);
        return;
    }
    // 放行那一次拆图，先等对端那一次 `Release` **真的返回**（它会把结果码写回控制页）：
    // 这一段空转只读一个原子字，不取锁也不分配，故不会把对端的拆图饿住。
    put(words, handshake::off::GO, handshake::RELEASE);
    let until = clock::now().as_ticks() + clock::duration_to_ticks(Duration::from_millis(AWAIT_MS));
    while word(words, handshake::off::RELEASED) == handshake::NONE {
        if clock::now().as_ticks() >= until {
            put(words, handshake::off::STATE, handshake::state::TIMEOUT);
            return;
        }
        core::hint::spin_loop();
    }
    // 落地判据（全是内核侧读真对象）：压着源页的关系只剩一条，别人空间那个 VA
    // 上已经没有翻译了。
    let left =
        source_views::pressings_over(source, source_page).map_or(usize::MAX, |left| left.len());
    let gone = nest.translate(VirtAddr::wrap(va)).is_none();
    put(words, handshake::off::PRESS_AFTER, left as u64);
    // 两根读数的口径一致：`PEER_*` 报“那个 VA 上还有没有翻译”。
    put(words, handshake::off::PEER_AFTER, u64::from(!gone));
    let state = if left == 1 && gone {
        handshake::state::ARMED
    } else {
        handshake::state::TIMEOUT
    };
    put(words, handshake::off::STATE, state);
}

/// 共享控制页里第 `at` 个字节处那一个字。
fn word(words: &[AtomicU64; handshake::WORDS], at: usize) -> u64 {
    words[at / size_of::<u64>()].load(Ordering::Acquire)
}

fn put(words: &[AtomicU64; handshake::WORDS], at: usize, value: u64) {
    words[at / size_of::<u64>()].store(value, Ordering::Release);
}

/// 读走一格（一次性请求）：返回它之前那个值。
fn take(words: &[AtomicU64; handshake::WORDS], at: usize) -> u64 {
    words[at / size_of::<u64>()].swap(0, Ordering::AcqRel)
}
