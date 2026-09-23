#![no_std]
#![no_main]

//! probe-rule — **规矩那一格的证客**：一位**有身份**的任务把"这一格谁许用"落成
//! `Rule::Is` / `Under` / `In`，然后**自己按身份试三遍**，最后**换一条身份再试三遍**。
//!
//! 门禁那一刀的正文里，"用"那一轴（谁许用这一格）有**四条**判据（公开 / 就是某一位 /
//! 在某一位那一支里 / 在某枚盟里），而上一刀的真机上只有一条通电：`DEFAULT_RULE` 是一条全局
//! 常量，`Is` / `Under` / `In` 三条**一次没被问过**。本程序把它们搬到真机上——一台客人演两个
//! 身份，故"规矩随**身份**走、不随 TID 走"这一条也在同一行读数里。
//!
//! ```text
//!   0  上树 + 取两面门牌（名册那一枚 + **盟册那一枚**）
//!   1  p = resolve(self)；q = derive(p)            —— 我是 p，我底下还有一个 q
//!   2  分 /sys/rule；落三格：
//!        is     Rule::Is(p)
//!        under  Rule::Under(p)
//!        in     Rule::In(c)      （c 是本域刚立、刚入的那一枚盟）
//!   3  以 p 试三遍   ⇒ find 全答 OK(0)              —— 三条**正证**
//!   4  adopt(q)      —— **同一条 TID，换了一位代表**
//!   5  以 q 再试三遍 ⇒ is 答 DENIED(8)、in 答 DENIED(8)、under 仍答 OK(0)
//!      —— 前两条是**负证**（有身份、但不是那一位 / 不在那枚盟里），
//!         第三条是"`Under` 看的是**支**，不是相等"的正证（q 仍在 p 那一支里）。
//!   6  报一行读数就退场
//! ```
//!
//! # 为什么"另一台客人"也要来（`prog-probe-rule-other`）
//!
//! `Under(p)` 的**负证**在同一个域里做不到：`q = derive(p)` 一定在 p 那一支里，而 `adopt`
//! 只许**往下**领（`heir(current, q)`）。"不在那一支里"的那一位只能是**另一台**——那正是
//! `probe-rule-other` 那一格。
//!
//! # 这一台为什么把盟也带上
//!
//! `Rule::In` 是全仓**唯一**需要第二枚门牌（盟册）的判据：盟册那一枚没到持树者手里，
//! `amid` 就答"问不到"，那一格会翻成 `UNJUDGED(9)`——而**不是** `0` / `8`。故这一台的
//! `in` 那两格读数同时证两件事：规矩通了，**门也接上了**。
//!
//! 照实记：`found()` 只是**立一枚号**，"立了不等于进了"（见 `protocol::coalition::core`），
//! 故本域立完还要 `enter(c)` 一次，否则 `In(c)` 的正证当场变成负证。

// 本文件是一份**独立的 bin**（`programs/Cargo.toml` 的 `prog-probe-rule`），**不进 lib**
// ——与 `echo` / `probe-denied` 同一条：`programs/src/user/mod.rs` 里没有它。
//
// 两条 `extern crate` 缺一不可（实测）：`alloc` 是 `format!` 要用；`programs` **不是**为了
// 用它里面的东西，而是为了把 `libprograms` 链进来——**panic handler 与 `_start` 都住那份
// lib**（`programs/src/entry.rs`）。少了它，链接期报 `` `#[panic_handler]` function required ``。
extern crate alloc;
extern crate programs;

use alloc::format;
use core::time::Duration;

use env::{Name, PieToken, TaskId};
use protocol::coalition::call as ccall;
use protocol::coalition::client::Face as CoalitionFace;
use protocol::operator::call as ocall;
use protocol::operator::client as operator;
use protocol::operator::judge::Rule;
use protocol::operator::{EntryId, Where};
use protocol::principal::call as pcall;
use protocol::principal::client::Face as PrincipalFace;
use protocol::session::Quay;
use runtime::env::debug;
use runtime::env::mail;
use runtime::env::room::{self, exit_with_note};
use runtime::env::unit as utask;

