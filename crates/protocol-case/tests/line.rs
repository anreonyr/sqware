//! line 核心的门（**宿主台**）—— **账 + 界**，在宿主上真跑一遍。
//!
//! # 这批判据为什么住在这里
//!
//! 与 `operator` 靶同一条：`protocol` 是 `[lib] test = false`（riscv 目标上编不出 libtest），
//! 故主工作区那几道门**都不编**它的 `#[cfg(test)]`；而**链接 `protocol` 去测**也走不通——它依赖
//! `runtime`，`runtime/src/core/tls.rs` 里那两处 riscv 内联汇编在宿主编译器上编不出来。
//!
//! 故这一台也走那条现成的路：编外宿主 crate、**真依赖**「约」`contract`——
//! `crates/contract/src/driver/line/core.rs`（账）与 `frame.rs`（帧形与记号）逐字同一份源码。
//!
//! **照实记（这一台原先那一处桩已退场）**：线的核心写的是"有主那一格"，故它要一个 `Pier`。
//! `#[path]` 那一版把核心逐字编进靶时，靶里给了一个**同名的桩类型**（`pub struct Pier;`，
//! 外加一个可关掉的 `post` 失败开关）。**那是个影子**：桩与真那份只共享一个名字，形状漂了
//! 也没人喊。真依赖之后 `Pier` 是**会话核心铸的那一枚**（字段私有 ⇒ 只有 `Quay` 造得出），
//! 故靶得**真把一条泊位配齐**（`seat` → 对端那一枚进假表 → `claim`，见 [`pier`]）；
//! 那个失败开关搬到**假手表**上（`tests/session/call.rs` 的 `refuse`），判据一个字没松。
//! 桩量不了会话，量得了账——本台钉的仍是**账的界**。
//!
//! # 这批判据钉的是什么
//!
//! **线号 = 下标，越界不可表达**：`occupy` / `deliver` / `exhaust` / `vacate` 对 0 号与越界线号
//! 一律答 `Unknown`（不 panic）；`lane` 答 `None`；`busy` / `held` 只列账里的那些。
//!
//! **"报过没有"那一格与格同长**（[`Lines::told`]）：第一次 `true`、此后 `false`；
//! **越界 `false`——不是 panic**（**照实记**：这一格从前是 router 自己另开的一本定长账
//! `[u64; 2]`，容量与账不联动、越界是裸下标 ⇒ 自报 > 127 条线的控制器上第 128 条当场炸）；
//! **退场不清**：`vacate` 之后仍答 `false`（记的是"这一条线这一趟"，不是"这一位主人"）。

extern crate alloc;

/// 会话那一层：**假手表 + 真据**。
///
/// `#[path = "call.rs"]` 里的基准目录是 `tests/session/`（内联模块的 `#[path]` 基准跟着模块名走，
/// 照实记见 `roster` 靶）——故这一台与 `quay` 靶读的是**同一份假手表**。
mod session {
    /// 本台的**假手表**（十个假手 ＋ 测试侧的观察面）。
    ///
    /// **照实记（`#[allow(dead_code)]`）**：这一份是**两台共读**的（`quay` 靶与这一台），
    /// 而这一台只叫得着其中几件（`reset` / `name` / `put` / `refuse`）——挂 allow 而不是把
    /// 其余几件删掉：删了 `quay` 靶就编不过。
    #[allow(dead_code)]
    #[path = "call.rs"]
    pub mod call;

    /// 会话的据与表的形状——**真依赖** `contract`（逐字同一份源码）。
    pub use contract::session::{core, hands};
}

/// 账 + 帧形与记号——**真依赖** `contract::driver::line` 那两份（逐字同一份源码）。
///
/// **模块名照旧**（`core` / `call`）：帧那一份写的是 `use super::core::Fail;`（在协议里它与
/// `core.rs` 同住 `driver::line`）——**那份源码在 `contract` 里本来就成立**；靶这侧那些
/// `use crate::core::…` 也一个字不改。代价照实记：`core` 这个名字会遮住 `core` crate
/// ⇒ 本文件里凡要用标准库的就写 `std::…`。
///
/// **照实记（`#[path]` 退场）**：这两份原先各拿一行 `#[path]` 逐字编进靶，外加 `fail_codes!`
/// 码表一份（宏的可见性按正文先后 ⇒ `#[macro_use]` 那两行必须写在帧模块之前）。真依赖挂上
/// 之后三行一起退场：码表随 `frame.rs` 住在 `contract` 里，本台一处都不碰。
use contract::driver::line::{core, frame as call};

use env::Mark;
use session::call as fake;
use session::core::{Pier, Quay};

use crate::call::{OCCUPY, OCCUPY_LEN, pack_occupy, unpack_occupy};
use crate::core::{Fail, Lines};
use plan::Key;

/// 对端（另一个域）。
const PEER: env::TaskId = env::TaskId::new(7);

