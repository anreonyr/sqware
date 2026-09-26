//! inner — **本域里的三枚内件**（iii：它们与编排者共用一份字节、各占一枚线程）。
//!
//! ```text
//!   Role::Tree      operator     持树者   客人上树要它在
//!   Role::Roster    principal    名册     身份从它来（名册还是身份那把钥匙）
//!   Role::League    coalition    盟册     它是名册的客人
//! ```
//!
//! **照实记（这三行原先在 `plan::assembly::ALL` 上）**：那时 principal / coalition / operator
//! 各是一个**程序**（自己的 bin、自己的域、自己的 `[[bin]]`）。iii 之后它们与编排者**共用
//! 一份字节**（`prog-system`），在**本域**里各占一枚线程 ⇒ 它们不是清单里的东西（镜像里没有
//! 它们的字节），但仍是"本域要起的东西" ⇒ 搬出来，由本域的装配单（`scenario.rs`）与镜像里
//! 那几台**接成一条名册**。
//!
//! **照实记（特权级）**：principal 与 coalition 原先是 **U 态**（"不持有、不授予、不解释任何
//! Pie"）。住进编排域之后它们**随域**变成 S 态——这是用户裁定接受的代价（iii 那一句
//! "接受 U 态那两台升 S 态"）；同域四枚线程**共享一张页表**是这条路的另一半代价。
//!
//! **照实记（这张表为什么住 lib，而不随 bin 走）**：iii 之后它一度与 `scenario.rs` 分居两处
//! （一个在 lib、一个在 bin），而读它的正是那张装配单——**两张表本是一件事**（"内件三枚接在
//! 镜像那几台前面"）。这一刀把它们并进本目录 `assemble/`：**文件夹就是那条判据**。
//! 名字仍叫 `inner`（不并进 `mod.rs`）：它是**一张表 + 一条名册**，与"怎么把一行 `Row` 投成
//! `Program`"（[`super::plan`]）是两件事。
//!
//! **照实记（原来还有第二个读者，它已退场）**：那个读者是**板那本账的界**——`Desk::CAP` 要数
//! "内件里有几位上板"，而它住在 bin 里数不到 ⇒ 那个界就只剩一个手挑的数。两本客人账并成一本、
//! 那个常数退场之后这里不再是"两处、不同 crate"。留这段是因为它记的是**一个界曾经靠装配事实
//! 撑着**——同一个教训现在写在 `contract::system::desk` 的并本记里。

use alloc::vec::Vec;

use plan::assembly::{Announce, Eyes};
use plan::assembly::{E_COALITION, E_PRINCIPAL, E_TREE};

use crate::service::{Program, Role};

// **照实记（那三枚号搬回装配单了）**：这里原有一个私有 `mod died`，握着 `E_TREE` /
// `E_PRINCIPAL` / `E_COALITION` = 10 / 14 / 16（"由那一处自己持有"）。**结果是两套号在跑**：
// 装配期失败答 10/14/16（`Plan::died` 这一格），而三枚内件**自己起手失败**（`serve()`）答的是
// 各域 `fail::Fail` 的 1..5——`operator` 的 `Sire` 甚至占了 `E_BOOT` 的 1。故三枚号搬回
// `plan::assembly`（与其余每一台同一条规矩：**号在装配单里**），本表照旧填。见那一处的照实记。

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
            // 价格照实：板上多占一位客人一格（客人账按需增长，再没有常数上限）。
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

// **照实记（那个界，以及它和这条断言一起退场）**：这里原先住着 `boarded_rows()` /
// `boarded()` 与末尾一条 `const _: () = assert!(boarded_rows() + boarded() <= Desk::CAP)`，
// 治的是一个真病——**板那本账满了是静默的**（`admit` 答 `Full`、调用方 `let _ =`，那一位
// 从此没人监督，日志一句话都没有）。挡它的办法原本是**一个上限**（`Desk::CAP`）＋一条编译期
// 断言。两本客人账并成一本、容器改成按需增长之后**那个病不存在了**：满了如实报 `Full`，
// 而两侧都在那一格上留了读数（板：`board: swept … occupied=…`；树：`operator: desk full`）
// ⇒ 断言、数它的两条函数、以及"数出这个界"的那条来路一并退场。
// **代价照实**：这件病从此**没有编译期的挡板**——它换成了"运行期如实报 + 有人读那一行"。

/// **本次要起的全部成员**，按起手次序：**内件三枚在前，镜像里那几台在后**。
///
/// **一处定义**：铸死亡道那一侧（`main.rs`）与起它们那一侧（`service::assemble`）都只读它
/// ——两处各排一遍次序，正是 `scenario.rs` 记过的那条"下标＝道位次"耦合的温床。
///
/// **住这里有三个出处**：它是**两张表接起来**的那一手（[`INNER`] 接 [`super::plan`]）、
/// 它的读者在 bin 那一侧（`main.rs`），而 `INNER` 本身是 lib 的。三者同住本文件，接线只有一处。
pub fn roster(catalog: &super::Catalog) -> Vec<(Option<Role>, Program)> {
    let mut all: Vec<(Option<Role>, Program)> =
        INNER.iter().map(|(role, p)| (Some(*role), *p)).collect();
    all.extend(super::plan(catalog).into_iter().map(|p| (None, p)));
    all
}
