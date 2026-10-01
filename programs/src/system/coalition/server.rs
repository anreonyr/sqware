//! coalition::server — **结盟服务那一台**：一枚线程守着盟册（一条关系 + 一枚计数器）。
//!
//! 载体是 rtc / principal 那一面已经量过的形状——**门牌自带回信孔**：客人替这一趟铸一枚回信
//! 孔借过来、把帧推上门牌，本域从**门牌那一枚**读（发送者由内核在 `Push` 那一刻盖章），办完事
//! 从那枚孔答回去、当场放下。故这里**没有客人账**：一位客人不需要本域记住任何东西。
//!
//! ```text
//!   起手：读自己的 Sire（**只为上板与上树两条会话**——盟无主，之后不落任何字段）
//!         → 上板（板看得见本域的死）→ 铸门牌**两枚**（两面各一枚）
//!           两枚都经 LAND 落到树上 `/svc/sys/coalition/{ask,set}`，再逐面 FIND 回来验一遍
//!           **只把问面那枚交给持树者**（它只叫 `Amid`；定面留在树上给客人）
//!         → FIND `/svc/sys/principal/ask`（**带重试**）拿一份身份服务的**问面**门牌
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

use crate::program::coalition::E_COALITION;
use crate::system::control::service::Start;
use env::Wait;

