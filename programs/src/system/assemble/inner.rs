//! inner — **本域里的三枚内件**（iii：它们与编排者共用一份字节、各占一枚线程）。
//!
//! ```text
//!   Role::Tree      operator     持树者   客人上树要它在
//!   Role::Roster    principal    名册     身份从它来（名册还是身份那把钥匙）
//!   Role::League    coalition    盟册     它是名册的客人
//! ```

use alloc::vec::Vec;

use plan::assembly::{Announce, Eyes};
use plan::assembly::{E_COALITION, E_PRINCIPAL, E_TREE};

use crate::service::{Program, Role};

/// **内件三条**：住本域的四枚线程里，除编排者自己以外那三枚。
///
/// 次序即契约：持树者排第一（客人上树要它在），名册第二（其后的身份都从它来），盟册第三
/// （它是名册的客人）。这三位与镜像那几台的 `order`（3..18）相接之后，与从前的次序**逐字相同**。
///
/// 装配参数里那几格（`announce` / `tokens` / `channels` / `needs`）对它们**都是空的**：
/// 内件不领配给、不开通道、不宣布"我起来了"——它们与编排者同域，起来就是起来。
pub const INNER: &[(Role, Program)] = &[
    (
        Role::Tree,
        Program {
            name: "operator",
            announce: Announce::None,
            tokens: &[],
            channels: &[],
            needs: None,
            // **它上板**：板据此看得见这一枚的死——与同域另两枚内件（名册 / 盟册）**同形**。
            board: true,
            operator: false,
            bind: true,
            holds_tree: true,
            eyes: None,
            died: E_TREE,
        },
    ),
    (
        Role::Roster,
        Program {
            name: "principal",
            announce: Announce::None,
            tokens: &[],
            channels: &[],
            needs: None,
            board: true,
            operator: true,
            bind: true,
            holds_tree: false,
            eyes: Some(Eyes::Roster),
            died: E_PRINCIPAL,
        },
    ),
    (
        Role::League,
        Program {
            name: "coalition",
            announce: Announce::None,
            tokens: &[],
            channels: &[],
            needs: None,
            board: true,
            operator: true,
            bind: true,
            holds_tree: false,
            eyes: Some(Eyes::League),
            died: E_COALITION,
        },
    ),
];

/// **本次要起的全部成员**，按起手次序：**内件三枚在前，镜像里那几台在后**。
///
/// **一处定义**：铸死亡道那一侧（`main.rs`）与起它们那一侧（`service::assemble`）都只读它。
///
/// **住这里有三个出处**：它是**两张表接起来**的那一手（[`INNER`] 接 [`super::plan`]）、
/// 它的读者在 bin 那一侧（`main.rs`），而 `INNER` 本身是 lib 的。三者同住本文件，接线只有一处。
pub fn roster(catalog: &super::Catalog) -> Vec<(Option<Role>, Program)> {
    let mut all: Vec<(Option<Role>, Program)> =
        INNER.iter().map(|(role, p)| (Some(*role), *p)).collect();
    all.extend(super::plan(catalog).into_iter().map(|p| (None, p)));
    all
}
