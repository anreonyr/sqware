//! operator::core::judge —— **门外那一问**：这一位许不许动这一格。**不带载体、不碰内核。**
//!
//! **照实记（它原先住 `protocol::system::operator::core::judge`）**：那一份同时养着两半——
//! **上线的两样**（`Permit` / `Ruling`，归 protocol）与**判据那一半**（`Facts` 四问 ＋
//! `judge`）。判据的读者只有本域，故它回实现侧；两侧之间的缝就是那一个 [`Facts`]。
//!
//! 三格答案的意义（`Allow` / `Deny` / `Unjudged`）与"哪些因会好"照实记在协议那一侧
//! （`protocol::system::operator::frame` 的 `Ruling`）。

use env::TaskId;

use protocol::service::coalition::CoalitionId;
use protocol::system::operator::{EntryId, Permit, Ruling};
use protocol::service::principal::PrincipalId;

// ── 门外那一问要问的四条边（**一个** trait）─────────────────

/// 名册 / 谱系 / 盟册 / 树——**判一格要问的全部事实**，四条边一个出口。
///
/// **照实记（这一层原先有两重，已折平）**：从前是四枚单方法 trait（`Who` / `Branch` /
/// `League` / `Door`），而 `gate::Facts<'a, C>` 再把 `gate::Control` 的四问逐个转发过去
/// ——同一条链上两层 trait 说同一件事。它们存在的理由只有一个**已经删掉的宿主靶**
/// （"换一份假实现就能单独推理"）；那台靶一走，留下的就是纯粹的绕路。故按"一个动作不许有
/// 两套类型"折成**这一个** trait：`judge` 直接问它，`gate::verdict` 也直接收它。
///
/// 四格的读法（**逐字保留原样**）：
///
/// - [`Facts::who`]：`Ok(None)` = 没绑（**不是失败**，是"没有身份"）；`Err` = 问不到
///   （⇒ [`Ruling::Unjudged`]）。
/// - [`Facts::heir`]：`Err` = 问不到（⇒ `Unjudged`）；`Ok(false)` = 不在这一支里（⇒ `Deny`）。
/// - [`Facts::amid`]：**两格都要**（盟册那一问本来就是"谁在哪个盟里"）。
/// - [`Facts::opens`]：`Ok(Some(tid))` = 那一格是枚 `Tile`、开者是 `tid`；`Ok(None)` =
///   **没有那一位**——碑 / 那一号是块 `Pane` / 开者那扇门封印了（**三因同落**：判据只需要
///   "有没有那一位"这一件事）。后两因**永远好不了**，与"对面暂时不答"同落 ⇒ 都判
///   [`Ruling::Unjudged`]。把三因分开的是**读数**，不是码：树那一侧本来就分得开
///   （`Operator::opens` 答 `Unknown` / `NotATile` / `Dead`），由
///   `programs/src/system/operator/server.rs` 的 `Court::opens` 把它们说进读数。
///   `Err(())` = 树自己问不到 ⇒ `Unjudged`（**今天没有生产者**：`Court::opens` 一律返
///   `Ok(None)`，它连 `_` 都不写，就为了将来 `Fail` 多一格时**编不过**；留这一格是因为
///   四条边同形，缺一条就不是"四问"了）。
///
/// 答的是 **TID 不是号**：树的读答"谁开的这扇门"（内核戳），名册那一边答"这个 TID 是谁"
/// ——两件事两个落点，故 `Permit::Opener` 那一格要走两问。
///
/// **四问的荷载是真的类型**（`PrincipalId` / `CoalitionId` / `EntryId`）：两个号空间本来不同型，
/// 混用是编译错误——故这几格的签名各自点名，不经一个裸 `u64` 中转。
pub trait Facts {
    fn who(&self, tid: TaskId) -> Result<Option<PrincipalId>, ()>;
    fn heir(&self, a: PrincipalId, b: PrincipalId) -> Result<bool, ()>;
    fn amid(&self, me: PrincipalId, at: CoalitionId) -> Result<bool, ()>;
    fn opens(&self, at: EntryId) -> Result<Option<TaskId>, ()>;
}

// ── 那一问 ─────────────────────────────────────────────────

/// 判一格。`who` 是内核在 `Push` 那一刻盖的章；`permit` 是那一格自己那一句话。
///
/// **先问身份**——这一格是契约的一半：
///
/// - 先问：`who` 答不出就当场 [`Ruling::Deny`]，后面那几条边**一次都不发**（省一次 envcalls，
///   也让"没身份"与"不在支里"不会混成同一个答案）；
/// - **只问一次**——[`Permit::Opener`] 那一格是唯一的例外：它要**再问一次名册**，问的是**另一条
///   TID**（那一格的开者）"此刻代表谁"。这是晚绑定的代价，写在签名边上，不藏在实现里
///   （照实记：这一句原来写的是"只问一次"，一个字不含糊；加第五格时它被破了，故改口径）。
///
/// **`Unset` 只许住下面那一格**：它是"这一格没记许可"，判据到此为止（要的事实就是上面那一问
/// 的答案）。**不许把它提到 `who` 之前**——那样"没绑身份"就不再是 [`Ruling::Deny`]，
/// 那道门会静默变成放行。
pub fn judge(f: &impl Facts, who: TaskId, permit: Permit) -> Ruling {
    // 一、谁在问。没绑 ⇒ 没资格（编排域落的正是这一格）；**问不到 ⇒ 判不了**。
    let Some(me) = (match f.who(who) {
        Ok(found) => found,
        Err(()) => return Ruling::Unjudged,
    }) else {
        return Ruling::Deny;
    };
    // 二、照那一句话问那一条边。**三种"答不是"都是 `Ok(false)`，不是失败**。
    match permit {
        Permit::Unset => Ruling::Allow,
        Permit::Trunk(p) => allow(me == p),
        Permit::Bough(p) => match f.heir(p, me) {
            Ok(true) => Ruling::Allow,
            Ok(false) => Ruling::Deny,
            Err(()) => Ruling::Unjudged,
        },
        Permit::Among(c) => match f.amid(me, c) {
            Ok(true) => Ruling::Allow,
            Ok(false) => Ruling::Deny,
            Err(()) => Ruling::Unjudged,
        },
        // 三、**两问**：先问树"那一格谁开着"，再问名册"那位此刻代表谁"。两问的失败域各自落格，
        //    与上面几条同一分法：**"没有那一位"是判不了**（三因同落，其中两因永久，见 [`Facts::opens`]），
        //    **"那一位没身份"是终态拒**。
        Permit::Opener(at) => match f.opens(at) {
            Ok(Some(that)) => match f.who(that) {
                Ok(Some(theirs)) => allow(me == theirs),
                Ok(None) => Ruling::Deny,
                Err(()) => Ruling::Unjudged,
            },
            Ok(None) => Ruling::Unjudged,
            Err(()) => Ruling::Unjudged,
        },
    }
}

const fn allow(ok: bool) -> Ruling {
    if ok { Ruling::Allow } else { Ruling::Deny }
}
