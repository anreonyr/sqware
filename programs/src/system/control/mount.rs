//! control::mount — **把这一面挂上树**：`/sys/control` 那一格。
//!
//! ```text
//!   ① 铸入口：本线程自己那一枚（记号 = 服务入口那一格，与三枚服务同一格）
//!   ② 上树：分 `/sys` → 落 `/sys/control` → 查回来验一遍（同 principal / coalition 那一趟）
//! ```
//!
//! **这一趟与 principal / coalition 那两趟逐句同源**，只有两点是这一档特有的，两点都在
//! "谁在铸"上：
//!
//! - **入口由这一枚线程自己铸**（不是由一枚一边铸一边死的边沿线程）——树表里那份副本的派生边
//!   指着**铸它的那一枚线程**（`kernel/src/work/unit/gate/accord.rs` 的 `sire`），而每个 reaped
//!   任务都跑 `gate::doom`、`cull` 又沿 `snap::heirs` **跨任务**摘后代（`kernel/src/boot.rs` 的
//!   `EXIT_HOOKS` ＋ `kernel/src/work/unit/gate/cull.rs`）⇒ **铸入口的那一枚必须长命**。本手
//!   的调用者是编排域主线程，它此后就进监督那一趟（本域活多久它活多久）——这一条就是原先
//!   那条挂载路死掉的原因，也是这一刀成立的原因。原委写在 [`crate::system::Assembly::supervise`]
//!   的照实记里。
//! - **会话是自己给自己开的**（本域既是装配者、又是这棵树的客人）：见
//!   [`Tree::self_session`](crate::system::operator::bridge::Tree::self_session)。
//!
//! **本手不自问自答**：这里那一趟只到"树上查得回那一枚"为止（读数是 `got`）；"它指不指得回
//! 原物"由**真客人**证——`harness/src/probe_control.rs` 照同一条路找上门、问一句 control 的话。

use env::Wait;
use env::{Name, PieToken};
use protocol::debug;
use protocol::communication::session::Session;
use protocol::system::board::ENTRY_MARK;
use protocol::system::control as ccall;
use protocol::system::operator::client::{Face, Mine};
use protocol::system::operator::Rule;
use runtime::env::mail;

/// 挂载那几趟的额度（毫秒）。**必须有界**：持树者死在头几步时本域不能陪着挂死。
///
/// 第一趟（`part`）还要额外留出"持树者刚把本域认成客人"那一点时间：提示是本域自己在
/// [`Tree::self_session`] 里推的，而持树者按 ≤1 ms 的节拍补齐两本账
/// （`programs/src/system/operator/server.rs` 的 `SETTLE_MS`）——本域那一问**等在门外**，
/// 它一到就醒。
const MOUNT_MS: usize = 1000;

/// **把这一面挂到 `/sys/control`**，答那一枚待客的入口。
///
/// 返 `Err(哪一步)`：名字非法 / 树那边四趟任一没成。对调用方是同一件事（这一面没挂上），
/// 但"死在哪一步"正是诊断要的那一格。
pub fn mount(session: &Session) -> Result<PieToken, &'static str> {
    let tree = Face::from(session);
    // 一、**本线程自己那一枚入口**：与三枚服务同一格记号（`entry`）。
    //
    // **只铸一次**：这一枚此后是本域的表里那一枚待客的孔（`Watch::face`），铸第二枚就会有
    // 一枚永远没人读它的推（同 `operator/server.rs::claim` 那条"一个域只铸一枚"）。
    let entry = mail::unseal_hole(ENTRY_MARK).map_err(|_| "control:entry")?;
    let dir = Name::new(ccall::frame::DIR).map_err(|_| "control:name")?;
    let me = Name::new(ccall::frame::NAME).map_err(|_| "control:name")?;
    // 二、分目录 → 落门牌 → 查回来验一遍：分与落各自**答出那一格的号**（"号出门"那一手）。
    //
    // **分目录**：`open` 是**幂等**的——`/sys` 早已由 principal / coalition 立起来（它们排在
    // 前头），重复 `part` 只答同一个号。
    let root = tree.root();
    let at = root
        .open(dir, Wait::AtMost(MOUNT_MS))
        .map_err(|_| "control:part")?;
    // **落门牌**：`e` 是客人手里那一枚；`bind` 顺手把它经会话交给持树者（`R|W ＋ VEST`——
    // 客人此后从树上取回自己那一份时要能再授出，见 `Pane::bind`）。`Rule::Public` 与
    // principal / coalition 那两块门牌同一格：**任何已绑身份都取得回**。
    let plate = at
        .bind(me, entry, Rule::Public, Mine::No, Wait::AtMost(MOUNT_MS))
        .map_err(|_| "control:land")?;
    // 查回来验一遍：**按号**（名字只在上面那两格用过，此后一律按号）——把那一枚要回来。
    let got = match tree.tile(&[dir, me], Wait::AtMost(MOUNT_MS)) {
        Ok(tile) => tile.token(Wait::AtMost(MOUNT_MS)).is_ok(),
        Err(_) => false,
    };
    debug!(
        "control: mount dir={} plate={} entry={} find={got}",
        dir.as_str(),
        plate.id().get(),
        entry.get(),
    );
    Ok(entry)
}
