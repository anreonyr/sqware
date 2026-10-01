#![no_std]
#![no_main]

//! # 为什么读数是"一位客人两枚身份"
//! 路（树上的条目是"名字 → 一枚 Pie"，**存不了号**；装配表的 args 没接）。故真机读数用**派生
//! 出来的第二条身份**：它一样是名册里真有的一格，盟册看得见"同一枚盟里有两位"。
//! # 为什么不上板
//! 装配表的最后一条：**收场由 `canonical` 那一条给**。

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use alloc::string::String;

use alloc::format;
use env::PieToken;
use protocol::common::path::Path;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::service::coalition as ccall;
use protocol::service::coalition::client::{Band, Bloc, Coalition, Face as CoalitionFace};
use protocol::service::coalition::{CoalitionId, Fail};
use protocol::service::operator::Fail as TreeFail;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::Face as TreeFace;
use protocol::service::principal as pcall;
use protocol::service::principal::Fail as PolicyFail;
use protocol::service::principal::PrincipalId;
use protocol::service::principal::client::Face as PolicyFace;
use protocol::wire::id::Id;
use runtime::env::unit as utask;

const MS: usize = 1000;

const E_OK: usize = 0;
const E_NO_SERVICE: usize = 1;

/// 册外那个号（伪造的线上值）。铸过的号是 `0..next`，故这个一定在册外。
const OUTSIDE: usize = 4095;

