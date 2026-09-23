//! 会话核心的门（**宿主台**）—— 码头 / 泊位 / 认领那台机器，在宿主上真跑一遍。
//!
//! # 这一台钉的是什么
//!
//! `crates/protocol/src/session/mod.rs` 那 **九条实测事实**里，能机械检查的那几条在这里成了
//! 判据。规格写在靶子里（照 `crates/line-case` 的做法），核心那份源码**一个字不动**：
//!
//! ```text
//!   seat     同名闸（同一位、同一记号只可能有一枚）；记号 = 这条泊位名字的指纹
//!   claim    两格判据（谁开的 + 刻的什么）；一枚孔只配一条泊位（已经用掉的不再认）
//!   claim    三格失败域：额度为零 ⇒ Partial、什么都没到 ⇒ Timeout、到了一些不齐 ⇒ Partial
//!   unseat   过线那一句话 + 放下本端那一枚；拆不存在的泊位不是错误
//!   shut     把本端持有的都放下，**不发任何一句话**（对端那一侧的通知归 unseat）
//!   ready    每一条路的两头都齐了才算通
//!   Pier     post / pull 成对：没有写端就发不出去（不猜、不空转）
//! ```
//!
//! # 桩在哪
//!
//! `tests/session/call.rs`（运行时那一层：假表 + 假钟）。**桩量不了内核，量得了账**——
//! 而那九条事实说的正是账的规矩。

extern crate alloc;

mod session;

use env::Mark;
use session::call as fake;
use session::core::{Claim, Pier, Quay, Seat};

use fake::{ME, name};

/// 另一个域（对端）。
const PEER: env::TaskId = env::TaskId::new(7);

fn quay() -> Quay {
    fake::reset();
    Quay::open(PEER)
}

#[test]
fn seat_refuses_a_duplicate_name() {
    // **同名闸**（`session` 事实 5）：一个泊位名只装得下一条路——"同一位、同一记号只可能有一枚"
    // 靠的就是它，归位那一侧的次序因此不再承担语义。
    let mut q = quay();
    assert!(q.seat(name("records")).is_ok());
    assert_eq!(q.seat(name("records")).err(), Some(Seat::NoName), "同名的不许再装");
    // **拒了这一手不许留痕**：闸在"铸之前"——照实记：这一句是牙口量出来的。把 `seat` 开头
    // 那道闸挪走之后，同名那一问**照样答 `NoName`**（`install` 里还有一道），但它已经先
    // 铸了一枚、交出去一枚、然后把它漏在那儿 ⇒ 只查答码的那一版全门照绿，查副作用才逮得住。
    assert_eq!(fake::shipped().len(), 1, "被拒的那一次不该交孔");
    assert_eq!(fake::unshipped().len(), 0, "也不该有放下（它什么都没铸）");
    assert_eq!(
        q.seat(env::Name::EMPTY).err(),
        Some(Seat::NoName),
        "空名字非法"
    );
    // 换一个名字照样装得上（闸认的是"这一条路已经在了"）。
    assert!(q.seat(name("control")).is_ok());
}

#[test]
fn seat_ships_my_hole_with_the_name_as_its_mark() {
    // **记号 = 这条泊位名字的指纹**：孔是铸在**本端**这张表里的，`mark` 刻的是这个名字；
    // 而**装上那一刻还不算配齐**（写端要等对端把它那一枚交进来）。
    let mut q = quay();
    let pier = *q.seat(name("records")).expect("装得上");
    assert_eq!(pier.name(), name("records"));
    assert!(!pier.paired(), "对端那一枚还没到 ⇒ 没配齐");
    assert_eq!(pier.at_peer(), None);

    let shipped = fake::shipped();
    assert_eq!(shipped.len(), 1, "交出去的正好一枚");
    assert_eq!(shipped[0].1, PEER, "交给对端");
    assert_eq!(shipped[0].0, pier.hole(), "交的是本端这一枚");
    // 那枚孔上刻的记号 = 名字的指纹（`reserve` 那两格里读得出来）。
    let (owner, mark) = fake::reserve(pier.hole());
    assert_eq!(owner, Some(ME), "谁开的 = 本端");
    assert_eq!(mark, Mark::of("records"), "刻的是这条路的记号");
}

#[test]
fn claim_without_any_berth_is_partial_not_a_wait() {
    // **一条泊位都没装上 ⇒ `Partial`**：对方无从知道该给我几条，这不是"白等"（`Claim::Partial`
    // 的头注写着这一格）。故它**一次都不该去等**（假钟不许被拨动）。
    let mut q = quay();
    assert_eq!(q.claim(ME, Mark::of("records"), 5), Err(Claim::Partial));
    assert_eq!(fake::now_ns(), 0, "额度为零时不该进等待");
}

