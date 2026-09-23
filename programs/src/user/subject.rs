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

use alloc::format;
use alloc::string::String;
use core::time::Duration;

use env::{Name, PieToken, TaskId};
use protocol::operator::call as ocall;
use protocol::operator::client as operator;
use protocol::principal::call as pcall;
use protocol::principal::client::Face;
use protocol::principal::core::{Fail, PrincipalId};
use protocol::session::Quay;
use runtime::env::debug;
use runtime::env::room::{self, exit_with_note};
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

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let Ok(sire) = utask::sire() else {
        bail("subject: no sire")
    };
    let Ok(me) = utask::self_id() else {
        bail("subject: no self id")
    };

    // 上树：本域只开一条会话——按名字找那面身份服务。
    let Ok((tree, host)) = operator::open(sire, MS) else {
        bail("subject: no tree link")
    };
    let Ok(talk) = operator::ask_hole(host) else {
        bail("subject: no tree ask")
    };
    let Some(entry) = find_face(&tree, talk, host) else {
        bail("subject: no face")
    };
    let Ok(face) = Face::of(entry) else {
        bail("subject: bad face")
    };

    // 一、此刻代表谁——装配期绑的那一条（服务一起来就答得出）。
    let mine = face.resolve(me, MS);
    say(&format!("policy: me={}", one_opt(mine)));
    let Ok(Some(p)) = mine else {
        bail("subject: unbound")
    };

    // 二、三态的头两格。
    say(&format!(
        "policy: sire(root)={}",
        one_opt(face.sire(PrincipalId::ROOT, MS))
    ));
    say(&format!("policy: sire(me)={}", one_opt(face.sire(p, MS))));

    // 三、自反。
    say(&format!(
        "policy: heir(me,me)={}",
        flag(face.heir(p, p, MS))
    ));

    // 四、向下派生一条自己的子身份。
    let sub = face.derive(p, MS);
    say(&format!("policy: derive(me)={}", one(sub)));
    // 五、否定：子代不是祖先（拿刚派生出来的那一条问）。
    if let Ok(q) = sub {
        say(&format!(
            "policy: heir(sub,me)={}",
            flag(face.heir(q, p, MS))
        ));
    }

    // 六、第三态：树外的号。
    say(&format!(
        "policy: heir(out,me)={}",
        flag(face.heir(PrincipalId::new(OUTSIDE), p, MS))
    ));

    // 七、越权一趟：名册只有装配者能写，本域不是它。
    say(&format!(
        "policy: bind(self)={}",
        done(face.bind(me, p, MS))
    ));

    // ── 转换那两条（刀 2）────────────────────────────────────
    //
    // 八、领：换到自己刚派生出来的那一支里（`sub` 一定在 `p` 那一支里）。
    let Some(q) = sub.ok() else {
        bail("subject: no sub identity")
    };
    say(&format!("policy: adopt(sub)={}", done(face.adopt(q, MS))));

    // 九、名册真的改了（不是打个印记）。
    say(&format!("policy: me={}", one_opt(face.resolve(me, MS))));

    // 十、**钥匙反证**：已不代表 `p`，故"从 `p` 派生"被拒。
    say(&format!("policy: derive(old)={}", one(face.derive(p, MS))));

    // 十一、向上 / 跨支：`p` 是 `sub` 的父，不在 `sub` 那一支里。
    say(&format!("policy: adopt(up)={}", done(face.adopt(p, MS))));

    // 十二、树外。
    say(&format!(
        "policy: adopt(out)={}",
        done(face.adopt(PrincipalId::new(OUTSIDE), MS))
    ));

    // 十三、弃：回到装配给我的那一条（不删格）。
    say(&format!("policy: waive={}", done(face.waive(MS))));

    // 十四、回到起点。
    say(&format!("policy: me={}", one_opt(face.resolve(me, MS))));

    exit_with_note(E_OK, "subject: done")
}

/// 找那面服务：`FIND "/sys/principal"`，**找不到就再问**（有界）——门牌是本域起来之后落的。
///
/// 找到之后那一枚**从会话里**进本域表（报文里没有号）：认的是"持树者刚授进来的那一份"。
fn find_face(link: &Quay, talk: PieToken, host: TaskId) -> Option<PieToken> {
    let (Ok(dir), Ok(me)) = (Name::new(pcall::DIR), Name::new(pcall::NAME)) else {
        return None;
    };
    let road = [dir, me];
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
        Fail::NoRoom => "no-room",
    }
}

/// 报一行就走（本域没有控制台，调试面是唯一能说话的地方）。
fn bail(msg: &str) -> ! {
    exit_with_note(E_NO_SERVICE, msg)
}

/// 打一行。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
