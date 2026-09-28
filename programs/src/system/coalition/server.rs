//! coalition::server — **结盟服务那一台**：一枚线程守着盟册（一条关系 + 一枚计数器）。
//!
//! 载体是 rtc / principal 那一面已经量过的形状——**门牌自带回信孔**：客人替这一趟铸一枚回信
//! 孔借过来、把帧推上门牌，本域从**门牌那一枚**读（发送者由内核在 `Push` 那一刻盖章），办完事
//! 从那枚孔答回去、当场放下。故这里**没有客人账**：一位客人不需要本域记住任何东西。
//!
//! ```text
//!   起手：读自己的 Sire（**只为上板与上树两条会话**——盟无主，之后不落任何字段）
//!         → 上板（板看得见本域的死）→ 铸门牌**两枚**（两面各一枚）
//!           两枚都经 LAND 落到树上 `/sys/coalition/{ask,set}`，再逐面 FIND 回来验一遍
//!           **只把问面那枚交给持树者**（它只叫 `Amid`；定面留在树上给客人）
//!         → FIND `/sys/principal/ask`（**带重试**）拿一份身份服务的**问面**门牌
//!   常驻：一只组等那两枚 —— **从哪一枚读到**就是哪一面 → 先过名册问"你是谁" → 交给核心 → 答回去
//! ```
//!
//! **两条锚为什么都在这儿**：`Sire` 是内核盖的（比任何自报都硬，且是弱引用：装配者一退它
//! 就答 0），树那条路是"按名找人"的现成一步；而身份那一份门牌**只能按名字找**——本域不是
//! 装配者，拿不到它手里那一份副本（正文 K7 的被否项：转授要新装配机制）。
//!
//! **面为什么长在门牌上**（开面那一刀，同 principal）：本族**没有会话**——门牌自己就是那条路，
//! 所有人往同一枚孔推帧，故服务端原先**分不出面**。今天两枚门牌、两只孔：**从哪一枚读到**就是
//! 哪一面，而"这一问属不属于这一面"由 [`Grant::of_wire`] 当场对一次（对不上答
//! [`ccall::DENIED`]）。⇒ **交给持树者的那枚门牌做不出 `Found`**（立一枚盟）——它一辈子只叫
//! `Amid`。为什么是两面、那一个生产持有者各要哪几条，见 [`ccall::grant`] 的文件头。

use crate::system::control::service::Start;
use env::Wait;

use env::{Name, PieToken, TaskId};
use protocol::debug;
use protocol::communication::sender::Sender;
use protocol::communication::session::Session;
use crate::system::board::client as board;
use crate::system::carrier::carrier;
use crate::system::operator::bridge;
use protocol::system::coalition as ccall;
use crate::system::coalition::core::Coalition;
use crate::system::coalition::mount;
use protocol::system::coalition::Fail;
use protocol::system::operator::client as operator;
use protocol::system::operator::client::{Face as TreeFace, Mine};
use protocol::system::principal as pcall;
use protocol::system::principal::client::Face;
use protocol::system::principal::PrincipalId;
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail::{self, HolePie};