#[programs::entry]
fn main() -> Report<'static> {
    let sire = utask::sire();
    let me = utask::self_id();

    let Ok(session) = Session::open(sire, operator::BERTH, Wait::AtMost(MS)) else {
        return bail("member: no tree link");
    };
    let tree = TreeFace::of(session);
    // 盟册那**两面**：三条"问"的（`Amid` / `Band` / `Bloc`）在 Grant::Ask 上，
    // 三条"定"的（`Found` / `Enter` / `Leave`）在 Grant::Set 上。这一台**两面都要**——它立盟、
    // 入、出，也问盟籍、点名册。
    // **同一枚盟要两枚柄**（这是两面分开的代价）：`Coalition` 那个柄**绑在它来自的那一
    // 面上**（`Face::coalition(id)` 只是把一个宾语固定下来），故 `cset.found()` 拿到的柄
    // `enter` / `leave` 得动，而 `holds` / `members` 要走 `cask.coalition(id)` 那一枚。
    let (Some(cask), Some(cset)) = (
        ccall::DIR.try_join(ccall::Grant::Ask.name()),
        ccall::DIR.try_join(ccall::Grant::Set.name()),
    ) else {
        return bail("member: bad coalition face name");
    };
    let Some(entry) = find_face(&tree, &cask) else {
        return bail("member: no coalition ask face");
    };
    let Ok(cask) = CoalitionFace::of(entry) else {
        return bail("member: bad coalition ask face");
    };
    let Some(entry) = find_face(&tree, &cset) else {
        return bail("member: no coalition set face");
    };
    let Ok(cset) = CoalitionFace::of(entry) else {
        return bail("member: bad coalition set face");
    };

    // **两面各找一次**：三条"问"的（`Resolve` / `Sire` / `Heir`）在
    // Grant::Ask 上，四条"定"的（`Bind` / `Derive` / `Adopt` / `Waive`）在 Grant::Set
    // 上；下面每一处按**它问的是哪一类**挑门牌。
    let (Some(ask), Some(set)) = (
        pcall::DIR.try_join(pcall::Grant::Ask.name()),
        pcall::DIR.try_join(pcall::Grant::Set.name()),
    ) else {
        return bail("member: bad identity face name");
    };
    let Some(entry) = find_face(&tree, &ask) else {
        return bail("member: no identity ask face");
    };
    let Ok(ask) = PolicyFace::of(entry) else {
        return bail("member: bad identity ask face");
    };
    let Some(entry) = find_face(&tree, &set) else {
        return bail("member: no identity set face");
    };
    let Ok(set) = PolicyFace::of(entry) else {
        return bail("member: bad identity set face");
    };

    // 一、此刻代表谁——装配期绑的那一条。
    let mine = ask
        .task(me)
        .principal(Wait::AtMost(MS))
        .map(|found| found.map(|p| p.id()));
    debug!("member: me={}", one_opt(mine));
    let Ok(Some(p)) = mine else {
        return bail("member: unbound");
    };

    // 二、立两枚盟：号由服务发——**全局单一序列，只增**。
    // 更要紧的是：**"第一枚是零号"没有并发客人能证**（要证它得保证自己是第一枚），"号不跳"
    // **号只增**。至于号**能用**（进得去、查得着、放得下），由后面那一整串
    // （`enter` / `leave` / `waive` / `band` / `bloc`）证，不靠这两格。
    let c0 = cset.found(Wait::AtMost(MS));
    debug!("member: found={}", one_id(&c0));
    let c1 = cset.found(Wait::AtMost(MS));
    debug!("member: found={}", one_id(&c1));
    let (Ok(c0), Ok(c1)) = (c0, c1) else {
        return bail("member: no coalition id");
    };
    // 读那一侧的两枚柄：**同一枚盟，换一枚门牌**（见上面那条）。
    let r0 = cask.coalition(c0.id());
    let r1 = cask.coalition(c1.id());
    {
        assert!(c1.id().get() > c0.id().get())
    }

    // 三、立了不等于进了。
    let apart = r0.holds(p, Wait::AtMost(MS));
    debug!("member: amid(me,c0)={}", flag(apart));
    {
        assert_eq!(apart, Ok(false))
    }

    // 四、入：名册真的改了，而且**再入一遍还是 ok**（集合没有"第二次"）。
    let entered = c0.enter(Wait::AtMost(MS));
    debug!("member: enter(c0)={}", done(entered));
    let inside = r0.holds(p, Wait::AtMost(MS));
    debug!("member: amid(me,c0)={}", flag(inside));
    let again = c0.enter(Wait::AtMost(MS));
    debug!("member: enter(c0)={}", done(again));
    assert!(entered.is_ok());
    {
        assert_eq!(inside, Ok(true))
    }
    assert!(again.is_ok());

    // 五、同一条身份可以在第二枚盟里。
    let in_c1 = c1.enter(Wait::AtMost(MS));
    debug!("member: enter(c1)={}", done(in_c1));
    let amid_c1 = r1.holds(p, Wait::AtMost(MS));
    debug!("member: amid(me,c1)={}", flag(amid_c1));
    {
        assert!(in_c1.is_ok())
    }
    {
        assert_eq!(amid_c1, Ok(true))
    }

    let sub = set
        .principal(p)
        .derive(Wait::AtMost(MS))
        .map(|child| child.id());
    debug!("member: derive(me)={}", one_policy(sub));
    let Some(q) = sub.ok() else {
        return bail("member: no sub identity");
    };
    let adopted = set.principal(p).adopt(q, Wait::AtMost(MS));
    debug!("member: adopt(sub)={}", done(adopted));
    // `p` ⇒ 我**不是**它的盟主 ⇒ 这一问该被拒（Fail::NotChief，不是 `Unknown`：盟在、
    // 我也在册上，缺的只是"这一枚盟归不归你代报名"）。
    // **正证在设备账那一台手里**（生产里唯一的持有者）：`hub` 每类立一枚盟、再替四位驱动
    // `admit`（protocol::service::hub 的 `bond`）。本台不抢那一份读数。
    let not_chief = c0.admit(me, Wait::AtMost(MS));
    debug!("member: admit(c0)={}", done(not_chief.clone()));
    assert!(matches!(not_chief, Err(Fail::NotChief)));
    // 客侧没有"我是谁"那一格）。
    let q_in = c0.enter(Wait::AtMost(MS));
    debug!("member: enter(c0)={}", done(q_in));
    let p_there = r0.holds(p, Wait::AtMost(MS));
    debug!("member: amid(me,c0)={}", flag(p_there));
    let q_there = r0.holds(q, Wait::AtMost(MS));
    debug!("member: amid(sub,c0)={}", flag(q_there));
    {
        assert!(adopted.is_ok())
    }
    {
        assert!(q_in.is_ok())
    }
    {
        {
            assert_eq!(p_there, Ok(true));
            assert_eq!(q_there, Ok(true));
        }
    }

    // 七、**出的是那一对，不是那个人**：此刻代表 `sub`，故出掉的是 `sub` 那一行
    let left = c0.leave(Wait::AtMost(MS));
    debug!("member: leave(c0)={}", done(left));
    let q_gone = r0.holds(q, Wait::AtMost(MS));
    debug!("member: amid(sub,c0)={}", flag(q_gone));
    let p_still = r0.holds(p, Wait::AtMost(MS));
    debug!("member: amid(me,c0)={}", flag(p_still));
    assert!(left.is_ok());
    {
        {
            assert_eq!(q_gone, Ok(false));
            assert_eq!(p_still, Ok(true));
        }
    }

    // 八、弃回起点：键 = 身份那条定理的另一半——第一条身份那一行照旧在。
    let waived = set.principal(q).waive(Wait::AtMost(MS));
    debug!("member: waive={}", done(waived));
    let after_waive = r0.holds(p, Wait::AtMost(MS));
    debug!("member: amid(me,c0)={}", flag(after_waive));
    assert!(waived.is_ok());
    {
        assert_eq!(after_waive, Ok(true))
    }

    // 九、第三态：没铸过的盟（号是伪造的线上值）。
    let outside = CoalitionId::new(OUTSIDE);
    // 故两枚柄**都得是"面过了、核心拒"**那一条——若拿问面去 `enter`，量到的是 `Denied`
    let rout = cask.coalition(outside);
    let wout = cset.coalition(outside);
    let out_amid = rout.holds(p, Wait::AtMost(MS));
    debug!("member: amid(me,out)={}", flag(out_amid));
    let out_enter = wout.enter(Wait::AtMost(MS));
    debug!("member: enter(out)={}", done(out_enter));
    let out_leave = wout.leave(Wait::AtMost(MS));
    debug!("member: leave(out)={}", done(out_leave));
    {
        assert!(matches!(out_amid, Err(Fail::Unknown)))
    }
    {
        assert!(matches!(out_enter, Err(Fail::Unknown)))
    }
    {
        assert!(matches!(out_leave, Err(Fail::Unknown)))
    }

    // 十、伪造的**身份**号：答 false，**不是失败**——`p` 是标签，本册不去问名册。
    let forged = r1.holds(PrincipalId::new(OUTSIDE), Wait::AtMost(MS));
    debug!("member: amid(out,me)={}", flag(forged));
    {
        assert_eq!(forged, Ok(false))
    }

    // 十一、**一串**（取窗两条）：`band` 答成员、`bloc` 答盟籍（序都是号序）。
    let band = r0.members(None, Wait::AtMost(MS));
    debug!("member: band(c0)={}", band_ids(&band));
    // 拿末一枚当游标接着取：**阈值**语义下再往后没有了 ⇒ 空窗，**不是错**（也不是"过期游标"）。
    let after = band.as_ref().ok().and_then(|w| w.iter().last());
    let empty = r0.members(after, Wait::AtMost(MS));
    debug!("member: band(c0,next)={}", band_ids(&empty));
    // 没铸过的那枚盟：取窗这一条**有失败域**（同 amid）。
    let out_band = rout.members(None, Wait::AtMost(MS));
    debug!("member: band(out)={}", band_ids(&out_band));
    let bloc = cask.bloc(p, None, Wait::AtMost(MS));
    debug!("member: bloc(me)={}", bloc_ids(&bloc));
    {
        assert_eq!(band.as_ref().ok().map(|w| w.iter().count()), Some(1))
    }
    {
        assert_eq!(empty.as_ref().ok().map(|w| w.iter().count()), Some(0))
    }
    {
        assert!(matches!(out_band, Err(Fail::Unknown)))
    }
    {
        assert_eq!(bloc.as_ref().ok().map(|w| w.iter().count()), Some(2))
    }

    // 十二、**面那一格**：同一条问、同一个发送者，**只换门牌**——定面成、问面拒。
    // 这一对量得出来的正是"面"这件事本身；而"面不对"在**门外**就拦下了（连盟册都没看）。
    // （客人的下一步一样：换一枚门牌 / 换目标、别重试）。分得开它们的是服务那一行读数
    // `coalition: face=… asked=… denied`。
    let set_ok = cset.found(Wait::AtMost(MS));
    debug!("member: found(set)={}", one_id(&set_ok));
    let ask_no = cask.found(Wait::AtMost(MS));
    debug!("member: found(ask)={}", one_id(&ask_no));
    {
        assert!(set_ok.is_ok())
    }
    {
        assert!(matches!(ask_no, Err(Fail::Denied)))
    }

    return Report::note(E_OK, "member: done");
}

