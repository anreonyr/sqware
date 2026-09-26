#![no_std]
#![no_main]

//! subject — **主体**：问身份服务"我是谁"，把这一刀要验的读数打出来。
//!
//! ```text
//!   1  树那条路：seat(树) + claim(生我者, 树) + 另铸一枚问话孔给持树者
//!   2  FIND "/sys/principal" ⇒ 门牌那一枚**经会话**授进本域表里（报文里没有号）
//!   3  resolve(self)      ⇒ 本域此刻代表哪个号（**装配期**绑的那一条）
//!   4  sire(root) / sire(me)  ⇒ 三态的头两格：**根答"没有"，不是"Unknown"**
//!   5  heir(me, me)       ⇒ 自反
//!   6  derive(me)         ⇒ 向下派生一条自己的子身份（钥匙 = 当前正好代表 p）
//!   7  heir(sub, me)      ⇒ 否定：子代不是祖先
//!   8  heir(树外号, me)    ⇒ **第三态**：Unknown（与"不是祖先"分得开）
//!   9  bind(self)         ⇒ Denied：名册只有装配者能写
//!  10  adopt(sub)         ⇒ **领**：换到自己派生出来的那一支里
//!  11  resolve(self)      ⇒ 名册真的改了（不是打个印记）
//!  12  derive(旧起点)      ⇒ **钥匙反证**：已不代表起点 ⇒ Denied
//!  13  adopt(向上) / adopt(树外) ⇒ Denied / Unknown
//!  14  waive()            ⇒ **弃**：回到装配给我的那一条（不删格）
//! ```
//!
//! # 为什么不上板
//!
//! 本域只做一件事——问身份；生死那本账与本域无关（同 `lodger` 那一档）。它也不是装配单的
//! 最后一条：**收场由 `echo` 那一条给**（编排域等的是它退场）。
//!
//! # 树外那个号是**故意伪造的**
//!
//! 树只增不删 ⇒ 号不会失效，"树外"只能由伪造或损坏的帧产生——这正是三态第三格存在的理由
//! （内核那两条身份凭证都答不出"这条号住不住在树上"；只有 Server 那张表答得出）。

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use alloc::format;
use alloc::string::String;
use core::time::Duration;

use env::{Name, PieToken};
use protocol::session::Quay;
use protocol::system::operator as ocall;
use protocol::system::operator::client as operator;
use protocol::system::principal as pcall;
use protocol::system::principal::client::Face;
use protocol::system::principal::core::{Fail, PrincipalId};
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

/// 树外那个号（伪造的线上值）。
const OUTSIDE: usize = 4095;

#[programs::entry]
fn main() -> Report<'static> {
    let Ok(sire) = utask::sire() else {
        return bail("subject: no sire");
    };
    let Ok(me) = utask::self_id() else {
        return bail("subject: no self id");
    };

    // 上树：本域只开一条会话——按名字找那面身份服务。
    let Ok((tree, host)) = operator::open(sire, Wait::AtMost(MS)) else {
        return bail("subject: no tree link");
    };
    let Ok(talk) = operator::ask_hole(host) else {
        return bail("subject: no tree ask");
    };
    let Some(entry) = find_face(&tree, talk) else {
        return bail("subject: no face");
    };
    let Ok(face) = Face::of(entry) else {
        return bail("subject: bad face");
    };

    // 一、此刻代表谁——装配期绑的那一条（服务一起来就答得出）。
    let mine = face.resolve(me, Wait::AtMost(MS));
    say(&format!("policy: me={}", one_opt(mine)));
    let Ok(Some(p)) = mine else {
        return bail("subject: unbound");
    };

    // 判据就地登记（用户裁定"服务台搬进 SUT"）：**只搬本域已经在判的东西**——下面每一例的期望，
    // 都是本域头注那 14 步里写着的那一句（旧宿主靶上 `policy: …` 那 12 条钉的就是它们）。
    // 台名 = 本域打的那个前缀，门按它钉逐台基线。

    // 二、三态的头两格。
    let no_sire = face.sire(PrincipalId::ROOT, Wait::AtMost(MS));
    say(&format!("policy: sire(root)={}", one_opt(no_sire)));
    let sired = face.sire(p, Wait::AtMost(MS));
    say(&format!("policy: sire(me)={}", one_opt(sired)));
    {
        assert_eq!(no_sire, Ok(None))
    }
    {
        assert_eq!(sired, Ok(Some(PrincipalId::ROOT)))
    }

    // 三、自反。
    let reflexive = face.heir(p, p, Wait::AtMost(MS));
    say(&format!("policy: heir(me,me)={}", flag(reflexive)));
    {
        assert_eq!(reflexive, Ok(true))
    }

    // 四、向下派生一条自己的子身份。
    let sub = face.derive(p, Wait::AtMost(MS));
    say(&format!("policy: derive(me)={}", one(sub)));
    let child = sub.ok();
    assert!(sub.is_ok());

    // 五、否定：子代不是祖先（拿刚派生出来的那一条问）。
    let not_ancestor = child.map(|q| face.heir(q, p, Wait::AtMost(MS)));
    if let Some(r) = not_ancestor {
        say(&format!("policy: heir(sub,me)={}", flag(r)));
    }
    {
        assert_eq!(not_ancestor, Some(Ok(false)))
    }

    // 六、第三态：树外的号。
    let out_heir = face.heir(PrincipalId::new(OUTSIDE), p, Wait::AtMost(MS));
    say(&format!("policy: heir(out,me)={}", flag(out_heir)));
    {
        assert!(matches!(out_heir, Err(Fail::Unknown)))
    }

    // 七、越权一趟：名册只有装配者能写，本域不是它。
    let bound = face.bind(me, p, Wait::AtMost(MS));
    say(&format!("policy: bind(self)={}", done(bound)));
    {
        assert!(matches!(bound, Err(Fail::Denied)))
    }

    // ── 转换那两条（刀 2）────────────────────────────────────
    let Some(q) = child else {
        return bail("subject: no sub identity");
    };
    // 八、领：换到自己刚派生出来的那一支里（`sub` 一定在 `p` 那一支里）。
    let adopted = face.adopt(q, Wait::AtMost(MS));
    say(&format!("policy: adopt(sub)={}", done(adopted)));
    {
        assert!(adopted.is_ok())
    }

    // 九、名册真的改了（不是打个印记）。
    let led = face.resolve(me, Wait::AtMost(MS));
    say(&format!("policy: me={}", one_opt(led)));

    // 十、**钥匙反证**：已不代表 `p`，故"从 `p` 派生"被拒。
    let stale = face.derive(p, Wait::AtMost(MS));
    say(&format!("policy: derive(old)={}", one(stale)));
    {
        assert!(matches!(stale, Err(Fail::Denied)))
    }

    // 十一、向上 / 跨支：`p` 是 `sub` 的父，不在 `sub` 那一支里。
    let up = face.adopt(p, Wait::AtMost(MS));
    say(&format!("policy: adopt(up)={}", done(up)));
    {
        assert!(matches!(up, Err(Fail::Denied)))
    }

    // 十二、树外。
    let outside = face.adopt(PrincipalId::new(OUTSIDE), Wait::AtMost(MS));
    say(&format!("policy: adopt(out)={}", done(outside)));
    {
        assert!(matches!(outside, Err(Fail::Unknown)))
    }

    // 十三、弃：回到装配给我的那一条（不删格）。
    let waived = face.waive(Wait::AtMost(MS));
    say(&format!("policy: waive={}", done(waived)));
    assert!(waived.is_ok());

    // 十四、回到起点。
    let back = face.resolve(me, Wait::AtMost(MS));
    say(&format!("policy: me={}", one_opt(back)));

    // 三条 `policy: me=`（装配绑的 / 领之后 / 弃之后）的关系：**绑 ≠ 领 = 弃**。
    //
    // **照实记（这一条原先住在宿主靶上）**：它是 `soak::verdict` 里那段 `values(...)` 比较 ——
    // 宿主数了三行、比了两个关系；而那三行是**本域自己打的**，本域当然也知道它们该是什么关系。
    // 搬进来之后宿主那一侧不必再数那三行（见四处那一段的改动）。
    {
        {
            assert_ne!(led, mine);
            assert_eq!(back, mine);
        }
    }

    return Report::note(E_OK, "subject: done");
}