/// 一条**配齐了的**泊位——真的 [`Pier`]（会话核心铸的那一枚），不是桩。
///
/// 三步就是线上那一趟：本端 `seat`（铸本端那一枚、交给对端）→ 对端那一枚**进假表**
/// （`put`：主人是对端、记号是同一条路的名字）→ `claim` 把它认到这条路上。
/// 认完 `Quay` 就可以丢：`Pier` 是 `Copy`、那两枚孔住在假表里，与码头本体的寿命无关。
fn pier(text: &str) -> Pier {
    fake::reset();
    let mut q = Quay::open(PEER, session::call::hands());
    q.seat(fake::name(text)).expect("装得上");
    fake::put(9, PEER, Mark::of(text));
    q.claim(PEER, Mark::of(text), 0).expect("对端那一枚到了");
    *q.find(fake::name(text)).expect("在")
}

/// 一个够用的账（`device_count = 4` ⇒ 线号 0..=4）。
fn account() -> Lines {
    Lines::new(4).expect("账放着")
}

#[test]
fn a_line_outside_the_account_is_not_a_line() {
    let mut lines = account();
    // 0 号：控制器领到空的那一格，永远不是一条线。
    assert_eq!(lines.occupy(0, pier("zero")), Err(Fail::Unknown));
    assert_eq!(lines.deliver(0, &[1]), Err(Fail::Unknown));
    assert_eq!(lines.exhaust(0), Err(Fail::Unknown));
    assert_eq!(lines.vacate(0), Err(Fail::Unknown));
    assert!(lines.lane(0).is_none());
    // 越界（`device_count = 4` ⇒ 5 号之外没有格子）。
    assert_eq!(lines.occupy(5, pier("beyond")), Err(Fail::Unknown));
    assert_eq!(lines.deliver(9, &[1]), Err(Fail::Unknown));
    assert!(lines.lane(9).is_none());
}

#[test]
fn occupying_twice_is_taken_and_the_lane_comes_back() {
    let mut lines = account();
    let first = pier("uart");
    assert_eq!(lines.occupy(1, first), Ok(()));
    assert_eq!(lines.occupy(1, pier("echo")), Err(Fail::Taken));
    assert_eq!(
        lines.lane(1).map(|p| p.name()),
        Some(first.name()),
        "还回来的仍是那一条泊位"
    );
    assert_eq!(lines.vacate(1), Ok(()));
    assert!(lines.lane(1).is_none());
    // 退场之后再占上：同一条线可以换主人。
    assert_eq!(lines.occupy(1, pier("rtc")), Ok(()));
}

#[test]
fn a_frame_that_cannot_be_posted_leaves_the_line_idle() {
    // **`deliver` 的那一半契约**：推不出去 ⇒ 答 `Denied`，而且**不置忙**——"那一帧没送到"
    // 这件事必须在账上留痕，否则排空那一侧会去排一条从没收到东西的线。
    let mut lines = account();
    let owner = pier("uart");
    assert_eq!(lines.occupy(1, owner), Ok(()));
    fake::refuse(true);
    assert_eq!(lines.deliver(1, &[1, 2, 3]), Err(Fail::Denied));
    fake::refuse(false);
    assert!(
        lines.busy().next().is_none(),
        "推不出去的那一帧不该把线置忙"
    );
    assert_eq!(
        lines.lane(1).map(|p| p.name()),
        Some(owner.name()),
        "主人照旧（失败的是这一帧，不是这一格）"
    );
    // 放开之后同一格照常投得进、也照常置忙。
    assert_eq!(lines.deliver(1, &[4]), Ok(()));
    assert_eq!(lines.busy().collect::<Vec<_>>(), std::vec![1]);
}

#[test]
fn busy_means_delivered_and_not_yet_drained() {
    let mut lines = account();
    assert_eq!(lines.occupy(2, pier("uart")), Ok(()));
    assert_eq!(lines.busy().collect::<Vec<_>>(), Vec::<u32>::new());
    assert_eq!(lines.deliver(2, &[1]), Ok(()));
    assert_eq!(lines.busy().collect::<Vec<_>>(), vec![2]);
    assert_eq!(lines.exhaust(2), Ok(()));
    assert_eq!(lines.busy().collect::<Vec<_>>(), Vec::<u32>::new());
    // 有主归 `held`（探活按它走）。
    assert_eq!(lines.held().collect::<Vec<_>>(), vec![2]);
    assert_eq!(lines.vacate(2), Ok(()));
    assert_eq!(lines.held().collect::<Vec<_>>(), Vec::<u32>::new());
}

#[test]
fn told_is_once_per_line() {
    let mut lines = account();
    assert!(lines.told(1));
    assert!(!lines.told(1));
    assert!(lines.told(2));
}

#[test]
fn told_outside_the_account_answers_false_instead_of_panicking() {
    // 这一格从前是 `[u64; 2]` 的裸下标：95 条线的账上问第 128 格就炸。
    let mut small = Lines::new(95).expect("账放着");
    assert!(!small.told(128));
    assert!(!small.told(4096));
    // 记账长的那一侧照旧：账里的格子答真。
    let mut large = Lines::new(300).expect("账放着");
    assert!(large.told(200));
    assert!(!large.told(200));
}

