//! inner — **本域里的三枚内件**（iii：它们与编排者共用一份字节、各占一枚线程）。
//!
//! ```text
//!   Role::Tree      operator     持树者   客人上树要它在
//!   Role::Roster    principal    名册     身份从它来（名册还是身份那把钥匙）
//!   Role::League    coalition    盟册     它是名册的客人
//! ```
//!
//! **照实记（这三行原先在 `env::assembly::ALL` 上）**：那时 principal / coalition / operator
//! 各是一个**程序**（自己的 bin、自己的域、自己的 `[[bin]]`）。iii 之后它们与编排者**共用
//! 一份字节**（`prog-system`），在**本域**里各占一枚线程 ⇒ 它们不是清单里的东西（镜像里没有
//! 它们的字节），但仍是"本域要起的东西" ⇒ 搬出来，由本域的装配单（`scenario.rs`）与镜像里
//! 那几台**接成一条名册**。
//!
//! **照实记（特权级）**：principal 与 coalition 原先是 **U 态**（"不持有、不授予、不解释任何
//! Pie"）。住进编排域之后它们**随域**变成 S 态——这是用户裁定接受的代价（iii 那一句
//! "接受 U 态那两台升 S 态"）；同域四枚线程**共享一张页表**是这条路的另一半代价。
//!
//! **照实记（这张表为什么住 lib，而不是随装配单住 bin）**：读它的有**两处，且不同 crate**——
//! ① bin 的装配单（`scenario.rs`：把它接在镜像那几台前面，接成一条名册）；② **板那本账的界**
//! （`board/desk.rs` 的 `Desk::CAP` 要数"内件里有几位上板"）。住在 bin 里，后者数不到，那个
//! 界就只剩一个手挑的数。

use env::assembly::{Announce, Eyes};

use crate::supervisor::service::{Program, Role};

/// 内件起手失败的三枚号（与原先 `env::assembly` 上那三枚**同值**）。
mod died {
    use env::assembly::Died;
    pub const E_TREE: Died = 10;
    pub const E_PRINCIPAL: Died = 14;
    pub const E_COALITION: Died = 16;
}
use died::{E_COALITION, E_PRINCIPAL, E_TREE};

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
            // **它上板**（乙那一刀）：板据此看得见这一枚的死——与同域另两枚内件（名册 / 盟册）
            // **同形**。照实记（原先它是 `false`）：那时"上板"确实还不等于"看得见死"——板把
            // "谁 → 道"记在 **REGISTER** 那一刻，而道按名字认领，三枚内件都不登记 ⇒ 三枚的
            // 死编排域**一个都看不见**（实测：让盟册在起手之后死掉，`board: swept n=1` 有、
            // `system: gone coalition` 一条都没有）。乙2 把名字的来源从客人挪到**装配者**
            // （提示那一格多带一格名字）⇒ 此后 `board: true` 就是"板看得见它的死"。
            // 价格照实：板的账多占一格——那一格今天由装配单数出来（见 `Desk::CAP`）。
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

/// 内件里**上板**的那几枚——板那本账的界要它（见 `board::desk::Desk::CAP`）。
///
/// **一处定义**：这里数的是同一张 [`INNER`]，不是另抄一句"三枚都上板"。
pub const fn boarded() -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < INNER.len() {
        if INNER[i].1.board {
            n += 1;
        }
        i += 1;
    }
    n
}
