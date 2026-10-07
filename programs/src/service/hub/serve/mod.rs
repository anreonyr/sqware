//! 一枚线程守着一本账（这一台机器上有哪些设备、谁在驱它们）。
//! （记的是**设备**那一本账）。
//! # 三个为什么

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use env::{HoleDir, Access, Name, MailFail, Mark, Entry as ResourceEntry, PieKind, PieToken, Policy, TaskId, Wait};
use ipc::hand::{Sender, Receiver, RecvFail};
use ipc::session::{Session, establish};
use protocol::debug;
use protocol::service::hub::{Wire, Said, Window, self, Deed, Enroll, Grant};
use protocol::system::identity::{Organization, Query, CoalitionId, PageId as _, Selector};
use protocol::system::operator::{Permit, Face as Face};
use protocol::system::operator::client as operator;
use protocol::wire::message::Message;
use env::PAGE_SIZE;
use ::resource::dock::Dock;
use ::resource::pile::Pile;
use ::resource::port;
use env::unit;

use crate::service::hub::core::{Entry, Ledger, Owner};
use crate::system::common::face::mount;
use crate::system::common::machine::Machine;
use crate::system::control::serve::task::Start;
use crate::unit::hub::{CHANNEL, E_HUB, READY};
use protocol::system::control::Scope;

use self::sweep::alive;
use env::pie;
use ::resource::raw::{Hole, reserve};

const MS: usize = 1000;

const LOAD_TRIES: usize = 20;

const PROBE_MS: usize = 1000;

pub mod bond;
pub mod claim;
pub mod list;
pub mod sweep;

struct League {
    query: Query,
    organization: Organization,
}

