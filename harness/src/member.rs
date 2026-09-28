#![no_std]
#![no_main]

//! member — **盟友**：立盟、进出、问在不在，把这一族要验的读数打出来。
//!
//! ```text
//!   1  树那条路：seat(树) + claim(生我者, 树) + 另铸一枚问话孔给持树者
//!   2  FIND "/svc/coalition/{ask,set}" ⇒ 结盟服务**两面**的门牌；FIND "/svc/principal/{ask,set}"
//!      ⇒ 身份服务**两面**的门牌（本台**四面都要**：两侧都要读、都要写）
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
//! 路（树上的条目是"名字 → 一枚 Pie"，**存不了号**；装配表的 args 没接）。故真机读数用**派生
//! 出来的第二条身份**：它一样是名册里真有的一格，盟册看得见"同一枚盟里有两位"。
//!
//! # 为什么不上板
//!
//! 本域只做一件事——结盟；生死那本账与本域无关（同 `subject` / `lodger` 那一档）。它也不是
//! 装配表的最后一条：**收场由 `canonical` 那一条给**。

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use alloc::string::String;

use alloc::format;
use env::{Name, PieToken};
use protocol::communication::session::Session;
use protocol::debug;
use protocol::id::Id;
use protocol::system::coalition as ccall;
use protocol::system::coalition::client::{Band, Bloc, Coalition, Face as CoalitionFace};
use protocol::system::coalition::{CoalitionId, Fail};
use protocol::system::operator::client as operator;
use protocol::system::operator::client::Face as TreeFace;
use protocol::system::operator::Fail as TreeFail;
use protocol::system::principal as pcall;
use protocol::system::principal::client::Face as PolicyFace;
use protocol::system::principal::Fail as PolicyFail;
use protocol::system::principal::PrincipalId;
use runtime::env::unit as utask;

/// 等树 / 等答 / 找门牌的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 退场码：走通了 / 没走通（都不是 panic；kernel 会把那一行连同域号打出来）。
const E_OK: usize = 0;
const E_NO_SERVICE: usize = 1;

/// 册外那个号（伪造的线上值）。铸过的号是 `0..next`，故这个一定在册外。
const OUTSIDE: usize = 4095;

