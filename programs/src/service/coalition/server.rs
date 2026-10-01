//! coalition::server — **结盟服务那一台**：一枚线程守着盟册（一条关系 + 一枚计数器）。
//! 载体是 rtc / principal 那一面已经量过的形状——**门牌自带回信孔**：客人替这一趟铸一枚回信
//! 孔借过来、把帧推上门牌，本域从**门牌那一枚**读（发送者由内核在 `Push` 那一刻盖章），办完事
//! 从那枚孔答回去、当场放下。故这里**没有客人账**：一位客人不需要本域记住任何东西。
//! ```text
//!   起手：读自己的 Sire（**只为上板与上树两条会话**——盟无主，之后不落任何字段）
//!         → 上板（板看得见本域的死）→ 铸门牌**两枚**（两面各一枚）
//!           两枚都经 LAND 落到树上 `/svc/sys/coalition/{ask,set}`，再逐面 FIND 回来验一遍
//!           **只把问面那枚交给持树者**（它只叫 `Amid`；定面留在树上给客人）
//!         → FIND `/svc/sys/principal/ask`（**带重试**）拿一份身份服务的**问面**门牌
//!   常驻：一只组等那两枚 —— **从哪一枚读到**就是哪一面 → 先过名册问"你是谁" → 交给核心 → 答回去
//! ```
//! **两条锚为什么都在这儿**：`Sire` 是内核盖的（比任何自报都硬，且是弱引用：装配者一退它
//! 就答 0），树那条路是"按名找人"的现成一步；而身份那一份门牌**只能按名字找**——本域不是
//! 装配者，拿不到它手里那一份副本（正文 K7 的被否项：转授要新装配机制）。
//! **面为什么长在门牌上**：本族**没有会话**——门牌自己就是那条路，

use crate::system::control::service::Start;
use crate::unit::coalition::E_COALITION;
use env::Wait;

use crate::service::coalition::core::Coalition;
use crate::service::operator::bridge;
use crate::system::carrier::carrier;
use crate::system::mount;
use env::{PieToken, TaskId};
use protocol::communication::sender::Sender;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::service::coalition as ccall;
use protocol::service::coalition::Fail;
use protocol::service::operator::Permit;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::{Face as TreeFace, Mine};
use protocol::service::principal as pcall;
use protocol::service::principal::PrincipalId;
use protocol::service::principal::client::Face;
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail::{self, HolePie};

