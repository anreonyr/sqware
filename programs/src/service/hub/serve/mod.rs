//! 一枚线程守着一本账（这一台机器上有哪些设备、谁在驱它们）。
//! （记的是**设备**那一本账）。
//! # 三个为什么

use alloc::string::String;
use alloc::vec::Vec;

use ::resource::dock::Dock;
use ::resource::pile::Pile;
use env::PAGE_SIZE;
use env::unit;
use env::{Access, HoleCondition, Name, Page, PieKind, PieToken, Policy, TaskId, Trap, Wait};
use hub_api::{self as hub, Deed, Grant, Inventory, Said, Window, Wire};
use ipc::hand::Sender;
use ipc::session::{Session, establish};
use programs::debug;
use system_api::identity::CoalitionId;
use system_api::identity::PageId as _;
use system_api::identity::Selector;
use system_api::operator::{self as operator_api, Permit};
use system_client::identity::Organization;
use system_client::identity::Query;
use system_client::operator;
use system_client::operator::Face;
use wire::Message;

use super::Start;
use crate::service::hub::core::{Entry, Ledger, Owner};
use crate::support::machine::Machine;
use crate::unit::hub::{E_HUB, INVENTORY_MARK};

use self::sweep::alive;
use ::resource::raw::{Hole, reserve};
use env::pie;

const MS: usize = 1000;

const PROBE_MS: usize = 1000;

pub mod bond;
pub mod claim;
mod grant;
use self::grant::ship;
pub mod list;
pub mod sweep;

struct League {
    query: Query,
    organization: Organization,
    activation: system_client::identity::Face,
}