/// 起服务：**收物料 → 立账 → 上树 → 立盟 → 落格 → 一枚线程招待所有客人**
/// **起手那几步收在一个闭包**（与持树者 / 名册 / 盟册同形）：它们清一色是"不成 ⇒ 这域起不来"
/// 的早退步，失败域在末尾**折一次**
pub fn serve() -> Result<(), Start> {
    protocol::debug::put("hub: serve enter");
    // 一～七：起手。
    let (mut ledger, league, plates, doors, _dtb) = (|| {
        let sire = unit::sire();

        let up = establish::endpoint(sire, Mark::of(CHANNEL), Wait::POLL)
            .map_err(|_| Start::Load(E_HUB))?;
        let enroll = take(up.rx()).ok_or(Start::Load(E_HUB))?;

        // 二、立账：树那一页 ＋ 每一条记录对上一台（名 / 类 / 线随树一起来）。
        let (mut ledger, dtb) = book(&enroll)?;

        // 三、树那条会话。
        let session = Session::open(sire, operator::BERTH, Wait::AtMost(MS))
            .map_err(|_| Start::Tree(E_HUB))?;
        let tree = Face::of(session);

        // 四、盟册的**定面**（立盟 ＋ 代报名都在它上面）。
        let league = find_league(&tree).ok_or(Start::Face(E_HUB))?;

        // 五、**逐类立一枚盟**：那一枚盟号就是 permit 里那句"许驱这一类"的对象（也留给 `bond`
        let classes = ledger.classes().ok_or(Start::Tree(E_HUB))?;
        let me = unit::self_id();
        let subject = league
            .query
            .resolve(me, Wait::AtMost(MS))
            .map_err(|_| Start::Face(E_HUB))?
            .ok_or(Start::Face(E_HUB))?
            .current
            .principal;
        // **先立齐，再一趟报**（原来是每类一趟 `activate`：十几类就是十几趟同步往返，
        // 全压在"报就绪"之前——那一族见 `kernel/src/layout.rs` 头注）。枚数由
        // `ACTIVATE_MAX` 把关，装不下就是装配错、当场收手。
        let mut leagues = [CoalitionId::EMPTY; hub::activation::ACTIVATE_MAX];
        let mut n = 0usize;
        for class in &classes {
            let Ok(id) = league.organization.found(Wait::AtMost(MS)) else {
                return Err(Start::Face(E_HUB));
            };
            if league
                .organization
                .admit(id, subject, Wait::AtMost(MS))
                .is_err()
            {
                return Err(Start::Face(E_HUB));
            }
            let Some(slot) = leagues.get_mut(n) else {
                return Err(Start::Face(E_HUB));
            };
            *slot = id;
            n += 1;
            ledger.league(class.clone(), || id);
        }
        crate::service::hub::bridge::activate(me, &leagues[..n]).map_err(|_| Start::Face(E_HUB))?;

        let (bond, bond_name) =
            mount::entry(Grant::Bond.mark(), Grant::Bond.name()).map_err(|_| Start::Tree(E_HUB))?;
        let (list, list_name) =
            mount::entry(Grant::List.mark(), Grant::List.name()).map_err(|_| Start::Tree(E_HUB))?;
        let (claim, claim_name) = mount::entry(Grant::Claim.mark(), Grant::Claim.name())
            .map_err(|_| Start::Tree(E_HUB))?;
        // 本族那一族的路：容器那一段（`/svc`）接上本族那一段（`hub`）——一处都不自己拼。
        let publisher = protocol::system::control::publication::Client::injected()
            .map_err(|_| Start::Tree(E_HUB))?;
        for (name, entry) in [
            (bond_name.as_str(), bond),
            (list_name.as_str(), list),
            (claim_name.as_str(), claim),
        ] {
            let target = protocol::system::control::publication::Target::Service {
                scope: Scope::Hub,
                group: "".into(),
                name: name.into(),
            };
            publisher
                .publish(target, entry, Permit::Public, Wait::AtMost(MS))
                .map_err(|_| Start::Tree(E_HUB))?;
        }

        // 七、**逐类落 `/dev/<类>/<名>`**：每一格带 MemberOf(c_类)，检查有效选择而不是资格。
        //     名字取自册（`doors`），故"这一类落哪几台"与账是同一份事实。
        for class in &classes {
            let Some(coalition) = ledger.coalition_of(class.clone()) else {
                return Err(Start::Tree(E_HUB));
            };
            let doors: Vec<(&str, PieToken)> = ledger
                .doors(class.clone())
                .map(|(name, door)| (name.as_str(), door))
                .collect();
            for (name, entry) in doors {
                let target = protocol::system::control::publication::Target::Service {
                    scope: Scope::Device,
                    group: class.clone(),
                    name: name.into(),
                };
                publisher
                    .publish(
                        target,
                        entry,
                        Permit::Identity(Selector::MemberOf(coalition)),
                        Wait::AtMost(MS),
                    )
                    .map_err(|_| Start::Tree(E_HUB))?;
            }
        }
        debug!("hub: {} devices, {} classes", ledger.count(), classes.len());

        // **八、报"我起完了"**（Setup::Machine 的 `ready` 那条通道）：铸一枚刻它的孔、**交给
        // （设备格都在树上、盟都立好了）。
        establish::endpoint(sire, Mark::of(READY), Wait::POLL).map_err(|_| Start::Desk(E_HUB))?;

        // 九、挂组那一张表：三枚面 ＋ 每一台那一枚门（**挂在孔上，面由孔推**）。
        let mut doors: Vec<PieToken> = Vec::new();
        doors
            .try_reserve(3 + ledger.count())
            .map_err(|_| Start::Desk(E_HUB))?;
        doors.extend([bond, list, claim]);
        for class in &classes {
            doors.extend(ledger.doors(class.clone()).map(|(_, door)| door));
        }
        Ok::<_, Start>((ledger, league, (bond, list, claim), doors, dtb))
    })()?;

    // 十、常驻：一只组等那几枚孔；**从哪一枚读到就是哪一面**（设备门那一枚的面由"它不是面那
    //     三枚"推出来，认台靠**这一枚孔自己**）。
    let pile = Pile::unseal(false).map_err(|_| Start::Desk(E_HUB))?;
    for token in &doors {
        pile.attach(*token, HoleDir::Pull)
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
                let hole = Hole::from_raw(token);
                while let Ok((len, from)) = hole.pull(&mut buf, Wait::POLL) {
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
        reserve(back),
        Ok((_vestor, owner, mark)) if owner == from && mark == hub::BACK_MARK
    ) {
        return;
    }
    let mine = face_of(plates, token);
    let Some(ask) = ask.filter(|ask| Grant::of_wire(ask) == mine.at()) else {
        send_status(mine, hub::DENIED, back);
        return;
    };
    match (mine, ask) {
        (Grant::Bond, Wire::Bond(class)) => bond::bond(ledger, league, class, from, back),
        (Grant::List, Wire::List(class, from_index)) => list::list(ledger, class, from_index, back),
        (
            Grant::Claim,
            Wire::Claim {
                kind,
                access,
                policy,
                sensor,
            },
        ) => claim::claim(ledger, token, from, sensor, kind, access, policy, back),
        // 构造上到不了（`of_wire` 那一句已经把面与码对齐过）。
        _ => send_status(mine, hub::BAD, back),
    }
    let _ = pie::release(back);
}

pub(super) fn put_said(back: PieToken, status: u8) {
    let mut tx = Sender::<Said>::from_raw(back);
    let _ = tx.send(Said::of(status));
}

/// 递一句 `Deed`（同上）
pub(super) fn put_deed(back: PieToken, deed: Deed) {
    let mut tx = Sender::<Deed>::from_raw(back);
    let _ = tx.send(deed);
}

pub(super) fn send_status(mine: Grant, status: u8, back: PieToken) {
    match mine {
        Grant::Bond => put_said(back, status),
        Grant::List => {
            let mut tx = Sender::<Window>::from_raw(back);
            let _ = tx.send(Window {
                status,
                ..Window::EMPTY
            });
        }
        Grant::Claim => put_deed(back, Deed::of(status)),
    }
}

/// **这一帧从哪一枚孔进来**：三枚面各是各的，**其余的孔都是某一台那一枚门**
pub(super) fn face_of(plates: (PieToken, PieToken, PieToken), token: PieToken) -> Grant {
    let (bond, list, claim) = plates;
    if token == bond {
        Grant::Bond
    } else if token == list {
        Grant::List
    } else if token == claim {
        Grant::Claim
    } else {
        // 每一台那一枚门的面**也是 `Claim`**——而"认的是哪一台"靠这一枚孔自己（见 turn）。
        Grant::Claim
    }
}

/// **授出那一手**：把那台设备那一页交一份给认领者，返**在它表里**的号
/// 给不给读写"这两件事的判据只有一处——客人那一格 ＋ 内核那一格
pub(super) fn ship(
    entry: Entry,
    to: TaskId,
    kind: PieKind,
    access: Access,
    policy: Policy,
) -> Result<PieToken, ()> {
    let shipped = match kind {
        PieKind::Pole => port::ship(entry.page, to, access, policy),
        PieKind::Nole => port::ship(entry.page, to, access, policy),
        PieKind::Hole | PieKind::Tole => return Err(()),
    };
    shipped.map(|seat| seat.seed()).map_err(|_| ())
}

/// **三格失败分得开**（RecvFail）：没收到 ⇒ 再试（装配者还在授出）；那一枚孔用不动
fn take(rx: PieToken) -> Option<Enroll> {
    let receiver = Receiver::<Enroll>::from_raw(rx);
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

/// 把装配者推来那一段读成一本账：**树那一页 ＋ 每一条记录对上一台**
/// 两步都是硬的
/// 的格是"名 / 类"两格 ⇒ 没有树就一台都落不下去
/// 2. **逐条对**：段里那几条记录给的是**坐标 ＋ 号**，而"这一条是哪一台"由坐标对树
/// （Machine::devices 那张表就是那个对照）
/// 按坐标认出来、按 hub::BOOT 那一类入册（`/dev/boot/{dtb,supervisor_external}`）——于是"取法"只有一条
/// （认领那一套原样用），而"哪一类"那一格也有了诚实的答案
fn book(enroll: &Enroll) -> Result<(Ledger, Dock), Start> {
    let mut ledger = Ledger::new();
    // 一、树那一页（**留着不掉**：Machine::of 借的就是它映射进来的那段字节）。
    let Some(dtb) = record(enroll, Name::Page(env::Page::Dtb)) else {
        return Err(Start::Load(E_HUB));
    };
    let dock = Dock::open(dtb.token()).map_err(|_| Start::Load(E_HUB))?;
    let machine = Machine::of(dock.view()).map_err(|_| Start::Load(E_HUB))?;
    let devices = machine.devices().ok_or(Start::Load(E_HUB))?;
    // 二、逐条对：坐标 → 那一台（**对不上的跳过**：装配者按同一张表枚举，对不上说明那一条
    for i in 0..enroll.len() {
        let Some(pair) = enroll.record(i) else {
            break;
        };
        let key = pair.name();
        let (name, class, line) = match key {
            k if k == Name::Page(env::Page::Dtb) => (hub::DTB, hub::BOOT, 0),
            k if k == Name::Trap(env::Trap::SupervisorExternal) => (hub::SUPERVISOR_EXTERNAL, hub::BOOT, 0),
            _ => match devices.iter().find(|d| d.resource == key) {
                Some(device) => (device.name.as_str(), device.class.as_str(), device.line),
                None => continue,
            },
        };
        let (name, class) = (name.to_string(), class.to_string());
        // **每一台铸一枚孔**：那一枚此后就挂在那一格上（"哪一台"由"哪一枚孔响了"回答）。
        // 门与页是同一个词的两面：`page` = 装配者交来那一份（认领时授出去），
        let door = pie::unseal_hole(Grant::Claim.mark()).map_err(|_| Start::Load(E_HUB))?;
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

/// 段里按坐标取那一条（照单取源那一套的读侧：坐标唯一）
fn record(enroll: &Enroll, key: Name) -> Option<ResourceEntry> {
    (0..enroll.len())
        .filter_map(|i| enroll.record(i))
        .find(|pair| pair.name() == key)
}

/// Discover Identity through the authority anchor issued directly by Control.
fn find_league(tree: &Face) -> Option<League> {
    let authority = crate::system::identity::client::source::authority()?;
    Some(League {
        query: Query::discover(tree, authority, Wait::AtMost(MS)).ok()?,
        organization: Organization::discover(tree, authority, Wait::AtMost(MS)).ok()?,
    })
}