#[programs::entry]
fn main() -> Report<'static> {
    let sire = utask::sire();
    let me = utask::self_id();

    // 上树：本域只开一条链，走两趟按名字找（结盟服务那一面 + 身份服务那一面）。
    //
    // **照实记（这一处为什么包成 `Face`，task-2 那一刀）**：那条链上本域只要"名字 → 入口"
    // 两趟，裸孔一个都不用 ⇒ 按"已持 `Session` 则用 `Face`"交给 [`TreeFace::of`]（吃所有权）。
    // 别名 `TreeFace` 是**避让**下面两面各自的 `Face`（CoalitionFace / PolicyFace）。
    let Ok(session) = Session::open(sire, operator::BERTH, Wait::AtMost(MS)) else {
        return bail("member: no tree link");
    };
    let tree = TreeFace::of(session);
    // 盟册那**两面**（开面那一刀）：三条"问"的（`Amid` / `Band` / `Bloc`）在 `Grant::Ask` 上，
    // 三条"定"的（`Found` / `Enter` / `Leave`）在 `Grant::Set` 上。这一台**两面都要**——它立盟、
    // 入、出，也问盟籍、点名册。
    //
    // **同一枚盟要两枚柄**（照实记：这是两面分开的代价）：`Coalition` 那个柄**绑在它来自的那一
    // 面上**（`Face::coalition(id)` 只是把一个宾语固定下来），故 `cset.found()` 拿到的柄
    // `enter` / `leave` 得动，而 `holds` / `members` 要走 `cask.coalition(id)` 那一枚。
    let (Ok(cdir), Ok(cseg)) = (Name::new(ccall::DIR), Name::new(ccall::NAME)) else {
        return bail("member: bad coalition name");
    };
    let (Ok(cask), Ok(cset)) = (
        Name::new(ccall::Grant::Ask.name()),
        Name::new(ccall::Grant::Set.name()),
    ) else {
        return bail("member: bad coalition face name");
    };
    let Some(entry) = find_face(&tree, &[cdir, cseg, cask]) else {
        return bail("member: no coalition ask face");
    };
    let Ok(cask) = CoalitionFace::of(entry) else {
        return bail("member: bad coalition ask face");
    };
    let Some(entry) = find_face(&tree, &[cdir, cseg, cset]) else {
        return bail("member: no coalition set face");
    };
    let Ok(cset) = CoalitionFace::of(entry) else {
        return bail("member: bad coalition set face");
    };

    // 身份那**两面**：**本域自己也要用它们**（问"我代表谁"，派生第二条身份、领、弃）。
    //
    // **两面各找一次**（开面那一刀）：三条"问"的（`Resolve` / `Sire` / `Heir`）在
    // [`Grant::Ask`] 上，四条"定"的（`Bind` / `Derive` / `Adopt` / `Waive`）在 [`Grant::Set`]
    // 上；下面每一处按**它问的是哪一类**挑门牌。
    let (Ok(dir), Ok(segment)) = (Name::new(pcall::DIR), Name::new(pcall::NAME)) else {
        return bail("member: bad identity name");
    };
    let (Ok(ask), Ok(set)) = (
        Name::new(pcall::Grant::Ask.name()),
        Name::new(pcall::Grant::Set.name()),
    ) else {
        return bail("member: bad identity face name");
    };
    let Some(entry) = find_face(&tree, &[dir, segment, ask]) else {
        return bail("member: no identity ask face");
    };
    let Ok(ask) = PolicyFace::of(entry) else {
        return bail("member: bad identity ask face");
    };
    let Some(entry) = find_face(&tree, &[dir, segment, set]) else {
        return bail("member: no identity set face");
    };
    let Ok(set) = PolicyFace::of(entry) else {
        return bail("member: bad identity set face");
    };

    // 一、此刻代表谁——装配期绑的那一条。
    //
    // **照实记（task-2 那一刀）**：`resolve` 折进 `Task::principal`（返 `Principal` 柄）；
    // 本台只读数，故在调用点把柄投影回它那一枚号（下面每一处都这样收）。
    let mine = ask
        .task(me)
        .principal(Wait::AtMost(MS))
        .map(|found| found.map(|p| p.id()));
    debug!("member: me={}", one_opt(mine));
    let Ok(Some(p)) = mine else {
        return bail("member: unbound");
    };

    // 判据就地登记（用户裁定"服务台搬进 SUT"）：**只搬本域已经在判的东西**——下面每一例的期望，
    // 都是本域头注那 16 步里写着的那一句（旧宿主靶上 `member: …` 那 21 条钉的就是它们）。
    // 台名 = 本域打的前缀，门按它钉逐台基线。
    //
    // **照实记（这里比宿主那 21 条更强）**：宿主那边是 `need`（**存在一次**就绿），而
    // `amid(me,c0)` 这一形在本域脚本里出现**四趟**（立了之后 / 入之后 / 领了之后 / 弃了之后）
    // ——这里每一趟各判一次，四趟答错任何一处都会点名。

    // 二、立两枚盟：号由服务发——**全局单一序列，只增**。
    //
    // **照实记（这两格从前钉的是绝对值，`soak` 门因此有一张红脸）**：原来两条断言是
    // `c0 == 0` 与 `c1 == 1`——钉的是"**我这两枚是全机器头两枚**"。可盟号是**全局**序列，
    // 而 `harness/src/probe_rule.rs` 那台（位次 15）**也**调 `found()`（它要一枚号来挂规矩）
    // ⇒ 谁先到谁拿 0。实测同一份 ELF：47 份现场里 **44 份 `found=0`、3 份 `found=1`**
    // ——时序说了算，不是机器性质。
    //
    // 更要紧的是：**"第一枚是零号"没有并发客人能证**（要证它得保证自己是第一枚），"号不跳"
    // （稠密）同理——两次 `found` 之间**谁都可以插一脚**。故那一对换成唯一可证的那条：
    // **号只增**。至于号**能用**（进得去、查得着、放得下），由后面那一整串
    // （`enter` / `leave` / `waive` / `band` / `bloc`）证，不靠这两格。
    let c0 = cset.found(Wait::AtMost(MS));
    debug!("member: found={}", one_id(&c0));
    let c1 = cset.found(Wait::AtMost(MS));
    debug!("member: found={}", one_id(&c1));
    let (Ok(c0), Ok(c1)) = (c0, c1) else {
        return bail("member: no coalition id");
    };
    // 读那一侧的两枚柄：**同一枚盟，换一枚门牌**（见上面那条照实记）。
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
    // **这一手不收"谁"**：进的是本端**此刻代表**的那一位（名册的答案）。
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

    // 六、领到第二条身份，**并且当场换成它**（`adopt`），于是这一步进的是 `sub`。
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
    // 六·五、**代报名那一格**（K2 翻案那一刀）的**负证**：此刻我代表 `sub`，而 `c0` 的盟主是
    // `p` ⇒ 我**不是**它的盟主 ⇒ 这一问该被拒（[`Fail::NotChief`]，不是 `Unknown`：盟在、
    // 我也在册上，缺的只是"这一枚盟归不归你代报名"）。
    //
    // **正证在设备账那一台手里**（生产里唯一的持有者）：`hub` 每类立一枚盟、再替四位驱动
    // `admit`（`protocol::driver::hub` 的 `bond`）。本台不抢那一份读数。
    let not_chief = c0.admit(me, Wait::AtMost(MS));
    debug!("member: admit(c0)={}", done(not_chief.clone()));
    {
        {
            assert!(matches!(not_chief, Err(Fail::NotChief)))
        }
    }
    // **我此刻代表 `sub`** ⇒ 这一手进的是 `sub`（不小看这一步：`q` 只出现在 `holds` 那一侧，
    // 它作为参数的日子随"客侧没有'我是谁'这一格"那条口径一起退场）。
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
    // （这一手同样不收"谁"——主体由印章说）。
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
    // **两枚柄都要**（读走问面、写走定面）：这一格量的正是"没铸过的那枚盟 ⇒ `Unknown`"，
    // 故两枚柄**都得是"面过了、核心拒"**那一条——若拿问面去 `enter`，量到的是 `Denied`
    // （面那一格），把这一格要证的"核心里那枚盟不存在"顶掉了。
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
    // 反向那一趟：这条身份在哪些盟里（**没有失败域**：不在任何盟里就是空窗）。
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

    // 十二、**面那一格**（开面那一刀）：同一条问、同一个发送者，**只换门牌**——定面成、问面拒。
    // 这一对量得出来的正是"面"这件事本身；而"面不对"在**门外**就拦下了（连盟册都没看）。
    //
    // **照实记（它与核心那几格同码，分开它们的是读数）**：这一格答的 `Denied` 与别的因同码
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

