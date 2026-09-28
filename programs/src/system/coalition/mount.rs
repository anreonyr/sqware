//! coalition::mount — **这一族的坐标与铸**：`/sys/coalition` 那一段叫什么、两面各自的入口怎么铸。
//!
//! 本文件只有**两条事实**：目录那一段的名字（[`SEGMENT`]）与两面各自那一段（[`Grant`]）。往树上
//! 立那条路不在这里——那是本域自己那一趟（[`super::server::serve`] 的 `serve_tree`：它既是这一族
//! 的服务端，也是这一族在树上的落牌人）。
//!
//! # `/sys/coalition` 自己不是一格（开面那一刀换过一格）
//!
//! 照实记：这一格**从前就是那一枚门牌**（`/sys/coalition` 是一枚 `Tile`，谁 `seek` 到它谁就拿到
//! 整面）。开面之后它变成那条路上的**一段前缀**（第一条路的前缀由落牌那一趟就地立成一块
//! `Pane`）——**没有它自己的入口、没有它的 Pie、也不是任何能力的别名**。故
//! `seek("/sys/coalition")` 此后答 [`NotATile`](protocol::system::operator::Fail::NotATile)：那一段
//! 是块窗格，到头了的是它底下那两格。同 `/sys/operator` 与 `/sys/principal` 那两格。
//!
//! # 入口为什么要长命
//!
//! 铸这两枚的是**本域主线程**，不是一枚一次性线程：内核那条派生链（`accord` 写 `sire`、`doom`
//! 沿 `sire` 级联摘后代）会把短命铸造者的副本一并摘下 ⇒ 树上查得到、门闩取不回。那三处内核
//! 事实与实测见 [`Assembly::supervise`](crate::system::Assembly::supervise) 那一节。

use env::{Name, PieToken};
use protocol::system::coalition as ccall;
use runtime::env::mail;

/// **那一段目录的名字**（`/sys/coalition` 底下那一段）：两面各自那条路的**中间那一段**，与目录
/// 自己那一段是同一个——故它**只有一处**（协议那一侧那一格：[`ccall::NAME`]），这里不重抄。
pub const SEGMENT: &str = ccall::NAME;

/// 那段目录的名字那一枚 [`Name`]（`/sys/coalition` 自己**不是一格**，故只有名字，没有孔）。
pub fn segment() -> Result<Name, &'static str> {
    Name::new(SEGMENT).map_err(|_| "coalition:name")
}

/// **铸某一面的待客入口**，并交出它**自己那一段**名字（`/sys/coalition/{name}` 的 `name`）。
///
/// 路的第一段（`sys`）不由本文件给：那是各族共用那一格坐标（[`ccall::DIR`]），由落牌那一趟组路
/// 时给。
///
/// **每一面只铸一枚**：这一枚此后就是 `/sys/coalition/{name}` 那一格背后那一枚，也是交给持树者
/// 的那一枚（按 `(开者, 记号)` 查回来）；铸第二枚就会有一枚永远没人读它的推。
pub fn entry(grant: ccall::Grant) -> Result<(PieToken, Name), &'static str> {
    let entry = mail::unseal_hole(grant.mark()).map_err(|_| "coalition:grant")?;
    let name = Name::new(grant.name()).map_err(|_| "coalition:name")?;
    Ok((entry, name))
}