use crate::system::carrier::carrier;
use crate::system::coalition::core::Coalition;
use crate::system::mount;
use crate::system::operator::bridge;
use env::{PieToken, TaskId};
use protocol::communication::sender::Sender;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::system::coalition as ccall;
use protocol::system::coalition::Fail;
use protocol::system::operator::Permit;
use protocol::system::operator::client as operator;
use protocol::system::operator::client::{Face as TreeFace, Mine};
use protocol::system::principal as pcall;
use protocol::system::principal::PrincipalId;
use protocol::system::principal::client::Face;
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

        // **照实记（"上板"那一格退场：撤板那一刀）**：与名册同形——只为让板看得见它的死；板那一族
        // 的死信号整片退场（监督那一趟改读内核那一格）⇒ 这一格退场。

        // 三、门牌**两枚**：**一面一枚**（[`mount::entry`] 按"记号 ＋ 面名"给那一枚孔与末段名）。
        //
        // **两面各一枚、不是一枚两面**：本族没有会话，服务端只能从**它自己表里哪一枚孔**收到来
        // 认面（见文件头）。两枚都长命——铸它们的是本线程自己（`Assembly::supervise` 那条照实记）。
        let (ask, ask_name) = mount::entry(ccall::Grant::Ask.mark(), ccall::Grant::Ask.name())
            .map_err(|_| Start::Tree(E_COALITION))?;
        let (set, set_name) = mount::entry(ccall::Grant::Set.mark(), ccall::Grant::Set.name())
            .map_err(|_| Start::Tree(E_COALITION))?;

        // 四、上树：分 `/svc` ＋ 分 `/svc/sys/coalition`、逐面落那两格、再逐面查回来验一遍
        //     （同 router / rtc / principal）。
        //
        // **照实记（这一处为什么包成 `Face`，task-2 那一刀）**：本域此后只要树上那几手
        // （`part` / `land` / `find` / `name` / `entry_of`：上树那一趟 ＋ 找身份那份门牌），
        // 那条会话的裸孔一个都不再要 ⇒ 按"已持 `Session` 则用 `Face`"把它交给
        // [`TreeFace::of`]（吃所有权），`find_face` 改收 `&TreeFace`。
        //
        // **照实记（`serve_tree` 那一枚壳随回炉退场）**：它原先包着下面这一句，而包的理由只有
        // 一个——"给这一趟一个名字"。它没有自己的状态、没有自己的判断（`Mine::No` 与 `MS` 都是
        // 常量），去掉之后调用点直接叫 [`bridge::land`]，读数行一字不变。
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
    // 必须说得出这一句（与三台驱动、设备账那两处**同一手**，见 `programs/src/program.rs` 那一格）。
    let _ = protocol::communication::establish::endpoint(
        runtime::env::unit::sire(),
        env::Mark::of(crate::program::READY),
        env::Wait::POLL,
    );

        // 四之后：**门禁那一枚**——把这一枚门牌**直接交给持树者**（`host` = 持树者的号，
        // `Session::open` 收下的那一格）。它据此才判得了"这一位在那枚盟里吗"（`Permit::Among`）。
        //
        // 与 principal 那一格同一形状。这一枚在手时权限是
        // `FETCH|STORE|VEST`，故子集 `FETCH|STORE` 不越界。装配者那一侧按装配表上那一格
        // 交给持树者。**这一件事由本域自己做**（从前装配者还要按声明上那一格递号过来——那一格
        // 整段退场了，见 `Relation` 的头注：持树者按这一枚的**记号**认，不看谁开的、也不看几号）。
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
        .map_err(|_| Start::Tree(E_COALITION))?;

        // 五、**身份那一份门牌**：本域是它的客人（K7）。带重试——它可能落得比本域晚。
        //
        // **名字叫 `roster`**（照实记：它从前叫 `face`，而这一刀之后"面"指的是本族那两枚门牌）：
        // 它是**名册的问面**，唯一用处是 `who()` 那一句"发送者此刻代表谁"。
        //
        // **照实记（这一处拆成两句话，量"折在哪一步"）**：这一格从前两处共用一个
        // `Start::Face(E_COALITION)`（= `coalition: no identity plate`），而那是**两件事**：
        // ①[`find_face`] 空手（它两条腿各有 `deny=` 一行，见那边）；②**门牌拿回来了、却用不动**
        // （[`Face::of`] 问不出开者）。前一件在**树那一侧**，后一件在**本端表里**——下一步不同，
        // 读数也不该合成一句。（①那一侧**成功**的那条读数按判决退了场，见 [`find_face`]。）
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
                // （见 `establish::opened_by` 的照实记）——把本端读得到的三格一起报出来。这条路
                // 至今走不到（`find_face` 那一侧量过：`mark` 与 `want` 逐字节相同），故这一句是
                // "它真出了"时的第一手现场。
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
    // **照实记（那一格共用的答话存根退场了）**：同 `principal` 那一面——答话那一格从前是循环外
    // 一枚 `Outbox<ccall::Union>`，一位不回头的客人就能把整台盟册按在下一趟的 `send` 里。
    // 今天它跟着**那一趟**走（`turn` 里的 `Sender`）。
    carrier(
        E_COALITION,
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
fn turn(
    book: &mut Coalition,
    roster: &Face,
    mine: ccall::Grant,
    from: TaskId,
    frame: &[u8],
) {
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
    //
    // **收口必须在 `release` 之前**（照实记，与 `principal` 那一面同一条，量出来的）：那一等要用
    // 本域表里这一枚（走权限那一关），先放下它再等 ⇒ `Denied` 当场返回，而孔上那只手还指着
    // 这一帧的栈——下一个 `turn` 复用同一片栈，取的人复制到的是**别人的字节**。
    {
        let mut tx = Sender::<ccall::Union>::from_token(back);
        let _ = tx.send(answer(book, roster, mine, from, ask));
    }
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
        // `found` 的钥匙是"你得是个已绑定的身份"（K3），而**解析出来那条号从 K2 翻案起不再
        // 只当门卫**：它成为这一枚盟的**盟主**（记进册里）。钥匙与"记下谁"是同一句话的两半。
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

/// 找**身份服务**那份门牌（**问面**那一条）：`"/svc/sys/principal/ask"`，**译不出就再问**（有界）。
///
/// 门牌是 principal 自己跑完它那一段才落下的（它比本域先起来，但"就绪"与"上树"不是同一步）
/// ——故那一趟**必须带重试**：名字 → 号（撞 `UNKNOWN` 就退避一拍再来）→ 入口。
/// 这一趟与另外七处（`canonical` / `sleeper` / `probe-rule-other` / `subject` / `member` / `probe-rule` / `guest`）逐字同构，
/// 已并进 [`TreeFace::tile`] ＋ [`Tile::token`] 那一趟。
///
/// **照实记（"额度"那句随退避那一刀改口径）**：译号那条腿今天是**真时限 ＋ 节拍退避**
/// （`operator/client.rs` 的 `road_to_id`），取门闩那一问仍是"就地问一次"——两腿各管自己那一问，
/// 没有"合起来算额度"的函数。**而它仍不是"整趟时限"**：推不进去会当场答 `Busy`（门是单槽，
/// 那一位可能正被别的域问着），故这一族的 `Wait` 只承诺"**本端愿意等多久**"。
///
/// **照实记（收 `&TreeFace`，不再收 `&Session`）**：本域**已持**一面（上树那一趟包出来的），
/// 故这一手只借它——`Face` 把 Session 藏在里面，签名上不再出现那条线。
///
/// **要的是名册的「问面」**（开面那一刀）：本域只用 `Resolve`（"这一位此刻代表谁"），而它今天
/// 落在 `/svc/sys/principal/ask` 那一格上——`/svc/sys/principal` 自己已是那段前缀（一块 `Pane`）。
///
/// # 照实记（**"两条腿各报一行"这条读数退了场，两条 `deny=` 留下**）
///
/// 起手那一句 `coalition: no identity plate` 从前只说"没拿到"，而"没拿到"在这条路上是**两件
/// 事**，下一步完全不同：
///
///   - **`tile` 那条腿**（路 → 号）：路还没落下 / 树那边答不上来 —— 折在**树那一侧**，
///     且它带的退避重试已经在 [`TreeFace::tile`] 里跑尽（放弃那一行读数在那边）；
///   - **`token` 那条腿**（号 → 那一枚门牌）：号有了、`Find` 答不回来（那一号是块窗格、
///     或它后面那一位没了）—— 这是"名册上树了但那一格不对"。
///
/// 曾为它每条腿各挂一行读数，且**成功那一行**把拿回来的那枚的号 ＋ `Reserve` 三格一起报出来
/// （用来判"拿到的那一枚**根本不是它**"）。**判决：无罪**——debug 档 `product` 景里本域起手
/// **每一趟都是 `ok`**、`mark` 与 `want` **逐字节相同**（`entry=193 vestor=12 owner=14
/// mark=0xc388b8d828255205 want=0xc388b8d828255205`），那一形**没发生过**；那几趟红的病根在
/// **内核复制**（`mail::copy` 把两侧段表锁步走却各推一整段，见 `operator/client.rs` 的 `call`）。
///
/// 故**成功那一行连同它那一枚 `Reserve` 退场**（证伪即收：它在每台机器都要走的那条起手路上，
/// debug 档每趟多一次 envcall），而两条腿的 `deny=` 留在**失败那一路**上——"折在哪条腿上"正是
/// 下一次本域起不来时要问的第一件事。读数与命令见提交 `3c366ef`。
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