/// 等板 / 等树 / 问名册的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 起服务：**读锚 → 上板 → 铸门牌上树 → 找身份那一份门牌 → 一枚线程招待所有客人**。
///
/// **起手那几步收在一个闭包**（与持树者 / 名册那两台同形）：它们清一色是"不成 ⇒ 这域
/// 起不来"的早退步，从前每步一段 `let Ok(..) = .. else { return Err(..) }`——报的是同一个死法、
/// 写的是七段岔口，主脉络因此被岔口切碎。收进闭包之后全走 `?`、失败域在末尾**折一次**。
pub fn serve() -> Result<(), Start> {
    // 一～六：起手（读锚 → 上板 → 铸两枚门牌 → 上树 → 找身份那一份 → 空册）。
    let (mut book, roster, ask, set) = (|| {
        // 一、锚：`Sire` = 装配者。**只为上板与上树两条会话**——盟无主，核心不需要它
        //     （对照 principal：那边把它当名册钥匙，注入核心那一格）。
        // **起我那一枚线程**：本域是装配者建的，故 `Sire` 答的就是它——只有这一条来源。
        let assembler = runtime::env::unit::sire();

        // 二、上板：只为让板看得见本域的死（它常驻，编排域据此记账）。
        let _board = Session::open(assembler, board::BERTH, Wait::AtMost(MS))
            .map_err(|_| Start::Board)?;

        // 三、门牌**两枚**：**一面一枚**（[`mount::entry`] 按面给记号与末段名）。
        //
        // **两面各一枚、不是一枚两面**：本族没有会话，服务端只能从**它自己表里哪一枚孔**收到来
        // 认面（见文件头）。两枚都长命——铸它们的是本线程自己（同 `operator::mount` 那条照实记）。
        let (ask, ask_name) = mount::entry(ccall::Grant::Ask).map_err(|_| Start::Tree)?;
        let (set, set_name) = mount::entry(ccall::Grant::Set).map_err(|_| Start::Tree)?;

        // 四、上树：分 `/sys` ＋ 分 `/sys/coalition`、逐面落那两格、再逐面查回来验一遍
        //     （同 router / rtc / principal）。
        //
        // **照实记（这一处为什么包成 `Face`，task-2 那一刀）**：本域此后只要树上那几手
        // （`part` / `land` / `find` / `name` / `entry_of`：上树那一趟 ＋ 找身份那份门牌），
        // 那条会话的裸孔一个都不再要 ⇒ 按"已持 `Session` 则用 `Face`"把它交给
        // [`TreeFace::of`]（吃所有权），`serve_tree` / `find_face` 一并改收 `&TreeFace`。
        let session = Session::open(assembler, operator::BERTH, Wait::AtMost(MS))
            .map_err(|_| Start::Tree)?;
        let tree = TreeFace::of(session);
        serve_tree(
            &tree,
            [
                (ccall::Grant::Ask, ask, ask_name),
                (ccall::Grant::Set, set, set_name),
            ],
        );

        // 四之后：**门禁那一枚**——把这一枚门牌**直接交给持树者**（`host` = 持树者的号，
        // `Session::open` 收下的那一格）。它据此才判得了"这一位在那枚盟里吗"（`Permit::Among`）。
        //
        // 与 principal 那一格同一形状。这一枚在手时权限是
        // `FETCH|STORE|VEST`，故子集 `FETCH|STORE` 不越界。装配者那一侧按装配表上那一格
        // （`Eyes::League`）递——两枚门牌**分两帧、次序不定**，持树者收到哪一枚补哪一枚。
        //
        // **只交问面那一枚**（照实记：从前这一份就是唯一那一枚，两面都在里面）：持树者只叫
        // `Amid`（判 [`Permit::Among`](protocol::system::operator::Permit::Among)），而
        // "立盟 / 入 / 出"那三条在定面上——它拿不到，也就做不出。
        port::ship(
            &HolePie::from_token(ask),
            tree.host(),
            Access::FETCH | Access::STORE,
            Policy::NONE,
        )
        .map_err(|_| Start::Tree)?;

        // 五、**身份那一份门牌**：本域是它的客人（K7）。带重试——它可能落得比本域晚。
        //
        // **名字叫 `roster`**（照实记：它从前叫 `face`，而这一刀之后"面"指的是本族那两枚门牌）：
        // 它是**名册的问面**，唯一用处是 `who()` 那一句"发送者此刻代表谁"。
        let roster_entry = find_face(&tree).ok_or(Start::Face)?;
        let roster = Face::of(roster_entry).map_err(|_| Start::Face)?;

        // 六、一本空册：一枚号都还没铸（**起手不失败**——空册不分配）。
        let book = Coalition::new();
        Ok::<_, Start>((book, roster, ask, set))
    })()?;

    // 七、常驻：**一只组等那两枚门牌**（[`carrier`] 那一趟：立组 → 挂两枚 → 备一页 → 等 →
    // **从哪一枚读到就是哪一面** → 把这一批取干净 → 交给 [`turn`]）。这一族没有会话可读记号，
    // 故"面"只有这一条来路。
    carrier(
        &[(ask, ccall::Grant::Ask), (set, ccall::Grant::Set)],
        |face, from, frame| turn(&mut book, &roster, face, from, frame),
    )
}

/// 门上一句话：解帧 → 先过名册 → 交给核心 → **从这一趟自带的那枚孔答回去**。
///
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
    // `.ok()`：装不上那一格按构造到不了（`Buf` 由本族 `Message` 自己给，见 `Sender::send`）；
    // 真到了那里，这一答就发不出去。
    let _ = Sender::<ccall::Union>::from_token(back)
        .send(answer(book, roster, mine, from, ask), Wait::Forever)
        .ok();
    let _ = mail::release(back);
}

