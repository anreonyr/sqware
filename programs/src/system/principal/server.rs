//! principal::server — **身份服务那一台**：一枚线程守着两张表（名册与谱系）。
//!
//! 载体是 rtc 那一面已经量过的形状——**门牌自带回信孔**：客人替这一趟铸一枚回信孔借过来、
//! 把帧推上门牌，本域从**门牌那一枚**读（发送者由内核在 `Push` 那一刻盖章），办完事从那枚孔
//! 答回去、当场放下。故这里**没有客人账**：一位客人不需要本域记住任何东西，"往哪回"那一格
//! 就在这一趟的孔上。
//!
//! ```text
//!   起手：读自己的 Sire（= 装配者，名册的钥匙）
//!         → 上板（板看得见本域的死）→ 铸门牌**两枚**（两面各一枚）
//!           定面（Set）一份交给生我者（装配期用它 derive + bind，不必上树查自己）
//!           问面（Ask）一份直接交给持树者（门禁只问"这一位代表谁"）
//!           两枚都经 LAND 落到树上 `/svc/sys/principal/{ask,set}`（别的客人按名字找上门）
//!   常驻：一只组等那两枚 —— **从哪一枚读到**就是哪一面 → 交给核心 → 从这一趟的回信孔答回去
//! ```
//!
//! **面为什么长在门牌上**（开面那一刀，与 operator 不同的一格）：本族**没有会话**——门牌自己
//! 就是那条路，所有人往同一枚孔推帧，故服务端原先**分不出面**。开面之后是**两枚门牌、两只孔**：
//! 从哪一枚读到就是哪一面，而**那一问属不属于这一面**由 [`Grant::of_wire`] 当场对一次
//! （对不上答 [`pcall::DENIED`]）。理由与持有者那三行见 [`pcall::grant`] 的文件头。

use crate::program::principal::E_PRINCIPAL;
use crate::system::control::service::Start;
use env::Wait;

use crate::system::carrier::carrier;
use crate::system::mount;
use crate::system::operator::bridge;
use crate::system::principal::core::Principal;
use env::TaskId;
use protocol::communication::sender::Sender;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::system::operator::Permit;
use protocol::system::operator::client as operator;
use protocol::system::operator::client::Mine;
use protocol::system::principal as pcall;
use protocol::system::principal::PrincipalId;
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail::{self, HolePie};

