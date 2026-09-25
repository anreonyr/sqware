//! 公示板与待客台账的门（**宿主台**）—— 板核心的规矩，在宿主上真跑一遍。
//!
//! # 这一台为什么存在（照实记：这一批是"救活的"）
//!
//! `crates/contract/src/system/board/core.rs` 的 `#[cfg(test)]` 模块**从写下那天起一次没跑过**
//! ——`protocol` 是 `[lib] test = false`（riscv 上编不出 libtest），主工作区那几道门一道都不编它。
//! 与别的几台同一条路（**那张清单与理由住 `crates/gate/tests/host.rs` 的头注**——这里不写台数：
//! 台数每加一台就要改一遍，而"话要能指回源头"）：编外宿主 crate、**真依赖**「约」`contract`，
//! 门口 `crates/gate/tests/host.rs`。这一份**无桩**（只认 `env` 那几个类型）。
//!
//! # 这一台钉的是什么
//!
//! 见那一份源码自己的 `#[cfg(test)]` 模块（板那一格的规矩：立牌子、摘牌子、查名字、退场）。

extern crate alloc;

/// 板那本账与**帧那一半**——**真依赖** `contract::system::board` 那两份（逐字同一份源码）。
///
/// **模块名照旧**（`core` / `frame`）：帧那一份写的是 `use super::core::{Board, Fail};`，
/// **那份源码在 `contract` 里本来就成立**；靶这侧那两行 `use crate::core::…` / `use crate::frame as f`
/// 也一个字不改。代价照实记：`core` 这个名字会遮住 `core` crate ⇒ 本文件里用标准库的地方写 `std::…`。
///
/// **照实记（`#[path]` 退场）**：这三份原先各拿一行 `#[path]` 逐字编进靶，那张 `fail_codes!`
/// 码表也要跟着复制一份（宏的可见性按正文先后 ⇒ `#[macro_use]` 那两行必须写在帧模块之前）。
/// 真依赖挂上之后三行一起退场：码表随 `frame.rs` 住在 `contract` 里，本台一处都不碰。
use contract::system::board::{core, frame};

// ── 帧那一半（`system/board/frame.rs`）──────────────────────
//
// **照实记（这一组为什么值当）**：机器那几道门走的是**顺路**——客侧编一帧、板侧解一帧。
// 下面这些格子机器**一条都走不到**：短帧 / 空帧、**"这一码才带 seed"那一格**（不带 seed 的
// 那几码，入口那一格是全 0——服务侧**必须在 `unpack_ask` 之前按动作码分流**）、
// 名字那几格的判废（空 / 串尾有垃圾 / 不是 UTF-8）、以及失败码表的两端。

use crate::core::{Board, Fail};
use crate::frame as f;
use env::{Name, PieToken, TaskId};

fn name(text: &str) -> Name {
    Name::new(text).expect("名字合法")
}

#[test]
fn an_ask_carries_the_name_and_a_seed_only_for_the_code_that_has_one() {
    let seed = PieToken::from_bytes(&77u64.to_le_bytes()).unwrap();
    let frame = f::pack_ask(f::REGISTER, name("console"), Some(seed));
    assert_eq!(frame.len(), f::ASK_LEN);
    assert_eq!(f::op_of(&frame), Some(f::REGISTER));
    assert_eq!(
        f::unpack_ask(&frame),
        Some((name("console"), seed)),
        "名字与入口都在"
    );

    // **不带 seed 的那几码**：那一格全 0（服务侧在 `unpack_ask` 之前就按动作码分流出去），
    // 故这里的读数是"名字 + 零号"——不是"没有名字"。
    for op in [f::UNREGISTER, f::LOOKUP, f::EVICT] {
        let frame = f::pack_ask(op, name("console"), None);
        assert_eq!(f::op_of(&frame), Some(op));
        assert_eq!(
            f::unpack_ask(&frame).map(|(n, t)| (n, t.get())),
            Some((name("console"), 0)),
            "零号那一格是「没有带」，读的人按动作码分辨"
        );
    }
}