#[test]
fn claim_takes_only_holes_whose_both_cells_match() {
    // **两格判据都读内核盖的戳**：`owner`（谁开的）+ `mark`（刻的什么）。错主人、错记号都不认。
    let mut q = quay();
    let pier = *q.seat(name("records")).expect("装得上");

    // 主人对、**记号对不上** ⇒ 不认。
    fake::put(11, PEER, Mark::of("other"));
    assert_eq!(q.claim(PEER, Mark::of("records"), 0), Err(Claim::Timeout));
    assert!(!q.find(name("records")).expect("还在").paired());

    // 记号对、**主人不对** ⇒ 也不认（`owner` 那一格认的是"谁开的这扇门"）。
    fake::put(12, env::TaskId::new(9), Mark::of("records"));
    assert_eq!(q.claim(PEER, Mark::of("records"), 0), Err(Claim::Timeout));
    assert!(!q.find(name("records")).expect("还在").paired());

    // 两格都对 ⇒ 认下，且归到那条路上（不是归到别的路）。
    fake::put(13, PEER, Mark::of("records"));
    assert_eq!(q.claim(PEER, Mark::of("records"), 0), Ok(()));
    let got = q.find(name("records")).expect("还在");
    assert!(got.paired(), "认下了");
    assert_eq!(
        got.at_peer(),
        Some(fake::tok(13)),
        "认的是**那一枚**（在它原来那一格上）"
    );
    assert_eq!(got.hole(), pier.hole(), "本端那一枚没动");
}

#[test]
fn one_hole_is_paired_to_one_berth_only() {
    // **一枚孔只配一条泊位**（事实 5 的后半）：一座码头两条泊位时，第二次认领**不能**把第一条
    // 泊位的写端再配给下一条。两条路各来一枚孔 ⇒ 各归各。
    //
    // 照实记：这一格正是"板那条路长在 `records` 旁边"时必现的那一个 bug——
    // 不排除"已经用掉的那几枚"，第二条会拿到第一条的写端。
    let mut q = quay();
    let a = *q.seat(name("records")).expect("装得上");
    let b = *q.seat(name("control")).expect("装得上");
    let (ha, hb) = (a.hole(), b.hole());
    assert_ne!(ha, hb, "本端每一枚孔都不一样");

    fake::put(21, PEER, Mark::of("control"));
    fake::put(22, PEER, Mark::of("records"));

    assert_eq!(q.claim(PEER, Mark::of("records"), 0), Ok(()));
    assert_eq!(q.claim(PEER, Mark::of("control"), 0), Ok(()));
    let (ra, rb) = (
        q.find(name("records")).expect("在"),
        q.find(name("control")).expect("在"),
    );
    assert_eq!(ra.at_peer(), Some(fake::tok(22)), "records 那一枚");
    assert_eq!(rb.at_peer(), Some(fake::tok(21)), "control 那一枚");
    assert_ne!(ra.at_peer(), rb.at_peer(), "**不许两条路共用一枚写端**");
    assert!(q.ready(), "两条都齐了");
}

#[test]
fn a_hole_already_taken_is_not_paired_again() {
    // **"已经用掉的那几枚不再认"**（事实 5 的后半，照实记里那句"第二次认领会把第一条泊位的
    // 写端再配给下一条"）：同一条记号**重试一次**（`claim` 答 `Partial` 之后接着等的那种重试），
    // 那一枚孔**不许**再配到第二条路上——否则两条路的 `at_peer` 是同一枚，话全推进同一个槽。
    let mut q = quay();
    q.seat(name("records")).expect("装得上");
    q.seat(name("control")).expect("装得上");
    fake::put(23, PEER, Mark::of("records"));

    assert_eq!(q.claim(PEER, Mark::of("records"), 0), Ok(()));
    // 重试（记号与第一次一样）：那一枚已经在第一条路上了。
    assert_eq!(q.claim(PEER, Mark::of("records"), 0), Ok(()));
    let b = q.find(name("control")).expect("在");
    assert!(!b.paired(), "第二条路**不该**分到同一枚孔");
    assert_eq!(b.at_peer(), None);
    assert!(!q.ready(), "第二条路还没通");
}

#[test]
fn claim_says_nothing_arrived_when_the_clock_runs_out() {
    // **期限到、一笔都没到 ⇒ `Timeout`**（"白等"那一格，与 `Partial` 的下一步不同）。
    let mut q = quay();
    q.seat(name("records")).expect("装得上");
    assert_eq!(q.claim(PEER, Mark::of("records"), 5), Err(Claim::Timeout));
    assert!(fake::now_ns() > 0, "它真的等过（假钟被拨过）");
}

