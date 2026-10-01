#![no_std]
#![no_main]

//! # 为什么不上板
//! # 树外那个号是**故意伪造的**
//! 树只增不删 ⇒ 号不会失效，"树外"只能由伪造或损坏的帧产生——这正是三态第三格存在的理由
//! （内核那两条身份凭证都答不出"这条号住不住在树上"；只有 Server 那张表答得出）。

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use alloc::string::String;

use alloc::format;
use env::PieToken;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::service::operator::Fail as TreeFail;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::Face as TreeFace;
use protocol::service::principal as pcall;
use protocol::service::principal::client::Face;
use protocol::service::principal::{Fail, PrincipalId};
use runtime::env::unit as utask;

const MS: usize = 1000;

const E_OK: usize = 0;
const E_NO_SERVICE: usize = 1;

/// 树外那个号（伪造的线上值）
const OUTSIDE: usize = 4095;

#[programs::entry]
fn main() -> Report<'static> {
    let sire = utask::sire();
    let me = utask::self_id();

    let Ok(session) = Session::open(sire, operator::BERTH, Wait::AtMost(MS)) else {
        return bail("subject: no tree link");
    };
    let tree = TreeFace::of(session);
    // **两面各找一次**：这一台**两面都要**——它证的正是名册那七条原语，而七条分住
    // 两面（三条"问"的 `Ask` / 四条"定"的 `Set`）。下面每一处按**它问的是哪一类**挑门牌。
    let Some(entry) = find_face(&tree, pcall::Grant::Ask) else {
        return bail("subject: no ask face");
    };
    let Ok(ask) = Face::of(entry) else {
        return bail("subject: bad ask face");
    };
    let Some(entry) = find_face(&tree, pcall::Grant::Set) else {
        return bail("subject: no set face");
    };
    let Ok(set) = Face::of(entry) else {
        return bail("subject: bad set face");
    };

    // 一、此刻代表谁——装配期绑的那一条（服务一起来就答得出）。
    let mine = ask
        .task(me)
        .principal(Wait::AtMost(MS))
        .map(|found| found.map(|p| p.id()));
    debug!("policy: me={}", one_opt(mine));
    let Ok(Some(p)) = mine else {
        return bail("subject: unbound");
    };

    // 二、三态的头两格。
    let no_sire = ask
        .principal(PrincipalId::ROOT)
        .sire(Wait::AtMost(MS))
        .map(|found| found.map(|s| s.id()));
    debug!("policy: sire(root)={}", one_opt(no_sire));
    let sired = ask
        .principal(p)
        .sire(Wait::AtMost(MS))
        .map(|found| found.map(|s| s.id()));
    debug!("policy: sire(me)={}", one_opt(sired));
    {
        assert_eq!(no_sire, Ok(None))
    }
    {
        assert_eq!(sired, Ok(Some(PrincipalId::ROOT)))
    }

    // 三、自反。
    let reflexive = ask.principal(p).contains(p, Wait::AtMost(MS));
    debug!("policy: heir(me,me)={}", flag(reflexive));
    {
        assert_eq!(reflexive, Ok(true))
    }

    // 四、向下派生一条自己的子身份。
    let sub = set
        .principal(p)
        .derive(Wait::AtMost(MS))
        .map(|child| child.id());
    debug!("policy: derive(me)={}", one(sub));
    let child = sub.ok();
    assert!(sub.is_ok());

    // 五、否定：子代不是祖先（拿刚派生出来的那一条问）。
    let not_ancestor = child.map(|q| ask.principal(p).contains(q, Wait::AtMost(MS)));
    if let Some(r) = not_ancestor {
        debug!("policy: heir(sub,me)={}", flag(r));
    }
    {
        assert_eq!(not_ancestor, Some(Ok(false)))
    }

    // 六、第三态：树外的号。
    let out_heir = ask
        .principal(p)
        .contains(PrincipalId::new(OUTSIDE), Wait::AtMost(MS));
    debug!("policy: heir(out,me)={}", flag(out_heir));
    {
        assert!(matches!(out_heir, Err(Fail::Unknown)))
    }

    let bound = set.task(me).bind(p, Wait::AtMost(MS));
    debug!("policy: bind(self)={}", done(bound));
    {
        assert!(matches!(bound, Err(Fail::Denied)))
    }

    let Some(q) = child else {
        return bail("subject: no sub identity");
    };
    // 八、领：换到自己刚派生出来的那一支里（`sub` 一定在 `p` 那一支里）。
    let adopted = set.principal(p).adopt(q, Wait::AtMost(MS));
    debug!("policy: adopt(sub)={}", done(adopted));
    {
        assert!(adopted.is_ok())
    }

    // 九、名册真的改了（不是打个印记）。
    let led = ask
        .task(me)
        .principal(Wait::AtMost(MS))
        .map(|found| found.map(|p| p.id()));
    debug!("policy: me={}", one_opt(led));

    // 十、**钥匙反证**：已不代表 `p`，故"从 `p` 派生"被拒。
    let stale = set
        .principal(p)
        .derive(Wait::AtMost(MS))
        .map(|child| child.id());
    debug!("policy: derive(old)={}", one(stale));
    {
        assert!(matches!(stale, Err(Fail::Denied)))
    }

    // 十一、向上 / 跨支：`p` 是 `sub` 的父，不在 `sub` 那一支里。
    let up = set.principal(q).adopt(p, Wait::AtMost(MS));
    debug!("policy: adopt(up)={}", done(up));
    {
        assert!(matches!(up, Err(Fail::Denied)))
    }

    // 十二、树外。
    let outside = set
        .principal(q)
        .adopt(PrincipalId::new(OUTSIDE), Wait::AtMost(MS));
    debug!("policy: adopt(out)={}", done(outside));
    {
        assert!(matches!(outside, Err(Fail::Unknown)))
    }

    // 十三、弃：回到装配给我的那一条（不删格）。
    let waived = set.principal(q).waive(Wait::AtMost(MS));
    debug!("policy: waive={}", done(waived));
    assert!(waived.is_ok());

    // 十四、回到起点。
    let back = ask
        .task(me)
        .principal(Wait::AtMost(MS))
        .map(|found| found.map(|p| p.id()));
    debug!("policy: me={}", one_opt(back));

    // 三条 `policy: me=`（装配绑的 / 领之后 / 弃之后）的关系：**绑 ≠ 领 = 弃**。
    {
        {
            assert_ne!(led, mine);
            assert_eq!(back, mine);
        }
    }

    // 十五、**面那一格**：同一条问、同一个发送者、同一把钥匙，**只换门牌**——
    // 定面成、问面拒。两面各一枚门牌，而"面不对"在**门外**
    // 就拦下了（连账都没看）。
    // 那一枚）也答 Fail::Denied——两个因落在同一格码上（客人的下一步一样：换人 / 换门牌、
    // 别重试）。分得开它们的是持册者那一行读数 `principal: face=… asked=… denied`。
    let set_ok = set
        .principal(p)
        .derive(Wait::AtMost(MS))
        .map(|child| child.id());
    debug!("policy: derive(set,p)={}", one(set_ok));
    let ask_no = ask
        .principal(p)
        .derive(Wait::AtMost(MS))
        .map(|child| child.id());
    debug!("policy: derive(ask,p)={}", one(ask_no));
    {
        assert!(set_ok.is_ok())
    }
    {
        assert!(matches!(ask_no, Err(Fail::Denied)))
    }

    return Report::note(E_OK, "subject: done");
}

