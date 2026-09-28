//! hub::server — **设备账那一台**：一枚线程守着一本账（这一台机器上有哪些设备、谁在驱它们）。
//!
//! 载体是 rtc / principal / 盟册那几面已经量过的形状——**门牌自带回信孔**：客人替这一趟铸一枚
//! 回信孔借过来、把帧推上门牌，本域从**门牌那一枚**读（发送者由内核在 `Push` 那一刻盖章），
//! 办完事从那枚孔答回去、当场放下。故这里**没有客人账**：一位客人不需要本域记住任何东西
//! （记的是**设备**那一本账）。
//!
//! ```text
//!   起手：收**整机物料**（装配者从 `hub` 那条通道推来的一段记录）
//!         → 开设备树那一页（那一段的第一条就是它）→ 立账（名 / 类 / 线不在这段里，在树里）
//!         → 开树那条会话 → 找盟册的**定面**（`/svc/coalition/set`）
//!         → 逐类立一枚盟（盟册 `found`）→ 铸三枚面 ＋ 落 `/svc/hub/{bond,list,claim}`
//!         → 逐类落 `/dev/<类>/<名>`（`permit = Among(c_类)`——"许驱这一类"那条规矩的落点）
//!   常驻：三枚面 ＋ 每一台那一枚门，一只组等它们——**从哪一枚读到**就是哪一面；
//!         `claim` 那一面**靠"哪一枚孔响了"认台**；钟到就扫一遍账（探活 ⇒ 空出主人没了的格）
//! ```
//!
//! # 三个为什么
//!
//! - **为什么先收物料、后开树**：物料那一段是本域**唯一**的来路（装配者是唯一持引导域那个
//!   泊位的域），而"哪一条是哪一台"只有树说得清 ⇒ 树那一页在物料里、树那条路在物料之后。
//! - **为什么认领那一面长在设备格上**（不在本域会客室里）：见 [`protocol::driver::hub`] 的头注
//!   ——"哪一台"由"你找的是哪一格"回答，请求里因此没有"哪一件"这一格。
//! - **为什么探活要自己扫**：常驻驱动不退场，退场＝死 ⇒ "空出"那一手由本域自己看出来
//!   （主人交来的那枚报活孔答不出）。
//!
//! # 照实记（本域那一圈为什么不用 `system::carrier`）
//!
//! `carrier` 的判据是"**从哪一枚门牌读到就是哪一面**"，而它只等一个来路（那只组）。本域有两个：
//! **那只组**（三面 ＋ 每一台那一枚门）与**那一钟**（探活那一拍）。它自己的照实记里已经写明
//! "持树者那一处不在这里，因为它等的是两个不同来路"——本域加入那张名单，形状照着抄，
//! **另一条多出来的**是"从哪一枚孔读到"要交到门上（`claim` 那一面靠它认台，见文件头）。

use alloc::vec::Vec;

use env::{Access, Key, Kind, MailFail, Mark, Name, Pair, PieToken, Policy, TaskId, Wait};
use protocol::communication::establish;
use protocol::communication::receiver::{Receiver, RecvFail};
use protocol::communication::sender::Sender;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::driver::hub::frame::Wire;
use protocol::driver::hub::frame::{Said, Window};
use protocol::driver::hub::{self, Deed, Enroll, Grant};
use protocol::system::coalition as ccall;
use protocol::system::coalition::client::Face as League;
use protocol::system::operator::client as operator;
use protocol::system::operator::client::Face as TreeFace;
use protocol::system::operator::client::Mine;
use protocol::system::operator::Permit;
use runtime::core::dock::Dock;
use runtime::core::pile::Pile;
use runtime::core::port;
use env::HoleDir;
use protocol::message::Message;
use runtime::env::mail::{self, HolePie, NolePie, PolePie};
use runtime::env::unit as utask;
use runtime::PAGE_SIZE;

use crate::program::hub::{CHANNEL, E_HUB, READY};
use crate::system::control::service::Start;
use crate::system::hub::core::{Entry, Ledger, Owner};
use crate::system::machine::Machine;
use crate::system::mount;
use crate::system::operator::bridge;

/// 等树 / 等盟册 / 收物料的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 收物料那一趟的重试次数（**它是额度不是时限**：装配者要向引导域逐段领，那一段不在一瞬间）。
///
/// **为什么必须重试**（照实记）：本域"起来了"那一手是 `establish::endpoint`（装配者据此放行），
/// 而装配者要等本域起来**之后**才向引导域领那一段 ⇒ 两条边之间必然有一个窗口。
const LOAD_TRIES: usize = 20;