fn find_face(tree: &TreeFace, road: &Path) -> Option<PieToken> {
    // Pane::tile 与 Tile::token 上。
    let root = tree.root();
    let mut left = MS;
    loop {
        match root
            .tile(road, Wait::AtMost(MS))
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

/// 一条号 / 没绑 / 哪一格失败——**一行里说全**（读数靠这一行，不靠再跑一遍）。
fn one_opt<E: Why>(r: Result<Option<PrincipalId>, E>) -> String {
    match r {
        Ok(Some(p)) => format!("{}", p.get()),
        Ok(None) => String::from("none"),
        Err(fail) => format!("err:{}", fail.why()),
    }
}

/// 同一行读数：只答一条身份号的那几条（`derive`）。
fn one_policy<E: Why>(r: Result<PrincipalId, E>) -> String {
    match r {
        Ok(p) => format!("{}", p.get()),
        Err(fail) => format!("err:{}", fail.why()),
    }
}

/// 同一行读数：只答一枚盟号的那几条（`found`）。
fn one_id(r: &Result<Coalition<'_>, Fail>) -> String {
    match r {
        Ok(c) => format!("{}", c.id().get()),
        Err(fail) => format!("err:{}", fail.why()),
    }
}

/// 同一行读数：一窗的**三格事实**——几枚、窗外还有没有、是哪些号。
/// 三格分开写，是因为**只有前两格是判据**：号那一段跟着装配期铸出来的身份号走（同一份镜像、
/// 不同的启动次序就会差一位），拿它钉判据等于把一条与取窗无关的数钉进门里。
fn window_ids<T: Id>(more: bool, ids: impl Iterator<Item = T>) -> String {
    let mut out = String::new();
    let mut n = 0usize;
    for id in ids {
        if n > 0 {
            out.push(',');
        }
        out.push_str(&format!("{}", id.get()));
        n += 1;
    }
    if out.is_empty() {
        out.push('-');
    }
    format!("n{n} more={more} ids={out}")
}

/// 同一行读数：`band`（成员那一窗）答的那三格。
fn band_ids(r: &Result<Band<'_>, Fail>) -> String {
    match r {
        Ok(w) => window_ids(w.more(), w.iter()),
        Err(fail) => format!("err:{}", fail.why()),
    }
}

/// 同一行读数：`bloc`（盟籍那一窗）答的那三格。
fn bloc_ids(r: &Result<Bloc<'_>, Fail>) -> String {
    match r {
        Ok(w) => window_ids(w.more(), w.iter()),
        Err(fail) => format!("err:{}", fail.why()),
    }
}

/// 同一行读数：是 / 不是 / 哪一格失败。
fn flag<E: Why>(r: Result<bool, E>) -> String {
    match r {
        Ok(true) => String::from("true"),
        Ok(false) => String::from("false"),
        Err(fail) => format!("err:{}", fail.why()),
    }
}

/// 同一行读数：成了没有。
fn done<E: Why>(r: Result<(), E>) -> String {
    match r {
        Ok(()) => String::from("ok"),
        Err(fail) => format!("err:{}", fail.why()),
    }
}

/// 两个协议、两张失败表，但**读数要的是同一个形状**：一行字。
/// ——每家的格子名照它们自己那张线上表说，不另起词。
trait Why {
    fn why(&self) -> &'static str;
}

impl Why for Fail {
    fn why(&self) -> &'static str {
        match self {
            Fail::Unknown => "unknown",
            Fail::Full => "full",
            // 面那一格拒的码："你手里那一枚门牌给不了这一条"。
            Fail::Denied => "denied",
            // 代报名那一格拒的码："你不是这一枚盟的盟主"——本探针不代报名，
            Fail::NotChief => "not-chief",
        }
    }
}

impl Why for PolicyFail {
    fn why(&self) -> &'static str {
        match self {
            PolicyFail::Denied => "denied",
            PolicyFail::Unknown => "unknown",
            PolicyFail::Full => "full",
        }
    }
}

fn bail<'a>(msg: &'a str) -> Report<'a> {
    return Report::note(E_NO_SERVICE, msg);
}