/// 等板 / 等树 / 问名册的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 起服务：**读锚 → 上板 → 铸门牌上树 → 找身份那一份门牌 → 一枚线程招待所有客人**。
/// **起手那几步收在一个闭包**（与持树者 / 名册那两台同形）：它们清一色是"不成 ⇒ 这域
pub fn serve() -> Result<(), Start> {
    // 一～六：起手（读锚 → 上板 → 铸两枚门牌 → 上树 → 找身份那一份 → 空册）。
    let (mut book, roster, ask, set) = (|| {
        // 一、锚：`Sire` = 装配者。**只为上板与上树两条会话**——盟无主，核心不需要它
        //     （对照 principal：那边把它当名册钥匙，注入核心那一格）。
        // **起我那一枚线程**：本域是装配者建的，故 `Sire` 答的就是它——只有这一条来源。
        let assembler = runtime::env::unit::sire();

        // 三、门牌**两枚**：**一面一枚**（[`mount::entry`] 按"记号 ＋ 面名"给那一枚孔与末段名）。
        // **两面各一枚、不是一枚两面**：本族没有会话，服务端只能从**它自己表里哪一枚孔**收到来
        let (ask, ask_name) = mount::entry(ccall::Grant::Ask.mark(), ccall::Grant::Ask.name())
            .map_err(|_| Start::Tree(E_COALITION))?;
        let (set, set_name) = mount::entry(ccall::Grant::Set.mark(), ccall::Grant::Set.name())
            .map_err(|_| Start::Tree(E_COALITION))?;

        // 四、上树：分 `/svc` ＋ 分 `/svc/sys/coalition`、逐面落那两格、再逐面查回来验一遍
        //     （同 router / rtc / principal）。
        let session = Session::open(assembler, operator::BERTH, Wait::AtMost(MS))
            .map_err(|_| Start::Tree(E_COALITION))?;
        let tree = TreeFace::of(session);
        let _ = bridge::land(
            &tree,
            "coalition",
            &ccall::DIR,
            Mine::No,
            Permit::Unset,
            &[(ask_name.as_str(), ask), (set_name.as_str(), set)],
            Wait::AtMost(MS),
        );

        // **报"答得动了"**（`Setup::Ready`）：上面那一趟落完面、查回来验过才算——被 `after` 指着的台
        // 必须说得出这一句（与三台驱动、设备账那两处**同一手**，见 `programs/src/unit/catalog.rs` 那一格）。
        let _ = protocol::communication::establish::endpoint(
            runtime::env::unit::sire(),
            env::Mark::of(crate::unit::READY),
            env::Wait::POLL,
        );

        // 四之后：**门禁那一枚**——把这一枚门牌**直接交给持树者**（`host` = 持树者的号，
        // `Session::open` 收下的那一格）。它据此才判得了"这一位在那枚盟里吗"（`Permit::Among`）。
        // 与 principal 那一格同一形状。这一枚在手时权限是
        // `FETCH|STORE|VEST`，故子集 `FETCH|STORE` 不越界。装配者那一侧按装配表上那一格
        port::ship(
            &HolePie::from_token(ask),
            tree.host(),
            Access::FETCH | Access::STORE,
            Policy::NONE,
        )
        .map_err(|_| Start::Tree(E_COALITION))?;

        // 五、**身份那一份门牌**：本域是它的客人（K7）。带重试——它可能落得比本域晚。
        let roster_entry = match find_face(&tree) {
            Some(entry) => entry,
            None => {
                debug!("coalition: roster plate not found");
                return Err(Start::Face(E_COALITION));
            }
        };
        let roster = match Face::of(roster_entry) {
            Ok(roster) => roster,
            Err(fail) => {
                // 问不出开者 = `Reserve` 答不上来：那一枚**不在本表里 / 不是孔 / 已封印**
                match mail::reserve(roster_entry) {
                    Ok((vestor, owner, mark)) => debug!(
                        "coalition: roster plate unusable entry={} vestor={} owner={} mark={:#x} fail={fail:?}",
                        roster_entry.get(),
                        vestor.get(),
                        owner.get(),
                        mark.get()
                    ),
                    Err(_) => debug!(
                        "coalition: roster plate unusable entry={} reserve=no fail={fail:?}",
                        roster_entry.get()
                    ),
                }
                return Err(Start::Face(E_COALITION));
            }
        };

        // 六、一本空册：一枚号都还没铸（**起手不失败**——空册不分配）。
        let book = Coalition::new();
        Ok::<_, Start>((book, roster, ask, set))
    })()?;

    // 七、常驻：**一只组等那两枚门牌**（[`carrier`] 那一趟：立组 → 挂两枚 → 备一页 → 等 →
    // **从哪一枚读到就是哪一面** → 把这一批取干净 → 交给 [`turn`]）。这一族没有会话可读记号，
    // 故"面"只有这一条来路。
    carrier(
        E_COALITION,
        &[(ask, ccall::Grant::Ask), (set, ccall::Grant::Set)],
        |face, from, frame| turn(&mut book, &roster, face, from, frame),
    )
}

