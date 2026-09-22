// cull — 级联撤销：摘掉一枚门闩及其全部后代。
//
// 与结构面的 `messenger::cull` 同构同名：那边沿 `heir`（强引用）扑杀子域，
// 这边沿 `sire` 反查（查询面）撤销子树。三处共用：`revoke` / `release` / `doom`。
//
// 闭包靠 `snap::heirs` 逐层反查——**不存 heir 列表**（一条关系只存一次）。
//
// 锁纪律：调用方不得持任何 L3。本模块逐任务取放 `Task.pies`（绝不嵌套）；
// 摘除在锁内、**门闩在锁外 drop**（最后一份会跑 `Meta::drop`），Pole 撤映射在
// 全部摘完之后、同样无锁。

use alloc::sync::Arc;
use alloc::vec::Vec;

use env::{PieToken, TaskId};

use crate::work::mail::hole::{self, HoleMeta};
use crate::work::mail::nole::{self, NoleMeta};
use crate::work::mail::pole::{self, PoleMeta};
use crate::work::mail::tole::{self, ToleMeta};
use crate::work::unit::task::Task;

use super::pie::AnyPie;
use super::snap::{self, Snap};

/// 摘掉 `t` 表里 `token` 那枚，返回摘下的门闩（表里没有 → None）。
///
/// **调用方须在锁外 drop 返回值**：它可能是资源实体的最后一份强引用。
fn take(t: &Task, token: PieToken) -> Option<AnyPie> {
    let mut pies = t.pies.lock();
    let pos = pies.iter().position(|p| p.token() == token)?;
    Some(pies.remove(pos))
}

/// Pole 门闩的资源实体（Hole → None）。**先取强引用、后 drop 门闩**：即便这是
/// 最后一份，unmap 时 Meta 仍活。
fn pole_meta(pie: &AnyPie) -> Option<Arc<PoleMeta>> {
    match pie {
        AnyPie::Pole(p) => Some(p.meta().clone()),
        // Hole 无映射可撤；Nole 连载荷都没有——两者都不需要"先取强引用"这一步。
        // Hole 无映射可撤；Nole 与 Tole 连载荷都没有——都不需要「先取强引用」这一步。
        AnyPie::Hole(_) | AnyPie::Nole(_) | AnyPie::Tole(_) => None,
    }
}

/// 级联撤销：摘掉 `root`（`task` 表里 `token` 那枚）及其全部后代，并撤销各自
/// 已建立的 Pole 映射。
///
/// 前置：调用方不持任何 L3 锁。
/// 返回：摘掉的门闩数（含 root 本身；root 不在表里则为后代数）。
///
/// # 容量不够就**整个不做**，而不是做一半
///
/// 本条在**退场钩子**里跑（`Hook = fn(usize)`，无错误通道），而它要的每一张表
/// （`unmaps` / `frontier` / `next` / [`snap::heirs`] 的返回）都靠堆。就地逐个
/// `try_reserve` 会把"摘一半"变成可达状态：根已摘、后代的 `unmaps` 没记全 ⇒
/// **Pole 映射泄漏**（映射还在、门闩没了），而那比"这次不级联"糟得多。
///
/// 故**入口处一次备足**，备不出来直接返回 0（一枚未摘，状态原封不动）：
/// 失败是原子的，代价只是这一次放弃级联——内存缓过来后下次退场仍会级联。
pub(crate) fn cull(root: (Arc<Task>, PieToken), snap: &Snap) -> usize {
    let (root_task, root_token) = root;
    let mut removed = 0usize;
    let mut unmaps: Vec<(Arc<PoleMeta>, PieToken)> = Vec::new();

    // 上限估计：门闩总数 = 各任务 `pies` 长度之和。取不到容量 ⇒ 放弃本次。
    // （`snap` 是只读快照，长度在本次调用内不变，故这个上界是自洽的。）
    if unmaps.try_reserve(snap.len()).is_err() {
        return 0;
    }
    let mut frontier: Vec<PieToken> = Vec::new();
    if frontier.try_reserve(1).is_err() {
        return 0;
    }
    frontier.push(root_token);

    // 1. 摘根。
    if let Some(pie) = take(&root_task, root_token) {
        removed += 1;
        let meta = pole_meta(&pie);
        drop(pie); // 锁外：最后一份会跑 Meta::drop
        if let Some(m) = meta {
            unmaps.push((m, root_token));
        }
    }

    // 2. BFS：逐层反查 `sire ∈ frontier` 的子门闩。
    while !frontier.is_empty() {
        let mut next: Vec<PieToken> = Vec::new();
        let mut scan: Vec<(Arc<Task>, PieToken)> = Vec::new();
        for f in frontier.drain(..) {
            // 查询面容量不够 ⇒ 本层到此为止（已摘的照常记着，稍后统一撤映射）。
            let Some(kin) = snap::heirs(f, snap) else {
                break;
            };
            if scan.try_reserve(kin.len()).is_err() {
                break;
            }
            scan.extend(kin);
        }
        for (t, token) in scan {
            if let Some(pie) = take(&t, token) {
                removed += 1;
                if next.try_reserve(1).is_err() {
                    break;
                }
                next.push(token);
                let meta = pole_meta(&pie);
                drop(pie);
                if let Some(m) = meta {
                    unmaps.push((m, token));
                }
            }
        }
        frontier = next;
    }

    // 3. 无锁段：逐条撤 Pole 映射（幂等；未映射、Space 已死都返 `Ok`）。
    // `let _ =`：`space.release` 仍可能 `Denied`（段已不在），而这是收尾路径——
    // 一条撤不掉不该短路掉后面那些。（`pole::shut` 已不判资源存活：撤自己的图与
    // 资源活不活着无关，故这里曾要吞掉的那个 `Dead` 不存在了。）
    for (meta, token) in unmaps {
        let _ = pole::shut(&meta, token);
    }

    removed
}

