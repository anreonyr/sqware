//! control::mount — **把这一面挂上树**：`/sys/control` 那一格。
//!
//! 三件事，而**只有第一件在本域手里落下**：
//!
//! ```text
//!   ① 铸入口   本域主线程自己铸（记号 = 服务入口那一格，与三枚服务同一格）
//!   ② 递出去   交给持树者（`R|W ＋ VEST`），再把"落哪一块、叫什么"推上提示之路
//!   ③ 落       持树者在自己核里 `part /sys` ＋ `land /sys/control`（`Permit::Unset`）
//! ```
//!
//! ① 在本文件里（那一枚的记号与它那一段名字是**这份协议自己的事实**）；②③ 分别在
//! [`Tree::plate`](crate::system::operator::bridge::Tree::plate) 与持树者那一支
//! （`programs/src/system/operator/plate.rs::plate`）。
//!
//! # 照实记（"谁上树"这一格换过三次）
//!
//! | 那一版 | 谁把这一格落上树 | 死在哪 |
//! |---|---|---|
//! | task-4 | 一枚**一次性**边沿线程 | 它一收尾，持树者表里那枚入口副本被内核的派生链级联摘掉（三处证据见 [`crate::system::Assembly::supervise`]） |
//! | 上一版 | **装配者本人**当客人（要会话、要名册上那一行） | 能跑，但"客人"这份名单里多了一位不是域的东西，且装配者为此进了名册 |
//! | 这一版 | **持树者自己**（在自己核里落） | —— |
//!
//! 今天这一版里**没有第三方上树**：装配者递东西（那一枚 ＋ 两段名字），持树者落格——树是
//! 那一格的权威，而它当不了自己的客人（自指 ⇒ 环）。
//!
//! **本手不自问自答**：那一格落成没有、指不指得回原物，由**真客人**证——
//! `harness/src/probe_control.rs` 照 principal / coalition 同形的路找上门、问一句 control 的话。

use env::{Name, PieToken};
use protocol::system::board::ENTRY_MARK;
use protocol::system::control as ccall;
use runtime::env::mail;

/// **铸本域那一枚待客入口**，并交出它**自己那一段**名字（`/sys/{name}` 的 `name`）。
///
/// 路的第一段（`sys`）不由本文件给：那是**两族共用**的那一格坐标（[`ccall::frame::DIR`]），
/// 由装配者组路时给——本文件从前把它也交出去（一格 `dir`），而收帧那一侧**从来不读它**。
///
/// 返 `Err(哪一步)`：记号铸不出 / 名字非法。对调用方是同一件事（这一面没挂上），但"死在哪一步"
/// 正是诊断要的那一格。
///
/// **只铸一次**：这一枚此后就是监督那一趟那只组里的待客入口（`Watch` 的 `face` 那一格）；
/// 铸第二枚，就会有一枚永远没人读它的推。
pub fn entry() -> Result<(PieToken, Name), &'static str> {
    let entry = mail::unseal_hole(ENTRY_MARK).map_err(|_| "control:entry")?;
    let name = Name::new(ccall::frame::NAME).map_err(|_| "control:name")?;
    Ok((entry, name))
}
