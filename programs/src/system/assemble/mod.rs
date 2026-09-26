//! assemble — **装配那一份数据**：这一景起哪些程序、每一条的装配参数。
//!
//! 本目录把"投影"与"内件表"两件事并在一起：
//!
//!   - **本文件 = 投影**：一行 `plan::assembly::Row` → 编排域认识的 [`Program`]（[`plan`] 与
//!     [`plan_row`]）；
//!   - **[`inner`] = 内件三枚那张表 + 把两张表接成一条名册的那一手**（[`roster`] 从那里转出）。

use alloc::vec::Vec;

use crate::service::{Catalog, Program};

pub mod inner;

/// 这一景的装配单：**从 `plan::assembly::ALL` 派生**——`plan: Some` 的那些行里、**这张镜像真有的**
/// 那些，按 `order` 排。
///
/// 装配单是"本域认识的全部台"，镜像是"这一次真装了哪些"——两者**不必相等**（`product` 那一景
/// 只有 7 台，五台探针与六位常客都不在里面）。
///
/// **装配单**：本域按这个顺序起服务。
///
/// 持树者（`operator`）**排第一**：它是**服务**，但每位上树的客人都要它在——起来之后本域
/// 当场把它那条提示之路认到手（[`service::assemble`] 的第二段）。
///
/// 身份服务（`principal`）**紧随其后**：装配期每一条服务的 `derive` + `bind` 都要它在
/// （[`service::assemble`] 在它放行之后补绑它自己与树，其后的每一条都在放行前拿到身份）。
///
/// 结盟服务（`coalition`）**跟在身份服务之后**：它是身份服务的客人（起手按名字找
/// `/sys/principal`），故只能在它之后起——这也是本域能给的唯一次序保证（那一台自己还带一轮
/// 有界的重试，见 [`coalition`] 那一格）。
///
/// `echo` **必须在最后**：[`service::assemble`] 返**名册**的最后一条（`roster.last()`），本域等它退场
/// ——那正是"读到一行 `exit` 才收场"的那一格。**它得是"上板"的那一条**（`board: true`），
/// 板才看得见它的死。
///
/// 三台驱动紧跟在身份服务之后、其余之前：控制器先就位，线再开闸（`uart` / `rtc` 持有那两台设备）。
/// `sleeper` 排在 `lodger` 之后、`subject` 之前：它要找的那块门牌 `/device/rtc` 由 `rtc` 落。
pub fn plan(catalog: &Catalog) -> Vec<Program> {
    let mut rows: Vec<&plan::assembly::Row> = plan::assembly::ALL
        .iter()
        .filter(|row| row.plan.is_some() && catalog.find(row.name).is_some())
        .collect();
    rows.sort_by_key(|row| row.plan.as_ref().map(|p| p.order));
    rows.iter()
        .filter_map(|row| row.plan.as_ref().map(|p| plan_row(row.name, p)))
        .collect()
}

/// 装配单的一行 → 编排域认识的 [`Program`]（装配参数逐格搬，名字取自那一行）。
fn plan_row(name: &'static str, p: &plan::assembly::Plan) -> Program {
    Program {
        name,
        announce: p.announce,
        tokens: p.tokens,
        channels: p.channels,
        needs: p.needs,
        board: p.board,
        operator: p.operator,
        bind: p.bind,
        holds_tree: p.holds_tree,
        eyes: p.eyes,
        died: p.died,
    }
}

// ── 内件三枚（iii：住本域的那三枚）─────────────────────────────
//
// 它们那张**表**（`INNER`）与**把两张表接起来**的那一手（`roster`）住同目录的 `inner.rs`：
// 读它的是**本文件**（把内件三枚接在镜像那几台前面，接成一条名册）。

pub use inner::{INNER, roster};
