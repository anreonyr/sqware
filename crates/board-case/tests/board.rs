//! 公示板与待客台账的门（**宿主台**）—— 板核心的规矩，在宿主上真跑一遍。
//!
//! # 这一台为什么存在（照实记：这一批是"救活的"）
//!
//! `crates/protocol/src/system/board/core.rs` 的 `#[cfg(test)]` 模块**从写下那天起一次没跑过**
//! ——`protocol` 是 `[lib] test = false`（riscv 上编不出 libtest），主工作区那几道门一道都不编它。
//! 与别的几台同一条路（**那张清单与理由住 `scripts/host.sh` 的头注**——这里不写台数：
//! 台数每加一台就要改一遍，而"话要能指回源头"）：编外宿主 crate、只依赖 `env`、
//! 把核心源码**逐字未改**地 `#[path]` 进来，
//! 门口 `scripts/host.sh`。这一份**无桩**（只认 `env` 那几个类型）。
//!
//! # 这一台钉的是什么
//!
//! 见那一份源码自己的 `#[cfg(test)]` 模块（板那一格的规矩：立牌子、摘牌子、查名字、退场）。

extern crate alloc;

/// 码表宏（`fail_codes!`）自己一份源——**协议与宿主靶同读这一份**。
///
/// 两样都要（见那份文件的照实记）：`#[macro_use]` 把宏带进**下面那些模块**的作用域
/// （宏的可见性按正文先后 ⇒ 这一行必须在帧模块之前），`#[macro_export]` 保住"出 crate"那一份。
#[macro_use]
#[path = "../../protocol/src/fail_codes.rs"]
mod fail_codes;

/// 板那本账（就是 `crates/protocol/src/system/board/core.rs` 那一份，逐字未改）。
///
/// **模块名就叫 `core`**：帧那一份写的是 `use super::core::{Board, Fail};`——宿主靶里把这一份
/// 放在**同一层**、名字照旧，那一行才逐字成立（`judge-case` / `line-case` 当初也是这么叫的；
/// 代价是 `core` 这个名字会遮住 `core` crate ⇒ 本文件里用标准库的地方写 `std::…`）。
#[path = "../../protocol/src/system/board/core.rs"]
mod core;

/// **帧那一半**（`crates/protocol/src/system/board/frame.rs`，逐字未改）—— 在本台里跑判据。
#[allow(dead_code)]
#[path = "../../protocol/src/system/board/frame.rs"]
mod frame;

// ── 帧那一半（`system/board/frame.rs`）──────────────────────
//
// **照实记（这一组为什么值当）**：机器那几道门走的是**顺路**——客侧编一帧、板侧解一帧。
// 下面这些格子机器**一条都走不到**：短帧 / 空帧、**"这一码才带 seed"那一格**（不带 seed 的
// 那几码，入口那一格是全 0——服务侧**必须在 `unpack_ask` 之前按动作码分流**）、
// 名字那几格的判废（空 / 串尾有垃圾 / 不是 UTF-8）、以及失败码表的两端。

use env::{Mark, Name, PieToken};
use crate::core::Fail;
use crate::frame as f;

fn name(text: &str) -> Name {
    Name::new(text).expect("名字合法")
}

#[test]
fn an_ask_carries_the_name_and_a_seed_only_for_the_code_that_has_one() {
    let seed = PieToken::from_bytes(&77u64.to_le_bytes()).unwrap();
    let frame = f::pack_ask(f::REGISTER, name("console"), Some(seed));
    assert_eq!(frame.len(), f::ASK_LEN);
    assert_eq!(f::op_of(&frame), Some(f::REGISTER));
    assert_eq!(f::unpack_ask(&frame), Some((name("console"), seed)), "名字与入口都在");

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

#[test]
fn the_board_marks_and_the_lane_prefix_are_what_they_say() {
    // **面不相撞**：板这一面的问话孔记号与树那一面、与提示孔都不许撞（撞了就是那次装机塌掉）。
    assert_ne!(f::ASK_MARK, Mark::of("operator-ask"));
    assert_ne!(f::ASK_MARK, Mark::of("ask"));
    assert_ne!(f::ASK_MARK, f::TIP_MARK);
    assert_ne!(f::TIP_MARK, Mark::of("board-tip"), "孔上的记号与路名是两回事");
    assert_ne!(f::ENTRY_MARK, Mark::NONE);
    assert_eq!(f::LINK, "board");
    assert_eq!(f::TIP_NAME, "board-tip");
    // 待客台账里"那一位的名字"用的前缀（退场之后牌子留着，名字加上它）。
    assert_eq!(f::LANE_PREFIX, "gone-");
    assert!(Name::new(alloc::format!("{}console", f::LANE_PREFIX).as_str()).is_ok());
}

