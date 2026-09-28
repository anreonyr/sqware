#![no_std]
#![no_main]

//! probe-rule — **许可那一格的证客**：一位**有身份**的任务把"这一格许给谁"落成
//! `Permit::Trunk` / `Bough` / `Among` / `Opener`，然后**自己按身份试几遍**，最后**换一条身份再试**。
//!
//! 门禁那一刀的正文里，"用"那一轴（谁许用这一格）有**四条**判据（就是某一位 / 在某一位那一支里 /
//! 在某枚盟里 / 就是开着某一格的那一位），外加"**没记**"那一档；而那一刀的真机上只有"没记"通电：
//! 所有条目共用一条常量，`Trunk` / `Bough` / `Among` 三条**一次没被问过**。本程序把它们搬到
//! 真机上——一台客人演两个身份，故"许可随**身份**走、不随 TID 走"这一条也在同一行读数里。
//!
//! ```text
//!   0  上树 + 取两面门牌（名册那一枚 + **盟册那一枚**）
//!   1  p = resolve(self)；q = derive(p)            —— 我是 p，我底下还有一个 q
//!   2  分 /sys/rule；落六格：
//!        is      Permit::Trunk(p)
//!        under   Permit::Bough(p)
//!        in      Permit::Among(c)                     （c 是本域刚立、刚入的那一枚盟）
//!        door    ——本域自己挂的一枚门牌（一枚 Tile，**开者就是本域**）
//!        open    Permit::Opener(door 的号)           —— 许给"开着那一格的那位"（正是本域）
//!        foreign Permit::Opener(/sys/principal/ask 的号) —— 许给"开着**别人**那一格的那位"（不是本域）
//!        at-pane Permit::Opener(/sys 那一格)        —— 那一号是块 Pane（没有开者）
//!        temp    先落一枚门牌，再**剪掉**它
//!        gone-door Permit::Opener(temp 那个旧号)     —— 号不重用 ⇒ 那一格永久没有开者
//!   3  以 p 试七遍   ⇒ is / under / in / open 全答 OK(0)、foreign 答 DENIED(8)，
//!                      而 at-pane 与 gone-door 各答 UNJUDGED(9)（**判不了**，不是拒）
//!   4  adopt(q)      —— **同一条 TID，换了一位代表**
//!   5  以 q 再试四遍 ⇒ is 答 DENIED(8)、in 答 DENIED(8)、under 仍答 OK(0)、**open 仍答 OK(0)**
//!      —— 前两条是**负证**（有身份、但不是那一位 / 不在那枚盟里），
//!         第三条是"`Bough` 看的是**支**，不是相等"的正证（q 仍在 p 那一支里），
//!         第四条是**`Opener` 与 `Trunk` 的分野**：`Opener` 比的是"开着那一格的那条 TID 此刻代表谁"，
//!         而开者与问的人是**同一条 TID** ⇒ 换代表之后两边一起变 ⇒ 照旧过；
//!         第五条是**没记许可**那一格（`Permit::Unset`，见 §五点七）：判据只到"你有没有身份"
//!         ——它**不判"是不是你"** ⇒ 换了代表那位照样过（`Trunk(p)` 与它的分野就在这一条）。
//!   6  报一行读数就退场
//! ```
//!
//! # `Opener` 那一格：号从**树**上来
//!
//! 前四格只能指"自己人"（自己的号 / 自己那一支 / 自己在的盟），而"把这一格许给
//! `/sys/principal/ask` 那位"这句话原先**说不出来**：规矩里那个号是裸号，客人手里只有五条窄路，
//! 没有一条是"按名字点名"。`Opener` 补的正是它——**先 `seek` 把一条路译成号**（名字 → 号，
//! [`road_id`] 那一手），再把那个号写进规矩；判的时候持树者去问"此刻谁占着那一格"。
//! **树就是名录**。
//!
//! 照实记：`door` 那一格是必要的——`Opener` 的**正证**要一位"自己开着门牌"的客人；而
//! `foreign` 那一格指的是一枚**长命**门牌（`/sys/principal/ask`，整轮都活着）⇒ 它的负证**不依赖
//! 任何次序**（若改指一位用完就退场的客人，那一格会翻成 `9`（判不了）而不是 `8`）。
//!
//! # 两格 `UNJUDGED`：这一格里"判不了"第一次上了真机
//!
//! 门禁的三格答案里，`UNJUDGED`（判不了）此前**只在宿主靶上有过读数**（那台靶已删）——真机上要
//! 量到它，得让身份服务**不答**，而那会把整机拆掉。`Opener` 让它可以被**确定性地**量出来，而且两条因
//! 各不相同：那一号是块 `Pane`（没有开者这一说）、那一格已经**剪掉**（号不重用 ⇒ 永久没有开者）。
//! 判据里这两格必须落在 `9`：落 `8`（终态拒）会让客人白放弃，落 `0`（放行）等于门禁不存在。
//!
//! **照实记（后一刀）**：这两因**永远好不了**，而 `9` 里还有"会好"的那一类（对面不答 / 超时）
//! ——两类同格是刻意的（客人的下一步相同）。故持树者现在**各说一行读数**
//! （`operator: opens pane|gone|sealed n=…`）："为什么判不了"从此在真机上看得见。
//!
//! 照实记：`gone-door` 那一格顺带把 [`Permit::Opener`] 的一条**已知边界**量成了读数——"那一格被剪
//! 掉之后，指它的那条规矩永久判不了（重挂是**新号**）"。
//!
//! # 为什么"另一台客人"也要来（`prog-probe-rule-other`）
//!
//! `Bough(p)` 的**负证**在同一个域里做不到：`q = derive(p)` 一定在 p 那一支里，而 `adopt`
//! 只许**往下**领（`heir(current, q)`）。"不在那一支里"的那一位只能是**另一台**——那正是
//! `probe-rule-other` 那一格（它顺带对 `foreign` 也量一遍：第三台同样过不去）。
//!
//! # 这一台为什么把盟也带上
//!
//! `Permit::Among` 是全仓**唯一**需要第二枚门牌（盟册）的判据：盟册那一枚没到持树者手里，
//! `amid` 就答"问不到"，那一格会翻成 `UNJUDGED(9)`——而**不是** `0` / `8`。故这一台的
//! `in` 那两格读数同时证两件事：规矩通了，**门也接上了**。
//!
//! 照实记：`found()` 只是**立一枚号**，"立了不等于进了"（见 `protocol::system::coalition::core`），
//! 故本域立完还要 `enter(c)` 一次，否则 `Among(c)` 的正证当场变成负证。

