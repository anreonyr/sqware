//! scenario — **这一景起哪些程序**（装配单本身）。
//!
//! # 为什么装配单单独立一份源（照实记：用户裁定"测试和程序分开"）
//!
//! 这张表原先住 `system/main.rs`——编排的**机器**那一份正文里：15 份产品与 5 台试客
//! （`probe-*`）都在里面（外加按场景选的第二张表）。于是"某一景装哪些台"这件事，写在了产品
//! 程序的正文里。用户的原话是**"为什么 programs 里面的测试和程序混在一起？我不希望这样"**。
//!
//! 故场景归场景，三处各归各位：
//!
//!   - **本文件 = 数据**：一行台 = 一个 [`Program`]（名字 / 宣布 / 通道 / 需求 / 上不上板……）；
//!   - **`system/main.rs` = 机器**：登记、按序起、监督、收尾——它不认识具体哪一台；
//!   - **哪几台进哪张镜像** = `plan::assembly::ALL` 的 `scenes` 那一格（**装配单只有一处**）。
//!
//! **照实记（指标与"道"的位次按位耦合）**：装配单的下标就是"道"的位次（`supervise` 按
//! `plan.get(i)` 把道上响的那一位翻回名字）。故两张表必须**各自自洽**——第一版想"表里插一条
//! 空名字的行、调用点过滤"，默认台当场以 `system: manifest bad` 收场（soak 逮住）。

use alloc::vec::Vec;

use super::*;

use programs::service::Role;
use programs::system::inner::INNER;

/// 这一景的装配单：**从 `plan::assembly::ALL` 派生**——`plan: Some` 的那些行里、**这张镜像真有的**
/// 那些，按 `order` 排。
///
/// **照实记（为什么清单要从外面传进来）**：装配单是"本域认识的全部台"，镜像是"这一次真装了哪些"
/// ——两者**不必相等**（`product` 那一景只有 7 台，五台探针与六位常客都不在里面）。从前按整张表起
/// 之所以没出事，是因为验收镜像恰好装了表里每一条：那是**巧合**，不是契约。今天不相等了，而
/// [`assemble`](super::service::assemble) 那一条契约（单子上每一条都必须在镜像里，缺一条报
/// `E_PROGRAM`）**一个字都不改**——改的是**派生出哪张单**：单只取两边都有的那些。
///
/// **照实记（这里原先有 18 个 `const fn` ＋一份次序）**：那时 `kernel/build.rs` 另有三张同名的
/// 表（`PRODUCTS` / `PROBES` / `RIGS`），同一条事实写两处——名字、特权级、在不在表里都得改两遍。
/// 用户原话：**"我不想每次加一个 bin 就写一个装配表"**。现在**装配单只有一处**，本文件只把它
/// 的"装配参数 + 起手位次"投影成编排域认识的 [`Program`]。
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
/// 板才看得见它的死。照实记：试过把探针放最后，机器**不再停机**——探针不上板，那一等没人应。
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
        .filter_map(|row| row.plan.as_ref().map(|p| of(row.name, p)))
        .collect()
}

/// 装配单的一行 → 编排域认识的 [`Program`]（装配参数逐格搬，名字取自那一行）。
///
/// **照实记（为什么是自由函数，不是 `Program::of`）**：`Program` 住 **lib**（`service`），
/// 而本文件是 **bin** `prog-system` 的一部分 ⇒ inherent impl 落在"类型所属 crate 之外"，
/// `E0116` 当场拒绝。自由函数不受这条约束。
fn of(name: &'static str, p: &plan::assembly::Plan) -> Program {
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
// 它们那张**表**（`INNER`）搬去本域的实现侧（`inner.rs`）了：读它的是**本文件**（把它接在
// 镜像那几台前面，接成一条名册）。
//
// **照实记（原来还有第二个读者，它已退场）**：那个读者是**板那本账的界**（`Desk::CAP` 要数
// "内件里有几位上板"），住在 bin 里数不到 ⇒ 那个界就只剩一个手挑的数。两本客人账并成一本、
// 那个常数退场之后（见 `contract::system::desk` 的并本记），这里只剩本文件一处。

/// **本次要起的全部成员**，按起手次序：**内件三枚在前，镜像里那几台在后**。
///
/// **一处定义**：铸死亡道那一侧（`main.rs`）与起它们那一侧（`service::assemble`）都只读它
/// ——两处各排一遍次序，正是本文件记过的那条"下标＝道位次"耦合的温床。
pub fn roster(catalog: &Catalog) -> Vec<(Option<Role>, Program)> {
    let mut all: Vec<(Option<Role>, Program)> =
        INNER.iter().map(|(role, p)| (Some(*role), *p)).collect();
    all.extend(plan(catalog).into_iter().map(|p| (None, p)));
    all
}