/// 本域分出来的那一块：`/sys/rule`。
const DIR: &str = "sys";
const PANE: &str = "rule";
/// 三格的名字（各挂一条规矩）。
const IS: &str = "is";
const UNDER: &str = "under";
const IN: &str = "in";

/// 等树 / 等答 / 找门牌的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 门牌可能落得比本域晚：找不到就再问一次的间隔（毫秒）。
const RETRY_MS: usize = 1;

/// 退场码：走通了 / 没走通（都不是 panic；kernel 会把那一行连同域号打出来）。
const E_OK: usize = 0;
const E_TRIP: usize = 1;

const OK_NOTE: &str = "probe-rule: the three rules held";
const BAD_NOTE: &str = "probe-rule: a rule did NOT hold";

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let Ok(sire) = utask::sire() else {
        bail("probe-rule: no sire")
    };
    let Ok(me) = utask::self_id() else {
        bail("probe-rule: no self id")
    };

    // 一、上树：本域开一条会话，走两趟按名字找（盟册那一面 + 名册那一面）——与 `member` 同形。
    let Ok((tree, host)) = operator::open(sire, MS) else {
        bail("probe-rule: no tree link")
    };
    let Ok(talk) = operator::ask_hole(host) else {
        bail("probe-rule: no tree ask")
    };
    let Some(entry) = find_face(&tree, talk, host, ccall::DIR, ccall::NAME) else {
        bail("probe-rule: no coalition")
    };
    let Ok(coal) = CoalitionFace::of(entry) else {
        bail("probe-rule: bad coalition face")
    };
    let Some(entry) = find_face(&tree, talk, host, pcall::DIR, pcall::NAME) else {
        bail("probe-rule: no identity")
    };
    let Ok(policy) = PrincipalFace::of(entry) else {
        bail("probe-rule: bad identity face")
    };

    // 二、我是谁：装配期绑的那一条（`p`），以及它底下的一条（`q`，给"换一位代表"用）。
    let Ok(Some(p)) = policy.resolve(me, MS) else {
        bail("probe-rule: unbound")
    };
    let Ok(q) = policy.derive(p, MS) else {
        bail("probe-rule: no sub identity")
    };

    // 三、立一枚盟并**进去**（"立了不等于进了"：`found` 只发号，成员要靠 `enter`）。
    let Ok(c) = coal.found(MS) else {
        bail("probe-rule: no coalition id")
    };
    if coal.enter(c, MS).is_err() {
        bail("probe-rule: enter failed")
    }

    // 四、分 `/sys/rule`（"分"是幂等的，故重来一次也无事）。
    let Ok(dir) = Name::new(DIR) else {
        bail("probe-rule: bad name")
    };
    let Ok(pane) = Name::new(PANE) else {
        bail("probe-rule: bad name")
    };
    let Ok(at) = operator::part(talk, &tree, Where::Root, dir, MS) else {
        bail("probe-rule: no /sys")
    };
    let Ok(pane_id) = operator::part(talk, &tree, Where::At(at), pane, MS) else {
        bail("probe-rule: no /sys/rule")
    };

    // 五、落三格，各带一条规矩。`mine = false`：这一台证的是**"用"那一轴**，故不声明归属
    //     （那一轴由 `probe-owner` / `probe-lease` 那两台管）。
    let is_id = plate(talk, &tree, host, pane_id, IS, Rule::Is(p.get() as u64));
    let under_id = plate(
        talk,
        &tree,
        host,
        pane_id,
        UNDER,
        Rule::Under(p.get() as u64),
    );
    let in_id = plate(talk, &tree, host, pane_id, IN, Rule::In(c.get() as u64));
    let made = [is_id, under_id, in_id]
        .iter()
        .filter(|id| id.get() != 0)
        .count();

    // 六、以 `p` 试三遍——**正证**。
    let is = look(talk, &tree, is_id, MS);
    let under = look(talk, &tree, under_id, MS);
    let inside = look(talk, &tree, in_id, MS);

    // 七、**换一位代表**（同一个 TID）：领到自己派生的那条号底下。
    let adopt = policy.adopt(q, MS).is_ok();

    // 八、以 `q` 再试三遍——前两条**负证**，第三条仍是正证（"看支不看相等"）。
    let is_sub = look(talk, &tree, is_id, MS);
    let under_sub = look(talk, &tree, under_id, MS);
    let in_sub = look(talk, &tree, in_id, MS);

    // 九、一行读数。
    say(&format!(
        "probe-rule: tree part={} made={made} p={} adopt={} \
         is={is} under={under} in={inside} \
         is_sub={is_sub} under_sub={under_sub} in_sub={in_sub}",
        pane_id.get(),
        p.get(),
        adopt as u8,
    ));

    // 十、判据：三条正证全 `OK`、两条负证恰是 `DENIED`、`Under` 那一格换人之后仍 `OK`。
    //     **`9`（判不了）不算通过**：它单列的理由正是"这一格没通"（比如盟册那一枚门牌没到）。
    let held = is == ocall::OK
        && under == ocall::OK
        && inside == ocall::OK
        && is_sub == ocall::DENIED
        && in_sub == ocall::DENIED
        && under_sub == ocall::OK
        && adopt
        && made == 3;
    exit_with_note(
        if held { E_OK } else { E_TRIP },
        if held { OK_NOTE } else { BAD_NOTE },
    )
}

