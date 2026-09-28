//! operator::mount — **把一位操作面挂上树**：`/sys/operator/{part,land,…}` 那一格。
//!
//! 与 [`control::mount`](crate::system::control::mount) 同形三件事，而**只有第一件在本域手里落下**：
//!
//! ```text
//!   ① 铸入口   本域主线程自己铸（记号 = 那一位的记号，见 grant）
//!   ② 递出去   交给持树者（`R|W ＋ VEST`），再把"落哪一块、叫什么"推上提示之路
//!   ③ 落       持树者在自己核里 `part /sys/operator` ＋ `land /sys/operator/{name}`
//! ```
//!
//! ① 在本文件里（**那一位的记号与那一段名字只有一处**：`Grant` 与 [`SEGMENT`]）；②③ 分别在
//! [`Tree::land_plate`](crate::system::operator::bridge::Tree::land_plate) /
//! [`Tree::land_deep`](crate::system::operator::bridge::Tree::land_deep) 与持树者的 `settle`
//! 那一支（`programs/src/system/operator/server.rs::land_plate`）。
//!
//! # `/sys/operator` 自己不是一格
//!
//! 它是这条路上**顺手造出来的目录**（第一帧 `Layer::Segment` 时就地立出 `/sys/operator` 那块
//! `Pane`，其后每面幂等取回）——**没有它自己的入口、没有它的 Pie、也不是任何能力的别名**。故
//! `seek("/sys/operator")` 对客人答 [`Fail::NotATile`](protocol::system::operator::Fail::NotATile)：
//! 那一段是块窗格，到头了的是它底下那七格。
//!
//! **照实记（"第八格"是量出来的）**：这一版之前，那段目录**也铸了一枚自己的孔**，并走
//! `land_deep(segment, segment, "operator", "operator")` 挂上去——两帧里第二帧是 `Under`，
//! 于是持树者 `part /sys/operator` 之后**又往那一段里落了一格也叫 `operator` 的**。实机读数：
//! `/sys/operator` 底下**八格**（七位 ＋ 一格自己，且那一格上挂着目录那枚孔），与上面这段话
//! 正相反。它一直没显形，是因为旧探针**按名字数**（只数那七段名字，多出来的一格不进账）；
//! 改成"数格子"（`harness/src/probe_operator_gate.rs` 的 `count_under`）第一跑就撞上。
//! 今天目录**不铸孔、不落叶子**：一帧 [`Tree::land_segment`](crate::system::operator::bridge::Tree::land_segment)
//! 只说"把这一段立成 `Pane`"。
//!
//! # 入口为什么要长命
//!
//! 铸这七枚的是**本域主线程**（它此后就进监督那一趟），不是一枚一次性线程：内核那条派生链
//! （`accord` 写 `sire`、`doom` 沿 `sire` 级联摘后代）会把短命铸造者的副本一并摘下 ⇒ 树上
//! 查得到、门闩取不回。那三处内核事实与实测见
//! [`Assembly::supervise`](crate::system::Assembly::supervise) 那一节。

use env::{Name, PieToken};
use protocol::system::operator as ocall;
use runtime::env::mail;

/// **那一段目录的名字**（`/sys/operator` 底下那一段）：七位各自的 `dir` 与目录自己那一段是
/// **同一个**——故只有这一处写它。
pub const SEGMENT: &str = "operator";

/// 那段目录的名字那一枚 [`Name`]（`/sys/operator` 自己**不是一格**，故只有名字，没有孔）。
pub fn segment() -> Result<Name, &'static str> {
    Name::new(SEGMENT).map_err(|_| "operator:name")
}

/// **铸某一位的待客入口**，并交出它要落的两段名字（`dir` / `name`）。
///
/// `dir` 是 **`/sys/operator` 底下那一段的名字**（落手见 [`Tree::land_deep`]）；`name` 是
/// 这位面自己那一段（`Grant::name`）。
///
/// 返 `Err(哪一步)`：记号铸不出 / 名字非法。对调用方是同一件事（这一面没挂上），但"死在哪一步"
/// 正是诊断要的那一格。
///
/// **每一位只铸一枚**：这一枚此后就是 `/sys/operator/{name}` 那一格背后那一枚；铸第二枚就会
/// 有一枚永远没人读它的推（与 `control` 那一面同款）。
///
/// [`Tree::land_deep`]: crate::system::operator::bridge::Tree::land_deep
pub fn entry(grant: ocall::Grant) -> Result<(PieToken, Name, Name), &'static str> {
    let entry = mail::unseal_hole(grant.mark()).map_err(|_| "operator:grant")?;
    // **这一段名字说的是 `/sys/operator` 底下**（`land_deep` 那一手先走 `/sys/operator`）。
    let dir = segment()?;
    let name = Name::new(grant.name()).map_err(|_| "operator:name")?;
    Ok((entry, dir, name))
}