// 本文件是一份**独立的 bin**（`harness/Cargo.toml` 的 `prog-probe-rule`），**不进 lib**
// ——与 `canonical` / `probe-denied` 同一条：`programs/src/user/mod.rs` 里没有它。
//
// 两条 `extern crate` 缺一不可（实测）：`alloc` 是 `format!` 要用；`programs` **不是**为了
// 用它里面的东西，而是为了把 `libprograms` 链进来——**panic handler 与 `_start` 都住那份
// lib**（`programs/src/entry.rs`）。少了它，链接期报 `` `#[panic_handler]` function required ``。
extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use env::{Name, PieToken};
use protocol::communication::session::Session;
use protocol::debug;
use protocol::system::coalition as ccall;
use protocol::system::coalition::client::Face as CoalitionFace;
use protocol::system::operator::client as operator;
use protocol::system::operator::client::{Face as TreeFace, Mine, Pane};
use protocol::system::operator::{EntryId, Fail, Permit};
use protocol::system::principal as pcall;
use protocol::system::principal::client::Face as PrincipalFace;
use runtime::env::mail;
use runtime::env::unit as utask;

/// 本域分出来的那一块：`/sys/rule`。
const DIR: &str = "sys";
const PANE: &str = "rule";
/// 三格的名字（各挂一条规矩）。
const IS: &str = "is";
const UNDER: &str = "under";
const IN: &str = "in";
/// 本域**自己挂的那一枚门牌**（一枚 `Tile`，开者就是本域）——`Opener` 要指的就是它。
const DOOR: &str = "door";
/// 许给"**开着门牌那一格**的那位"的一格 ⇒ **正证**（开者正是本域）。
const OPEN: &str = "open";
/// 许给"**开着 `/sys/principal/ask` 那一格**的那位"的一格 ⇒ **负证**（那位不是本域）。
///
/// 这一格就是这一刀要补的那句话：**"把这一格许给某一位"**——号由 [`seek`] 从树上换来
/// （名字 → 号），不靠别人把号塞给我。
const FOREIGN: &str = "foreign";
/// 先落、再**剪掉**的一枚门牌——留给下面 `gone-door` 那一格指它那个**旧号**。
const TEMP: &str = "temp";
/// 规矩 = `Opener(/sys 那一格)` ⇒ 那一号是块 **`Pane`**（没有开者这一说）⇒ **判不了**。
const AT_PANE: &str = "at-pane";
/// 规矩 = `Opener(剪掉的那一枚门牌号)` ⇒ 号**不重用** ⇒ 那一格永远没有开者 ⇒ **判不了**。
const GONE_DOOR: &str = "gone-door";
/// 本域**声明归自己**（`mine = true`）的一格——"**改**"那一轴那一条（理由见 `operator::core` 的
/// `Node::Tile` 的 `owner` 那一格）。
const MINE: &str = "mine";