/// 落一格，带一条规矩；答那一格自己的号（`0` = 没落成）。
///
/// **`0` 当哨兵是安全的**：零号那一格是 `/sys`，本域跑起来的时候它早被占掉了（`principal`
/// / `coalition` 起头就分了它，见装配单），故这时落出来的号不可能是 `0`。
fn plate(
    talk: PieToken,
    link: &Quay,
    host: TaskId,
    at: EntryId,
    name: &str,
    rule: Rule<u64, u64>,
) -> EntryId {
    let Ok(entry) = mail::unseal_hole(env::Mark::of("rule-entry")) else {
        return EntryId::new(0);
    };
    let Ok(one) = Name::new(name) else {
        return EntryId::new(0);
    };
    operator::land(talk, link, host, Where::At(at), one, entry, rule, false, MS)
        .unwrap_or(EntryId::new(0))
}

/// 拿那一格去 `find`：答线上那一格码（`OK` = 放行；本程序只看码，不看那一枚）。
fn look(talk: PieToken, link: &Quay, id: EntryId, millis: usize) -> u8 {
    if id.get() == 0 {
        return ocall::UNKNOWN;
    }
    operator::find(talk, link, id, millis).unwrap_or(ocall::BAD)
}

/// 按名字找一面服务门牌（`seek` 译号 + `find` 取回）——与 `subject` / `member` 那两台同形。
///
/// **间接寻址那一手**：名字先经 `seek` 译成号（"还没挂上"那一格也在这里重试），此后按号。
/// `find` 把那一枚授过来（持树者 `ship`），本域按"谁给的"认领最后那一枚。
fn find_face(link: &Quay, talk: PieToken, host: TaskId, dir: &str, name: &str) -> Option<PieToken> {
    let (Ok(dir), Ok(me)) = (Name::new(dir), Name::new(name)) else {
        return None;
    };
    let road = [dir, me];
    let mut left = MS;
    let id = loop {
        match operator::seek(talk, link, &road, MS) {
            Ok(id) => break id,
            Err(ocall::UNKNOWN) if left > 0 => {
                let _ = room::sleep(Duration::from_millis(RETRY_MS as u64));
                left = left.saturating_sub(RETRY_MS);
            }
            Err(_) => return None,
        }
    };
    if operator::find(talk, link, id, MS).unwrap_or(ocall::BAD) != ocall::OK {
        return None;
    }
    operator::take(link, host)
}

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）。
fn bail(note: &str) -> ! {
    say(note);
    exit_with_note(E_TRIP, note)
}

/// 打一行。调试面是本域唯一的嘴（与 `echo` / `guest` 用的是同一格）。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
