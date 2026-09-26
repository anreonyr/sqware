//! inner — **本域里的三枚内件**（iii：它们与编排者共用一份字节、各占一枚线程）。
//!
//! ```text
//!   Role::Tree      operator     持树者   客人上树要它在
//!   Role::Roster    principal    名册     身份从它来（名册还是身份那把钥匙）
//!   Role::League    coalition    盟册     它是名册的客人
//! ```
//!
//! 次序即契约：持树者排第一（客人上树要它在），名册第二（其后的身份都从它来），盟册第三
//! （它是名册的客人）。这三位与镜像那几台的 `order`（3..18）相接之后，与从前的次序**逐字相同**。
//!
//! 三枚的 `setup` **都是空的**：内件不领配给、不开通道（它们与编排者同域，起来就是起来）。
//! **边**都在 [super::Edges] 上：三枚都上板（板据此看得见它们的死），名册与盟册上树、
//! 名册与盟册是那两双眼睛，持树者是 `holds_tree` 那一位。

use plan::assembly::{E_COALITION, E_PRINCIPAL, E_TREE, Eyes};

use crate::system::program::{Program, Role, Source};

use super::{Edges, Node};

/// **内件三条**：住本域的四枚线程里，除编排者自己以外那三枚。
pub const INNER: &[Node] = &[
    Node {
        program: Program {
            name: "operator",
            source: Source::Here(Role::Tree),
            setup: &[],
        },
        edges: Edges {
            // **它上板**：板据此看得见这一枚的死——与同域另两枚内件（名册 / 盟册）**同形**。
            board: true,
            operator: false,
            bind: true,
            holds_tree: true,
            eyes: None,
            died: E_TREE,
        },
    },
    Node {
        program: Program {
            name: "principal",
            source: Source::Here(Role::Roster),
            setup: &[],
        },
        edges: Edges {
            board: true,
            operator: true,
            bind: true,
            holds_tree: false,
            eyes: Some(Eyes::Roster),
            died: E_PRINCIPAL,
        },
    },
    Node {
        program: Program {
            name: "coalition",
            source: Source::Here(Role::League),
            setup: &[],
        },
        edges: Edges {
            board: true,
            operator: true,
            bind: true,
            holds_tree: false,
            eyes: Some(Eyes::League),
            died: E_COALITION,
        },
    },
];