/// 探活那一拍的周期（毫秒）。**它是本域唯一的钟**：`vacate` 只在它响时扫一遍。
const PROBE_MS: usize = 1000;

/// 起服务：**收物料 → 立账 → 上树 → 立盟 → 落格 → 一枚线程招待所有客人**。
///
/// **起手那几步收在一个闭包**（与持树者 / 名册 / 盟册同形）：它们清一色是"不成 ⇒ 这域起不来"
/// 的早退步，失败域在末尾**折一次**。
pub fn serve() -> Result<(), Start> {
    // 一～七：起手。
    let (mut ledger, league, plates, doors, _dtb) = (|| {
        let sire = utask::sire();

        // 一、**整机物料**：这一手（`establish::endpoint`）就同时是"我起来了"那一句——
        //     装配者据此放行，随后才向引导域领那一段（故这里**有界重试**地收）。
        let up = establish::endpoint(sire, Mark::of(CHANNEL), Wait::POLL)
            .map_err(|_| Start::Load(E_HUB))?;
        let enroll = take(up.rx()).ok_or(Start::Load(E_HUB))?;

        // 二、立账：树那一页 ＋ 每一条记录对上一台（名 / 类 / 线随树一起来）。
        //     **树那一页要活到本域退场**（`Machine::of` 的 SAFETY 靠它）——它随返回值交出去。
        let (mut ledger, dtb) = book(&enroll)?;

        // 三、树那条会话。
        let session = Session::open(sire, operator::BERTH, Wait::AtMost(MS))
            .map_err(|_| Start::Tree(E_HUB))?;
        let tree = TreeFace::of(session);

        // 四、盟册的**定面**（立盟 ＋ 代报名都在它上面）。
        let league = find_league(&tree).ok_or(Start::Face(E_HUB))?;

        // 五、**逐类立一枚盟**：那一枚盟号就是 permit 里那句"许驱这一类"的对象（也留给 `bond`
        //     代报名）。**次序是硬的**：permit 要在落格之前就有，故立盟只能在这儿——不是等到
        //     第一位报名的客人来。
        let classes = ledger.classes().ok_or(Start::Tree(E_HUB))?;
        let me = utask::self_id();
        for class in &classes {
            let Ok(coalition) = league.found(Wait::AtMost(MS)) else {
                return Err(Start::Face(E_HUB));
            };
            let id = coalition.id();
            // **本域也在里头**（照实记）：那一格的许可写着 `Among(c_类)`，而落完格那一趟要
            // **查回来验一遍**（`bridge::land` 那一步）——本域不在里头就当场 `Denied`。
            // 语义上也不勉强：本域手里握着这一类每一台那一份（它正是把它们发出去的那位），
            // "许动这一类的人"里当然有它。**它也是这一格的第一次正证**：`admit` 在生产里的
            // 第一位客人不是别的谁，就是立盟那位自己。
            if coalition.admit(me, Wait::AtMost(MS)).is_err() {
                return Err(Start::Face(E_HUB));
            }
            ledger.league(*class, || id);
        }

        // 六、三枚面：本域铸、本域落（`/svc/hub/{bond,list,claim}`）。
        //
        // **`claim` 那一枚是"正本"**（照实记）：认领真正打的是**每一台那一格**上的孔
        // （`/dev/<类>/<名>`），而这一枚是**发现入口与权柄边界**——它响的时候本域答 `Unknown`
        // （这一枚孔不指任何一台，见 [`turn`]）。它照样要落：这一族三面一条路。
        let (bond, bond_name) = mount::entry(Grant::Bond.mark(), Grant::Bond.name())
            .map_err(|_| Start::Tree(E_HUB))?;
        let (list, list_name) = mount::entry(Grant::List.mark(), Grant::List.name())
            .map_err(|_| Start::Tree(E_HUB))?;
        let (claim, claim_name) = mount::entry(Grant::Claim.mark(), Grant::Claim.name())
            .map_err(|_| Start::Tree(E_HUB))?;
        let (Ok(svc), Ok(segment)) = (Name::new(protocol::system::SVC), Name::new(hub::NAME)) else {
            return Err(Start::Tree(E_HUB));
        };
        let plated = bridge::land(
            &tree,
            "hub",
            &[svc.as_str(), segment.as_str()],
            Mine::No,
            Permit::Unset,
            &[
                (bond_name.as_str(), bond),
                (list_name.as_str(), list),
                (claim_name.as_str(), claim),
            ],
            Wait::AtMost(MS),
        );
        if plated.len() != 3 || plated.iter().any(|one| one.land.is_err() || one.find.is_err()) {
            return Err(Start::Tree(E_HUB));
        }

        // 七、**逐类落 `/dev/<类>/<名>`**：每一格带 `Among(c_类)`——"许驱这一类"那条规矩的落点。
        //     名字取自册（`doors`），故"这一类落哪几台"与账是同一份事实。
        let Ok(axis) = Name::new(hub::DEV) else {
            return Err(Start::Tree(E_HUB));
        };
        for class in &classes {
            let Some(coalition) = ledger.coalition_of(*class) else {
                return Err(Start::Tree(E_HUB));
            };
            let doors: Vec<(&str, PieToken)> = ledger
                .doors(*class)
                .map(|(name, door)| (name.as_str(), door))
                .collect();
            let Ok(segment) = Name::new(class.as_str()) else {
                return Err(Start::Tree(E_HUB));
            };
            let plated = bridge::land(
                &tree,
                "hub",
                &[axis.as_str(), segment.as_str()],
                Mine::No,
                Permit::Among(coalition),
                &doors,
                Wait::AtMost(MS),
            );
            if plated.len() != doors.len()
                || plated.iter().any(|one| one.land.is_err() || one.find.is_err())
            {
                return Err(Start::Tree(E_HUB));
            }
            debug!("hub: /dev/{} has {} devices", class.as_str(), doors.len());
        }
        // **本域那一行总读数**：几台、几类（"这台机器上有哪些设备"唯一一次陈述）。
        debug!("hub: {} devices, {} classes", ledger.count(), classes.len());

        // **八、报"我起完了"**（`Setup::Machine` 的 `ready` 那条通道）：铸一枚刻它的孔、**交给
        // 装配者**——它等齐了才往下起别人。**它必须是起手最后一件**：此后本域才真的答得了
        // （设备格都在树上、盟都立好了）。
        //
        // **照实记（"铸一枚"不够，得"交出去"）**：装配者那一侧等的是"**我表里多了一枚你说的
        // 那一枚**"（`Endpoint::claim` 按 `owner + 记号` 两格认）——只在本地铸、不 `ship`，
        // 对面一枚都看不见（实测：等满 5 s 判"没起来"）。故这一手与其余几条通道同一手
        // （[`establish::endpoint`]：铸 ＋ 交 ＋ 顺手认对面那一枚）。
        establish::endpoint(sire, Mark::of(READY), Wait::POLL).map_err(|_| Start::Desk(E_HUB))?;

        // 九、挂组那一张表：三枚面 ＋ 每一台那一枚门（**挂在孔上，面由孔推**）。
        let mut doors: Vec<PieToken> = Vec::new();
        doors
            .try_reserve(3 + ledger.count())
            .map_err(|_| Start::Desk(E_HUB))?;
        doors.extend([bond, list, claim]);
        for class in &classes {
            doors.extend(ledger.doors(*class).map(|(_, door)| door));
        }
        Ok::<_, Start>((ledger, league, (bond, list, claim), doors, dtb))
    })()?;

    // 十、常驻：一只组等那几枚孔；**从哪一枚读到就是哪一面**（设备门那一枚的面由"它不是面那
    //     三枚"推出来，认台靠**这一枚孔自己**）。
    let pile = Pile::unseal(false).map_err(|_| Start::Desk(E_HUB))?;
    for token in &doors {
        pile.attach(&HolePie::from_token(*token), HoleDir::Pull)
            .map_err(|_| Start::Desk(E_HUB))?;
    }
    let mut buf: Vec<u8> = Vec::new();
    if buf.try_reserve_exact(PAGE_SIZE).is_err() {
        return Err(Start::Room(E_HUB));
    }
    buf.resize(PAGE_SIZE, 0);
    loop {
        match pile.await_(Wait::AtMost(PROBE_MS)) {
            Ok(Some((token, _))) => {
                // 门牌是**单槽**：一次醒来的这一批要取干净（可能不止一位客人）。
                let hole = HolePie::from_token(token);
                while let Ok((len, from)) = hole.pull_timeout_from(&mut buf, Wait::POLL) {
                    turn(&mut ledger, &league, plates, token, from, &buf[..len]);
                }
            }
            // **挂起过 / 期限到**：这是常态（没有客人），故它顺带就是探活那一拍。
            Ok(None) => {}
            Err(_) => return Err(Start::Dead(E_HUB)),
        }
        // 探活：主人没了的格当场空出来（读数只在真空出东西时印一行）。
        let freed = ledger.vacate(alive);
        if freed > 0 {
            debug!("hub: vacated {freed}");
        }
    }
}

