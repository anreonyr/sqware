#![no_std]
#![no_main]

//! Publish selector fixtures through Control; verify use and publisher ownership independently.

extern crate alloc;
extern crate programs;

use alloc::string::ToString;

use env::Wait;
use programs::Report;

use protocol::common::path::Path;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::system::identity as icall;
use protocol::system::identity::client::{Organization, Query, SelfOps};
use protocol::system::identity::{Selector, Subject};
use protocol::system::operator::client as operator;
use protocol::system::operator::client::{Face as TreeFace, Mine, Pane};
use protocol::system::operator::{EntryId, Fail, Permit};
use protocol::system::control::publication;
use runtime::env::mail;
use runtime::env::unit as utask;

const DIR: &protocol::system::operator::Path = protocol::common::svc::SVC;
const PANE: &str = "rule";
/// 三格的名字（各挂一条规矩）
const IS: &str = "is";
const UNDER: &str = "under";
const IN: &str = "in";
const DOOR: &str = "door";
const OPEN: &str = "open";
/// （名字 → 号），不靠别人把号塞给我
const FOREIGN: &str = "foreign";
/// 先落、再**剪掉**的一枚门牌——留给下面 `gone-door` 那一格指它那个**旧号**
const TEMP: &str = "temp";
/// 规矩 = `Opener(/svc 那一格)` ⇒ 那一号是块 **`Pane`**（没有开者这一说）⇒ **判不了**
const AT_PANE: &str = "at-pane";
/// 规矩 = `Opener(剪掉的那一枚门牌号)` ⇒ 号**不重用** ⇒ 那一格永远没有开者 ⇒ **判不了**
const GONE_DOOR: &str = "gone-door";
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
    let authority = programs::system::identity::serve::source::authority()
        .expect("probe-rule: no Control-issued identity authority");
    let iask = Query::discover(&tree, authority, Wait::AtMost(MS))
        .expect("probe-rule: no identity query");
    let iset = SelfOps::discover(&tree, authority, Wait::AtMost(MS))
        .expect("probe-rule: no identity self actions");
    let coal = Organization::discover(&tree, authority, Wait::AtMost(MS))
        .expect("probe-rule: no organization actions");
    let p = iask.resolve(me, Wait::AtMost(MS)).unwrap()
        .expect("probe-rule: unbound").current.principal;
    let q = iset.derive(p, Wait::AtMost(MS)).expect("probe-rule: no sub identity");
    let c = coal.found(Wait::AtMost(MS)).expect("probe-rule: no coalition id");
    coal.admit(c, p, Wait::AtMost(MS)).expect("probe-rule: manager admit failed");
    assert_eq!(iask.amid(p, c, Wait::AtMost(MS)), Ok(true));
    // Boot installation deliberately selects no organizations. Admission must not
    // silently grant access; the MemberOf door below must therefore deny even p.

    let rule_road = DIR.try_join(PANE).unwrap();
    let root = tree.root();
    let is_id = plate(IS, Permit::Identity(Selector::Exact(p)));
    let under_id = plate(UNDER, Permit::Identity(Selector::DescendantOf(p)));
    let in_id = plate(IN, Permit::Identity(Selector::MemberOf(c)));
    let sys = tree.pane(DIR, Wait::AtMost(MS)).unwrap();
    let at = tree.pane(&rule_road, Wait::AtMost(MS)).unwrap();
    let pane_id = at.id();
    let made = [is_id, under_id, in_id]
        .iter()
        .filter(|id| id.get() != 0)
        .count();

    // 五点五、**点名那一格**（第五个规矩变体 `Opener`）：
    // 两个号都是**树上换来的**（`road` 把一条路译成号）——那一格的门牌在谁手里，由树说，
    // 不由别人告诉我。故这一台**没有 new 的任何机制**，只是把规矩那一格的号换了个来路。
    // `gone_id`）随之下岗（`plate` 仍照落，判据一条没动）。
    let door_id = plate(DOOR, Permit::Bound);
    let _ = plate(OPEN, Permit::Opener(door_id));
    // `/svc/sys/identity/resolve` 那一格的号：**点名那一手**（名字 → 号）。
    // 门闩——Pane::tile 就地问一次（不重试），Face::tile 带额度重试。本格用前者：这一台
    // 变松（见 denied 那边量同一件事的那一台）。
    // 这一问要的是**名册问面**那一格自己的号（规矩里那个 `Opener` 指它）。
    let foreign = icall::DIR
        .try_join(icall::Grant::Resolve.name())
        .and_then(|road| root.tile(&road, Wait::AtMost(MS)).map(|e| e.id()).ok());
    if let Some(principal) = foreign {
        let _ = plate(FOREIGN, Permit::Opener(principal));
    }

    let _ = plate(AT_PANE, Permit::Opener(sys.id()));
    let temp_id = plate(TEMP, Permit::Public);
    let trimmed = temp_id.get() != 0 && publication::Client::injected().unwrap().unpublish(target(TEMP), Wait::AtMost(MS)).is_ok();
    let _ = plate(GONE_DOOR, Permit::Opener(temp_id));

    // 下面在 `adopt(q)` **之后**再落一次同一格——这是要量的那件事：**归属记的是"命"而不是
    // "身份"**（`owner` 那一格记的是任务）⇒ 主人**换了代表照样能改自己的格子**，
    // 而同一次 Exact(p) 已经答了拒绝（"用"那一轴随身份走）。两条轴各问各的问题，各自自洽。
    let mine_id = plate(MINE, Permit::Bound);

    // **报"答得动了"**（Setup::Ready）：上面那几格全落完才算——`probe-rule-other` 读的就是它们
    // （与三台驱动、三台服务那几处**同一手**）。
    let _ = protocol::communication::session::establish::endpoint(
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
    let adopt = iset.adopt(Subject::new(q, &[]).unwrap(), Wait::AtMost(MS)).is_ok();

    //     `open` 那一格**照旧过**：开者与问的人是**同一条 TID**，换代表之后两边一起变成 `q`
    //     ——这正是"规矩随**身份**走、不随 TID 走"与 Exact 那一格（拒）的分野。
    let is_sub = look(&root, &rule_road, IS, Wait::AtMost(MS));
    let under_sub = look(&root, &rule_road, UNDER, Wait::AtMost(MS));
    let in_sub = look(&root, &rule_road, IN, Wait::AtMost(MS));
    let open_sub = look(&root, &rule_road, OPEN, Wait::AtMost(MS));
    // Bound 以 `q` 再问一遍，只判有没有绑定，不判是不是 p。
    // `mine` 那一格只被用来量「改」那一轴（下面的 `keep`），"用"那一轴没人问过它。
    let mine_sub = look(&root, &rule_road, MINE, Wait::AtMost(MS));
    let raw = at.bind(MINE.to_string(), mail::unseal_hole(env::Mark::of("rule-entry")).unwrap(),
        Permit::Bound, Mine::Yes, Wait::AtMost(MS));
    assert!(matches!(raw, Err(Fail::Denied)));
    let publisher = publication::Client::injected().unwrap();
    let keep = publisher.unpublish(target(MINE), Wait::AtMost(MS));
    assert_ne!(plate(MINE, Permit::Bound), mine_id, "retired mounts get new identifiers");

    // 九、**两行**读数（**错误那一格从数字变成名字**：新面答的是 Fail，不是裸码）。
    debug!(
        "probe-rule: tree part={} made={made} p={} adopt={} \
         is={is:?} under={under:?} in={inside:?} \
         door={} open={open:?} foreign={foreign:?} \
         trim={} at_pane={on_pane:?} gone_door={on_gone:?} mine={} \
        ",
        pane_id.get(),
        p.slot,
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
        assert_eq!(inside, Err(Fail::Denied), "eligible but inactive must be denied")
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
        assert_eq!(is_sub, Err(Fail::Denied), "换代表之后 Exact(p) 该拒")
    }
    {
        assert_eq!(in_sub, Err(Fail::Denied), "换代表之后不在那枚盟里了")
    }
    {
        assert_eq!(under_sub, Ok(()), "q 仍在 p 那一支里 ⇒ DescendantOf(p) 照旧过")
    }
    assert_eq!(open_sub, Ok(()), "开者与问的是同一条 TID ⇒ 两边一起变成 q");
    // —— 没记许可那一格：**"用"那一轴的第一格**（"你有没有身份"）——它不判"是不是你"。
    {
        assert_eq!(
            mine_sub,
            Ok(()),
            "Bound ⇒ 只判有没有绑定 ⇒ 换了代表那位该照样过"
        )
    }
    // —— "改"那一轴：归属记的是**命**，换代表之后自己那一格照样改得。
    {
        assert_eq!(keep, Ok(()), "归属记的是命，换代表照样改得")
    }

    let complete = protocol::communication::session::establish::claim(sire, env::Mark::of("probe-rule-verified"), Wait::AtMost(MS)).expect("probe-rule: peer completion channel");
    let mut verified = [0];
    runtime::env::mail::HolePie::from_token(complete).pull(&mut verified, Wait::AtMost(10_000)).expect("probe-rule: peer did not verify before retirement");
    return Report::note(E_OK, OK_NOTE);
}