#[test]
fn told_survives_a_vacate() {
    let mut lines = account();
    assert_eq!(lines.occupy(3, pier("uart")), Ok(()));
    assert!(lines.told(3));
    assert!(!lines.told(3));
    assert_eq!(lines.vacate(3), Ok(()));
    // 主人退了，但"这一条线报过"照旧记着——不然下一次上来的主人会把同一行再打一遍。
    assert!(!lines.told(3));
}

// ── 帧形那一半（`driver/line/frame.rs`）──────────────────────
//
// **照实记（这一组为什么值当）**：机器那几道门走的是**顺路**——客户端编一帧、路由者解一帧，
// 形状对了就继续。下面这些格子机器**一条都走不到**：短一帧 / 长一帧 / 动作码不对、
// **判别号不认识的坐标**、以及那张失败码表的两端（表外那一格与读不懂的码都答 `None`，
// 而这两个 `None` **不是同一件事**）。

#[test]
fn an_occupy_frame_is_the_action_code_then_the_coordinate() {
    let key = Key::region(0x1000_1000);
    let frame = pack_occupy(key);
    assert_eq!(frame.len(), OCCUPY_LEN);
    assert_eq!(frame[0], OCCUPY, "头一格是动作码");
    assert_eq!(&frame[1..], &key.bytes(), "其余是坐标，一字不差");
    assert_eq!(unpack_occupy(&frame), Some(key), "编了再解 = 原来那个坐标");
}

#[test]
fn a_frame_that_is_not_that_shape_is_not_guessed_at() {
    // **不是那个形状就答 `None`**（别人往这扇门推别的东西时，不猜）。
    let good = pack_occupy(Key::region(0x1000_1000));
    assert_eq!(unpack_occupy(&[]), None, "空帧");
    assert_eq!(unpack_occupy(&good[..OCCUPY_LEN - 1]), None, "短一字节");
    let long = [good.as_slice(), &[0u8]].concat();
    assert_eq!(unpack_occupy(&long), None, "长一字节");

    let mut wrong_op = good;
    wrong_op[0] = OCCUPY + 1;
    assert_eq!(unpack_occupy(&wrong_op), None, "动作码不对");
    wrong_op[0] = 0;
    assert_eq!(unpack_occupy(&wrong_op), None, "零不是动作码");
}

#[test]
fn an_unknown_coordinate_discriminator_is_refused_but_a_known_non_region_is_taken() {
    // 两件事分得开（文件头那句"不猜"）：
    //   - **判别号不认识** ⇒ `None`（这不是本仓的坐标，读它没有意义）；
    //   - **认识、但不是"区"的那两形**（设备树 / 中断）**照收**——路由者按坐标查表查不到，
    //     自然答 `UNKNOWN`（线挂在设备上，那是它的账）。
    let mut frame = pack_occupy(Key::region(0x1000));
    frame[1] = 0xEE; // 判别号那一格换成一个不认识的
    assert_eq!(unpack_occupy(&frame), None, "不认识的判别号");

    for key in [Key::dtb(), Key::irq()] {
        assert_eq!(
            unpack_occupy(&pack_occupy(key)),
            Some(key),
            "认识的非区坐标照收"
        );
    }
}

#[test]
fn the_failure_table_is_bijective_and_keeps_bad_outside() {
    use crate::call::{BAD, DENIED, OK, TAKEN, UNKNOWN, code_to_fail, fail_to_code};

    assert_eq!(fail_to_code(None), OK, "没失败 ⇒ OK");
    assert_eq!(fail_to_code(Some(Fail::Unknown)), UNKNOWN);
    assert_eq!(fail_to_code(Some(Fail::Taken)), TAKEN);
    assert_eq!(fail_to_code(Some(Fail::Denied)), DENIED);

    // **一端一格**（双射）：解回来是同一位。
    assert_eq!(code_to_fail(OK), None);
    assert_eq!(code_to_fail(UNKNOWN), Some(Fail::Unknown));
    assert_eq!(code_to_fail(TAKEN), Some(Fail::Taken));
    assert_eq!(code_to_fail(DENIED), Some(Fail::Denied));

    // **表外那一格与读不懂的码都答 `None`**，而这两个 `None` 不是同一件事——读的人靠**动作码**
    // 先分流：`BAD` 是"这一问读不懂"，`OK` 是"没失败"。
    assert_eq!(
        code_to_fail(BAD),
        None,
        "`BAD` 在失败域之外（照实记见宏的文档）"
    );
    assert_eq!(code_to_fail(200), None, "表外的码");
    assert_ne!(BAD, OK, "两者不同码，才分得开");
}

// ── 面不相撞那一条用例搬去了**编译期**（用户裁定"常量交给编译器"）────────────
//
// `the_two_marks_of_this_road_do_not_collide` 原先在这里，那几条现在写在
// `crates/contract/src/driver/line/frame.rs` 的 `const _: () = assert!(…)` 里。