/// 门上一句话：解帧 → 先过名册 → 交给核心 → **从这一趟自带的那枚孔答回去**。
/// 认那枚孔靠**帧里那一格** ＋ **一次 [`mail::reserve`] 验**（同 `principal` 那一面）；
/// `from` 是**内核盖的发送者**。
/// `roster` = 名册**问面**（`who()` 那一句用）；`mine` = **这一帧从本族哪一枚门牌进来**
/// （[`serve`] 那只组说的事实）。
fn turn(book: &mut Coalition, roster: &Face, mine: ccall::Grant, from: TaskId, frame: &[u8]) {
    let Some((ask, back)) = ccall::Wire::take(frame) else {
        // 不是那个形状（长度不对）：不猜、不动账、也不回话——没有可信的"往哪回"。
        return;
    };
    if !matches!(
        mail::reserve(back),
        Ok((_vestor, owner, mark)) if owner == from && mark == ccall::BACK
    ) {
        // 这一趟没把回信孔交进来、或那一格指的是别人的孔：没有可回的路，账一动不动。
        return;
    }
    // 答一句：**形由 [`ccall::Union`] 说**（三种答形合一：格状态 / 一格答 / 一窗号）——装与发
    // 都不在这一层写字节（缓冲在这一帧的栈上＝本族最大那一形）。
    // **写端跟着这一趟走**：落出作用域时等这只手被取走（`Drop`）——那一位客人不来取，卡的是
    // 他自己那一趟：**一枚孔一枚写端，"一格招待所有客人"编不出来**。
    {
        let mut tx = Sender::<ccall::Union>::from_token(back);
        let _ = tx.send(answer(book, roster, mine, from, ask));
    }
    let _ = mail::release(back);
}

/// 把一句问交给核心，编出一句答（**三种答形**：格状态 / 一格答 / 一窗号）。
/// **形状由 [`ccall::Wire`] 说**（收帧那一侧已按动作解好：两格载荷的意义随之定，不再是一枚裸码
/// ＋ 两个裸数）。三条**写**原语同一个起手：**先拿发送者过名册**（[`who`]）。三条读不过名册
/// ——`amid` 的 `p` 与两条取窗的键都是问的人给的标签（K6）。
fn answer(
    book: &mut Coalition,
    roster: &Face,
    mine: ccall::Grant,
    from: TaskId,
    ask: Option<ccall::Wire>,
) -> ccall::Union {
    // 表外的动作码：这一问有回信的路，只是这一码我不认（与"读不懂"同一格）。
    let Some(ask) = ask else {
        return ccall::Union::Status(ccall::BAD);
    };
    if ccall::Grant::of_wire(&ask) != mine.at() {
        debug!(
            "coalition: face={} asked={} denied",
            mine.name(),
            ccall::Grant::of_wire(&ask)
        );
        return ccall::Union::Status(ccall::DENIED);
    }
    match ask {
        // `found` 的钥匙是"你得是个已绑定的身份"（K3），**解析出来那条号是这一枚盟的盟主**（记进册里）。钥匙与"记下谁"是同一句话的两半。
        ccall::Wire::Found => match who(roster, from) {
            Ok(w) => match book.found(w) {
                Ok(c) => ccall::Union::One(ccall::Reply::value(c)),
                Err(fail) => ccall::Union::Status(ccall::fail_to_code(Some(fail))),
            },
            Err(fail) => ccall::Union::Status(ccall::fail_to_code(Some(fail))),
        },
        ccall::Wire::Enter(c) => match who(roster, from) {
            Ok(w) => match book.enter(w, c) {
                Ok(()) => ccall::Union::One(ccall::Reply::status(ccall::OK)),
                Err(fail) => ccall::Union::Status(ccall::fail_to_code(Some(fail))),
            },
            Err(fail) => ccall::Union::Status(ccall::fail_to_code(Some(fail))),
        },
        ccall::Wire::Leave(c) => match who(roster, from) {
            Ok(w) => match book.leave(w, c) {
                Ok(()) => ccall::Union::One(ccall::Reply::status(ccall::OK)),
                Err(fail) => ccall::Union::Status(ccall::fail_to_code(Some(fail))),
            },
            Err(fail) => ccall::Union::Status(ccall::fail_to_code(Some(fail))),
        },
        // **代报名**：两个人要解——发送者（得是盟主）与**这一问点名的那位**（`target` 是 TID，
        // 解名只有本域做得了：它持着名册问面）。两解任一不成 ⇒ 同一格 [`Fail::Unknown`]
        // （"这个 TID 没绑过"就是那一格的原文）。
        ccall::Wire::Admit(c, target) => {
            match who(roster, from).and_then(|chief| who(roster, target).map(|t| (chief, t))) {
                Ok((chief, t)) => match book.admit(chief, c, t) {
                    Ok(()) => ccall::Union::One(ccall::Reply::status(ccall::OK)),
                    Err(fail) => ccall::Union::Status(ccall::fail_to_code(Some(fail))),
                },
                Err(fail) => ccall::Union::Status(ccall::fail_to_code(Some(fail))),
            }
        }
        ccall::Wire::Amid(p, c) => match book.amid(p, c) {
            Ok(yes) => ccall::Union::One(ccall::Reply::yes(yes)),
            Err(fail) => ccall::Union::Status(ccall::fail_to_code(Some(fail))),
        },
        // 两条取窗：`a` 是键，`b` 是**游标 + 1**（`0` = 没有游标，见 [`ccall`] 的帧那一节）——
        // 游标那一手已经在 `Wire` 里解好了。
        ccall::Wire::Band(c, after) => match book.band(c, after) {
            Ok(window) => ccall::Union::seq(&window),
            Err(fail) => ccall::Union::Status(ccall::fail_to_code(Some(fail))),
        },
        // `bloc` 没有失败域（`p` 是标签，不在任何盟里就是空窗）。
        ccall::Wire::Bloc(p, after) => ccall::Union::seq(&book.bloc(p, after)),
    }
}

