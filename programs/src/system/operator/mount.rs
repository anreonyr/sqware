//! operator::mount — **这一族的坐标与铸**：`/sys/operator` 那一段叫什么、七位各自的入口怎么铸。
//!
//! 本文件只有**两条事实**：目录那一段的名字（[`SEGMENT`]）与七位各自那一段（[`Grant`]）。往树上
//! 立那条路不在这里——那是装配者那一趟的事（组路在
//! [`crate::system::Assembly::mount_grants`]、递上去在
//! [`Tree::plate`](crate::system::operator::bridge::Tree::plate)、落由持树者自己走）。
//!
//! # `/sys/operator` 自己不是一格
//!
//! 它是那条路上的**一段前缀**（第一位的路走前缀时就地把它立成一块 `Pane`）——**没有它自己的
//! 入口、没有它的 Pie、也不是任何能力的别名**。故 `seek("/sys/operator")` 对客人答
//! [`Fail::NotATile`](protocol::system::operator::Fail::NotATile)：那一段是块窗格，到头了的是
//! 它底下那七格。
//!
//! **照实记（"第八格"是量出来的，而它现在写不出来）**：从前的帧是"两段名字 ＋ 一格 `layer`"，
//! 目录与七位共用"两帧"那一手，而目录那两段名字是同一个（`"operator"`）⇒ 第二帧又往它里面
//! 落了一格也叫 `operator` 的。实机读数：`/sys/operator` 底下**八格**。今天一条路是**段列表**、
//! 末段由 `leaf` 定，而目录**根本不由谁单独立一帧**——它是第一位那条路的**前缀**（`part` 幂等）
//! ⇒"目录自己也是它底下的一格"**在形状上写不出来**，不必靠断言挡。
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

/// **那一段目录的名字**（`/sys/operator` 底下那一段）：七位各自那条路的**中间那一段**，与目录
/// 自己那一段是同一个——故只有这一处写它。
pub const SEGMENT: &str = "operator";

/// 那段目录的名字那一枚 [`Name`]（`/sys/operator` 自己**不是一格**，故只有名字，没有孔）。
pub fn segment() -> Result<Name, &'static str> {
    Name::new(SEGMENT).map_err(|_| "operator:name")
}

/// **铸某一位的待客入口**，并交出它**自己那一段**名字（`/sys/operator/{name}` 的 `name`）。
///
/// 路的第一段（`sys`）不由本文件给：那是**两族共用**的那一格坐标（正文是
/// [`ccall::frame::DIR`](protocol::system::control::frame::DIR)），由装配者组路时给。
///
/// 返 `Err(哪一步)`：记号铸不出 / 名字非法。对调用方是同一件事（这一面没挂上），但"死在哪一步"
/// 正是诊断要的那一格。
///
/// **每一位只铸一枚**：这一枚此后就是 `/sys/operator/{name}` 那一格背后那一枚；铸第二枚就会
/// 有一枚永远没人读它的推（与 `control` 那一面同款）。
pub fn entry(grant: ocall::Grant) -> Result<(PieToken, Name), &'static str> {
    let entry = mail::unseal_hole(grant.mark()).map_err(|_| "operator:grant")?;
    let name = Name::new(grant.name()).map_err(|_| "operator:name")?;
    Ok((entry, name))
}