/// 名字 → 号（译不出就重试）落在 Pane::tile 上，`find` 落在 Tile::token 上——**两格各
/// 一趟**
fn find_face(tree: &TreeFace, grant: pcall::Grant) -> Option<PieToken> {
    let road = pcall::DIR.try_join(grant.name())?;
    let root = tree.root();
    let mut left = MS;
    loop {
        match root
            .tile(&road, Wait::AtMost(MS))
            .and_then(|entry| entry.token(Wait::AtMost(MS)))
        {
            Ok(entry) => return Some(entry),
            Err(TreeFail::Unknown) if left > 0 => {
                let _ = runtime::env::room::sleep(core::time::Duration::from_millis(1));
                left = left.saturating_sub(1);
            }
            Err(_) => return None,
        }
    }
}

/// 一条号 / 没绑 / 哪一格失败——**一行里说全**（读数靠这一行，不靠再跑一遍）
fn one_opt(r: Result<Option<PrincipalId>, Fail>) -> String {
    match r {
        Ok(Some(p)) => format!("{}", p.get()),
        Ok(None) => String::from("none"),
        Err(fail) => format!("err:{}", why(fail)),
    }
}

/// 同一行读数：只答一条号的那几条（`derive`）
fn one(r: Result<PrincipalId, Fail>) -> String {
    match r {
        Ok(p) => format!("{}", p.get()),
        Err(fail) => format!("err:{}", why(fail)),
    }
}

/// 同一行读数：是 / 不是 / 哪一格失败
fn flag(r: Result<bool, Fail>) -> String {
    match r {
        Ok(true) => String::from("true"),
        Ok(false) => String::from("false"),
        Err(fail) => format!("err:{}", why(fail)),
    }
}

/// 同一行读数：成了没有
fn done(r: Result<(), Fail>) -> String {
    match r {
        Ok(()) => String::from("ok"),
        Err(fail) => format!("err:{}", why(fail)),
    }
}

/// 失败域那三格的名字（**照线上那张表说**，不另起词）
fn why(fail: Fail) -> &'static str {
    match fail {
        Fail::Denied => "denied",
        Fail::Unknown => "unknown",
        Fail::Full => "full",
    }
}

fn bail<'a>(msg: &'a str) -> Report<'a> {
    return Report::note(E_NO_SERVICE, msg);
}
