//! line 核心的门（**宿主台**）—— **账 + 界**，在宿主上真跑一遍。
//!
//! # 这批判据为什么住在这里
//!
//! 与 `operator` 靶同一条：`protocol` 是 `[lib] test = false`（riscv 目标上编不出 libtest），
//! 故主工作区那几道门**都不编**它的 `#[cfg(test)]`；而**链接 `protocol` 去测**也走不通——它依赖
//! `runtime`，`runtime/src/core/tls.rs` 里那两处 riscv 内联汇编在宿主编译器上编不出来。
//!
//! 故这一台也走那条现成的路：编外宿主 crate、`#[path]` 把
//! `crates/protocol/src/driver/line/core.rs` **逐字未改**地当一个模块读进来。
//!
//! **照实记（这一台多一处桩）**：线的核心写的是"有主那一格"，故它 `use crate::session::Pier`
//! ——宿主靶里给了一个**桩**（`Pier` 只要 `post` 一句：核心只跟泊位说这一句话，读/写泊位那一侧
//! 在适配层）。桩量不了会话，量得了账——本台钉的就是**账的界**。
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

/// 泊位那一格的**桩**：核心只跟它说一句 `post`（排空那一侧在适配层）。
///
/// **照实记（这一格是牙口量出来的）**：这个桩原先**恒答 `Ok`**，于是 `deliver` 里那条
/// "**推不出去 ⇒ 不置忙**"的契约**没有任何判据**——把 `post` 的失败忽略掉（"推不出去也置忙"）
/// 全门照绿（`crates/gate/src/mutations.rs` 里那一条变异逮出来的）。故给桩加一个**可关掉的失败开关**
/// （线程局部，照 `operator` 靶 / `judge` 靶那两台分配器的同款做法：libtest 每个用例
/// 各一枚线程，全局旗帜会随机打到别人身上）。
mod session {
    use std::cell::Cell;

    thread_local! {
        /// 本线程上 `post` 是否失败。
        static REFUSE: Cell<bool> = const { Cell::new(false) };
    }

    /// 把本线程的 `post` 打成失败（或放开）。
    pub fn refuse(on: bool) {
        REFUSE.with(|flag| flag.set(on));
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Pier;

    impl Pier {
        pub fn post(&mut self, _frame: &[u8]) -> Result<(), ()> {
            if REFUSE.with(Cell::get) {
                return Err(());
            }
            Ok(())
        }
    }
}

/// 码表宏（`fail_codes!`）自己一份源——**协议与宿主靶同读这一份**。
///
/// 两样都要（见那份文件的照实记）：`#[macro_use]` 把宏带进**下面那些模块**的作用域
/// （宏的可见性按正文先后 ⇒ 这一行必须在帧模块之前），`#[macro_export]` 保住"出 crate"那一份。
#[macro_use]
#[path = "../../protocol/src/fail_codes.rs"]
mod fail_codes;

/// 账的正文（就是 `crates/protocol/src/driver/line/core.rs` 那一份，逐字未改）。
///
/// **模块名就叫 `core`**：帧那一份写的是 `use super::core::Fail;`（在协议里它与 `core.rs` 同住
/// `driver::line`）——宿主靶里把这一份放在**同一层**、名字照旧，那一行才逐字成立
/// （`judge` 靶当初也是这么叫的；代价是 `core` 这个名字会遮住 `core` crate ⇒ 本文件里
/// 凡要用标准库的就写 `std::…`）。
#[path = "../../protocol/src/driver/line/core.rs"]
mod core;

/// **帧形与记号**那一份（`crates/protocol/src/driver/line/call.rs`，逐字未改）。
///
/// 照实记：**这是第一份上宿主的帧形**，而它**不用切结构**——线这一份 `use` 的只有 `env` 与
/// 同层 `core::Fail`，故"纯核心与适配分离"在这一份上本来就成立。
///
/// 照实记：`#[allow(dead_code)]` 是因为本台只叫了它的一部分（`LANE` 那几枚记号由适配层用）。
#[allow(dead_code)]
#[path = "../../protocol/src/driver/line/call.rs"]
mod call;

use crate::call::{OCCUPY, OCCUPY_LEN, pack_occupy, unpack_occupy};
use crate::core::{Fail, Lines};
use crate::session::Pier;
use env::Key;

/// 一个够用的账（`device_count = 4` ⇒ 线号 0..=4）。
fn account() -> Lines {
    Lines::new(4).expect("账放着")
}

#[test]
fn a_line_outside_the_account_is_not_a_line() {
    let mut lines = account();
    // 0 号：控制器领到空的那一格，永远不是一条线。
    assert_eq!(lines.occupy(0, Pier), Err(Fail::Unknown));
    assert_eq!(lines.deliver(0, &[1]), Err(Fail::Unknown));
    assert_eq!(lines.exhaust(0), Err(Fail::Unknown));
    assert_eq!(lines.vacate(0), Err(Fail::Unknown));
    assert_eq!(lines.lane(0), None);
    // 越界（`device_count = 4` ⇒ 5 号之外没有格子）。
    assert_eq!(lines.occupy(5, Pier), Err(Fail::Unknown));
    assert_eq!(lines.deliver(9, &[1]), Err(Fail::Unknown));
    assert_eq!(lines.lane(9), None);
}

#[test]
fn occupying_twice_is_taken_and_the_lane_comes_back() {
    let mut lines = account();
    assert_eq!(lines.occupy(1, Pier), Ok(()));
    assert_eq!(lines.occupy(1, Pier), Err(Fail::Taken));
    assert_eq!(lines.lane(1), Some(Pier));
    assert_eq!(lines.vacate(1), Ok(()));
    assert_eq!(lines.lane(1), None);
    // 退场之后再占上：同一条线可以换主人。
    assert_eq!(lines.occupy(1, Pier), Ok(()));
}

#[test]
fn a_frame_that_cannot_be_posted_leaves_the_line_idle() {
    // **`deliver` 的那一半契约**：推不出去 ⇒ 答 `Denied`，而且**不置忙**——"那一帧没送到"
    // 这件事必须在账上留痕，否则排空那一侧会去排一条从没收到东西的线。
    let mut lines = account();
    assert_eq!(lines.occupy(1, Pier), Ok(()));
    crate::session::refuse(true);
    assert_eq!(lines.deliver(1, &[1, 2, 3]), Err(Fail::Denied));
    crate::session::refuse(false);
    assert!(
        lines.busy().next().is_none(),
        "推不出去的那一帧不该把线置忙"
    );
    assert_eq!(
        lines.lane(1),
        Some(Pier),
        "主人照旧（失败的是这一帧，不是这一格）"
    );
    // 放开之后同一格照常投得进、也照常置忙。
    assert_eq!(lines.deliver(1, &[4]), Ok(()));
    assert_eq!(lines.busy().collect::<Vec<_>>(), std::vec![1]);
}

#[test]
fn busy_means_delivered_and_not_yet_drained() {
    let mut lines = account();
    assert_eq!(lines.occupy(2, Pier), Ok(()));
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
    assert_eq!(lines.occupy(3, Pier), Ok(()));
    assert!(lines.told(3));
    assert!(!lines.told(3));
    assert_eq!(lines.vacate(3), Ok(()));
    // 主人退了，但"这一条线报过"照旧记着——不然下一次上来的主人会把同一行再打一遍。
    assert!(!lines.told(3));
}

// ── 帧形那一半（`driver/line/call.rs`）──────────────────────
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
// `crates/protocol/src/driver/line/call.rs` 的 `const _: () = assert!(…)` 里。