/// 门上一句话：解帧 → 先认那枚回信孔 → 交给账 → **从这一趟自带的那枚孔答回去**。
///
/// 认那枚孔靠**帧里那一格** ＋ **一次 [`mail::reserve`] 验**（同盟册那一面）；`from` 是**内核
/// 盖的发送者**。`token` = **这一帧从本域哪一枚孔进来**（哪一面、哪一台，全靠它）。
fn turn(
    ledger: &mut Ledger,
    league: &League,
    plates: (PieToken, PieToken, PieToken),
    token: PieToken,
    from: TaskId,
    frame: &[u8],
) {
    let Some((ask, back)) = Wire::take(frame) else {
        // 不是那个形状（长度不对 / 动作码不认得）：不猜、不动账、也不回话——没有可信的"往哪回"。
        return;
    };
    if !matches!(
        mail::reserve(back),
        Ok((_vestor, owner, mark)) if owner == from && mark == hub::BACK_MARK
    ) {

        // 这一趟没把回信孔交进来、或那一格指的是别人的孔：没有可回的路，账一动不动。
        return;
    }
    let mine = face_of(plates, token);
    // 面与码对不上、或这一码我不认：**按那一面那一形**答一句失败（形状必须对得上，回声孔才用得上）。
    let Some(ask) = ask.filter(|ask| Grant::of_wire(ask) == mine.at()) else {
        let _ = send_status(plates, token, mine, hub::DENIED, back);
        return;
    };
    let _ = match (mine, ask) {
        (Grant::Bond, Wire::Bond(class)) => bond(ledger, league, class, from, back),
        (Grant::List, Wire::List(class, from_index)) => list(ledger, class, from_index, back),
        (Grant::Claim, Wire::Claim { kind, access, policy, sensor }) => {
            claim(ledger, token, from, sensor, kind, access, policy, back)
        }
        // 构造上到不了（`of_wire` 那一句已经把面与码对齐过）。
        _ => send_status(plates, token, mine, hub::BAD, back),
    };
    let _ = mail::release(back);
}

