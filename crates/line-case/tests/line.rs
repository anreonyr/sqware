//! line 核心的门（**宿主台**）—— **账 + 界**，在宿主上真跑一遍。
//!
//! # 这批判据为什么住在这里
//!
//! 与 `operator-case` 同一条：`protocol` 是 `[lib] test = false`（riscv 目标上编不出 libtest），
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
/// 全门照绿（`scripts/teeth.py` 里那一条变异逮出来的）。故给桩加一个**可关掉的失败开关**
/// （线程局部，照 `operator-case` / `judge-case` 那两台分配器的同款做法：libtest 每个用例
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

/// 账的正文（就是 `crates/protocol/src/driver/line/core.rs` 那一份，逐字未改）。
#[path = "../../protocol/src/driver/line/core.rs"]
mod line;

use crate::line::{Fail, Lines};
use crate::session::Pier;

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
    assert_eq!(lines.lane(1), Some(Pier), "主人照旧（失败的是这一帧，不是这一格）");
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