/// 落一格，带一条规矩；答那一格自己的号（`0` = 没落成）
/// Identity 起头就分了 /svc，故这时落出来的号不可能是 `0`
fn target(name: &str) -> publication::Target {
    publication::Target::Service { scope: publication::Scope::Fixture, group: PANE.into(), name: name.into() }
}
fn plate(name: &str, permit: Permit) -> EntryId {
    let entry = mail::unseal_hole(env::Mark::of("rule-entry")).unwrap();
    publication::Client::injected().unwrap().publish(target(name), entry, permit, Wait::AtMost(MS)).unwrap()
}

/// 拿那一格去 `find`：`Ok(())` = 放行；答不出 / 门禁答"不"落 Fail（本程序只看那一格，不看
/// 要回来的那一枚）
/// **不走 Face::tile**：它会先 `find` 一次（授一枚没人接的副本），随后 Tile::token 再
/// `find` 一次——这两格是自己刚落的，故译号不必重试
/// 同一条路照样答 Fail::Unknown
fn look(root: &Pane<'_>, base: &Path, name: &str, millis: Wait) -> Result<(), Fail> {
    let road = base.try_join(name).ok_or(Fail::Unknown)?;
    root.tile(&road, millis)?.token(millis).map(|_| ())
}

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）
fn bail<'a>(note: &'a str) -> Report<'a> {
    debug!("{}", note);
    return Report::note(E_TRIP, note);
}