/// **报名**：这个类不在册上 ⇒ `Unknown`（这台机器没有这一类）；否则**代报名**——把发送者放进
/// 这一类那枚盟（盟册 `admit`，钥匙 = "你是不是立盟那位"）。
///
/// 驱动不需要知道盟号：它只说"我要驱这一类"。**这一位不是盟主就答 `Denied`**（本域总是盟主，
/// 故这一格在生产里到不了——它是"本域坏了"的读数，不是客人的错）。
fn bond(
    ledger: &mut Ledger,
    league: &League,
    class: Name,
    from: TaskId,
    back: PieToken,
) -> Result<(), ()> {
    let Some(coalition) = ledger.coalition_of(class) else {
        return Sender::<Said>::from_token(back)
            .send(Said::of(hub::UNKNOWN), Wait::Forever)
            .map_err(|_| ());
    };
    let status = match league.coalition(coalition).admit(from, Wait::AtMost(MS)) {
        Ok(()) => hub::OK,
        Err(_) => hub::DENIED,
    };
    Sender::<Said>::from_token(back)
        .send(Said::of(status), Wait::Forever)
        .map_err(|_| ())
}

/// **列册**：这一类此刻有哪几台、哪几台有主（越界答空窗——是答案，不是错误）。
fn list(ledger: &Ledger, class: Name, from: u32, back: PieToken) -> Result<(), ()> {
    let window = if ledger.coalition_of(class).is_some() {
        ledger.list(class, from)
    } else {
        Window {
            status: hub::UNKNOWN,
            ..Window::EMPTY
        }
    };
    Sender::<Window>::from_token(back)
        .send(window, Wait::Forever)
        .map_err(|_| ())
}

