#![no_std]
#![no_main]

//! member — **盟友**：立盟、进出、问在不在，把这一族要验的读数打出来。
//!
//! ```text
//!   1  树那条路：seat(树) + claim(生我者, 树) + 另铸一枚问话孔给持树者
//!   2  FIND "/sys/coalition" ⇒ 结盟服务的门牌；FIND "/sys/principal" ⇒ 身份服务的门牌
//!   3  resolve(self)          ⇒ 本域此刻代表哪个号（装配期绑的那一条）
//!   4  found() × 2            ⇒ 立两枚盟：号 0 与 1（号由服务发：单调、稠密）
//!   5  amid(me, c0)           ⇒ **立了不等于进了**：false
//!   6  enter(c0) → amid → enter(c0)  ⇒ 入、真的改了、**再入一遍答 ok（幂等）**
//!   7  enter(c1)              ⇒ 同一条身份可以在第二枚盟里
//!   8  derive(me) + adopt(sub) ⇒ 领到第二条身份
//!   9  enter(c0)              ⇒ 此刻代表的是 sub ⇒ 这枚盟里**有两位**
//!  10  amid(me,c0) / amid(sub,c0) ⇒ 两条都在（**出的是那一对，不是那个人**）
//!  11  leave(c0)             ⇒ 出；amid(sub,c0)=false、amid(me,c0) 照旧 true
//!  12  waive()               ⇒ 弃回起点，盟籍照旧（键 = 身份那条定理）
//!  13  没铸过的号             ⇒ amid / enter / leave 都答 Unknown（**第三态**）
//!  14  伪造的身份号            ⇒ amid 答 false（**不是失败**——`p` 是标签，本册不问名册）
//!  15  取窗：band(c0) ⇒ 一位；band(c0, 末一枚) ⇒ **空窗**（游标是阈值）；band(out) ⇒ Unknown
//!  16  bloc(me)               ⇒ 反向：两条盟籍
//! ```
//!
//! # 为什么读数是"一位客人两枚身份"
//!
//! 号由服务铸（正文 K3）⇒ 要有第二方进同一枚盟，得先有人把号交到它手里；本仓今天没有那条
//! 路（树上的条目是"名字 → 一枚 Pie"，**存不了号**；装配单的 args 没接）。故真机读数用**派生
//! 出来的第二条身份**：它一样是名册里真有的一格，盟册看得见"同一枚盟里有两位"。
//!
//! # 为什么不上板
//!
//! 本域只做一件事——结盟；生死那本账与本域无关（同 `subject` / `lodger` 那一档）。它也不是
//! 装配单的最后一条：**收场由 `echo` 那一条给**。

extern crate alloc;
extern crate programs;

use programs::Report;

use alloc::format;
use alloc::string::String;
use core::time::Duration;

use env::{Name, PieToken, TaskId};
use protocol::coalition::call as ccall;
use protocol::coalition::client::Face as CoalitionFace;
use protocol::coalition::core::{CoalitionId, Fail, Id, Window};
use protocol::operator::call as ocall;
use protocol::operator::client as operator;
use protocol::principal::call as pcall;
use protocol::principal::client::Face as PolicyFace;
use protocol::principal::core::Fail as PolicyFail;
use protocol::principal::core::PrincipalId;
use protocol::session::Quay;
use cases::Suite;
use runtime::env::debug;
use runtime::env::room;
use runtime::env::unit as utask;

/// 等树 / 等答 / 找门牌的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 门牌可能落得比本域晚：找不到就再问一次的间隔（毫秒）。
const RETRY_MS: usize = 1;

/// 退场码：走通了 / 没走通（都不是 panic；kernel 会把那一行连同域号打出来）。
const E_OK: usize = 0;
const E_NO_SERVICE: usize = 1;

/// 册外那个号（伪造的线上值）。铸过的号是 `0..next`，故这个一定在册外。
const OUTSIDE: usize = 4095;