#[test]
fn a_board_frame_that_is_not_that_shape_is_not_guessed_at() {
    assert_eq!(f::unpack_ask(&[]), None, "空帧");
    assert_eq!(f::op_of(&[]), None, "空帧连动作码都没有");
    let short = [0u8; f::ASK_LEN - 1];
    assert_eq!(f::unpack_ask(&short), None, "短一字节");
    // 长一字节：`unpack_ask` 只看 [`f::ASK_LEN`] 那一段，多出来的字节不参与解读
    // （真服务侧那只缓冲正好是 `ASK_LEN`，故"读一帧"从不看尾巴）。
    let mut long = [0u8; f::ASK_LEN + 1];
    long[..f::ASK_LEN].copy_from_slice(&f::pack_ask(f::LOOKUP, name("x"), None));
    assert_eq!(
        f::unpack_ask(&long).map(|(n, t)| (n, t.get())),
        Some((name("x"), 0)),
        "尾巴不参与"
    );
}

#[test]
fn reading_a_name_out_of_bytes_judges_all_four_ways() {
    let mut good = [0u8; env::wire::NAME_LEN];
    good[..4].copy_from_slice(b"uart");
    assert_eq!(f::name_of(&good), Some(name("uart")), "尾部补零是正当的");

    assert_eq!(f::name_of(&[0u8; env::wire::NAME_LEN]), None, "全零 ⇒ 空名");
    let mut garbage = [0u8; env::wire::NAME_LEN];
    garbage[..4].copy_from_slice(b"uart");
    garbage[5] = 7; // 串尾（第一个 0 之后）还有非零字节
    assert_eq!(f::name_of(&garbage), None, "串尾有垃圾 ⇒ 判废");
    let mut bad_utf8 = [0u8; env::wire::NAME_LEN];
    bad_utf8[0] = 0xFF;
    assert_eq!(f::name_of(&bad_utf8), None, "不是 UTF-8 ⇒ 判废");
    assert_eq!(f::name_of(&[1u8; 3]), None, "短于一个名字 ⇒ 读不出");
}

#[test]
fn the_board_failure_table_is_bijective_and_keeps_bad_outside() {
    use f::{BAD, DENIED, FULL, OK, TAKEN, UNKNOWN, code_to_fail, fail_to_code};
    assert_eq!(fail_to_code(None), OK);
    for (fail, code) in [
        (Fail::Unknown, UNKNOWN),
        (Fail::Taken, TAKEN),
        (Fail::Denied, DENIED),
        (Fail::Full, FULL),
    ] {
        assert_eq!(fail_to_code(Some(fail)), code, "{fail:?}");
        assert_eq!(code_to_fail(code), Some(fail), "一端一格");
        assert_ne!(code, OK, "失败不许与 OK 同码");
    }
    assert_eq!(code_to_fail(OK), None);
    assert_eq!(code_to_fail(BAD), None, "读不懂那一格在失败域之外");
    assert_eq!(code_to_fail(200), None, "表外的码");
}

// ── 面不相撞那一条用例搬去了**编译期**（用户裁定"常量交给编译器"）────────────
//
// `the_board_marks_and_the_lane_prefix_are_what_they_say` 原先在这里：真会撞的那几对比的是
// 常量，现在写在 `crates/contract/src/system/board/frame.rs` 的 `const _: () = assert!(…)` 里；
// 余下几条（`LINK == "board"` 那一类）是**同义反复**，随用例一起去掉。
// ── 板那本账的判据（原住 `system/board/core.rs` 的 `#[cfg(test)]`）────────────
//
// **照实记（用户裁定）**："我希望测试和运行环境分开，而不是交叉在一起" ⇒ 那 **7 条**用例不再
// 住运行时源里，整段搬到这里（`board/core.rs` 留下的是结论与去处）。下面这些假表与助手是它们
// 原来在源里就有的（逐字照搬，只把 `core::sync::atomic` 改成 `std::sync::atomic`——本文件里
// `core` 那个名字被板上正文的模块占了，见上面那条照实记）。

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 假表是**进程级**的（`VestedBy` 是函数指针，捕不了环境），故测试彼此串行。
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// 甲（挂牌那位）与乙（另一个也拿着入口的服务）。
const A: TaskId = TaskId::new(1);
const B: TaskId = TaskId::new(2);