/// 按名字找一面服务：`FIND` 那一条路（2 段：盟册那一面；3 段：名册那两面各一条），
/// **找不到就再问**（有界）——门牌是本域起来之后落的。
///
/// 找到之后那一枚**从会话里**进本域表（报文里没有号）：按"谁给的"认，取**最后**那一枚
/// （一次一问一答只授一枚，故最后那一枚就是这一趟的）。
///
/// **照实记（收 `&TreeFace`，不再收 `&Session`）**：调用方**已持**一面（task-2 那一刀包出来的），
/// 故这一手只借它——签名上不再出现那条链。
fn find_face(tree: &TreeFace, road: &[Name]) -> Option<PieToken> {
    // 名字 → 号（**译不出就重试**：门牌是别的域落的，它可能落得比本域晚）→ 入口：两格在
    // [`Pane::tile`] 与 [`Tile::token`] 上（旧 `entry_of` 那一趟；本域从前自己抄了一遍）。
    //
    // **照实记（task-2 那一刀；为什么不用 `Face::tile`）**：`entry` 自己已经译号一次 + `find`
    // 一次，随后 `Tile::token` 又 `find` 一次 ⇒ 每趟多授一枚没人接的副本进本域表。旧面只有
    // 一枚，故这里也照一枚写（重试那一圈照旧留着）。
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
///
/// **照实记（task-2 那一刀）**：`found` 现在答一面 [`Coalition`] 柄（号绑进柄），故这里按
/// `&Result<Coalition, Fail>` 读它那一枚 `id()`。
fn one_id(r: &Result<Coalition<'_>, Fail>) -> String {
    match r {
        Ok(c) => format!("{}", c.id().get()),
        Err(fail) => format!("err:{}", fail.why()),
    }
}

/// 同一行读数：一窗的**三格事实**——几枚、窗外还有没有、是哪些号。
///
/// 三格分开写，是因为**只有前两格是判据**：号那一段跟着装配期铸出来的身份号走（同一份镜像、
/// 不同的启动次序就会差一位），拿它钉判据等于把一条与取窗无关的数钉进门里。
///
/// **照实记（task-2 那一刀）**：旧面那两问各答一枚 `Window<T>`（有 `len` / `last`），新面答
/// [`Band`] / [`Bloc`]（只有 `more` / `iter` / `next`）——"几枚"改由 `iter().count()` 说，
/// 游标取末一枚改由 `iter().last()` 说。
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
            Fail::Full => "full",
            // 开面那一刀添的那一格（"你手里那一枚门牌给不了这一条"）。
            Fail::Denied => "denied",
            // K2 翻案那一刀添的那一格（"你不是这一枚盟的盟主"）——本探针叫不动它
            // （它不代报名），故这一格在这儿只为**match 穷尽**，不是一条读数。
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

/// 报一行就走（本域没有控制台，调试面是唯一能说话的地方）。
fn bail<'a>(msg: &'a str) -> Report<'a> {
    return Report::note(E_NO_SERVICE, msg);
}