/// 退出钩子：任务消亡时，它名下每一枚门闩各自 `cull`（派生链随其断）。
///
/// 签名受 `Hook = fn(usize)` 约束，故自取快照并在其中按 id 找到该任务
/// （此刻收尾者 `reap` 仍持强引用，Weak 可升级）——**不查调度器**。
///
/// # 资源寿命边：开者退场 ⇒ 它开的资源一起封印
///
/// `owner` 原是"谁能 `Seal`"的判据（envcall 适配层读它）。这里给它**第二重读法**：
/// 一枚资源挂在它的开辟者身上——开者退场，它开的每一件资源一起封印（[`seal_owned`]）。
///
/// 为什么非要这条边：资源寿命 = 能力寿命（最后一份门闩消失即回收，见 `pie.rs` 头注），
/// 于是一件"**开者是甲方、主用者是乙方**"的资源，乙方死了它**不死**——甲方的门闩还在，
/// 孔还活着，只是再没人会往里推。等在它上面的乙方拿到的是永久的 `Busy`，不是 `Dead`：
/// 调用方分不清"对端还没回"与"对端已经没了"。
///
/// 实证是控制台会话的回信孔：它服务于"服务端出话"，却由**收话的客户端**开
/// （`runtime::core::port::Port::open` 自建）⇒ 服务被打死后客户端永久挂住。
/// 那条路已在协议层翻面（回信孔改由服务端开），本条是它成立的**机制一半**。
///
/// # 实测（探针已撤：读数在这里，代码里不留打印点）
///
/// 当年那对探针（封印点 `[z1]`、唤醒点 `[z2]`）打出的读数——
/// 封印**确实发生**，且与 `wipe` 一一对上：
///
/// ```text
/// [z1] seal hole id=23 … 36            ← 开者退场时它开的每一枚孔都被封印
/// [z2] wipe hole id=24 dir=Pull woke=1 ← 有人挂着时**当场醒**（链是通的）
/// [z2] wipe hole id=23 dir=Pull woke=0 ← 没人挂着时自然是 0（语义正确，不是漏唤醒）
/// ```
///
/// 故"等它的人当场醒"这条承诺**成立**，但它只在"那一刻正好有人挂着"时兑现；
/// 客户端是"醒了才发现"，不是"被叫醒"——**这不是缺陷**：`messenger::wait` 第 ④ 步
/// 入链时锁内判死活（`Life::dead`），死在挂起之后到达的等待者当场返回 `Dead`。
/// 据此，本仓**不需要**再往客户端加"看门狗"式的补丁去接"服务没了"这件事。
///
/// 一条不肯放过的次序：**封印在 cull 之前**。反过来的话，`cull` 摘掉根那枚、若它
/// 正好是 `Arc<Meta>` 的最后一份强引用，`Meta::drop` 自己就会置死并唤醒——效果相同，
/// 但那是**引用计数的巧合**：一旦别处还留着一份副本，"开者退场 ⇒ 资源死"就静默失效。
pub(crate) fn doom(tid: TaskId) {
    let snap = snap::snap();
    let Some(task) = snap::find(tid, &snap) else {
        return;
    };
    // 先摘出 token 清单（放锁），再逐个 cull——cull 内部会再取同一张表的锁。
    //
    // **清单可失败**：本条是 `Hook = fn(usize)`（**每个任务退场都跑一次**，签名
    // 里没有错误通道），故"这里不可失败"就等于"一个任务退场时堆一紧 ⇒ 整机
    // halt"。容量备不出来就**放弃级联**——与本条开头"找不到该任务就 `return`"
    // 是同一条语义（不做，而不是崩）。
    let tokens: Vec<PieToken> = {
        let pies = task.pies.lock();
        let mut v: Vec<PieToken> = Vec::new();
        if v.try_reserve(pies.len()).is_err() {
            return;
        }
        v.extend(pies.iter().map(|p| p.token()));
        v
    };
    // 寿命边：先封印（次序是判据，见头注），再断派生链。
    seal_owned(tid, &task);
    for token in tokens {
        cull((task.clone(), token), &snap);
    }
}

