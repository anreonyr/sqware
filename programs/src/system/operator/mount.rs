//! operator::mount — **把一位操作面挂上树**：`/sys/operator/{part,land,…}` 那一格。
//!
//! 与 [`control::mount`](crate::system::control::mount) 同形三件事，而**只有第一件在本域手里落下**：
//!
//! ```text
//!   ① 铸入口   本域主线程自己铸（记号 = 那一位的记号，见 grant）
//!   ② 递出去   交给持树者（`R|W ＋ VEST`），再把"落哪一块、叫什么"推上提示之路
//!   ③ 落       持树者在自己核里 `part /sys` ＋ `land /sys/operator/{name}`
//! ```
//!
//! ① 在本文件里（**那一位的记号与那一段名字只有一处**：`Grant`）；②③ 分别在
//! [`Tree::land_plate`](crate::system::operator::bridge::Tree::land_plate) 与持树者的 `settle`
//! 那一支（`programs/src/system/operator/server.rs::land_plate`）。
//!
//! # `/sys/operator` 自己不是一格
//!
//! 它是这条路上**顺手造出来的目录**（第一面 `part sys` 时就地立出 `/sys/operator` 那块 `Pane`，
//! 其后六面幂等取回）——**没有它自己的入口、没有它的 Pie、也不是任何能力的别名**。故
//! `seek("/sys/operator")` 对客人答 [`Fail::NotATile`](protocol::system::operator::Fail::NotATile)：
//! 那一段是块窗格，到头了的是它底下那七格。
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

/// `/sys/operator` 那一格背后的孔的记号——**本族自己的坐标**（与七位面名都不相同，见下面
/// 那条编译期断言）。
pub const PANE_MARK: env::Mark = env::Mark::of("operator-pane");

/// **铸那段目录自己的那一格**（`/sys/operator`）：两段名字 ＋ 一枚本域自己的孔。
///
/// **它不是任何能力的别名**：它是一块**实打实的 `Pane`**（由本域铸一枚自己的孔当它的去处，
/// 由持树者落格，七位落在它底下）。故 `seek("/sys/operator")` 对客人答
/// [`Fail::NotATile`](protocol::system::operator::Fail::NotATile)——那是一块窗格，到头了的是
/// 它底下那七格；而 `/sys/operator/land` 是一条**普通**的名字 → 号。
///
/// **为什么它也要一枚自己的孔**：`land_deep` 那一手与 [`Self::entry`] 同形——它落的是
/// "一枚 -> 一个名字"，而树上那一格是哪一种（`Pane` 还是 `Tile`）由落的时候给。给一枚孔，
/// 这一段目录此后就在树上**有个名称、有枚门闩**，与 `/sys/control` 那一格同形（不是路径上的
/// 一段假层）。
pub fn pane() -> Result<(PieToken, Name, Name), &'static str> {
    let hole = mail::unseal_hole(PANE_MARK).map_err(|_| "operator:grant")?;
    let dir = Name::new("sys").map_err(|_| "operator:name")?;
    let name = Name::new("operator").map_err(|_| "operator:name")?;
    Ok((hole, dir, name))
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
    let dir = Name::new("operator").map_err(|_| "operator:name")?;
    let name = Name::new(grant.name()).map_err(|_| "operator:name")?;
    Ok((entry, dir, name))
}

// **不相撞**：这一枚记号与七位面名（`Grant::mark`）都不同——与 `grant.rs` 那一条同一句正文。
const _: () = assert!(PANE_MARK.get() != env::Mark::of("operator-ask-land").get());
const _: () = assert!(PANE_MARK.get() != env::Mark::of("operator-ask").get());
