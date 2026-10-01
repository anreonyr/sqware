//! principal::serve — **身份服务那一台**：一枚线程守着两张表（名册与谱系）。
//! 载体是 rtc 那一面已经量过的形状——**门牌自带回信孔**：客人替这一趟铸一枚回信孔借过来、
//! 把帧推上门牌，本域从**门牌那一枚**读（发送者由内核在 `Push` 那一刻盖章），办完事从那枚孔
//! 答回去、当场放下。故这里**没有客人账**：一位客人不需要本域记住任何东西，"往哪回"那一格
//! 就在这一趟的孔上。
//! ```text
//!   起手：读自己的 Sire（= 装配者，名册的钥匙）
//!         → 上板（板看得见本域的死）→ 铸门牌**两枚**（两面各一枚）
//!           定面（Set）一份交给生我者（装配期用它 derive + bind，不必上树查自己）
//!           问面（Ask）一份直接交给持树者（门禁只问"这一位代表谁"）
//!           两枚都经 LAND 落到树上 `/svc/sys/principal/{ask,set}`（别的客人按名字找上门）
//!   常驻：一只组等那两枚 —— **从哪一枚读到**就是哪一面 → 交给核心 → 从这一趟的回信孔答回去
//! ```
//! **面为什么长在门牌上**：本族**没有会话**——门牌自己

use crate::system::common::life::service::Start;
use crate::unit::principal::E_PRINCIPAL;
use env::Wait;

use crate::service::operator::bridge;
use crate::service::principal::core::Principal;
use crate::system::common::face::carrier::carrier;
use crate::system::common::face::mount;
use env::TaskId;
use protocol::communication::sender::Sender;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::service::operator::Permit;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::Mine;
use protocol::service::principal as pcall;
use protocol::service::principal::PrincipalId;
use runtime::core::res::port::{self, Access, Policy};
use runtime::env::mail::{self, HolePie};

/// 等板 / 等树的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 起服务：**读锚 → 上板 → 铸门牌两枚 → 一枚线程招待所有客人**。
/// **起手那几步收在一个闭包**（与持树者 / 盟册那两台同形）：它们清一色是"不成 ⇒ 这域
pub fn serve() -> Result<(), Start> {
    // 一～六：起手（读锚 → 上板 → 铸两枚门牌 → 上树 → 两张表 → 常驻那只组）。
    let (mut book, ask, set) = (|| {
        // 一、锚：**生我者就是装配者**。名册只认这一枚——`Sire` 是内核盖的，比任何自报都硬；
        let assembler = runtime::env::unit::sire();

        // 三、门牌**两枚**：**一面一枚**（[`mount::entry`] 按"记号 ＋ 面名"给那一枚孔与末段名）。
        // **两面各一枚、不是一枚两面**：本族没有会话，服务端只能从**它自己表里哪一枚孔**收到来
        let (ask, ask_name) = mount::entry(pcall::Grant::Ask.mark(), pcall::Grant::Ask.name())
            .map_err(|_| Start::Tree(E_PRINCIPAL))?;
        let (set, set_name) = mount::entry(pcall::Grant::Set.mark(), pcall::Grant::Set.name())
            .map_err(|_| Start::Tree(E_PRINCIPAL))?;

        // **定面先交给生我者**：装配期要靠它 derive + bind，而那条路不必先上树查自己。
        // **本域自己交、不是装配者转授**：门牌由各域自己交（见 `service/operator/bridge.rs` 的
        // `COORD` 段）。装配者用这一枚只有**一条**路：往里**推帧**（`derive` / `bind`）；
        // 答话走每一趟自己铸的那枚回信孔（`communication::establish::lend_out` ＋ `HolePie::push`：铸孔 → 交
        port::ship(
            &HolePie::from_token(set),
            assembler,
            Access::STORE,
            Policy::NONE,
        )
        .map_err(|_| Start::Tree(E_PRINCIPAL))?;

        // 四、上树：分 `/svc`、分 `/svc/sys/principal`、落那两格，再**逐面**查回来验一遍。
        // **这一趟住在 [`bridge::land`]**（四族＋驱动四处逐字同构、收在一处）；本处只剩两件
        // **本族的事实**——路（`/svc` ＋ `/svc/sys/principal`）与那两枚门牌。
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
        // 必须说得出这一句（与三台驱动、设备账那两处**同一手**，见 `programs/src/unit/catalog.rs` 那一格）。
        let _ = protocol::communication::establish::endpoint(
            runtime::env::unit::sire(),
            env::Mark::of(crate::unit::READY),
            env::Wait::POLL,
        );

        // 四之后：**问面那一枚交给持树者**（`host` = 持树者的号，`Session::open` 收下的那一格）。
        // 它据此才判得了"这一位此刻代表谁"；而它**做不出** `Adopt`——那条在定面上。
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
    carrier(
        E_PRINCIPAL,
        &[(ask, pcall::Grant::Ask), (set, pcall::Grant::Set)],
        |face, from, frame| turn(&mut book, from, face, frame),
    )
}

/// 门上一句话：解帧 → 交给核心 → **从这一趟自带的那枚孔答回去**。
/// 认那枚孔靠**帧里那一格** ＋ **一次 [`mail::reserve`] 验**：那一格是
/// "客人借来的那枚回信孔**在我表里**是几号"，而"是谁给的、刻的什么"仍要当场读出来核对——
/// 否则客人能让本域往**别人的孔**里写。
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
    {
        let mut tx = Sender::<pcall::Reply>::from_token(back);
        let _ = tx.send(answer(book, from, ask, face));
    }
    let _ = mail::release(back);
}

/// 把一句问交给核心，编出一句答（**一格**：读不懂也答，答 `BAD`）。
/// **形状由 [`pcall::Wire`] 说**（收帧那一侧已按动作解好了——两格载荷的意义随之定，不再是一枚
/// 裸码 ＋ 两个裸数）。答案与失败分开放（见 [`pcall`]：`OK` + `flag` 是答案，负码表只装失败）。
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
        debug!(
            "principal: face={} asked={} denied",
            face.name(),
            pcall::Grant::of_wire(&ask)
        );
        return pcall::Reply::status(pcall::DENIED);
    }
    match ask {
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
        pcall::Wire::Heir(a, b) => match book.heir(a, b) {
            Ok(yes) => pcall::Reply::yes(yes),
            Err(fail) => pcall::Reply::status(pcall::fail_to_code(Some(fail))),
        },
        // 转换那两条都只答状态那一格（成功 = `OK`）；钥匙是**发送者**，报文里没有"我是谁"。
        pcall::Wire::Adopt(p) => match book.adopt(from, p) {
            Ok(()) => pcall::Reply::status(pcall::OK),
            Err(fail) => pcall::Reply::status(pcall::fail_to_code(Some(fail))),
        },
        pcall::Wire::Waive => match book.waive(from) {
            Ok(()) => pcall::Reply::status(pcall::OK),
            Err(fail) => pcall::Reply::status(pcall::fail_to_code(Some(fail))),
        },
        pcall::Wire::Drop => match book.drop(from) {
            Ok(()) => pcall::Reply::status(pcall::OK),
            Err(fail) => pcall::Reply::status(pcall::fail_to_code(Some(fail))),
        },
    }
}