#[test]
fn claim_says_partial_when_something_arrived_but_that_one_is_gone() {
    // **`Partial`**：到了一些、不齐——已配齐的那条路背后那一枚**已经不在我表里了**
    // （`reserve` 答 `(None, _)`），而 `(of, mark)` 那一枚始终没到。
    let mut q = quay();
    q.seat(name("records")).expect("装得上");
    fake::put(31, PEER, Mark::of("records"));
    assert_eq!(q.claim(PEER, Mark::of("records"), 0), Ok(()));
    // 那一枚走了（对端退场会把它的副本一起带走）。
    fake::forget(31);
    assert_eq!(q.claim(PEER, Mark::of("records"), 0), Err(Claim::Partial));
}

#[test]
fn unseat_says_the_word_and_drops_my_hole() {
    // **拆下要告诉对方**（否则它会一直往没人收的通道里推）：过线的是那一句 `UNSEAT`，
    // 同时放下本端那一枚。**拆不存在的泊位不是错误**（要的结果已经成立）。
    let mut q = quay();
    q.seat(name("records")).expect("装得上");
    fake::put(41, PEER, Mark::of("records"));
    assert_eq!(q.claim(PEER, Mark::of("records"), 0), Ok(()));
    let hole = q.find(name("records")).expect("在").hole();

    q.unseat(name("records"));
    assert_eq!(fake::said(), vec![(fake::tok(41), fake::UNSEAT.to_vec())], "往对端那一枚说了一句");
    assert_eq!(fake::unshipped(), vec![hole], "放下的是本端那一枚");
    assert!(q.find(name("records")).is_none(), "这条泊位不在了");

    // 撞空：不 panic、也不发话。
    let said = fake::said().len();
    q.unseat(name("records"));
    q.unseat(name("never-seated"));
    assert_eq!(fake::said().len(), said, "拆不存在的泊位不该说话");
}

#[test]
fn shut_drops_everything_and_says_nothing() {
    // **打烊**：把本端持有的一切放下（半途失败也用它），而**不发任何一句话**——对端那一侧的
    // 通知归 `unseat`，对端表里那几枚副本的寿命随对端。
    let mut q = quay();
    q.seat(name("records")).expect("装得上");
    q.seat(name("control")).expect("装得上");
    let said = fake::said().len();
    q.shut();
    assert_eq!(fake::unshipped().len(), 2, "两条都放下了");
    assert_eq!(fake::said().len(), said, "一句都没说");
    assert!(q.find(name("records")).is_none() && q.find(name("control")).is_none());
    assert!(!q.ready(), "空了 ⇒ 不算通");
}

#[test]
fn ready_means_every_berth_has_both_ends() {
    let mut q = quay();
    assert!(!q.ready(), "一条都没有 ⇒ 不算通");
    q.seat(name("records")).expect("装得上");
    assert!(!q.ready(), "装上了、对端那一枚还没到 ⇒ 还不算通");
    fake::put(51, PEER, Mark::of("records"));
    assert_eq!(q.claim(PEER, Mark::of("records"), 0), Ok(()));
    assert!(q.ready(), "两头都齐了");
    // 第二条路一来，整体又变回"不通"（`ready` 说的是**每一条**）。
    q.seat(name("control")).expect("装得上");
    assert!(!q.ready());
}

#[test]
fn a_pier_without_a_write_end_refuses_to_post() {
    // **没有写端就发不出去**（不猜、不空转）；收的那一端总是有的（本端那一枚）。
    let mut q = quay();
    let pier: Pier = *q.seat(name("records")).expect("装得上");
    assert_eq!(pier.post(b"hi"), Err(()), "对端那一枚还没到");
    assert_eq!(pier.try_post(b"hi"), Err(()), "同一条");
    assert!(fake::said().is_empty());

    fake::put(61, PEER, Mark::of("records"));
    assert_eq!(q.claim(PEER, Mark::of("records"), 0), Ok(()));
    let pier = *q.find(name("records")).expect("在");
    assert_eq!(pier.post(b"hi"), Ok(()));
    assert_eq!(fake::said(), vec![(fake::tok(61), b"hi".to_vec())], "推到对端那一枚上");

    // 收的那一头走本端那一枚（`pull`），与 `post` 成对。
    fake::put_inbox(1 + 1, b"yo");
    let mut buf = [0u8; 8];
    assert_eq!(pier.pull(&mut buf, 0), Err(()), "那一枚上没有话");
}