/// **认领**：那一台由"哪一枚孔响了"回答（`door`）；主人是发送者 ＋ 它交来的那枚报活孔。
fn claim(
    ledger: &mut Ledger,
    door: PieToken,
    from: TaskId,
    sensor: PieToken,
    kind: u8,
    access: u32,
    policy: u32,
    back: PieToken,
) -> Result<(), ()> {
    let deed = {
        let (Some(kind), Some(access), Some(policy)) = (
            Kind::of(kind),
            Access::from_bits(access),
            Policy::from_bits(policy),
        ) else {
            return Sender::<Deed>::from_token(back)
                .send(Deed::of(hub::BAD), Wait::Forever)
                .map_err(|_| ());
        };
        let owner = Owner { task: from, sensor };
        match ledger.claim(door, owner, alive) {
            Ok(entry) => match ship(entry, from, kind, access, policy) {
                Ok(page) => Deed::granted(entry.name, entry.line, page),
                // 授不出 ⇒ `Denied`（"装配错"那一格）。**账照记**：认领者拿到这一格就退场，
                // 它一死探活那一拍把这一格空出来（照实记，见 `hub::core::Ledger::claim`）。
                Err(_) => Deed::of(hub::DENIED),
            },
            Err(hub::Fail::Taken) => Deed::of(hub::TAKEN),
            Err(_) => Deed::of(hub::UNKNOWN),
        }
    };
    Sender::<Deed>::from_token(back)
        .send(deed, Wait::Forever)
        .map_err(|_| ())
}

/// 按那一面的那一形答一句状态（面与码对不上、或这一码我不认时用）。
fn send_status(
    _plates: (PieToken, PieToken, PieToken),
    _token: PieToken,
    mine: Grant,
    status: u8,
    back: PieToken,
) -> Result<(), ()> {
    match mine {
        Grant::Bond => Sender::<Said>::from_token(back)
            .send(Said::of(status), Wait::Forever)
            .map_err(|_| ()),
        Grant::List => Sender::<Window>::from_token(back)
            .send(
                Window {
                    status,
                    ..Window::EMPTY
                },
                Wait::Forever,
            )
            .map_err(|_| ()),
        Grant::Claim => Sender::<Deed>::from_token(back)
            .send(Deed::of(status), Wait::Forever)
            .map_err(|_| ()),
    }
}

/// **这一帧从哪一枚孔进来**：三枚面各是各的，**其余的孔都是某一台那一枚门**。
fn face_of(plates: (PieToken, PieToken, PieToken), token: PieToken) -> Grant {
    let (bond, list, claim) = plates;
    if token == bond {
        Grant::Bond
    } else if token == list {
        Grant::List
    } else if token == claim {
        Grant::Claim
    } else {
        // 每一台那一枚门的面**也是 `Claim`**——而"认的是哪一台"靠这一枚孔自己（见 [`turn`]）。
        Grant::Claim
    }
}

/// **授出那一手**：把那台设备那一页交一份给认领者，返**在它表里**的号。
///
/// 形态**照客人要的**：本域不替它挑（内核校验"`ONLY` 与源枚一致"）。故"这台能不能独占、
/// 给不给读写"这两件事的判据只有一处——客人那一格 ＋ 内核那一格。
fn ship(
    entry: Entry,
    to: TaskId,
    kind: Kind,
    access: Access,
    policy: Policy,
) -> Result<PieToken, ()> {
    let shipped = match kind {
        Kind::Pole => port::ship(&PolePie::from_token(entry.page), to, access, policy),
        Kind::Nole => port::ship(&NolePie::from_token(entry.page), to, access, policy),
    };
    shipped.map(|seat| seat.seed()).map_err(|_| ())
}

/// **"这一枚的主人还在吗"**——只有一条判据：那一枚还在本域表里。
///
/// 主人一死，内核把它的派生边（交给本域的那一份副本）一起摘掉 ⇒ `reserve` 答不出就是"没了"。
/// 与线路由者那条探活同一手。
fn alive(sensor: PieToken) -> bool {
    mail::reserve(sensor).is_ok()
}

/// 收物料那一趟：**有界重试**地从那条通道上取一段。
///
/// **三格失败分得开**（[`RecvFail`]）：没收到 ⇒ 再试（装配者还在向引导域领）；那一枚孔用不动
/// 了 ⇒ 收摊；解不动 ⇒ 真就是"这一段读不懂"（与"没收到"同一落点：这一台起不来）。
fn take(rx: PieToken) -> Option<Enroll> {
    let receiver = Receiver::<Enroll>::from_token(rx);
    let mut buf = Enroll::EMPTY;
    let mut left = LOAD_TRIES;
    loop {
        match receiver.recv(buf.as_mut(), Wait::AtMost(MS)) {
            Ok(enroll) => return Some(enroll),
            Err(RecvFail::Mail(e)) if !matches!(e, MailFail::Dead | MailFail::Denied) => {
                left = left.checked_sub(1)?;
            }
            Err(_) => return None,
        }
    }
}