/// 等树 / 等答 / 找门牌的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 退场码：走通了 / 没走通（都不是 panic；kernel 会把那一行连同域号打出来）。
const E_OK: usize = 0;
const E_TRIP: usize = 1;

const OK_NOTE: &str = "probe-rule: the rules held";

#[programs::entry]
fn main() -> Report<'static> {
    let sire = utask::sire();
    let me = utask::self_id();

    // 一、上树：本域开一条会话，走两趟按名字找（盟册那一面 + 名册那一面）——与 `member` 同形。
    let Ok(session) = Session::open(sire, operator::BERTH, Wait::AtMost(MS)) else {
        return bail("probe-rule: no tree link");
    };
    // 照实记（"再要一次问话孔"那一格**读数退场**）：从前这里再叫一次 `operator::ask_hole`，
    // 量"第二次叫回来的是同一枚"（客侧先找后铸）。开会话那一手抬进
    // [`protocol::communication::session`] 之后，"只铸一枚"从**纪律**变成**构造**（`ask` 先找
    // 后铸，见那边）——从外面叫不出第二次 ⇒ 这条判据与它的读数（`ask2` / `ask_same`）一起退。
    //
    // **照实记（这一台为什么整体改走 `Face`，task-2 那一刀）**：本台每一问（`open` / `bind` /
    // `road` / `trim` / `token`）都在 [`TreeFace`] / [`Pane`] / [`Tile`] 的面上，裸孔一个都不用
    // （从前那行 `&session.link, session.talk, session.host` 因此整行退场）⇒ 交给
    // [`TreeFace::of`]（吃所有权）；三个帮手 `plate` / `look` / `find_face` 一并从裸
    // `(talk, link, host)` 改收那一面 / 那块 Pane。
    let tree = TreeFace::of(session);
    // 盟册那面**只要"定面"**（开面那一刀）：这一台立一枚盟、把本域入进去——两条都在 `Set` 上
    // （`Found` / `Enter`）；它不查盟籍、不点名册。
    let (Ok(cdir), Ok(cseg), Ok(cset)) = (
        Name::new(ccall::DIR),
        Name::new(ccall::NAME),
        Name::new(ccall::Grant::Set.name()),
    ) else {
        return bail("probe-rule: bad coalition name");
    };
    let Some(entry) = find_face(&tree, &[cdir, cseg, cset]) else {
        return bail("probe-rule: no coalition set face");
    };
    let Ok(coal) = CoalitionFace::of(entry) else {
        return bail("probe-rule: bad coalition set face");
    };
    // 身份那**两面**（开面那一刀）：三条"问"的（`Resolve` / `Sire` / `Heir`）在 `Grant::Ask` 上，
    // 四条"定"的（`Bind` / `Derive` / `Adopt` / `Waive`）在 `Grant::Set` 上。这一台**两面都要**
    // ——它要 `Resolve` 问"我代表谁"、要 `Derive` 派生一条子身份、要 `Adopt` 换一位代表。
    let (Ok(idir), Ok(iseg)) = (Name::new(pcall::DIR), Name::new(pcall::NAME)) else {
        return bail("probe-rule: bad identity name");
    };
    let (Ok(iask), Ok(iset)) = (
        Name::new(pcall::Grant::Ask.name()),
        Name::new(pcall::Grant::Set.name()),
    ) else {
        return bail("probe-rule: bad identity face name");
    };
    let Some(entry) = find_face(&tree, &[idir, iseg, iask]) else {
        return bail("probe-rule: no identity ask face");
    };
    let Ok(iask) = PrincipalFace::of(entry) else {
        return bail("probe-rule: bad identity ask face");
    };
    let Some(entry) = find_face(&tree, &[idir, iseg, iset]) else {
        return bail("probe-rule: no identity set face");
    };
    let Ok(iset) = PrincipalFace::of(entry) else {
        return bail("probe-rule: bad identity set face");
    };

    // 二、我是谁：装配期绑的那一条（`p`），以及它底下的一条（`q`，给"换一位代表"用）。
    let Ok(Some(p)) = iask
        .task(me)
        .principal(Wait::AtMost(MS))
        .map(|found| found.map(|x| x.id()))
    else {
        return bail("probe-rule: unbound");
    };
    let Ok(q) = iset
        .principal(p)
        .derive(Wait::AtMost(MS))
        .map(|child| child.id())
    else {
        return bail("probe-rule: no sub identity");
    };

    // 三、立一枚盟并**进去**（"立了不等于进了"：`found` 只发号，成员要靠 `enter`）。
    // 这一手不收"谁"：进的是本端此刻代表的那一位。
    let Ok(c) = coal.found(Wait::AtMost(MS)) else {
        return bail("probe-rule: no coalition id");
    };
    if c.enter(Wait::AtMost(MS)).is_err() {
        return bail("probe-rule: enter failed");
    }

    // 四、分 `/sys/rule`（"分"是幂等的，故重来一次也无事）。
    let Ok(dir) = Name::new(DIR) else {
        return bail("probe-rule: bad name");
    };
    let Ok(pane) = Name::new(PANE) else {
        return bail("probe-rule: bad name");
    };
    let Ok(mine) = Name::new(MINE) else {
        return bail("probe-rule: bad name");
    };
    let root = tree.root();
    let Ok(sys) = root.open(dir, Wait::AtMost(MS)) else {
        return bail("probe-rule: no /sys");
    };
    let Ok(at) = sys.open(pane, Wait::AtMost(MS)) else {
        return bail("probe-rule: no /sys/rule");
    };
    let pane_id = at.id();

    // 五、落三格，各带一条规矩。`Mine::No`：这一台证的是**"用"那一轴**，故不声明归属
    //     （那一轴由 `probe-owner` / `probe-lease` 那两台管）。
    let is_id = plate(&at, IS, Permit::Trunk(p), Mine::No);
    let under_id = plate(&at, UNDER, Permit::Bough(p), Mine::No);
    let in_id = plate(&at, IN, Permit::Among(c.id()), Mine::No);
    let made = [is_id, under_id, in_id]
        .iter()
        .filter(|id| id.get() != 0)
        .count();

    // 五点五、**点名那一格**（第五个规矩变体 `Opener`）：
    //   door    —— 本域自己挂的一枚门牌（一枚 `Tile`，**开者就是本域**）
    //   open    —— 规矩 = `Opener(door 的号)`：许给"开着那一格的那位" ⇒ 正是本域
    //   foreign —— 规矩 = `Opener(/sys/principal/ask 的号)`：许给"开着**别人**那一格的那位" ⇒ 不是本域
    //
    // 两个号都是**树上换来的**（`road` 把一条路译成号）——那一格的门牌在谁手里，由树说，
    // 不由别人告诉我。故这一台**没有 new 的任何机制**，只是把规矩那一格的号换了个来路。
    //
    // **照实记（这几格的号今天不用留）**：下面那几问按**名**寻（[`look`]），不再按号——
    // 故这几格只留"落上了没有"那一件事，号那一格（旧 `open_id` / `foreign_id` / `at_pane_id` /
    // `gone_id`）随之下岗（`plate` 仍照落，判据一条没动）。
    let door_id = plate(&at, DOOR, Permit::Unset, Mine::No);
    let _ = plate(&at, OPEN, Permit::Opener(door_id), Mine::No);
    // `/sys/principal/ask` 那一格的号：**点名那一手**（名字 → 号），与 `find_face` 走同一条路。
    //
    // **照实记（开面那一刀：这一格从 `/sys/principal` 挪到底下那一格）**：`/sys/principal` 从前
    // **就是**名册那枚门牌（一枚 `Tile`）；开面之后它成了那段前缀（一块 `Pane`），而这一格要的是
    // **别人开着的一枚砖**——`Opener` 指着一块窗格只会答 `Unjudged`（"没有开者这一说"），
    // `foreign` 那条负证当场变色。故往底下那一格去。
    //
    // **照实记（旧 `id_of` 只译号，故这里走 `Pane::tile`）**：两手的差别是**重试**，不是飞不飞
    // 门闩——`Pane::tile` 就地问一次（不重试），`Face::tile` 带额度重试。本格用前者：这一台
    // **不重试**是因为紧跟着那一问（`Opener` 判据）本身要的是"此刻拒"——重试会把"立刻拒"这一格
    // 变松（见 [`denied`](probe_rule_other.rs) 那边量同一件事的那一台）。
    let foreign = Name::new(pcall::NAME)
        .ok()
        .zip(Name::new(pcall::Grant::Ask.name()).ok())
        .and_then(|(p, leaf)| root.tile(&[dir, p, leaf], Wait::AtMost(MS)).map(|e| e.id()).ok());
    if let Some(principal) = foreign {
        let _ = plate(&at, FOREIGN, Permit::Opener(principal), Mine::No);
    }

    // 五点六、**"判不了"（`Unjudged`）那两格**——都是确定性的，不看时序：
    //   at-pane   —— 规矩 = `Opener(/sys 那一格)`：那一号是块 `Pane`，**没有开者这一说**；
    //   gone-door —— 规矩 = `Opener(temp 那个**旧号**)`：先把 `temp` 落上、再剪掉，
    //                而**号不重用** ⇒ 那一格永久没有开者。
    // 照实记：`Unjudged` 这一格此前**只在宿主靶上有过读数**（那台靶已删；身份服务不答那一版在
    // 真机上要拆整机）。这两格与它同格不同因：判据不需要"那一格为什么没人"，只需要"**有没有那一位**"。
    let _ = plate(&at, AT_PANE, Permit::Opener(sys.id()), Mine::No);
    let temp_id = plate(&at, TEMP, Permit::Unset, Mine::No);
    let trimmed = temp_id.get() != 0 && at.trim(temp_id, Wait::AtMost(MS)).is_ok();
    let _ = plate(&at, GONE_DOOR, Permit::Opener(temp_id), Mine::No);

    // 五点七、**"改"那一轴那一格**：本域声明归自己（`Mine::Yes`）。
    //
    // 下面在 `adopt(q)` **之后**再落一次同一格——那是这一刀要量的那件事：**归属记的是"命"而不是
    // "身份"**（`owner` 那一格记的是任务；从前那本账里 `who` + `pie` 两格也都是任务级的）
    // ⇒ 主人**换了代表照样能改自己的格子**，
    // 而同一次 `Trunk(p)` 已经答了 `8`（"用"那一轴随身份走）。两条轴各问各的问题，各自自洽。
    let mine_id = plate(&at, MINE, Permit::Unset, Mine::Yes);

    // 六、以 `p` 试五遍——前三条**正证**，后两条是 `Opener` 的正负两面。
    let is = look(&root, dir, pane, IS, Wait::AtMost(MS));
    let under = look(&root, dir, pane, UNDER, Wait::AtMost(MS));
    let inside = look(&root, dir, pane, IN, Wait::AtMost(MS));
    let open = look(&root, dir, pane, OPEN, Wait::AtMost(MS));
    let foreign = look(&root, dir, pane, FOREIGN, Wait::AtMost(MS));
    let on_pane = look(&root, dir, pane, AT_PANE, Wait::AtMost(MS));
    let on_gone = look(&root, dir, pane, GONE_DOOR, Wait::AtMost(MS));

    // 七、**换一位代表**（同一个 TID）：领到自己派生的那条号底下。
    let adopt = iset.principal(p).adopt(q, Wait::AtMost(MS)).is_ok();

    // 八、以 `q` 再试——前两条**负证**、第三条仍是正证（"看支不看相等"）；
    //     `open` 那一格**照旧过**：开者与问的人是**同一条 TID**，换代表之后两边一起变成 `q`
    //     ——这正是"规矩随**身份**走、不随 TID 走"与 `Trunk` 那一格（拒）的分野。
    let is_sub = look(&root, dir, pane, IS, Wait::AtMost(MS));
    let under_sub = look(&root, dir, pane, UNDER, Wait::AtMost(MS));
    let in_sub = look(&root, dir, pane, IN, Wait::AtMost(MS));
    let open_sub = look(&root, dir, pane, OPEN, Wait::AtMost(MS));
    // **没记许可那一格**以 `q` 再问一遍：`Unset` 的判据只到"你有没有身份"那一格，它**不判
    // "是不是你"**——这正是它与 `Trunk(p)`（上面 `is_sub` 答拒）的分野。这一条此前**零断言**：
    // `mine` 那一格只被用来量「改」那一轴（下面的 `keep`），"用"那一轴没人问过它。
    let mine_sub = look(&root, dir, pane, MINE, Wait::AtMost(MS));
    // 再用一枚**新孔重落**自己那一格（换绑）：走的就是 `claimable` 那一支。
    let keep: Result<(), Fail> = match mail::unseal_hole(env::Mark::of("rule-entry")) {
        Ok(entry) if mine_id.get() != 0 => at
            .bind(mine, entry, Permit::Unset, Mine::Yes, Wait::AtMost(MS))
            .map(|_| ()),
        _ => Err(Fail::Unknown),
    };

    // 九、**两行**读数（**错误那一格从数字变成名字**：新面答的是 [`Fail`]，不是裸码）。
    //
    // **照实记（一行读数有个硬上限：256 字节，这一刀撞上了）**：`debug!` 在核里被
    // `DBCN_MAX = 256` 截断（`kernel/src/runtime/switcher/envcall/debug.rs`；`runtime::env::debug`
    // 的正文里也写着"超了就印前 256 字节，**这不是错误**"）。本刀往这一行加了 `mine_sub` 那一格
    // ⇒ 长度到 **257**，尾巴当场没了：实测那一行停在 `… mine=36 mine_s`，**`keep` 那一格再也
    // 看不见**（判据还在，读数丢了）。故按"以 `p` 那一趟 / 以 `q` 那一趟"拆两行：一格不少，
    // 两行都在限内。
    debug!(
        "probe-rule: tree part={} made={made} p={} adopt={} \
         is={is:?} under={under:?} in={inside:?} \
         door={} open={open:?} foreign={foreign:?} \
         trim={} at_pane={on_pane:?} gone_door={on_gone:?} mine={} \
        ",
        pane_id.get(),
        p.get(),
        adopt as u8,
        door_id.get(),
        trimmed as u8,
        mine_id.get(),
    );
    debug!(
        "probe-rule: tree(q) is_sub={is_sub:?} under_sub={under_sub:?} in_sub={in_sub:?} \
         open_sub={open_sub:?} mine_sub={mine_sub:?} keep={keep:?}"
    );

    // 十、判据：**一例一条**（用户裁定"程序侧 pilot"）。
    //
    // 照实记：原先这十几条被 `&&` 成**一个** `held`，红了只知道 `probe-rule: a rule did NOT
    // hold`——还得回头看上面那 19 个计数器才认得出是哪一条。现在一例一个名字，而**名字就是
    // 结论**（每一例后面那句"为什么"，与头注里那几条同源）。
    // —— 装配：一条规矩没落上，后面全没意义。故它排第一：红了不会被后面的假红淹没。
    {
        assert!(made == 3, "made={made}")
    }
    // —— 以 p 试（`p = resolve(self)`）：三条正证。
    assert_eq!(is, Ok(()));
    {
        assert_eq!(under, Ok(()))
    }
    {
        assert_eq!(inside, Ok(()))
    }
    // —— `Opener` 的正负两面。
    {
        assert_eq!(open, Ok(()))
    }
    {
        assert_eq!(foreign, Err(Fail::Denied), "别人开着的那一格，我该被拒")
    }
    // —— 两格「判不了」：`Unjudged` 单列的理由正是"这一格没通"，故它**不算通过**。
    {
        assert_eq!(on_pane, Err(Fail::Unjudged), "那一号是块 Pane：没有开者这一说")
    }
    {
        assert_eq!(on_gone, Err(Fail::Unjudged), "那一格剪掉了 ⇒ 永久没有开者")
    }
    // —— `trim` 那一手真的落下去了（上面 `gone-door` 那一格的前提）。
    {
        assert!(trimmed, "temp 没剪掉")
    }
    // —— 换一位代表（**同一条 TID**）：`adopt` 成功；两条负证、两条仍是正证。
    {
        assert!(adopt, "adopt(q) 没成功")
    }
    {
        assert_eq!(is_sub, Err(Fail::Denied), "换代表之后 Trunk(p) 该拒")
    }
    {
        assert_eq!(in_sub, Err(Fail::Denied), "换代表之后不在那枚盟里了")
    }
    {
        assert_eq!(under_sub, Ok(()), "q 仍在 p 那一支里 ⇒ Bough(p) 照旧过")
    }
    {
        {
            assert_eq!(
                open_sub,
                Ok(()),
                "开者与问的是同一条 TID ⇒ 两边一起变成 q"
            )
        }
    }
    // —— 没记许可那一格：**"用"那一轴的第一格**（"你有没有身份"）——它不判"是不是你"。
    {
        assert_eq!(
            mine_sub,
            Ok(()),
            "没记许可 ⇒ 只判有没有身份 ⇒ 换了代表那位该照样过（与 Trunk(p) 的分野）"
        )
    }
    // —— "改"那一轴：归属记的是**命**，换代表之后自己那一格照样改得。
    {
        assert_eq!(keep, Ok(()), "归属记的是命，换代表照样改得")
    }

    return Report::note(E_OK, OK_NOTE);
}

