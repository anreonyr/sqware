#![no_std]
#![no_main]

//! :Trunk / Bough / Among / Opener，然后自己按身份试几遍，最后换一条身份再试。
//! 在某枚盟里 / 就是开着某一格的那一位），外加"**没记**"那一档；而真机上只有"没记"通电：
//! 真机上——一台客人演两个身份，故"许可随**身份**走、不随 TID 走"这一条也在同一行读数里。
//! # `Opener` 那一格：号从**树**上来
//! 没有一条是"按名字点名"。`Opener` 补的正是它——**先 `seek` 把一条路译成号**（名字 → 号，
//! road_id 那一手），再把那个号写进规矩；判的时候持树者去问"此刻谁占着那一格"。
//! **树就是名录**。
//! 门禁的三格答案里，`UNJUDGED`（判不了）此前**真机上没有读数**：要量它得让身份服务
//! **不答**，而那会把整机拆掉。`Opener` 让它可以被**确定性地**量出来，而且两条因
//! 各不相同：那一号是块 `Pane`（没有开者这一说）、那一格已经**剪掉**（号不重用 ⇒ 永久没有开者）。
//! 判据里这两格必须落在 `9`：落 `8`（终态拒）会让客人白放弃，落 `0`（放行）等于门禁不存在。
//! 这两因**永远好不了**，而 `9` 里还有"会好"的那一类（对面不答 / 超时）
//! ——两类同格是刻意的（客人的下一步相同）。持树者**各说一行读数**
//! （`operator: opens pane|gone|sealed n=…`）："为什么判不了"在真机上看得见。
//! `gone-door` 那一格顺带把 Permit::Opener 的一条**已知边界**量成了读数——"那一格被剪
//! 掉之后，指它的那条规矩永久判不了（重挂是**新号**）"。
//! # 为什么"另一台客人"也要来（`prog-probe-rule-other`）
//! 只许**往下**领（`heir(current, q)`）。"不在那一支里"的那一位只能是**另一台**——那正是
//! `probe-rule-other` 那一格（它顺带对 `foreign` 也量一遍：第三台同样过不去）。
//! # 这一台为什么把盟也带上
//! Permit::Among 是全仓**唯一**需要第二枚门牌（盟册）的判据：盟册那一枚没到持树者手里，
//! `amid` 就答"问不到"，那一格会翻成 `UNJUDGED(9)`——而**不是** `0` / `8`。故这一台的
//! `in` 那两格读数同时证两件事：规矩通了，**门也接上了**。
//! `found()` 只是**立一枚号**，"立了不等于进了"（见 protocol::service::coalition::core），

// ——与 `canonical` / `probe-denied` 同一条：`programs/src/user/mod.rs` 里没有它。
// 两条 `extern crate` 缺一不可（实测）：`alloc` 是 `format!` 要用；`programs` **不是**为了
// 用它里面的东西，而是为了把 `libprograms` 链进来——**panic handler 与 `_start` 都住那份
// lib**（`programs/src/entry.rs`）。少了它，链接期报 `` `#[panic_handler]` function required ``。
extern crate alloc;
extern crate programs;

use alloc::string::ToString;

use env::Wait;
use programs::Report;

use env::PieToken;
use protocol::common::path::Path;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::service::coalition as ccall;
use protocol::service::coalition::client::Face as CoalitionFace;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::{Face as TreeFace, Mine, Pane};
use protocol::service::operator::{EntryId, Fail, Permit};
use protocol::service::principal as pcall;
use protocol::service::principal::client::Face as PrincipalFace;
use runtime::env::mail;
use runtime::env::unit as utask;

const DIR: &protocol::service::operator::Path = protocol::common::svc::SVC;
const PANE: &str = "rule";
/// 三格的名字（各挂一条规矩）。
const IS: &str = "is";
const UNDER: &str = "under";
const IN: &str = "in";
const DOOR: &str = "door";
const OPEN: &str = "open";
/// （名字 → 号），不靠别人把号塞给我。
const FOREIGN: &str = "foreign";
/// 先落、再**剪掉**的一枚门牌——留给下面 `gone-door` 那一格指它那个**旧号**。
const TEMP: &str = "temp";
/// 规矩 = `Opener(/svc 那一格)` ⇒ 那一号是块 **`Pane`**（没有开者这一说）⇒ **判不了**。
const AT_PANE: &str = "at-pane";
/// 规矩 = `Opener(剪掉的那一枚门牌号)` ⇒ 号**不重用** ⇒ 那一格永远没有开者 ⇒ **判不了**。
const GONE_DOOR: &str = "gone-door";
/// Node::Tile 的 `owner` 那一格）。
const MINE: &str = "mine";

const MS: usize = 1000;

const E_OK: usize = 0;
const E_TRIP: usize = 1;

const OK_NOTE: &str = "probe-rule: the rules held";

