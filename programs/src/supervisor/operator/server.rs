//! operator::server — **持树者**：自己的域里的一枚线程守着那棵树（一枚线程 + 一个组，无轮询）
//!
//! 三侧分家之后本文件只放**持树者**：自己的域里的一枚线程守着那棵树（一枚线程 + 一个组，无轮询）；两侧共用的图与说明见 [`super`] 的"载体"那一节，
//! 帧与记号见 [`protocol::operator::call`]。

use env::{HoleDir, Name, PieToken, TaskId};
use runtime::core::port::{self, Access, Policy};
use runtime::core::tole::Tole;
use runtime::env::mail;
use runtime::env::room::exit_with;
use runtime::env::unit as utask;

use protocol::operator::call as ocall;
pub use protocol::operator::{ASK_MARK, LINK, TIP_MARK};
use protocol::operator::{Fail, Operator};

use super::desk::{Desk, Guest, desk};

/// 还在"补齐两本账"（答话路未认领 / 问话孔未挂上）时，一轮等多久（毫秒）。
///
/// **不是轮询**：账补齐之后这一等就变成 `usize::MAX`（由组唤醒）；这个短期限只在装配窗口
/// 里用——那几步的到达是**别人**在做（装配者转授、客人自己交孔）。
const SETTLE_MS: usize = 1;

/// 持树者起不来时的编号（指"死在头几步的哪一步"）。
const E_SIRE: usize = 1;
const E_TIP: usize = 2;
const E_GROUP: usize = 3;

// ── 持树者侧（本域的服务线程）────────────────────────────────

/// 起服务：**铸提示孔交给装配者，然后一枚线程招待所有客人**。
///
/// 头两步是契约：装配者按 `(本域, tip)` 两格认领提示孔（[`attach`] 的 `host_of`），
/// 而提示一到它就认为"答话路必已在本表里"（转授在前、提示在后）。
pub fn serve() -> ! {
    let Ok(assembler) = utask::sire() else {
        say("operator: no sire");
        exit_with(E_SIRE);
    };
    // 提示孔：本线程铸的那一枚（客人号从这里进来），副本交给生我者。**记号 = `tip`**。
    let Ok(tip) = mail::unseal_hole(TIP_MARK) else {
        say("operator: no tip");
        exit_with(E_TIP);
    };
    let tip_hole = mail::HolePie::from_token(tip);
    if port::ship(
        &tip_hole,
        assembler,
        Access::FETCH | Access::STORE,
        Policy::VEST,
    )
    .is_err()
    {
        say("operator: tip not handed");
        exit_with(E_TIP);
    }
    // **一个组**：提示孔 + 每位客人的问话孔。提示孔也挂进来，故"来客人了"与"有人问话"
    // 是**同一个等待**。本线程独享它（`shared = false`）。
    let Ok(tole) = Tole::unseal(false) else {
        say("operator: no group");
        exit_with(E_GROUP);
    };
    if tole.attach(&tip_hole, HoleDir::Pull).is_err() {
        say("operator: tip not hung");
        exit_with(E_GROUP);
    }

    let mut tree = ocall::tree();
    let mut desk = desk();
    loop {
        // 一、补齐两件事（收提示 + 认领答话路、认出问话孔并挂组）。
        let settling = settle(&mut desk, &tole, &tip_hole);
        // 二、等一格有事。**一个等待**：提示孔或任意一位客人的问话孔。
        let ms = if settling { SETTLE_MS } else { usize::MAX };
        let Ok(Some((tok, _dir))) = tole.await_(ms) else {
            let _ = desk.sweep();
            continue;
        };
        // 提示孔那一格由下一轮的 `settle` 收（它非阻塞地拉）；这里只管"是哪位客人的问话孔"。
        if tok != tip
            && let Some(guest) = desk.guest(tok).copied()
        {
            serve_one(&mut tree, guest);
        }
        // 三、**看出来的**那一档：那一枚答不出 ⇒ 剔格子（没有"他说走了"那一档）。
        let _ = desk.sweep();
    }
}

/// 补齐两件事，返"还有没有没补齐的"。
///
/// - **提示**：装配者推来的客人号（一个号，8 字节）。**非阻塞地拉**——必须在这里拉，
///   不能只在"组唤醒"那一支拉：装配者的推**可能早于本线程把提示孔挂进组**（那一条推
///   落在一个还没有转发登记的站点上），醒不来就得靠这一拉吃到它；
/// - **答话路**：装配者转授来的那一枚 ⇒ `admit` 收一位客人；
/// - **问话孔**：客人**自己**交来的那一枚 ⇒ 认出来就 `arm` + 挂进组。
fn settle(desk: &mut Desk, tole: &Tole, tip: &mail::HolePie) -> bool {
    // 提示：拉干净（单槽，一位客人一条）。**非阻塞**——它的到达是别人在做的事。
    let mut id = [0u8; 8];
    let mut pending = false;
    while let Ok(8) = tip.pull_timeout(&mut id, 0) {
        let client = TaskId::new(u64::from_le_bytes(id) as usize);
        match reply_of(client) {
            // 已经在账上（提示是单槽，可能重放）：不换掉原来那位。
            Some(reply) => {
                let _ = desk.admit(client, reply);
            }
            // 次序被破坏（提示先到、答话路不在本表里）：报一句；客人那边会报它自己的超时。
            None => say("operator: no reply"),
        }
    }
    // 先抄一份"还没挂上的"：`unarmed` 借住这本账，而下面要改它。
    let mut waiting = [(0usize, TaskId::new(0)); Desk::CAP];
    let mut n = 0;
    for (slot, who) in desk.unarmed() {
        waiting[n] = (slot, who);
        n += 1;
    }
    for &(slot, who) in &waiting[..n] {
        match ask_of(who) {
            Some(ask) => {
                let hung = desk.arm(slot, ask).is_ok()
                    && tole
                        .attach(&mail::HolePie::from_token(ask), HoleDir::Pull)
                        .is_ok();
                if !hung {
                    let _ = desk.unarm(slot);
                    pending = true;
                }
            }
            None => pending = true,
        }
    }
    pending
}

