//! system::mount — **挂面那一趟的两件共同的事**：把一枚记号取成孔、把一段面名取成 [`String`]。
//!
//! ```text
//!   entry(mark, name) → (那一枚孔, 那一段面名)     一原语一面，两面各一枚
//! ```
//!
//! **它为什么是一处**（照实记，量出来的）：这一手从前**四份逐字同构**——`operator` /
//! `principal` / `coalition` / `control` 各一份 `mount.rs`，逐行比差的只有两样：族的 `Grant`
//! 类型、与错误那个字面量。仓里那条规矩"**两台以上逐字同构 ⇒ 收**"（`board/client.rs` 那句
//! 原文）到第二台就该动手，而它一直拖到第四台——故这一刀把它收在这里。
//!
//! **收它不需要给四族的 `Grant` 立一个共享 trait**（照实记：那条路被否）：它们各自那枚枚举上
//! 已经有 `name()` / `mark()`（[`faces!`](protocol::faces) 生成的），调用方**把这两个值传进来**
//! 就够——不必新造一个类型名（而"面"那个名字已经被 `client::Face`（客侧那具柄）占了）。
//!
//! **错误那两格不带族名**（照实记）：从前的字面量是 `"operator:grant"` / `"control:entry"`
//! 这样的族前缀，而四个调用点里**两个印、两个丢**；印的那两处外面本来就写着族名
//! （`system: control face not mounted (…)`）⇒ 前缀在那里是**同一件事说两遍**。故这一手只答
//! "死在哪一步"：`"grant"`（记号取不回）或 `"name"`（面名非法）。
//!
//! **本文件不管往树上立路**：那是装配者那一趟的事（组路在 `Assembly::mount_control` /
//! `mount_grants`，递上去在 [`Tree::plate`](crate::system::operator::bridge::Tree::plate)，
//! 落由持树者自己走）。各族那一段路归**协议侧那一族的 `DIR`**（`/svc/sys/<族>`）——装配侧只引它。

use alloc::string::String;
use alloc::string::ToString;
use env::Mark;
use env::PieToken;
use runtime::env::mail;

/// **铸某一面的待客入口**，并交出它**自己那一段名字**（`/svc/{族}/{面名}` 的末段）。
///
/// 两步：记号 → 那一枚孔（`mail::unseal_hole`），面名 → 一枚 [`String`]。
///
/// 返 `Err(哪一步)`：`"grant"`（记号铸不出）或 `"name"`（面名非法）。对调用方是同一件事
/// （这一面没挂上），但"死在哪一步"正是诊断要的那一格。
///
/// **每一面只铸一枚**：这一枚此后就是 `/svc/{族}/{面名}` 那一格背后那一枚；铸第二枚就会有一枚
/// 永远没人读它的推（与"入口为什么要长命"同一条照实记，见 `Assembly::mount_control`）。
pub fn entry(mark: Mark, name: &'static str) -> Result<(PieToken, String), &'static str> {
    let entry = mail::unseal_hole(mark).map_err(|_| "grant")?;
    let name = name.to_string();
    Ok((entry, name))
}