/// 落一格，带一条规矩；答那一格自己的号（`0` = 没落成）。
///
/// **`0` 当哨兵是安全的**：零号那一格是 `/sys`，本域跑起来的时候它早被占掉了（`principal`
/// / `coalition` 起头就分了它，见装配表），故这时落出来的号不可能是 `0`。
///
/// **照实记（收 `&Pane`，task-2 那一刀）**：落那一手在 [`Pane::bind`] 上（宾语 = 那一块窗格），
/// 故不再收裸 `(talk, link, host)` + 坐标——对端号与那条线都在那一面里面；
/// `Mine` 那一格也不再中途降成裸布尔。
fn plate(pane: &Pane<'_>, name: &str, permit: Permit, mine: Mine) -> EntryId {
    let Ok(entry) = mail::unseal_hole(env::Mark::of("rule-entry")) else {
        return EntryId::new(0);
    };
    let Ok(one) = Name::new(name) else {
        return EntryId::new(0);
    };
    pane.bind(one, entry, permit, mine, Wait::AtMost(MS))
        .map(|landed| landed.id())
        .unwrap_or(EntryId::new(0))
}

/// 拿那一格去 `find`：`Ok(())` = 放行；答不出 / 门禁答"不"落 [`Fail`]（本程序只看那一格，不看
/// 要回来的那一枚）。
///
/// **照实记（task-2 那一刀）**：旧面按**号**寻（`Face::find` 答 `(码, 入口)`）；新面只有
/// "按路取那一格"（[`Pane::tile`]）与"要那一枚"（[`Tile::token`]）两手——那一格由**路**认
/// （名字 → 号 → 门闩这一趟在那一面里面），故这里收一条**从根出发**的路（挂在那块根 Pane 上）。
///
/// **不走 `Face::tile`**：它会先 `find` 一次（授一枚没人接的副本），随后 `Tile::token` 再
/// `find` 一次——那两块号都是旧面没有的；这两格是自己刚落的，故译号不必重试。
///
/// **`id` 那一个哨兵退场**：从前"牌没落上（号 = 0）"答 `UNKNOWN`；今天没落上就是那一格不在树上，
/// 同一条路照样答 [`Fail::Unknown`]。
fn look(root: &Pane<'_>, dir: Name, pane: Name, name: &str, millis: Wait) -> Result<(), Fail> {
    let Ok(one) = Name::new(name) else {
        return Err(Fail::Unknown);
    };
    root.tile(&[dir, pane, one], millis)?
        .token(millis)
        .map(|_| ())
}