/// 假表：第 i 位 = 令牌 i 的授与人（0 = 这枚不在我表里）。
///
/// 比板**多一位**：撑满板要 `CAP` 枚活令牌（0..CAP），第 `CAP` 位是没有出处的
/// 那一枚——"板满了"必须是 `Full`，不能先被活性判成 `Denied`（判据的次序是契约）。
static TABLE: [AtomicUsize; Board::CAP + 1] = [const { AtomicUsize::new(0) }; Board::CAP + 1];
static FREED: AtomicUsize = AtomicUsize::new(0);

/// 造一枚号给核心用：**唯一的门是"收号"**（`PieToken::from_bytes`），
/// 本模块自己不造号（`env::wire::handle`）。
fn tok(n: usize) -> PieToken {
    PieToken::from_bytes(&(n as u64).to_le_bytes()).expect("8 字节")
}

fn fake_vested_by(entry: PieToken) -> Option<TaskId> {
    match entry.get() {
        at if at < TABLE.len() => match TABLE[at].load(Ordering::Relaxed) {
            0 => None,
            who => Some(TaskId::new(who)),
        },
        _ => None,
    }
}

/// 记下"这一枚被放下了"。假表有 `CAP` 位，越界的令牌不记。
fn fake_unship(entry: PieToken) -> Result<(), ()> {
    if entry.get() < TABLE.len() {
        FREED.fetch_or(1usize << entry.get(), Ordering::Relaxed);
    }
    Ok(())
}

fn board() -> Board {
    for slot in &TABLE {
        slot.store(0, Ordering::Relaxed);
    }
    FREED.store(0, Ordering::Relaxed);
    Board::new(fake_vested_by, fake_unship)
}

/// 令牌 `entry` 此刻在我表里，且是 `who` 授的。
fn mine(entry: usize, who: TaskId) {
    if let Some(slot) = TABLE.get(entry) {
        slot.store(who.get(), Ordering::Relaxed);
    }
}

/// 那枚不在了（放下了 / 令牌越界）。
fn gone(entry: usize) {
    if let Some(slot) = TABLE.get(entry) {
        slot.store(0, Ordering::Relaxed);
    }
}

fn unshipped(entry: usize) -> bool {
    FREED.load(Ordering::Relaxed) & (1usize << entry) != 0
}

fn stand(b: &mut Board, text: &str, entry: usize, who: TaskId) -> Result<PieToken, Fail> {
    mine(entry, who);
    b.register(name(text), tok(entry), who)
}

#[test]
fn register_requires_the_entry_to_be_mine() {
    let _serial = serial();
    let mut b = board();
    gone(1);
    assert_eq!(b.register(name("console"), tok(1), A), Err(Fail::Denied));
    mine(2, B);
    assert_eq!(b.register(name("console"), tok(2), A), Err(Fail::Denied));
    assert_eq!(stand(&mut b, "console", 1, A), Ok(tok(1)));
    assert_eq!(b.lookup(name("console")), Ok(tok(1)));
}

#[test]
fn a_name_standing_for_someone_else_is_taken() {
    let _serial = serial();
    let mut b = board();
    assert_eq!(stand(&mut b, "console", 1, A), Ok(tok(1)));
    mine(2, B);
    assert_eq!(b.register(name("console"), tok(2), B), Err(Fail::Taken));
    assert_eq!(b.unregister(name("console"), B), Err(Fail::Denied));
    assert!(!unshipped(1));
    assert_eq!(b.unregister(name("console"), A), Ok(()));
    assert!(unshipped(1));
    assert_eq!(b.lookup(name("console")), Err(Fail::Unknown));
    assert_eq!(b.unregister(name("console"), A), Err(Fail::Unknown));
}

#[test]
fn repeated_register_overwrites_and_frees_the_old_entry() {
    let _serial = serial();
    let mut b = board();
    stand(&mut b, "console", 1, A);
    stand(&mut b, "console", 2, A);
    assert_eq!(b.lookup(name("console")), Ok(tok(2)));
    assert!(unshipped(1) && !unshipped(2));
    assert_eq!(b.find(name("console")), Some(0));
    assert_eq!(b.rows().count(), 1);
}

