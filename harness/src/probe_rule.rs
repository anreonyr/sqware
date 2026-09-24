#![no_std]
#![no_main]

//! probe-rule — **规矩那一格的证客**：一位**有身份**的任务把"这一格谁许用"落成
//! `Rule::Is` / `Under` / `In` / `Opens`，然后**自己按身份试几遍**，最后**换一条身份再试**。
//!
//! 门禁那一刀的正文里，"用"那一轴（谁许用这一格）有**五条**判据（公开 / 就是某一位 /
//! 在某一位那一支里 / 在某枚盟里 / 就是开着某一格的那一位），而上一刀的真机上只有一条通电：
//! `DEFAULT_RULE` 是一条全局常量，`Is` / `Under` / `In` 三条**一次没被问过**。本程序把它们搬到
//! 真机上——一台客人演两个身份，故"规矩随**身份**走、不随 TID 走"这一条也在同一行读数里。
//!
//! ```text
//!   0  上树 + 取两面门牌（名册那一枚 + **盟册那一枚**）
//!   1  p = resolve(self)；q = derive(p)            —— 我是 p，我底下还有一个 q
//!   2  分 /sys/rule；落六格：
//!        is      Rule::Is(p)
//!        under   Rule::Under(p)
//!        in      Rule::In(c)                     （c 是本域刚立、刚入的那一枚盟）
//!        door    ——本域自己挂的一枚门牌（一枚 Tile，**开者就是本域**）
//!        open    Rule::Opens(door 的号)           —— 许给"开着那一格的那位"（正是本域）
//!        foreign Rule::Opens(/sys/principal 的号) —— 许给"开着**别人**那一格的那位"（不是本域）
//!        at-pane Rule::Opens(/sys 那一格)        —— 那一号是块 Pane（没有开者）
//!        temp    先落一枚门牌，再**剪掉**它
//!        gone-door Rule::Opens(temp 那个旧号)     —— 号不重用 ⇒ 那一格永久没有开者
//!   3  以 p 试七遍   ⇒ is / under / in / open 全答 OK(0)、foreign 答 DENIED(8)，
//!                      而 at-pane 与 gone-door 各答 UNJUDGED(9)（**判不了**，不是拒）
//!   4  adopt(q)      —— **同一条 TID，换了一位代表**
//!   5  以 q 再试四遍 ⇒ is 答 DENIED(8)、in 答 DENIED(8)、under 仍答 OK(0)、**open 仍答 OK(0)**
//!      —— 前两条是**负证**（有身份、但不是那一位 / 不在那枚盟里），
//!         第三条是"`Under` 看的是**支**，不是相等"的正证（q 仍在 p 那一支里），
//!         第四条是**`Opens` 与 `Is` 的分野**：`Opens` 比的是"开着那一格的那条 TID 此刻代表谁"，
//!         而开者与问的人是**同一条 TID** ⇒ 换代表之后两边一起变 ⇒ 照旧过。
//!   6  报一行读数就退场
//! ```
//!
//! # `Opens` 那一格：号从**树**上来
//!
//! 前四格只能指"自己人"（自己的号 / 自己那一支 / 自己在的盟），而"把这一格许给
//! `/sys/principal` 那位"这句话原先**说不出来**：规矩里那个号是裸号，客人手里只有五条窄路，
//! 没有一条是"按名字点名"。`Opens` 补的正是它——**先 `seek` 把一条路译成号**（名字 → 号，
//! [`road_id`] 那一手），再把那个号写进规矩；判的时候持树者去问"此刻谁占着那一格"。
//! **树就是名录**。
//!
//! 照实记：`door` 那一格是必要的——`Opens` 的**正证**要一位"自己开着门牌"的客人；而
//! `foreign` 那一格指的是一枚**长命**门牌（`/sys/principal`，整轮都活着）⇒ 它的负证**不依赖
//! 任何次序**（若改指一位用完就退场的客人，那一格会翻成 `9`（判不了）而不是 `8`）。
//!
//! # 两格 `UNJUDGED`：这一格里"判不了"第一次上了真机
//!
//! 门禁的三格答案里，`UNJUDGED`（判不了：可重试）此前**只有宿主台的读数**——真机上要量到它，
//! 得让身份服务**不答**，而那会把整机拆掉。`Opens` 让它可以被**确定性地**量出来，而且两条因
//! 各不相同：那一号是块 `Pane`（没有开者这一说）、那一格已经**剪掉**（号不重用 ⇒ 永久没有开者）。
//! 判据里这两格必须落在 `9`：落 `8`（终态拒）会让客人白放弃，落 `0`（放行）等于门禁不存在。
//!
//! 照实记：`gone-door` 那一格顺带把 [`Rule::Opens`] 的一条**已知边界**量成了读数——"那一格被剪
//! 掉之后，指它的那条规矩永久判不了（重挂是**新号**）"。
//!
//! # 为什么"另一台客人"也要来（`prog-probe-rule-other`）
//!
//! `Under(p)` 的**负证**在同一个域里做不到：`q = derive(p)` 一定在 p 那一支里，而 `adopt`
//! 只许**往下**领（`heir(current, q)`）。"不在那一支里"的那一位只能是**另一台**——那正是
//! `probe-rule-other` 那一格（它顺带对 `foreign` 也量一遍：第三台同样过不去）。
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