/// 按名字找一面服务门牌——与 `subject` / `member` 那两台同形。
///
/// **间接寻址那一手**（名字 → 号：**译不出就重试**，那两个域可能落得比本域晚。`road` 是那条路
/// ——2 段：盟册那一面；3 段：名册那两面各一条 `/sys/principal/{ask,set}`）落在
/// [`Pane::tile`] 上，`find`（把那枚门闩授过来）落在 [`Tile::token`] 上——**两格各一趟**，
/// 与旧 `Face::tile` 逐格同形。
///
/// **照实记（task-2 那一刀；为什么不用 `Face::tile`）**：`entry` 自己已经译号一次 + `find`
/// 一次，随后 `Tile::token` 又 `find` 一次 ⇒ **每趟多授一枚没人接的副本**进本域表。旧面
/// `entry_of` 只有一枚，故这里也照一枚写（那一套重试圈照旧留着）。
///
/// **照实记（收 `&TreeFace`，不再是 `&Session`）**：调用方**已持**一面（task-2 那一刀包出来的）。
fn find_face(tree: &TreeFace, road: &[Name]) -> Option<PieToken> {
    let root = tree.root();
    let mut left = MS;
    loop {
        match root
            .tile(road, Wait::AtMost(MS))
            .and_then(|entry| entry.token(Wait::AtMost(MS)))
        {
            Ok(entry) => return Some(entry),
            Err(Fail::Unknown) if left > 0 => {
                let _ = runtime::env::room::sleep(core::time::Duration::from_millis(1));
                left = left.saturating_sub(1);
            }
            Err(_) => return None,
        }
    }
}

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）。
fn bail<'a>(note: &'a str) -> Report<'a> {
    debug!("{}", note);
    return Report::note(E_TRIP, note);
}