/// 找那面服务：`FIND "/sys/principal"`，**找不到就再问**（有界）——门牌是本域起来之后落的。
///
/// 找到之后那一枚**从会话里**进本域表（报文里没有号）：认的是"持树者刚授进来的那一份"。
fn find_face(link: &Quay, talk: PieToken) -> Option<PieToken> {
    let (Ok(dir), Ok(me)) = (Name::new(pcall::DIR), Name::new(pcall::NAME)) else {
        return None;
    };
    let road = [dir, me];
    // **间接寻址那一手**：名字先经 `seek` 译成号（"还没挂上"那一格也在这里重试），此后按号。
    let mut left = MS;
    let id = loop {
        match operator::seek(talk, link, &road, Wait::AtMost(MS)) {
            Ok(id) => break id,
            Err(ocall::UNKNOWN) if left > 0 => {
                let _ = room::sleep(Duration::from_millis(RETRY_MS as u64));
                left = left.saturating_sub(RETRY_MS);
            }
            Err(_) => return None,
        }
    };
    match operator::find(talk, link, id, Wait::AtMost(MS)) {
        Ok((ocall::OK, Some(entry))) => Some(entry),
        _ => None,
    }
}

/// 一条号 / 没绑 / 哪一格失败——**一行里说全**（读数靠这一行，不靠再跑一遍）。
fn one_opt(r: Result<Option<PrincipalId>, Fail>) -> String {
    match r {
        Ok(Some(p)) => format!("{}", p.get()),
        Ok(None) => String::from("none"),
        Err(fail) => format!("err:{}", why(fail)),
    }
}

/// 同一行读数：只答一条号的那几条（`derive`）。
fn one(r: Result<PrincipalId, Fail>) -> String {
    match r {
        Ok(p) => format!("{}", p.get()),
        Err(fail) => format!("err:{}", why(fail)),
    }
}

/// 同一行读数：是 / 不是 / 哪一格失败。
fn flag(r: Result<bool, Fail>) -> String {
    match r {
        Ok(true) => String::from("true"),
        Ok(false) => String::from("false"),
        Err(fail) => format!("err:{}", why(fail)),
    }
}

/// 同一行读数：成了没有。
fn done(r: Result<(), Fail>) -> String {
    match r {
        Ok(()) => String::from("ok"),
        Err(fail) => format!("err:{}", why(fail)),
    }
}

/// 失败域那三格的名字（**照线上那张表说**，不另起词）。
fn why(fail: Fail) -> &'static str {
    match fail {
        Fail::Denied => "denied",
        Fail::Unknown => "unknown",
        Fail::Full => "full",
    }
}

/// 报一行就走（本域没有控制台，调试面是唯一能说话的地方）。
fn bail<'a>(msg: &'a str) -> Report<'a> {
    return Report::note(E_NO_SERVICE, msg);
}

/// 打一行。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