use harness::cases;

/// 本域分出来的那一块：`/sys/rule`。
const DIR: &str = "sys";
const PANE: &str = "rule";
/// 三格的名字（各挂一条规矩）。
const IS: &str = "is";
const UNDER: &str = "under";
const IN: &str = "in";
/// 本域**自己挂的那一枚门牌**（一枚 `Tile`，开者就是本域）——`Opens` 要指的就是它。
const DOOR: &str = "door";
/// 许给"**开着门牌那一格**的那位"的一格 ⇒ **正证**（开者正是本域）。
const OPEN: &str = "open";
/// 许给"**开着 `/sys/principal` 那一格**的那位"的一格 ⇒ **负证**（那位不是本域）。
///
/// 这一格就是这一刀要补的那句话：**"把这一格许给某一位"**——号由 [`seek`] 从树上换来
/// （名字 → 号），不靠别人把号塞给我。
const FOREIGN: &str = "foreign";
/// 先落、再**剪掉**的一枚门牌——留给下面 `gone-door` 那一格指它那个**旧号**。
const TEMP: &str = "temp";
/// 规矩 = `Opens(/sys 那一格)` ⇒ 那一号是块 **`Pane`**（没有开者这一说）⇒ **判不了**。
const AT_PANE: &str = "at-pane";
/// 规矩 = `Opens(剪掉的那一枚门牌号)` ⇒ 号**不重用** ⇒ 那一格永远没有开者 ⇒ **判不了**。
const GONE_DOOR: &str = "gone-door";
/// 本域**声明归自己**（`mine = true`）的一格——"**改**"那一轴那一条（见 §7.8）。
const MINE: &str = "mine";

/// 等树 / 等答 / 找门牌的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 门牌可能落得比本域晚：找不到就再问一次的间隔（毫秒）。
const RETRY_MS: usize = 1;

/// 退场码：走通了 / 没走通（都不是 panic；kernel 会把那一行连同域号打出来）。
const E_OK: usize = 0;
const E_TRIP: usize = 1;