#[programs::entry]
fn main() -> Report<'static> {
    let Ok(sire) = utask::sire() else { return bail("member: no sire") };
    let Ok(me) = utask::self_id() else { return bail("member: no self id") };

    // 上树：本域只开一条链，走两趟按名字找（结盟服务那一面 + 身份服务那一面）。
    let Ok((tree, host)) = operator::open(sire, MS) else { return bail("member: no tree link") };
    let Ok(talk) = operator::ask_hole(host) else { return bail("member: no tree ask") };

    let Some(entry) = find_face(&tree, talk, host, ccall::DIR, ccall::NAME) else { return bail("member: no coalition") };
    let Ok(coal) = CoalitionFace::of(entry) else { return bail("member: bad coalition face") };

    // 身份那一面：**本域自己也要用它**（派生第二条身份、领、弃）。
    let Some(entry) = find_face(&tree, talk, host, pcall::DIR, pcall::NAME) else { return bail("member: no identity") };
    let Ok(policy) = PolicyFace::of(entry) else { return bail("member: bad identity face") };

    // 一、此刻代表谁——装配期绑的那一条。
    let mine = policy.resolve(me, MS);
    say(&format!("member: me={}", one_opt(mine)));
    let Ok(Some(p)) = mine else { return bail("member: unbound") };

    // 判据就地登记（用户裁定"服务台搬进 SUT"）：**只搬本域已经在判的东西**——下面每一例的期望，
    // 都是本域头注那 16 步里写着的那一句（旧宿主靶上 `member: …` 那 21 条钉的就是它们）。
    // 台名 = 本域打的前缀，门按它钉逐台基线。
    //
    // **照实记（这里比宿主那 21 条更强）**：宿主那边是 `need`（**存在一次**就绿），而
    // `amid(me,c0)` 这一形在本域脚本里出现**四趟**（立了之后 / 入之后 / 领了之后 / 弃了之后）
    // ——这里每一趟各判一次，四趟答错任何一处都会点名。
    let mut suite = Suite::new("member");

    // 二、立两枚盟：号由服务发（**单调、稠密**——第一枚是零号）。
    let c0 = coal.found(MS);
    say(&format!("member: found={}", one_id(c0)));
    let c1 = coal.found(MS);
    say(&format!("member: found={}", one_id(c1)));
    let (Ok(c0), Ok(c1)) = (c0, c1) else { return bail("member: no coalition id") };
    suite.case("the_first_coalition_is_zero", move || {
        assert_eq!(c0.get(), 0)
    });
    suite.case("the_second_coalition_is_one", move || {
        assert_eq!(c1.get(), 1)
    });

    // 三、立了不等于进了。
    let apart = coal.amid(p, c0, MS);
    say(&format!("member: amid(me,c0)={}", flag(apart)));
    suite.case("standing_apart_is_not_membership", move || {
        assert_eq!(apart, Ok(false))
    });

    // 四、入：名册真的改了，而且**再入一遍还是 ok**（集合没有"第二次"）。
    let entered = coal.enter(c0, MS);
    say(&format!("member: enter(c0)={}", done(entered)));
    let inside = coal.amid(p, c0, MS);
    say(&format!("member: amid(me,c0)={}", flag(inside)));
    let again = coal.enter(c0, MS);
    say(&format!("member: enter(c0)={}", done(again)));
    suite.case("entering_answers_ok", move || assert!(entered.is_ok()));
    suite.case("entering_really_changed_it", move || {
        assert_eq!(inside, Ok(true))
    });
    suite.case("entering_twice_answers_ok", move || assert!(again.is_ok()));

    // 五、同一条身份可以在第二枚盟里。
    let in_c1 = coal.enter(c1, MS);
    say(&format!("member: enter(c1)={}", done(in_c1)));
    let amid_c1 = coal.amid(p, c1, MS);
    say(&format!("member: amid(me,c1)={}", flag(amid_c1)));
    suite.case("the_second_coalition_takes_the_same_identity", move || {
        assert!(in_c1.is_ok())
    });
    suite.case("that_identity_is_in_the_second_coalition", move || {
        assert_eq!(amid_c1, Ok(true))
    });

    // 六、领到第二条身份，把它也放进 c0 ⇒ 这枚盟里有**两位**。
    let sub = policy.derive(p, MS);
    say(&format!("member: derive(me)={}", one_policy(sub)));
    let Some(q) = sub.ok() else { return bail("member: no sub identity") };
    let adopted = policy.adopt(q, MS);
    say(&format!("member: adopt(sub)={}", done(adopted)));
    let q_in = coal.enter(c0, MS);
    say(&format!("member: enter(c0)={}", done(q_in)));
    let p_there = coal.amid(p, c0, MS);
    say(&format!("member: amid(me,c0)={}", flag(p_there)));
    let q_there = coal.amid(q, c0, MS);
    say(&format!("member: amid(sub,c0)={}", flag(q_there)));
    suite.case("a_derived_identity_can_be_adopted", move || {
        assert!(adopted.is_ok())
    });
    suite.case("adopting_lets_it_enter_the_coalition", move || {
        assert!(q_in.is_ok())
    });
    suite.case("both_identities_are_in_that_coalition", move || {
        assert_eq!(p_there, Ok(true));
        assert_eq!(q_there, Ok(true));
    });

    // 七、**出的是那一对，不是那个人**：此刻代表 `sub`，故出掉的是 `sub` 那一行。
    let left = coal.leave(c0, MS);
    say(&format!("member: leave(c0)={}", done(left)));
    let q_gone = coal.amid(q, c0, MS);
    say(&format!("member: amid(sub,c0)={}", flag(q_gone)));
    let p_still = coal.amid(p, c0, MS);
    say(&format!("member: amid(me,c0)={}", flag(p_still)));
    suite.case("leaving_answers_ok", move || assert!(left.is_ok()));
    suite.case("leaving_took_out_that_pair_not_the_person", move || {
        assert_eq!(q_gone, Ok(false));
        assert_eq!(p_still, Ok(true));
    });

    // 八、弃回起点：键 = 身份那条定理的另一半——第一条身份那一行照旧在。
    let waived = policy.waive(MS);
    say(&format!("member: waive={}", done(waived)));
    let after_waive = coal.amid(p, c0, MS);
    say(&format!("member: amid(me,c0)={}", flag(after_waive)));
    suite.case("waiving_answers_ok", move || assert!(waived.is_ok()));
    suite.case("waiving_keeps_membership", move || {
        assert_eq!(after_waive, Ok(true))
    });

    // 九、第三态：没铸过的盟（号是伪造的线上值）。
    let outside = CoalitionId::new(OUTSIDE);
    let out_amid = coal.amid(p, outside, MS);
    say(&format!("member: amid(me,out)={}", flag(out_amid)));
    let out_enter = coal.enter(outside, MS);
    say(&format!("member: enter(out)={}", done(out_enter)));
    let out_leave = coal.leave(outside, MS);
    say(&format!("member: leave(out)={}", done(out_leave)));
    suite.case("an_unknown_coalition_is_unknown_not_false", move || {
        assert!(matches!(out_amid, Err(Fail::Unknown)))
    });
    suite.case("an_unknown_coalition_cannot_be_entered", move || {
        assert!(matches!(out_enter, Err(Fail::Unknown)))
    });
    suite.case("an_unknown_coalition_cannot_be_left", move || {
        assert!(matches!(out_leave, Err(Fail::Unknown)))
    });

    // 十、伪造的**身份**号：答 false，**不是失败**——`p` 是标签，本册不去问名册。
    let forged = coal.amid(PrincipalId::new(OUTSIDE), c1, MS);
    say(&format!("member: amid(out,me)={}", flag(forged)));
    suite.case("a_forged_identity_is_false_not_a_failure", move || {
        assert_eq!(forged, Ok(false))
    });

    // 十一、**一串**（取窗两条）：`band` 答成员、`bloc` 答盟籍（序都是号序）。
    let band = coal.band(c0, None, MS);
    say(&format!("member: band(c0)={}", window_ids(band)));
    // 拿末一枚当游标接着取：**阈值**语义下再往后没有了 ⇒ 空窗，**不是错**（也不是"过期游标"）。
    let after = band.as_ref().ok().and_then(|w| w.last());
    let empty = coal.band(c0, after, MS);
    say(&format!("member: band(c0,next)={}", window_ids(empty)));
    // 没铸过的那枚盟：取窗这一条**有失败域**（同 amid）。
    let out_band = coal.band(outside, None, MS);
    say(&format!("member: band(out)={}", window_ids(out_band)));
    // 反向那一趟：这条身份在哪些盟里（**没有失败域**：不在任何盟里就是空窗）。
    let bloc = coal.bloc(p, None, MS);
    say(&format!("member: bloc(me)={}", window_ids(bloc)));
    suite.case("the_window_holds_that_one_member", move || {
        assert_eq!(band.as_ref().ok().map(|w| w.len()), Some(1))
    });
    suite.case("the_cursor_is_a_threshold_so_the_next_window_is_empty", move || {
        assert_eq!(empty.as_ref().ok().map(|w| w.len()), Some(0))
    });
    suite.case("an_unknown_coalition_has_no_window", move || {
        assert!(matches!(out_band, Err(Fail::Unknown)))
    });
    suite.case("the_reverse_window_holds_both_coalitions", move || {
        assert_eq!(bloc.as_ref().ok().map(|w| w.len()), Some(2))
    });
    suite.run();

    return Report::note(E_OK, "member: done")
}