/// 等板 / 等树的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 起服务：**读锚 → 上板 → 铸门牌两枚 → 一枚线程招待所有客人**。
///
/// **起手那几步收在一个闭包**（与持树者 / 盟册那两台同形）：它们清一色是"不成 ⇒ 这域
/// 起不来"的早退步，从前每步一段 `let Ok(..) = .. else { return Err(..) }`——报的是同一个死法、
/// 写的是七段岔口，主脉络因此被岔口切碎。收进闭包之后全走 `?`、失败域在末尾**折一次**。
/// `serve` 的主干于是只剩两步：**起手 → 常驻**。
///
/// 起手要交出去的三样：`ask` / `set`（那两枚门牌的号——**从哪一枚读到就是哪一面**）与 `book`
/// （[`turn`] 收它）。常驻那一趟（立组、收帧那一页、面的判定）在 [`carrier`]。
pub fn serve() -> Result<(), Start> {
    // 一～六：起手（读锚 → 上板 → 铸两枚门牌 → 上树 → 两张表 → 常驻那只组）。
    let (mut book, ask, set) = (|| {
        // 一、锚：**生我者就是装配者**。名册只认这一枚——`Sire` 是内核盖的，比任何自报都硬；
        //    它还是弱引用，装配者一退这一格就答 0（那之后没人能写名册，也不该有）。
        // **起我那一枚线程**：本域是装配者建的，故 `Sire` 答的就是它——只有这一条来源。
        let assembler = runtime::env::unit::sire();

        // **照实记（"上板"那一格退场：撤板那一刀）**：本域从前开一条 `board::BERTH` 会话，只为让
        // 板看得见它的死。板那一族的死信号整片退场（监督那一趟改读内核那一格）⇒ 这一格退场。

        // 三、门牌**两枚**：**一面一枚**（[`mount::entry`] 按"记号 ＋ 面名"给那一枚孔与末段名）。
        //
        // **两面各一枚、不是一枚两面**：本族没有会话，服务端只能从**它自己表里哪一枚孔**收到来
        // 认面（见文件头）。两枚都长命——铸它们的是本线程自己（`Assembly::supervise` 那条照实记）。
        let (ask, ask_name) = mount::entry(pcall::Grant::Ask.mark(), pcall::Grant::Ask.name())
            .map_err(|_| Start::Tree(E_PRINCIPAL))?;
        let (set, set_name) = mount::entry(pcall::Grant::Set.mark(), pcall::Grant::Set.name())
            .map_err(|_| Start::Tree(E_PRINCIPAL))?;

        // **定面先交给生我者**：装配期要靠它 derive + bind，而那条路不必先上树查自己。
        //
        // **本域自己交、不是装配者转授**：门牌由各域自己交（见 `operator/bridge.rs` 的
        // `COORD` 段）。装配者用这一枚只有**一条**路：往里**推帧**（`derive` / `bind`）；
        // 答话走每一趟自己铸的那枚回信孔（`communication::establish::lend_out` ＋ `HolePie::push`：铸孔 → 交
        // `STORE` → 把"那一格"编进帧 → 推），读端在装配者这边。⇒ **`STORE` 就是这一格的全部需要**。
        //
        // **它拿不到问面**（照实记：从前这一份就是唯一那一枚，两面都在里面）：装配期不需要读身份
        // ——`derive` / `bind` 两条都在定面上。
        port::ship(
            &HolePie::from_token(set),
            assembler,
            Access::STORE,
            Policy::NONE,
        )
        .map_err(|_| Start::Tree(E_PRINCIPAL))?;

        // 四、上树：分 `/svc`、分 `/svc/sys/principal`、落那两格，再**逐面**查回来验一遍。
        //
        // **这一趟住在 [`bridge::land`]**（四族＋驱动四处逐字同构、收在一处）；本处只剩两件
        // **本族的事实**——路（`/svc` ＋ `/svc/sys/principal`）与那两枚门牌。
        //
        // **照实记（`serve_tree` 那一枚壳随回炉退场）**：它原先包着下面这一句，而包的理由只有
        // 一个——"给这一趟一个名字"。它没有自己的状态、没有自己的判断（`Mine::No` 与 `MS` 都是
        // 常量），去掉之后调用点直接叫 `land`，读数行一字不变。
        //
        // `got` 只是"认出了那一枚"；它指不指得回原物，由**真客人**证（`harness` 那几台）。
        let session = Session::open(assembler, operator::BERTH, Wait::AtMost(MS))
            .map_err(|_| Start::Tree(E_PRINCIPAL))?;
        let _ = bridge::land(
            &operator::Face::from(&session),
            "principal",
            &pcall::DIR,
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

        // 四之后：**问面那一枚交给持树者**（`host` = 持树者的号，`Session::open` 收下的那一格）。
        // 它据此才判得了"这一位此刻代表谁"；而它**做不出** `Adopt`——那条在定面上。
        //
        // 这一枚在手时权限是 `FETCH|STORE|VEST`，故子集 `FETCH|STORE` 不越界。
        port::ship(
            &HolePie::from_token(ask),
            session.host,
            Access::FETCH | Access::STORE,
            Policy::NONE,
        )
        .map_err(|_| Start::Tree(E_PRINCIPAL))?;

        // 五、两张表：名册空着，谱系只有根（零号节点）。
        let book = Principal::new(assembler).map_err(|_| Start::Book(E_PRINCIPAL))?;
        Ok::<_, Start>((book, ask, set))
    })()?;

    // 六、常驻：**一只组等那两枚门牌**（[`carrier`] 那一趟：立组 → 挂两枚 → 备一页 → 等 →
    // **从哪一枚读到就是哪一面** → 把这一批取干净 → 交给 [`turn`]）。这一族没有会话可读记号，
    // 故"面"只有这一条来路。
    // **照实记（那一格共用的答话存根退场了）**：从前这里住着一枚 `Outbox<pcall::Reply>`，
    // 孔另由每一趟现给 ⇒ 一位不回头的客人就能把整台名册按在下一趟的 `send` 里。今天答话那一格
    // 跟着**那一趟**走（`turn` 里的 `Sender`）。
    carrier(
        E_PRINCIPAL,
        &[(ask, pcall::Grant::Ask), (set, pcall::Grant::Set)],
        |face, from, frame| turn(&mut book, from, face, frame),
    )
}

/// 门上一句话：解帧 → 交给核心 → **从这一趟自带的那枚孔答回去**。
///
/// 认那枚孔靠**帧里那一格** ＋ **一次 [`mail::reserve`] 验**：那一格是
/// "客人借来的那枚回信孔**在我表里**是几号"，而"是谁给的、刻的什么"仍要当场读出来核对——
/// 否则客人能让本域往**别人的孔**里写。
///
/// `from` 是**内核盖的发送者**，名册与谱系的钥匙判据（装配者 / 当前正好代表 `p`）用的就是它。
/// `face` 是**这一帧从哪一枚门牌进来的**（[`serve`] 那只组说的事实）。
fn turn(book: &mut Principal, from: TaskId, face: pcall::Grant, frame: &[u8]) {
    let Some((ask, back)) = pcall::Wire::take(frame) else {
        // 不是那个形状（长度不对）：不猜、不动账、也不回话——没有可信的"往哪回"。
        return;
    };
    if !matches!(
        mail::reserve(back),
        Ok((_vestor, owner, mark)) if owner == from && mark == pcall::BACK
    ) {
        // 这一趟没把回信孔交进来、或那一格指的是别人的孔：没有可回的路，账一动不动。
        return;
    }
    // 答一句：**一格**（[`pcall::Reply`] 那一形）——走这一趟那枚回信孔，装与发都不在这一层
    // 写字节（缓冲在这一帧的栈上：这一形定长 10）。
    // **写端跟着这一趟走**：落出作用域时等这只手被取走（`Drop`）——那一位客人不来取，
    // 卡的是他自己那一趟：**一枚孔一枚写端，"一格招待所有客人"编不出来**。
    //
    // **收口必须在 `release` 之前**（照实记，量出来的）：那一等用的是**本域表里这一枚**
    // （`wait(HoleDir::Push)` 要走权限那一关），先放下它、再等 ⇒ `Denied` 当场返回，
    // 而孔上那只手还指着这一帧的栈——下一个 `turn` 复用同一片栈，取的人复制到的就是**别人的
    // 字节**（症状：客侧 `recv-unread`，而驱动的 `hand_over` 读数一切正常）。故这里用一层
    // 作用域把"收口"钉在"放下"之前。
    {
        let mut tx = Sender::<pcall::Reply>::from_token(back);
        let _ = tx.send(answer(book, from, ask, face));
    }
    let _ = mail::release(back);
}

/// 把一句问交给核心，编出一句答（**一格**：读不懂也答，答 `BAD`）。
///
/// **形状由 [`pcall::Wire`] 说**（收帧那一侧已按动作解好了——两格载荷的意义随之定，不再是一枚
/// 裸码 ＋ 两个裸数）。答案与失败分开放（见 [`pcall`]：`OK` + `flag` 是答案，负码表只装失败）。
///
/// **面这一道在交给核心之前**（[`Grant::of_wire`] 对那一条线上一码）：这一帧是从 Face 那枚门牌
/// 进来的，而它问的那件事属不属于那一面，只有这一句说得清。对不上答 [`pcall::DENIED`]
/// ——**终态**（换一枚门牌 / 别重试），与 kernel 那几格失败分开。
///
/// **照实记（这一道在 core 之外）**：核心那三条 `Fail`（`Denied` / `Unknown` / `Full`）讲的都是
/// **名册与谱系本身**的事（你是谁、那条号在不在、备不备得下）；"你手里那一枚门牌给不给这一条"
/// 是**载体**的事——故它在这一层，且它先于核心（连账都不必看一眼）。
fn answer(
    book: &mut Principal,
    from: TaskId,
    ask: Option<pcall::Wire>,
    face: pcall::Grant,
) -> pcall::Reply {
    // 表外的动作码：这一问有回信的路，只是这一码我不认（与"读不懂"同一格）。
    let Some(ask) = ask else {
        return pcall::Reply::status(pcall::BAD);
    };
    if pcall::Grant::of_wire(&ask) != face.at() {
        // 这一条不在它手里那一面上：不开那一面的门牌做不出这件事（照实记：从前两面是一面，
        // 持树者手里那枚门牌因此**做得出** `Adopt`）。
        //
        // **这一句读数不是装饰**：核那一条拒（"你不是写名册的那一枚"）也答 `Fail::Denied`
        // ——两个因同码（客人的下一步一样），分得开它们的只有这一行。
        debug!(
            "principal: face={} asked={} denied",
            face.name(),
            pcall::Grant::of_wire(&ask)
        );
        return pcall::Reply::status(pcall::DENIED);
    }
    match ask {
        // 照实记（这一格有两种因，同码）：发送者不是装配者、或那一条号不在树里——两者都答
        // `Fail::Denied` / `Unknown`，分开它们的是持册者那一侧要看得见的读数。
        pcall::Wire::Bind(tid, p) => match book.bind(from, tid, p) {
            Ok(()) => pcall::Reply::status(pcall::OK),
            Err(fail) => pcall::Reply::status(pcall::fail_to_code(Some(fail))),
        },
        pcall::Wire::Resolve(tid) => match book.resolve(tid) {
            Some(p) => pcall::reply_present(true, p),
            None => pcall::reply_present(false, PrincipalId::ROOT),
        },
        pcall::Wire::Derive(p) => match book.derive(from, p) {
            Ok(q) => pcall::Reply::value(q),
            Err(fail) => pcall::Reply::status(pcall::fail_to_code(Some(fail))),
        },
        pcall::Wire::Sire(p) => match book.sire(p) {
            Ok(Some(q)) => pcall::reply_present(true, q),
            Ok(None) => pcall::reply_present(false, PrincipalId::ROOT),
            Err(fail) => pcall::Reply::status(pcall::fail_to_code(Some(fail))),
        },
        pcall::Wire::Heir(a, b) => {
            match book.heir(a, b) {
                Ok(yes) => pcall::Reply::yes(yes),
                // "不是祖先"是一句答（`Ok(false)`），"查无此号"才是这一格。
                Err(fail) => pcall::Reply::status(pcall::fail_to_code(Some(fail))),
            }
        }
        // 转换那两条都只答状态那一格（成功 = `OK`）；钥匙是**发送者**，报文里没有"我是谁"。
        pcall::Wire::Adopt(p) => match book.adopt(from, p) {
            Ok(()) => pcall::Reply::status(pcall::OK),
            Err(fail) => pcall::Reply::status(pcall::fail_to_code(Some(fail))),
        },
        pcall::Wire::Waive => match book.waive(from) {
            Ok(()) => pcall::Reply::status(pcall::OK),
            Err(fail) => pcall::Reply::status(pcall::fail_to_code(Some(fail))),
        },
    }
}