/// 把 `tid` **开的**那些资源封印掉（`owner == tid`），返回封印了几件。
///
/// 读 Meta 上那个字段（不由门闩推）：`owner()` 带 `alive()` 闸、封印后答不出
/// "谁开的"，而这里问的正是那个事实（死没死另说，封印幂等）。
///
/// **`owner == 0` 不参与**（内核自建，没有"开者退场"这回事），`tid == 0` 也进不来
/// （0 是"无任务"，真出现会一次封印掉全部内核资源——故显式挡在门外，而不是靠
/// "0 号任务永不退场"这条口头约定）。
///
/// # 摘不摘表项
///
/// **不摘**。本条只做 `seal`（`mail::hole`/`pole`/`nole` 各有一个，同一个语义）：
/// 置死 + 唤醒两个方向的全部等待者；表项仍由各持有者 `Release` 自己收（与 `Seal`
/// 同一条契约，见 `envcall::pie::seal` 头注）。这儿跟着 `cull` 摘一遍会与 `cull`
/// 撞车（同一枚摘两次），且把"封印"与"回收"两件事混成一件。
///
/// # 不可失败
///
/// 与 [`cull`] 同款：本条在**退场钩子**里（`Hook = fn(usize)`，无错误通道），
/// 分配失败 ⇒ 这一次封印整个不做（[`cull`] 会补上"最后一份门闩消失即回收"那条
/// 既有路径），而不是拿整机去换一次封印。
fn seal_owned(tid: TaskId, task: &Arc<Task>) -> usize {
    if tid.get() == 0 {
        return 0;
    }
    // 先挑出"我开的"那些 token（持自己那张表）。**不就地封印**：`doom` 的调用链上
    // `reap` 无锁，而持表锁调 `messenger::wipe`（L3）就是 3→3 自己撞自己。
    let owned: Vec<PieToken> = {
        let pies = task.pies.lock();
        let mut v: Vec<PieToken> = Vec::new();
        if v.try_reserve(pies.len()).is_err() {
            return 0;
        }
        v.extend(
            pies.iter()
                .filter(|p| p.owner_task() == tid)
                .map(|p| p.token()),
        );
        v
    };
    let mut sealed = 0;
    for token in owned {
        // 表在 `doom` 里不变（`reap` 之后没人再进这张任务表），故按 token 找得到。
        let meta = {
            let pies = task.pies.lock();
            pies.iter().find(|p| p.token() == token).map(|p| match p {
                AnyPie::Hole(h) => Resource::Hole(h.meta().clone()),
                AnyPie::Pole(pl) => Resource::Pole(pl.meta().clone()),
                AnyPie::Nole(n) => Resource::Nole(n.meta().clone()),
                AnyPie::Tole(t) => Resource::Tole(t.meta().clone()),
            })
        };
        // **锁外**封印：`seal` 只碰 Meta 自己的锁 + 唤醒站点，不需要任务表。
        match meta {
            Some(Resource::Hole(m)) => {
                hole::seal(&m);
                sealed += 1;
            }
            Some(Resource::Pole(m)) => {
                pole::seal(&m);
                sealed += 1;
            }
            Some(Resource::Nole(m)) => {
                nole::seal(&m);
                sealed += 1;
            }
            Some(Resource::Tole(m)) => {
                tole::seal(&m);
                sealed += 1;
            }
            None => {}
        }
    }
    sealed
}

/// 一件资源实体的强引用（封印要用它，且必须**先取出来、后放表锁**）。
///
/// 与 [`pole_meta`] 分开而不合并：那个只认 Pole（撤映射要它），这个三种都要
/// （封印三种都要），而两者都从同一枚门闩上取——合起来会让 Pole 多带一个"我是不是
/// 为了封印才取的"参数。
enum Resource {
    Hole(Arc<HoleMeta>),
    Pole(Arc<PoleMeta>),
    Nole(Arc<NoleMeta>),
    Tole(Arc<ToleMeta>),
}