/// 按名字找一面服务：`FIND "/<dir>/<name>"`，**找不到就再问**（有界）——门牌是本域起来之后落的。
///
/// 找到之后那一枚**从会话里**进本域表（报文里没有号）：按"谁给的"认，取**最后**那一枚
/// （一次一问一答只授一枚，故最后那一枚就是这一趟的）。
fn find_face(link: &Quay, talk: PieToken, host: TaskId, dir: &str, name: &str) -> Option<PieToken> {
    let (Ok(dir), Ok(name)) = (Name::new(dir), Name::new(name)) else {
        return None;
    };
    let road = [dir, name];
    // **间接寻址那一手**：名字先经 `seek` 译成号（"还没挂上"那一格也在这里重试），此后按号。
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
fn one_id<E: Why>(r: Result<CoalitionId, E>) -> String {
    match r {
        Ok(c) => format!("{}", c.get()),
        Err(fail) => format!("err:{}", fail.why()),
    }
}

/// 同一行读数：一窗的**三格事实**——几枚、窗外还有没有、是哪些号。
///
/// 三格分开写，是因为**只有前两格是判据**：号那一段跟着装配期铸出来的身份号走（同一份镜像、
/// 不同的启动次序就会差一位），拿它钉判据等于把一条与取窗无关的数钉进门里。
fn window_ids<T: Id>(r: Result<Window<T>, Fail>) -> String {
    match r {
        Ok(w) => format!("n{} more={} ids={}", w.len(), w.more(), ids(&w)),
        Err(fail) => format!("err:{}", fail.why()),
    }
}

/// 一窗号拼成 `11,22`（空窗拼成 `-`）。
fn ids<T: Id>(w: &Window<T>) -> String {
    let mut out = String::new();
    for id in w.iter() {
        if !out.is_empty() {
            out.push(',');
        }
        out.push_str(&format!("{}", id.get()));
    }
    if out.is_empty() {
        out.push('-');
    }
    out
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
///
/// 本探针是这一族里第一个**同时问两家服务**的客人（结盟 + 身份），故这条小小的桥只此一处
/// ——每家的格子名照它们自己那张线上表说，不另起词。
trait Why {
    fn why(&self) -> &'static str;
}

impl Why for Fail {
    fn why(&self) -> &'static str {
        match self {
            Fail::Unknown => "unknown",
            Fail::NoRoom => "no-room",
        }
    }
}

impl Why for PolicyFail {
    fn why(&self) -> &'static str {
        match self {
            PolicyFail::Denied => "denied",
            PolicyFail::Unknown => "unknown",
            PolicyFail::NoRoom => "no-room",
        }
    }
}

/// 报一行就走（本域没有控制台，调试面是唯一能说话的地方）。
fn bail<'a>(msg: &'a str) -> Report<'a> {
    return Report::note(E_NO_SERVICE, msg)
}

/// 打一行。
fn say(msg: &str) {
    let _ = debug::put(msg);
}