const OK_NOTE: &str = "probe-rule: the rules held";

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
    // 一点五、**再要一次问话孔**：一个域只该铸一枚 ⇒ 第二次叫回来的是**同一枚**（客侧先找后铸），
    // 故下面每一问都改用**第二枚**那个号——它要是另一枚孔，持树者认的还是第一枚，
    // 这些话就全石沉大海（`session` 事实 9 那个症状）。两件事一次量：`ask_same` 与后面所有读数。
    let Ok(again) = operator::ask_hole(host) else {
        bail("probe-rule: no second tree ask")
    };
    let ask_same = again == talk;
    let talk = again;
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
    let Ok(mine) = Name::new(MINE) else {
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
    let is_id = plate(
        talk,
        &tree,
        host,
        pane_id,
        IS,
        Rule::Is(p.get() as u64),
        false,
    );
    let under_id = plate(
        talk,
        &tree,
        host,
        pane_id,
        UNDER,
        Rule::Under(p.get() as u64),
        false,
    );
    let in_id = plate(
        talk,
        &tree,
        host,
        pane_id,
        IN,
        Rule::In(c.get() as u64),
        false,
    );
    let made = [is_id, under_id, in_id]
        .iter()
        .filter(|id| id.get() != 0)
        .count();

    // 五点五、**点名那一格**（第五个规矩变体 `Opens`）：
    //   door    —— 本域自己挂的一枚门牌（一枚 `Tile`，**开者就是本域**）
    //   open    —— 规矩 = `Opens(door 的号)`：许给"开着那一格的那位" ⇒ 正是本域
    //   foreign —— 规矩 = `Opens(/sys/principal 的号)`：许给"开着**别人**那一格的那位" ⇒ 不是本域
    //
    // 两个号都是**树上换来的**（`seek` 把一条路译成号）——那一格的门牌在谁手里，由树说，
    // 不由别人告诉我。故这一台**没有 new 的任何机制**，只是把规矩那一格的号换了个来路。
    let door_id = plate(talk, &tree, host, pane_id, DOOR, Rule::Public, false);
    let open_id = plate(
        talk,
        &tree,
        host,
        pane_id,
        OPEN,
        Rule::Opens(door_id),
        false,
    );
    // `/sys/principal` 那一格的号：**点名那一手**（名字 → 号），与 `find_face` 走同一条路。
    let foreign_id = match Name::new(pcall::NAME)
        .ok()
        .and_then(|p| seek_id(&tree, talk, &[dir, p]))
    {
        Some(principal) => plate(
            talk,
            &tree,
            host,
            pane_id,
            FOREIGN,
            Rule::Opens(principal),
            false,
        ),
        None => EntryId::new(0),
    };

    // 五点六、**"判不了"（`UNJUDGED`）那两格**——都是确定性的，不看时序：
    //   at-pane   —— 规矩 = `Opens(/sys 那一格)`：那一号是块 `Pane`，**没有开者这一说**；
    //   gone-door —— 规矩 = `Opens(temp 那个**旧号**)`：先把 `temp` 落上、再剪掉，
    //                而**号不重用** ⇒ 那一格永久没有开者。
    // 照实记：`UNJUDGED` 这一格此前**只有宿主台的读数**（身份服务不答那一版在真机上要拆整机）。
    // 这两格与它同格不同因：判据不需要"那一格为什么没人"，只需要"**有没有那一位**"。
    let at_pane_id = match seek_id(&tree, talk, &[dir]) {
        Some(sys) => plate(talk, &tree, host, pane_id, AT_PANE, Rule::Opens(sys), false),
        None => EntryId::new(0),
    };
    let temp_id = plate(talk, &tree, host, pane_id, TEMP, Rule::Public, false);
    let trimmed = temp_id.get() != 0 && operator::trim(talk, &tree, temp_id, MS).is_ok();
    let gone_id = plate(
        talk,
        &tree,
        host,
        pane_id,
        GONE_DOOR,
        Rule::Opens(temp_id),
        false,
    );

    // 五点七、**"改"那一轴那一格**：本域声明归自己（`mine = true`）。
    //
    // 下面在 `adopt(q)` **之后**再落一次同一格——那是这一刀要量的那件事：**归属记的是"命"而不是
    // "身份"**（账里那两格 `who` + `pie` 都是任务级的）⇒ 主人**换了代表照样能改自己的格子**，
    // 而同一次 `Is(p)` 已经答了 `8`（"用"那一轴随身份走）。两条轴各问各的问题，各自自洽。
    let mine_id = plate(talk, &tree, host, pane_id, MINE, Rule::Public, true);

    // 六、以 `p` 试五遍——前三条**正证**，后两条是 `Opens` 的正负两面。
    let is = look(talk, &tree, is_id, MS);
    let under = look(talk, &tree, under_id, MS);
    let inside = look(talk, &tree, in_id, MS);
    let open = look(talk, &tree, open_id, MS);
    let foreign = look(talk, &tree, foreign_id, MS);
    let on_pane = look(talk, &tree, at_pane_id, MS);
    let on_gone = look(talk, &tree, gone_id, MS);

    // 七、**换一位代表**（同一个 TID）：领到自己派生的那条号底下。
    let adopt = policy.adopt(q, MS).is_ok();

    // 八、以 `q` 再试——前两条**负证**、第三条仍是正证（"看支不看相等"）；
    //     `open` 那一格**照旧过**：开者与问的人是**同一条 TID**，换代表之后两边一起变成 `q`
    //     ——这正是"规矩随**身份**走、不随 TID 走"与 `Is` 那一格（拒）的分野。
    let is_sub = look(talk, &tree, is_id, MS);
    let under_sub = look(talk, &tree, under_id, MS);
    let in_sub = look(talk, &tree, in_id, MS);
    let open_sub = look(talk, &tree, open_id, MS);
    // 再用一枚**新孔重落**自己那一格（换绑）：走的就是 `claimable` 那一支。
    let keep = match mail::unseal_hole(env::Mark::of("rule-entry")) {
        Ok(entry) if mine_id.get() != 0 => operator::land(
            talk,
            &tree,
            host,
            Where::At(pane_id),
            mine,
            entry,
            Rule::Public,
            true,
            MS,
        )
        .err()
        .unwrap_or(ocall::OK),
        _ => ocall::BAD,
    };

    // 九、一行读数。
    say(&format!(
        "probe-rule: tree part={} made={made} p={} adopt={} \
         is={is} under={under} in={inside} \
         is_sub={is_sub} under_sub={under_sub} in_sub={in_sub} \
         door={} open={open} foreign={foreign} open_sub={open_sub} \
         trim={} at_pane={on_pane} gone_door={on_gone} mine={} keep={keep} \
         ask2={} ask_same={}",
        pane_id.get(),
        p.get(),
        adopt as u8,
        door_id.get(),
        trimmed as u8,
        mine_id.get(),
        again.get(),
        ask_same as u8,
    ));

    // 十、判据：**一例一条**（用户裁定"程序侧 pilot"）。
    //
    // 照实记：原先这十三条被 `&&` 成**一个** `held`，红了只知道 `probe-rule: a rule did NOT
    // hold`——还得回头看上面那 19 个计数器才认得出是哪一条。现在一例一个名字，而**名字就是
    // 结论**（每一例后面那句"为什么"与头注 §3 / §5 那张表同源）。
    let mut suite = cases::Suite::new("probe-rule");
    // —— 装配：一条规矩没落上，后面全没意义。故它排第一：红了不会被后面的假红淹没。
    suite.case("three_rules_landed", move || {
        assert!(made == 3, "made={made}")
    });
    // —— 以 p 试（`p = resolve(self)`）：三条正证。
    suite.case("is_binds_that_identity", move || assert_eq!(is, ocall::OK));
    suite.case("under_covers_my_own_branch", move || {
        assert_eq!(under, ocall::OK)
    });
    suite.case("in_covers_that_league", move || {
        assert_eq!(inside, ocall::OK)
    });
    // 每一问用的都是**第二枚**孔那个号：它要是另一枚，持树者认的是第一枚，这些话全石沉大海。
    suite.case("a_second_ask_returns_the_same_hole", move || {
        assert!(
            ask_same,
            "先找后铸叫回来的不是同一枚孔：ask2={}",
            again.get()
        )
    });
    // —— `Opens` 的正负两面。
    suite.case("opens_covers_my_own_door", move || {
        assert_eq!(open, ocall::OK)
    });
    suite.case("opens_refuses_someone_elses_door", move || {
        assert_eq!(foreign, ocall::DENIED, "别人开着的那一格，我该被拒")
    });
    // —— 两格「判不了」：`9` 单列的理由正是"这一格没通"，故它**不算通过**。
    suite.case("opens_on_a_pane_is_unjudged", move || {
        assert_eq!(on_pane, ocall::UNJUDGED, "那一号是块 Pane：没有开者这一说")
    });
    suite.case("opens_on_a_trimmed_door_is_unjudged", move || {
        assert_eq!(on_gone, ocall::UNJUDGED, "那一格剪掉了 ⇒ 永久没有开者")
    });
    // —— `trim` 那一手真的落下去了（上面 `gone-door` 那一格的前提）。
    suite.case("trimming_retires_the_plate", move || {
        assert!(trimmed, "temp 没剪掉")
    });
    // —— 换一位代表（**同一条 TID**）：`adopt` 成功；两条负证、两条仍是正证。
    suite.case("rebinding_succeeds", move || {
        assert!(adopt, "adopt(q) 没成功")
    });
    suite.case("rebinding_leaves_that_identity", move || {
        assert_eq!(is_sub, ocall::DENIED, "换代表之后 Is(p) 该拒")
    });
    suite.case("leaving_the_league_is_seen", move || {
        assert_eq!(in_sub, ocall::DENIED, "换代表之后不在那枚盟里了")
    });
    suite.case("under_follows_the_branch_not_equality", move || {
        assert_eq!(under_sub, ocall::OK, "q 仍在 p 那一支里 ⇒ Under(p) 照旧过")
    });
    suite.case("opens_follows_the_opener_not_the_caller", move || {
        assert_eq!(
            open_sub,
            ocall::OK,
            "开者与问的是同一条 TID ⇒ 两边一起变成 q"
        )
    });
    // —— "改"那一轴：归属记的是**命**，换代表之后自己那一格照样改得。
    suite.case("a_rebound_owner_can_still_change_its_own_cell", move || {
        assert_eq!(keep, ocall::OK, "归属记的是命，换代表照样改得")
    });
    suite.run();

    exit_with_note(E_OK, OK_NOTE)
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
    mine: bool,
) -> EntryId {
    let Ok(entry) = mail::unseal_hole(env::Mark::of("rule-entry")) else {
        return EntryId::new(0);
    };
    let Ok(one) = Name::new(name) else {
        return EntryId::new(0);
    };
    operator::land(talk, link, host, Where::At(at), one, entry, rule, mine, MS)
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
    let (Ok(dir), Ok(one)) = (Name::new(dir), Name::new(name)) else {
        return None;
    };
    let id = seek_id(link, talk, &[dir, one])?;
    if operator::find(talk, link, id, MS).unwrap_or(ocall::BAD) != ocall::OK {
        return None;
    }
    operator::take(link, host)
}

/// **点名那一手**：把一条路译成号（名字 → 号），"还没挂上"那一格在那里重试。
///
/// 这是这一刀唯一新用到的读：规矩里那个号的来路从"别人告诉我"变成"**树上换来**"。
fn seek_id(link: &Quay, talk: PieToken, road: &[Name]) -> Option<EntryId> {
    let mut left = MS;
    loop {
        match operator::seek(talk, link, &road, MS) {
            Ok(id) => return Some(id),
            Err(ocall::UNKNOWN) if left > 0 => {
                let _ = room::sleep(Duration::from_millis(RETRY_MS as u64));
                left = left.saturating_sub(RETRY_MS);
            }
            Err(_) => return None,
        }
    }
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