/// 把装配者推来那一段读成一本账：**树那一页 ＋ 每一条记录对上一台**。
///
/// 两步都是硬的：
///
/// 1. **先开设备树那一页**（段里的 `Key::dtb()` 那一条）：名 / 类 / 线只有树说得清，而本域要落
///    的格是"名 / 类"两格 ⇒ 没有树就一台都落不下去；
/// 2. **逐条对**：段里那几条记录给的是**坐标 ＋ 号**，而"这一条是哪一台"由坐标对树
///    （[`Machine::devices`] 那张表就是那个对照）。
///
/// 那两件**不按类认**的东西（设备树本体 / 门铃）在段里也各占一条：它们的类不是树给的，故本域
/// 按坐标认出来、按 [`hub::BOOT`] 那一类入册（`/dev/boot/{dtb,irq}`）——于是"取法"只有一条
/// （认领那一套原样用），而"哪一类"那一格也有了诚实的答案。
fn book(enroll: &Enroll) -> Result<(Ledger, Dock), Start> {
    let mut ledger = Ledger::new();
    // 一、树那一页（**留着不掉**：`Machine::of` 借的就是它映射进来的那段字节）。
    let Some(dtb) = record(enroll, Key::dtb()) else {
        return Err(Start::Load(E_HUB));
    };
    let dock = Dock::open(PolePie::from_token(dtb.token())).map_err(|_| Start::Load(E_HUB))?;
    let machine = Machine::of(dock.view()).map_err(|_| Start::Load(E_HUB))?;
    let devices = machine.devices().ok_or(Start::Load(E_HUB))?;
    // 二、逐条对：坐标 → 那一台（**对不上的跳过**：装配者按同一张表枚举，对不上说明那一条
    //     不是本域读得懂的那一条——那一台不入册，别的照入）。
    for i in 0..enroll.len() {
        let Some(pair) = enroll.record(i) else {
            break;
        };
        let Some(key) = pair.key() else {
            continue;
        };
        let (name, class, line) = match key {
            k if k == Key::dtb() => (hub::DTB, hub::BOOT, 0),
            k if k == Key::irq() => (hub::IRQ, hub::BOOT, 0),
            _ => match devices.iter().find(|d| d.key == key) {
                Some(device) => (device.name.as_str(), device.class.as_str(), device.line),
                None => continue,
            },
        };
        let (Ok(name), Ok(class)) = (Name::new(name), Name::new(class)) else {
            continue;
        };
        // **每一台铸一枚孔**：那一枚此后就挂在那一格上（"哪一台"由"哪一枚孔响了"回答）。
        // 门与页是同一个词的两面：`page` = 引导域交来那一份（认领时授出去），
        // `door` = 本域为这一台铸的那一枚（落在 `/dev/<类>/<名>` 上）。
        let door = mail::unseal_hole(Grant::Claim.mark()).map_err(|_| Start::Load(E_HUB))?;
        ledger
            .enroll(Entry {
                name,
                class,
                line,
                page: pair.token(),
                door,
            })
            .map_err(|()| Start::Load(E_HUB))?;
    }
    Ok((ledger, dock))
}

/// 段里按坐标取那一条（照单取源那一套的读侧：坐标唯一）。
fn record(enroll: &Enroll, key: Key) -> Option<Pair> {
    (0..enroll.len())
        .filter_map(|i| enroll.record(i))
        .find(|pair| pair.key() == Some(key))
}

/// 找**盟册的定面**（`/svc/coalition/set`——立盟与代报名都在它上面）：`None` = 没找着。
///
/// **带重试**：盟册排在前面，但"先起"与"上树"不是同一步。那一趟（译号带重试 ＋ 取那一枚）在
/// [`TreeFace::tile`] 上。
fn find_league(tree: &TreeFace) -> Option<League> {
    let (Ok(dir), Ok(segment), Ok(leaf)) = (
        Name::new(ccall::DIR),
        Name::new(ccall::NAME),
        Name::new(ccall::Grant::Set.name()),
    ) else {
        return None;
    };
    let door = tree
        .tile(&[dir, segment, leaf], Wait::AtMost(MS))
        .ok()?
        .token(Wait::AtMost(MS))
        .ok()?;
    League::of(door).ok()
}