#[test]
fn unregistering_a_dead_entry_answers_unknown() {
    // **撤牌子也走那条"先扫后判"**：那一枚入口答不出（主人退场 / 不在我表里）⇒ 牌子当场被扫空
    // ⇒ 这一格"没有实例"，答 `Unknown`——**不是**拿一个死实例去比主人（那样会答 `Ok` 或
    // `Denied`，把"这一格已经空了"读成"这一格还归谁"）。
    //
    // 照实记：这一格是牙口量出来的——把 `unregister` 里那次 `sweep_at` 删掉，**原先全门照绿**
    // （没有一条规格走过"死实例 + 撤牌子"这条路）。
    let _serial = serial();
    let mut b = board();
    stand(&mut b, "console", 1, A);
    gone(1);
    assert_eq!(b.unregister(name("console"), A), Err(Fail::Unknown));
    assert_eq!(
        b.unregister(name("console"), B),
        Err(Fail::Unknown),
        "谁问都一样：这一格空了"
    );
    assert_eq!(b.rows().count(), 0, "扫干净了");
    assert_eq!(
        b.find(name("console")),
        Some(0),
        "**名字照旧**（撤牌子不动名字）"
    );
}

#[test]
fn a_dead_entry_is_swept_on_the_read_path() {
    let _serial = serial();
    let mut b = board();
    stand(&mut b, "console", 1, A);
    gone(1);
    assert_eq!(b.lookup(name("console")), Err(Fail::Unknown));
    assert!(unshipped(1));
    assert_eq!(b.rows().count(), 0);
    assert_eq!(b.find(name("console")), Some(0));
    assert_eq!(stand(&mut b, "console", 3, A), Ok(tok(3)));
    assert_eq!(b.lookup(name("console")), Ok(tok(3)));
}

#[test]
fn the_board_has_a_bottom() {
    let _serial = serial();
    let mut b = board();
    let mut texts = std::vec::Vec::new();
    for i in 0..Board::CAP {
        texts.push(std::format!("n{i}"));
        assert_eq!(
            stand(&mut b, &texts[i], i, A),
            Ok(tok(i)),
            "第 {i} 枚该挂得上"
        );
    }
    assert_eq!(b.rows().count(), Board::CAP);
    assert_eq!(
        b.register(name("n17"), tok(Board::CAP), A),
        Err(Fail::Denied),
        "没有出处的令牌先被活性挡住——判据的次序是契约"
    );
    assert_eq!(b.find(name("n17")), None);
    assert_eq!(
        b.register(name("n18"), tok(0), A),
        Err(Fail::Full),
        "令牌是真的、板是满的 ⇒ 这才是 Full"
    );
    assert_eq!(b.find(name("n18")), None);
    assert_eq!(b.unregister(name("n0"), A), Ok(()));
    assert_eq!(
        b.register(name("n18"), tok(0), A),
        Err(Fail::Full),
        "摘过牌的位子不还给新名字——空位只给**从未用过**的牌子"
    );
    assert_eq!(b.find(name("n0")), Some(0), "牌子留着，名字不流转");
    assert_eq!(b.find(name("n18")), None);
}

#[test]
fn unregister_keeps_the_sign_so_the_name_does_not_float_away() {
    let _serial = serial();
    let mut b = board();
    stand(&mut b, "console", 1, A);
    stand(&mut b, "irq", 2, A);
    assert_eq!(b.unregister(name("console"), A), Ok(()));
    assert_eq!(b.rows().count(), 1);
    assert_eq!(b.find(name("console")), Some(0));
    assert_eq!(b.find(name("irq")), Some(1));
    assert_eq!(b.lookup(name("console")), Err(Fail::Unknown));
    assert_eq!(b.lookup(name("irq")), Ok(tok(2)));
    assert_eq!(stand(&mut b, "serial", 3, A), Ok(tok(3)));
    assert_eq!(b.find(name("serial")), Some(2));
}