/// 发送者此刻代表谁——**"self"的全部护栏就是这一句**（正文"已知边界"）。
/// 两条失败压成一格——"这条 TID 没绑"与"身份服务答不上来（超时 / 对面没了）"：
/// **调用方的下一步在两种情况下相同**（别指望这条路）；principal 那枚 `Denied` 翻不过来，
/// 因为本族的 `Denied` 是空的（盟无主）。
fn who(roster: &Face, from: TaskId) -> Result<PrincipalId, Fail> {
    let task = roster.task(from);
    task.principal(Wait::AtMost(MS))
        .map_err(|_| Fail::Unknown)?
        .map(|p| p.id())
        .ok_or(Fail::Unknown)
}

/// 找**身份服务**那份门牌（**问面**那一条）：`"/svc/sys/principal/ask"`，**译不出就再问**（有界）。
/// 门牌是 principal 自己跑完它那一段才落下的（它比本域先起来，但"就绪"与"上树"不是同一步）
/// ——故那一趟**必须带重试**：名字 → 号（撞 `UNKNOWN` 就退避一拍再来）→ 入口。
/// 这一趟与另外七处（`canonical` / `sleeper` / `probe-rule-other` / `subject` / `member` / `probe-rule` / `guest`）逐字同构，
fn find_face(tree: &TreeFace) -> Option<PieToken> {
    // 路是**本族那一族的常量**（`/svc/sys/principal`）＋ 那一面的名——一处都不自己拼。
    let Some(road) = pcall::DIR.try_join(pcall::Grant::Ask.name()) else {
        debug!("coalition: find_face deny=join");
        return None;
    };
    let tile = match tree.tile(&road, Wait::AtMost(MS)) {
        Ok(tile) => tile,
        Err(fail) => {
            debug!("coalition: find_face deny=tile road={road} fail={fail:?}");
            return None;
        }
    };
    match tile.token(Wait::AtMost(MS)) {
        Ok(entry) => Some(entry),
        Err(fail) => {
            debug!("coalition: find_face deny=token road={road} fail={fail:?}");
            None
        }
    }
}