/// 把一句问交给核心，编出一句答（**三种答形**：格状态 / 一格答 / 一窗号）。
///
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
    // **面这一道在交给核心之前**（[`Grant::of_wire`] 对那一条线上一码）：这一帧从 `mine` 那枚
    // 门牌进来，而它问的那件事属不属于那一面，只有这一句说得清。对不上答 [`ccall::DENIED`]
    // ——**终态**（换一枚门牌 / 别重试），与核心那两格失败分开。
    //
    // **照实记（`DENIED` 这一码是这一刀添的）**：本族原先"没有 `Denied` 可落"（盟无主）——
    // 那说的是**核心**；这一句问的是**载体**。**这一行读数不是装饰**：与它同码的还有别的因，
    // 分得开它们的只有这一行。
    if ccall::Grant::of_wire(&ask) != mine.at() {
        debug!(
            "coalition: face={} asked={} denied",
            mine.name(),
            ccall::Grant::of_wire(&ask)
        );
        return ccall::Union::Status(ccall::DENIED);
    }
    match ask {
        // `found` 的钥匙是"你得是个已绑定的身份"（K3），**解析出来的那条号只当门卫**：
        // 盟无主（K2），不记铸造者——全族唯一一处。
        ccall::Wire::Found => match who(roster, from) {
            Ok(_) => ccall::Union::One(ccall::Reply::value(book.found())),
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
        ccall::Wire::Amid(p, c) => {
            match book.amid(p, c) {
                // "不在"是一句答（`Ok(false)`），"查无此盟"才是这一格。
                Ok(yes) => ccall::Union::One(ccall::Reply::yes(yes)),
                Err(fail) => ccall::Union::Status(ccall::fail_to_code(Some(fail))),
            }
        }
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
///
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

/// 找**身份服务**那份门牌（**问面**那一条）：`"/sys/principal/ask"`，**译不出就再问**（有界）。
///
/// 门牌是 principal 自己跑完它那一段才落下的（它比本域先起来，但"就绪"与"上树"不是同一步）
/// ——故那一趟**必须带重试**：名字 → 号（撞 `UNKNOWN` 就睡一拍再来，额度 [`MS`]）→ 入口。
/// 这一趟与另外七处（`canonical` / `sleeper` / `probe-rule-other` / `subject` / `member` / `probe-rule` / `guest`）逐字同构，
/// 已并进 [`TreeFace::tile`] ＋ [`Tile::token`] 那一趟（重试与额度都在里面）。**`MS` 是额度不是时限**——往返耗时
/// 不计账、推不进去还会等在门外，两处照实记见 `operator/client.rs` 的 `entry_of`。
///
/// **照实记（收 `&TreeFace`，不再收 `&Session`）**：本域**已持**一面（上树那一趟包出来的），
/// 故这一手只借它——`Face` 把 Session 藏在里面，签名上不再出现那条线。
///
/// **要的是名册的「问面」**（开面那一刀）：本域只用 `Resolve`（"这一位此刻代表谁"），而它今天
/// 落在 `/sys/principal/ask` 那一格上——`/sys/principal` 自己已是那段前缀（一块 `Pane`）。
fn find_face(tree: &TreeFace) -> Option<PieToken> {
    let (Ok(dir), Ok(segment), Ok(leaf)) = (
        Name::new(pcall::DIR),
        Name::new(pcall::NAME),
        Name::new(pcall::Grant::Ask.name()),
    ) else {
        return None;
    };
    tree.tile(&[dir, segment, leaf], Wait::AtMost(MS))
        .ok()?
        .token(Wait::AtMost(MS))
        .ok()
}

/// 上树那一趟：**分目录 → 落门牌 → 查回来验一遍**（同 rtc / principal 那一趟）。
///
/// `got` 只是"认出了那一枚"；它指不指得回原物，由**真客人**（`harness/src/member.rs`）证——它照同一条路
/// 找上门、立一枚盟、进进出出。故本域不自问自答。
///
/// **照实记（手上的裸孔换成了面上的五个方法）**：从前这里先 `(&session.link, session.talk,
/// session.host)` 把那条线拆出来，再一路叫自由函数；现在调用方给的是 [`TreeFace`]，四个动作与
/// 对端号都从它出（`host` 那一格只在门禁转授时用，见 `serve` 的"四之后"）。
fn serve_tree(tree: &TreeFace, faces: [(ccall::Grant, PieToken, Name); 2]) {
    // **这一趟本身住在 [`bridge::land`]**（四处逐字同构、收在一处；量的行数见它自己的照实记）：
    // 本手只剩两件本族的事实——路（`/sys` ＋ `/sys/coalition`）与那两枚门牌。
    let list = [
        (faces[0].2.as_str(), faces[0].1),
        (faces[1].2.as_str(), faces[1].1),
    ];
    let _ = bridge::land(
        tree,
        "coalition",
        &[ccall::DIR, mount::SEGMENT],
        Mine::No,
        &list,
        Wait::AtMost(MS),
    );
}