#[programs::entry]
fn main() -> Report<'static> {
    let sire = utask::sire();
    let me = utask::self_id();

    let Ok(session) = Session::open(sire, operator::BERTH, Wait::AtMost(MS)) else {
        return bail("probe-rule: no tree link");
    };
    let tree = TreeFace::of(session);
    // （`Found` / `Enter`）；它不查盟籍、不点名册。
    let Some(cset) = ccall::DIR.try_join(ccall::Grant::Set.name()) else {
        return bail("probe-rule: bad coalition name");
    };
    let Some(entry) = find_face(&tree, &cset) else {
        return bail("probe-rule: no coalition set face");
    };
    let Ok(coal) = CoalitionFace::of(entry) else {
        return bail("probe-rule: bad coalition set face");
    };
    // 身份那**两面**：三条"问"的（`Resolve` / `Sire` / `Heir`）在 Grant::Ask 上，
    // 四条"定"的（`Bind` / `Derive` / `Adopt` / `Waive`）在 Grant::Set 上。这一台**两面都要**
    // ——它要 `Resolve` 问"我代表谁"、要 `Derive` 派生一条子身份、要 `Adopt` 换一位代表。
    let (Some(iask), Some(iset)) = (
        pcall::DIR.try_join(pcall::Grant::Ask.name()),
        pcall::DIR.try_join(pcall::Grant::Set.name()),
    ) else {
        return bail("probe-rule: bad identity face name");
    };
    let Some(entry) = find_face(&tree, &iask) else {
        return bail("probe-rule: no identity ask face");
    };
    let Ok(iask) = PrincipalFace::of(entry) else {
        return bail("probe-rule: bad identity ask face");
    };
    let Some(entry) = find_face(&tree, &iset) else {
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
    let Ok(c) = coal.found(Wait::AtMost(MS)) else {
        return bail("probe-rule: no coalition id");
    };
    if c.enter(Wait::AtMost(MS)).is_err() {
        return bail("probe-rule: enter failed");
    }

    // 四、分 `/svc/rule`（"分"是幂等的，故重来一次也无事）。
    let Some(dir) = DIR.file_name() else {
        return bail("probe-rule: bad name");
    };
    let pane = PANE.to_string();
    // 本台那几问都从这一条路起（`/svc/rule`）——一处都不自己拼。
    let Some(rule_road) = DIR.try_join(PANE) else {
        return bail("probe-rule: bad name");
    };
    let mine = MINE.to_string();
    let root = tree.root();
    let Ok(sys) = root.open(dir.to_string(), Wait::AtMost(MS)) else {
        return bail("probe-rule: no /svc");
    };
    let Ok(at) = sys.open(pane, Wait::AtMost(MS)) else {
        return bail("probe-rule: no /svc/rule");
    };
    let pane_id = at.id();

    // 五、落三格，各带一条规矩。Mine::No：这一台证的是**"用"那一轴**，故不声明归属
    //     （那一轴由 `probe-owner` / `probe-lease` 那两台管）。
    let is_id = plate(&at, IS, Permit::Trunk(p), Mine::No);
    let under_id = plate(&at, UNDER, Permit::Bough(p), Mine::No);
    let in_id = plate(&at, IN, Permit::Among(c.id()), Mine::No);
    let made = [is_id, under_id, in_id]
        .iter()
        .filter(|id| id.get() != 0)
        .count();

    // 五点五、**点名那一格**（第五个规矩变体 `Opener`）：
    // 两个号都是**树上换来的**（`road` 把一条路译成号）——那一格的门牌在谁手里，由树说，
    // 不由别人告诉我。故这一台**没有 new 的任何机制**，只是把规矩那一格的号换了个来路。
    // `gone_id`）随之下岗（`plate` 仍照落，判据一条没动）。
    let door_id = plate(&at, DOOR, Permit::Unset, Mine::No);
    let _ = plate(&at, OPEN, Permit::Opener(door_id), Mine::No);
    // `/svc/sys/principal/ask` 那一格的号：**点名那一手**（名字 → 号），与 `find_face` 走同一条路。
    // 门闩——Pane::tile 就地问一次（不重试），Face::tile 带额度重试。本格用前者：这一台
    // 变松（见 denied 那边量同一件事的那一台）。
    // 这一问要的是**名册问面**那一格自己的号（规矩里那个 `Opener` 指它）。
    let foreign = pcall::DIR
        .try_join(pcall::Grant::Ask.name())
        .and_then(|road| root.tile(&road, Wait::AtMost(MS)).map(|e| e.id()).ok());
    if let Some(principal) = foreign {
        let _ = plate(&at, FOREIGN, Permit::Opener(principal), Mine::No);
    }

    let _ = plate(&at, AT_PANE, Permit::Opener(sys.id()), Mine::No);
    let temp_id = plate(&at, TEMP, Permit::Unset, Mine::No);
    let trimmed = temp_id.get() != 0 && at.trim(temp_id, Wait::AtMost(MS)).is_ok();
    let _ = plate(&at, GONE_DOOR, Permit::Opener(temp_id), Mine::No);

    // 下面在 `adopt(q)` **之后**再落一次同一格——这是要量的那件事：**归属记的是"命"而不是
    // "身份"**（`owner` 那一格记的是任务）⇒ 主人**换了代表照样能改自己的格子**，
    // 而同一次 `Trunk(p)` 已经答了 `8`（"用"那一轴随身份走）。两条轴各问各的问题，各自自洽。
    let mine_id = plate(&at, MINE, Permit::Unset, Mine::Yes);

    // **报"答得动了"**（Setup::Ready）：上面那几格全落完才算——`probe-rule-other` 读的就是它们
    // （与三台驱动、三台服务那几处**同一手**）。
    let _ = protocol::communication::establish::endpoint(
        utask::sire(),
        env::Mark::of(programs::unit::READY),
        env::Wait::POLL,
    );

    // 六、以 `p` 试五遍——前三条**正证**，后两条是 `Opener` 的正负两面。
    let is = look(&root, &rule_road, IS, Wait::AtMost(MS));
    let under = look(&root, &rule_road, UNDER, Wait::AtMost(MS));
    let inside = look(&root, &rule_road, IN, Wait::AtMost(MS));
    let open = look(&root, &rule_road, OPEN, Wait::AtMost(MS));
    let foreign = look(&root, &rule_road, FOREIGN, Wait::AtMost(MS));
    let on_pane = look(&root, &rule_road, AT_PANE, Wait::AtMost(MS));
    let on_gone = look(&root, &rule_road, GONE_DOOR, Wait::AtMost(MS));

    // 七、**换一位代表**（同一个 TID）：领到自己派生的那条号底下。
    let adopt = iset.principal(p).adopt(q, Wait::AtMost(MS)).is_ok();

    //     `open` 那一格**照旧过**：开者与问的人是**同一条 TID**，换代表之后两边一起变成 `q`
    //     ——这正是"规矩随**身份**走、不随 TID 走"与 `Trunk` 那一格（拒）的分野。
    let is_sub = look(&root, &rule_road, IS, Wait::AtMost(MS));
    let under_sub = look(&root, &rule_road, UNDER, Wait::AtMost(MS));
    let in_sub = look(&root, &rule_road, IN, Wait::AtMost(MS));
    let open_sub = look(&root, &rule_road, OPEN, Wait::AtMost(MS));
    // **没记许可那一格**以 `q` 再问一遍：`Unset` 的判据只到"你有没有身份"那一格，它**不判
    // "是不是你"**——这正是它与 `Trunk(p)`（上面 `is_sub` 答拒）的分野。这一条此前**零断言**：
    // `mine` 那一格只被用来量「改」那一轴（下面的 `keep`），"用"那一轴没人问过它。
    let mine_sub = look(&root, &rule_road, MINE, Wait::AtMost(MS));
    // 再用一枚**新孔重落**自己那一格（换绑）：走的就是 `claimable` 那一支。
    let keep: Result<(), Fail> = match mail::unseal_hole(env::Mark::of("rule-entry")) {
        Ok(entry) if mine_id.get() != 0 => at
            .bind(mine, entry, Permit::Unset, Mine::Yes, Wait::AtMost(MS))
            .map(|_| ()),
        _ => Err(Fail::Unknown),
    };

    // 九、**两行**读数（**错误那一格从数字变成名字**：新面答的是 Fail，不是裸码）。
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
    {
        assert_eq!(
            on_pane,
            Err(Fail::Unjudged),
            "那一号是块 Pane：没有开者这一说"
        )
    }
    {
        assert_eq!(on_gone, Err(Fail::Unjudged), "那一格剪掉了 ⇒ 永久没有开者")
    }
    // —— `trim` 那一手真的落下去了（上面 `gone-door` 那一格的前提）。
    {
        assert!(trimmed, "temp 没剪掉")
    }
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
    assert_eq!(open_sub, Ok(()), "开者与问的是同一条 TID ⇒ 两边一起变成 q");
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
/// / `coalition` 起头就分了它，见装配表），故这时落出来的号不可能是 `0`。
fn plate(pane: &Pane<'_>, name: &str, permit: Permit, mine: Mine) -> EntryId {
    let Ok(entry) = mail::unseal_hole(env::Mark::of("rule-entry")) else {
        return EntryId::new(0);
    };
    let one = name.to_string();
    pane.bind(one, entry, permit, mine, Wait::AtMost(MS))
        .map(|landed| landed.id())
        .unwrap_or(EntryId::new(0))
}

/// 拿那一格去 `find`：`Ok(())` = 放行；答不出 / 门禁答"不"落 Fail（本程序只看那一格，不看
/// 要回来的那一枚）。
/// **不走 Face::tile**：它会先 `find` 一次（授一枚没人接的副本），随后 Tile::token 再
/// `find` 一次——这两格是自己刚落的，故译号不必重试。
/// 同一条路照样答 Fail::Unknown。
fn look(root: &Pane<'_>, base: &Path, name: &str, millis: Wait) -> Result<(), Fail> {
    let road = base.try_join(name).ok_or(Fail::Unknown)?;
    root.tile(&road, millis)?.token(millis).map(|_| ())
}

/// 按名字找一面服务门牌——与 `subject` / `member` 那两台同形。
/// Pane::tile 上，`find`（把那枚门闩授过来）落在 Tile::token 上——**两格各一趟**，
fn find_face(tree: &TreeFace, road: &Path) -> Option<PieToken> {
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