/// 起服务：**收物料 → 立账 → 上树 → 立盟 → 落格 → 一枚线程招待所有客人**
/// **起手那几步收在一个闭包**（与持树者 / 名册 / 盟册同形）：它们清一色是"不成 ⇒ 这域起不来"
/// 的早退步，失败域在末尾**折一次**
pub fn serve() -> Result<(), Start> {
    programs::debug::put("hub: serve enter");
    // 一～七：起手。
    let (mut ledger, league, plates, doors, _dtb) = (|| {
        let sire = unit::sire();

        let inventory = receive_inventory(sire).map_err(|why| {
            crate::debug::put("hub: inventory descriptor failed");
            why
        })?;
        crate::debug::put("hub: inventory received");

        // 二、通过一个只读清单和按序直接交付的本地资源槽立账。
        let (mut ledger, dtb) = book(sire, inventory).map_err(|why| {
            crate::debug::put("hub: inventory ledger failed");
            why
        })?;

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
        // Identity 的 `MAX_ACTIVE_COALITIONS` 把关，装不下就是装配错、当场收手。
        let mut leagues = [CoalitionId::EMPTY; system_api::identity::limits::MAX_ACTIVE_COALITIONS];
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
        hub_client::activate(&league.activation, me, &leagues[..n])
            .map_err(|_| Start::Face(E_HUB))?;

        let bond = env::hole::unseal(Grant::Bond.mark(), env::HoleLimits::default())
            .map_err(|_| Start::Tree(E_HUB))?;
        let list = env::hole::unseal(Grant::List.mark(), env::HoleLimits::default())
            .map_err(|_| Start::Tree(E_HUB))?;
        let claim = env::hole::unseal(Grant::Claim.mark(), env::HoleLimits::default())
            .map_err(|_| Start::Tree(E_HUB))?;
        let publisher = system_client::operator::Capability::injected(0, sire, Wait::AtMost(MS))
            .map_err(|_| Start::Tree(E_HUB))?;
        let service_anchor = publisher.anchor().map_err(|_| Start::Tree(E_HUB))?;
        let endpoint_view =
            operator_api::ResourceView::new(env::Access::STORE).ok_or(Start::Tree(E_HUB))?;
        for (index, (name, entry)) in [
            (Grant::Bond.name(), bond),
            (Grant::List.name(), list),
            (Grant::Claim.name(), claim),
        ]
        .into_iter()
        .enumerate()
        {
            publisher
                .bind(
                    service_anchor,
                    name,
                    entry,
                    endpoint_view,
                    Permit::Public,
                    operator_api::OperationId(index as u64 + 1),
                    Wait::AtMost(MS),
                )
                .map_err(|_| Start::Tree(E_HUB))?;
        }
        let devices = system_client::operator::Capability::injected(1, sire, Wait::AtMost(MS))
            .map_err(|_| Start::Tree(E_HUB))?;
        let device_anchor = devices.anchor().map_err(|_| Start::Tree(E_HUB))?;
        let mut operation = 1u64;

        // Device entries are bound beneath the actual class Entry in Hub's device namespace.
        for class in &classes {
            let Some(coalition) = ledger.coalition_of(class.clone()) else {
                return Err(Start::Tree(E_HUB));
            };
            let group = devices
                .part(
                    device_anchor,
                    class,
                    operator_api::OperationId(operation),
                    Wait::AtMost(MS),
                )
                .map_err(|_| Start::Tree(E_HUB))?
                .entry;
            operation = operation.checked_add(1).ok_or(Start::Tree(E_HUB))?;
            let doors: Vec<(&str, PieToken)> = ledger
                .doors(class.clone())
                .map(|(name, door)| (name.as_str(), door))
                .collect();
            for (name, entry) in doors {
                devices
                    .bind(
                        group,
                        name,
                        entry,
                        endpoint_view,
                        Permit::Identity(Selector::MemberOf(coalition)),
                        operator_api::OperationId(operation),
                        Wait::AtMost(MS),
                    )
                    .map_err(|_| Start::Tree(E_HUB))?;
                operation = operation.checked_add(1).ok_or(Start::Tree(E_HUB))?;
            }
        }
        debug!("hub: {} devices, {} classes", ledger.count(), classes.len());

        // **八、报"我起完了"**（Setup::Machine 的 `ready` 那条通道）：铸一枚刻它的孔、**交给
        // （设备格都在树上、盟都立好了）。
        establish::lend(sire, crate::unit::hub::READY_MARK).map_err(|why| {
            crate::debug::put(&alloc::format!("hub: ready channel {why:?}"));
            Start::Desk(E_HUB)
        })?;

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
    let pile = Pile::unseal(false).map_err(|why| {
        crate::debug::put(&alloc::format!("hub: pile creation {why:?}"));
        Start::Desk(E_HUB)
    })?;
    for token in &doors {
        pile.attach(env::ToleEventSource::Hole {
            token: *token,
            condition: HoleCondition::Pull,
        })
        .map_err(|why| {
            crate::debug::put(&alloc::format!("hub: attach {} {why:?}", token.get()));
            Start::Desk(E_HUB)
        })?;
    }
    let mut buf: Vec<u8> = Vec::new();
    if buf.try_reserve_exact(PAGE_SIZE).is_err() {
        return Err(Start::Room(E_HUB));
    }
    buf.resize(PAGE_SIZE, 0);
    loop {
        match pile.await_(Wait::AtMost(PROBE_MS)) {
            Ok(env::ToleAwaitReply::Source {
                source: env::ToleEventSource::Hole { token, .. },
                ..
            }) => {
                // 门牌是**单槽**：一次醒来的这一批要取干净（可能不止一位客人）。
                let hole = Hole::from_raw(token);
                while let Ok((len, from)) = hole.pull(&mut buf, Wait::POLL) {
                    turn(&mut ledger, &league, plates, token, from, &buf[..len]);
                }
            }
            // **挂起过 / 期限到**：这是常态（没有客人），故它顺带就是探活那一拍。
            Ok(_) => {}
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
    let _ = pie::release(back, env::ReleaseMode::Revoke);
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

fn receive_inventory(system: TaskId) -> Result<Inventory, Start> {
    let descriptor = establish::claim(
        system,
        crate::unit::descriptor_mark(INVENTORY_MARK),
        Wait::AtMost(MS),
    )
    .map_err(|_| Start::Load(E_HUB))?;
    let mut bytes = crate::unit::SupplyFrame::EMPTY;
    let (len, sender) = Hole::from_raw(descriptor)
        .pull(&mut bytes, Wait::AtMost(MS))
        .map_err(|_| Start::Load(E_HUB))?;
    if sender != system {
        return Err(Start::Load(E_HUB));
    }
    let frame = crate::unit::SupplyFrame::fetch(&bytes[..len]).ok_or(Start::Load(E_HUB))?;
    let description = &frame.description[..usize::from(frame.length)];
    let count = usize::from(
        *description
            .first()
            .filter(|_| description.len() == 1)
            .ok_or(Start::Load(E_HUB))?,
    );
    Inventory::header(count).ok_or(Start::Load(E_HUB))?;
    let mark = INVENTORY_MARK;
    let manifest = resource::raw::pies()
        .find(|info| {
            info.alive
                && info.kind == PieKind::Pole
                && info.owner == system
                && info.vestor == system
                && info.mark == mark
                && info.permission.contains(env::Permission::FETCH)
        })
        .map(|info| info.token)
        .ok_or(Start::Load(E_HUB))?;
    let dock = Dock::open(manifest).map_err(|_| Start::Load(E_HUB))?;
    let view = dock.view();
    let manifest_len = Inventory::encoded_len_for(count).ok_or(Start::Load(E_HUB))?;
    let actual = if view.size() < manifest_len {
        None
    } else {
        let contents =
            unsafe { core::slice::from_raw_parts(view.base() as *const u8, manifest_len) };
        Inventory::fetch(contents)
    };
    dock.shut().map_err(|_| Start::Load(E_HUB))?;
    let inventory = actual.ok_or(Start::Load(E_HUB))?;
    if inventory.len() != count {
        return Err(Start::Load(E_HUB));
    }
    Ok(inventory)
}

/// Build the machine ledger from the read-only inventory and target-local lease slots.
fn book(system: TaskId, inventory: Inventory) -> Result<(Ledger, Dock), Start> {
    let mut ledger = Ledger::new();
    if inventory.len() < 2 {
        return Err(Start::Load(E_HUB));
    }
    let dtb_item = inventory.item(0).ok_or(Start::Load(E_HUB))?;
    if dtb_item.resource != Name::Page(Page::Dtb) || dtb_item.kind != PieKind::Pole {
        return Err(Start::Load(E_HUB));
    }
    let dtb = local_machine_token(system, dtb_item, 0)?;
    let dock = Dock::open(dtb).map_err(|_| Start::Load(E_HUB))?;
    let machine = Machine::of(dock.view()).map_err(|_| Start::Load(E_HUB))?;
    let devices = machine.devices().ok_or(Start::Load(E_HUB))?;
    if inventory.len() != devices.len() + 2 {
        return Err(Start::Load(E_HUB));
    }
    enroll(
        &mut ledger,
        Entry {
            name: hub::DTB.into(),
            class: hub::BOOT.into(),
            line: 0,
            page: dtb,
            door: env::hole::unseal(Grant::Claim.mark(), env::HoleLimits::default())
                .map_err(|_| Start::Load(E_HUB))?,
        },
    )?;
    let interrupt_item = inventory.item(1).ok_or(Start::Load(E_HUB))?;
    if interrupt_item.resource != Name::Trap(Trap::SupervisorExternal)
        || interrupt_item.kind != PieKind::Nole
    {
        return Err(Start::Load(E_HUB));
    }
    let interrupt = local_machine_token(system, interrupt_item, 1)?;
    enroll(
        &mut ledger,
        Entry {
            name: hub::SUPERVISOR_EXTERNAL.into(),
            class: hub::BOOT.into(),
            line: 0,
            page: interrupt,
            door: env::hole::unseal(Grant::Claim.mark(), env::HoleLimits::default())
                .map_err(|_| Start::Load(E_HUB))?,
        },
    )?;
    for (index, device) in devices.iter().enumerate() {
        let item_index = index + 2;
        let item = inventory.item(item_index).ok_or(Start::Load(E_HUB))?;
        if item.resource != device.resource || item.kind != PieKind::Pole {
            return Err(Start::Load(E_HUB));
        }
        let token = local_machine_token(system, item, item_index)?;
        let door = env::hole::unseal(Grant::Claim.mark(), env::HoleLimits::default())
            .map_err(|_| Start::Load(E_HUB))?;
        enroll(
            &mut ledger,
            Entry {
                name: device.name.clone(),
                class: device.class.clone(),
                line: device.line,
                page: token,
                door,
            },
        )?;
    }
    Ok((ledger, dock))
}

fn local_machine_token(
    system: TaskId,
    item: hub::InventoryItem,
    index: usize,
) -> Result<PieToken, Start> {
    let slot = hub::inventory_slot(index).ok_or(Start::Load(E_HUB))?;
    let found = resource::raw::pies()
        .find(|info| {
            info.alive
                && info.kind == item.kind
                && info.owner == item.owner
                && info.vestor == system
                && info.mark == slot
                && info.permission == item.permission
                && info
                    .permission
                    .contains(env::Permission::FETCH | env::Permission::VEST)
        })
        .map(|info| info.token);
    if found.is_none() {
        crate::debug::put(&alloc::format!(
            "hub: inventory slot {index} absent expected owner={} kind={:?} permission={:?}",
            item.owner.get(),
            item.kind,
            item.permission
        ));
        for info in resource::raw::pies().filter(|info| info.mark == slot) {
            crate::debug::put(&alloc::format!(
                "hub: slot actual owner={} vestor={} kind={:?} permission={:?} alive={}",
                info.owner.get(),
                info.vestor.get(),
                info.kind,
                info.permission,
                info.alive
            ));
        }
    }
    found.ok_or(Start::Load(E_HUB))
}

fn enroll(ledger: &mut Ledger, entry: Entry) -> Result<(), Start> {
    ledger.enroll(entry).map_err(|()| Start::Load(E_HUB))
}

/// Discover Identity through the authority anchor issued directly by Control.
fn find_league(tree: &Face) -> Option<League> {
    let authority = system_client::identity::authority()?;
    Some(League {
        query: Query::discover(tree, authority, Wait::AtMost(MS)).ok()?,
        organization: Organization::discover(tree, authority, Wait::AtMost(MS)).ok()?,
        activation: system_client::identity::Face::discover(
            tree,
            authority,
            system_api::identity::Grant::Activate,
            Wait::AtMost(MS),
        )
        .ok()?,
    })
}