/// 招待一位客人：从**它的问话孔**读一帧、交给树、把答话推进**它的答话路**。
///
/// 组已经说了"这一枚有话"，故这一读读得动；期限给 `0` 是**再确认**，不是轮询。
fn serve_one(tree: &mut Operator, guest: Guest) {
    let Some(ask) = guest.ask() else {
        return;
    };
    let mut buf = [0u8; ocall::ASK];
    let Ok(n) = mail::HolePie::from_token(ask).pull_timeout(&mut buf, 0) else {
        return;
    };
    let Some(want) = buf.get(..n) else {
        return;
    };
    let answer = answer(tree, want, guest.who());
    let _ = mail::HolePie::from_token(guest.reply()).push(&answer);
}

/// 把一句问交给树，编出一句答（**一格**：读不懂也答，答 `BAD`）。
///
/// **先读动作码、再解载荷**；`land` 那一码**必须带入口号**（没带就是一句读不懂的 `file`：
/// 不猜、不崩）。
fn answer(tree: &mut Operator, want: &[u8], who: TaskId) -> [u8; 1] {
    let Some(op) = ocall::op_of(want) else {
        return [ocall::BAD];
    };
    let Some((segs, count, seed)) = ocall::unpack(want) else {
        return [ocall::BAD];
    };
    // 路太长：**先按上限挡掉**，别把一条被截断的路当成真的（核心那四条也各有这条判据）。
    if count > Operator::PATH_MAX {
        return [ocall::FULL];
    }
    let path = &segs[..count];
    // 一句没带入口号的 `land`：这不是"树上没有这一段"，是**这一问读不懂** ⇒ `BAD`。
    if op == ocall::LAND && seed == PieToken::NONE {
        return [ocall::BAD];
    }
    let said = match op {
        ocall::LAND => tree.land(path, seed),
        ocall::PART => tree.part(path),
        // 查到就**把树上那一份转授给客人**：Pie 不从报文里走，从会话里走。
        // "查不到"与"授不出去"是两件事，故查的结论优先（`.and`）。
        ocall::FIND => {
            let mut grant = Ok(());
            tree.find(path, |pie| {
                grant = ocall::give(pie, who).map(|_| ());
            })
            .and(grant)
        }
        ocall::TRIM => tree.trim(path),
        // 没见过的动作码：与"这条路上没有这一段"同一句话（不另立一格）。
        _ => Err(Fail::Unknown),
    };
    [ocall::code(said.err())]
}

/// 转授来的那一枚答话路（**写端**，落在本表里）。
///
/// 两格判据，都是确定的号：
///
/// - `owner == who` —— **谁的**：那扇门是**这位客人**开的（副本共享同一事实）；
/// - **记号 == `operator`** —— 那一枚是**树路**上的一枚。
///
/// **照实记（分开之后的改动）**：原来还有第三格 `vestor == 装配者`——"谁转的"。它随
/// **装配者是谁**而失效：树这一侧现在有两个域跟它打交道（引导域起它、编排域接客人），
/// 而它认的本来就是"**这扇门是谁开的**、**走的哪条路**"，不是"谁转的"。次序那件事仍由
/// `settle` 管（答话路没到就先报一句，见那里）。
fn reply_of(who: TaskId) -> Option<PieToken> {
    let link = Name::new(LINK).ok()?;
    let mut index = 0usize;
    loop {
        let (token, _perm, vestor) = mail::collect(index).ok()?;
        // 越界哨兵：这一遍扫完了。
        if token.get() == 0 {
            return None;
        }
        index += 1;
        let _ = vestor;
        if ocall::opened_by(token) == Some(who) && ocall::mark_of(token) == Some(link) {
            return Some(token);
        }
    }
}

/// 这一位客人**自己**交来的那一枚问话孔。
///
/// 判据两格，缺一不可：`owner == who`（那扇门是它开的）**且** 记号 == `ask`（它亲手铸的
/// 那一枚）——客人交来的**入口**也满足前两格（都是它铸、它交的），两件事只有记号分得开。
fn ask_of(who: TaskId) -> Option<PieToken> {
    let ask = Name::new(ASK_MARK).ok()?;
    let mut index = 0usize;
    loop {
        let (token, _perm, _vestor) = mail::collect(index).ok()?;
        // 越界哨兵：这一遍扫完了。
        if token.get() == 0 {
            return None;
        }
        index += 1;
        if ocall::opened_by(token) == Some(who) && ocall::mark_of(token) == Some(ask) {
            return Some(token);
        }
    }
}

/// 持树者的读数：**只在出岔子时说话**（正常一轮什么都不打）。
fn say(msg: &str) {
    let _ = runtime::env::debug::put(msg);
}
