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
use hub_api::{self as hub, Deed, DmaCall, Grant, Inventory, Said, Window, Wire};
use ipc::hand::Sender;
use ipc::rpc::{self, request::Receiver as RequestReceiver};
use ipc::session::{Session, establish};
use programs::debug;
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

use self::directory::Directory;
use self::peers::Peers;
use self::sweep::alive;
use ::resource::raw::{Hole, reserve};
use env::pie;

const MS: usize = 1000;

const PROBE_MS: usize = 1000;

pub mod bond;
pub mod claim;
pub mod directory;
pub mod dma;
mod enrollment_failure;
mod execution;
mod launch;
mod launch_phase;
mod launch_reply;
mod returned;
mod return_reply;
mod grant;
use self::grant::ship;
pub mod list;
pub mod peers;
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
    let (mut ledger, league, plates, mut doors, _dtb, dma, retire, tree, devices, device_anchor, subject) =
        (|| {
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
            let devices = system_client::operator::Capability::injected(1, sire, Wait::AtMost(MS))
                .map_err(|_| Start::Tree(E_HUB))?;
            let device_anchor = devices.anchor().map_err(|_| Start::Tree(E_HUB))?;
            let directory = Directory {
                devices: &devices, anchor: device_anchor, league: &league, subject,
            };
            for class in &classes {
                ledger.prepare_league(class.clone()).map_err(|_| Start::Room(E_HUB))?;
                ledger.advance_league(class, &mut &directory)
                    .ok_or(Start::Face(E_HUB))?
                    .map_err(|_| Start::Face(E_HUB))?;
            }

            let bond = env::hole::unseal(Grant::Bond.mark(), env::HoleLimits::default())
                .map_err(|_| Start::Tree(E_HUB))?;
            let list = env::hole::unseal(Grant::List.mark(), env::HoleLimits::default())
                .map_err(|_| Start::Tree(E_HUB))?;
            let claim = env::hole::unseal(Grant::Claim.mark(), env::HoleLimits::default())
                .map_err(|_| Start::Tree(E_HUB))?;
            // DMA 安全绑定那一面：发现、认领、关闭、查看与整机结清共用这一扇门。
            let dma = env::hole::unseal(Grant::Dma.mark(), env::HoleLimits::default())
                .map_err(|_| Start::Tree(E_HUB))?;
            let retire = env::hole::unseal(Grant::Retire.mark(), env::HoleLimits::default())
                .map_err(|_| Start::Tree(E_HUB))?;
            let publisher =
                system_client::operator::Capability::injected(0, sire, Wait::AtMost(MS))
                    .map_err(|_| Start::Tree(E_HUB))?;
            let service_anchor = publisher.anchor().map_err(|_| Start::Tree(E_HUB))?;
            let endpoint_view =
                operator_api::ResourceView::new(env::Access::STORE).ok_or(Start::Tree(E_HUB))?;
            for (index, (name, entry)) in [
                (Grant::Bond.name(), bond),
                (Grant::List.name(), list),
                (Grant::Claim.name(), claim),
                (Grant::Dma.name(), dma),
                (Grant::Retire.name(), retire),
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

            // 九、挂组那一张表：四枚面 ＋ 每一台那一枚门（**挂在孔上，面由孔推**）。
            let mut doors: Vec<PieToken> = Vec::new();
            doors
                .try_reserve(5 + ledger.count())
                .map_err(|_| Start::Desk(E_HUB))?;
            doors.extend([bond, list, claim, dma, retire]);
            for class in &classes {
                doors.extend(ledger.doors(class.clone()).map(|(_, door)| door));
            }
            Ok::<_, Start>((
                ledger,
                league,
                (bond, list, claim),
                doors,
                dtb,
                dma,
                retire,
                tree,
                devices,
                device_anchor,
                subject,
            ))
        })()?;

    // 十、协调入口那几面：三个硬件域与 Control（树上查回来的门牌）；
    //     再加一本“设备目录”的装置（新设备的门牌与类联盟由它挂出去）。
    let mut peers = None;
    let mut executions = execution::Executions::try_new().map_err(|_| Start::Room(E_HUB))?;
    let directory = Directory {
        devices: &devices,
        anchor: device_anchor,
        league: &league,
        subject,
    };

    // 十一、常驻：一只组等那几枚孔；**从哪一枚读到就是哪一面**（设备门那一枚的面由"它不是面那
    //     三枚"推出来，认台靠**这一枚孔自己**；DMA 那一面自己那一枚）。
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
        // Advance launch/return work and admission deadlines before handling a door, so
        // an expired admission cannot claim in the same wake.
        if let Some(peers) = peers.as_ref() {
            execution::receive(&mut ledger, &mut executions);
            executions.advance(&mut ledger, peers, env::chrono::clock());
        }
        let wait = if executions.pending(&ledger) || ledger.open_bindings_all().any(|binding| {
            matches!(
                binding.state,
                hub_api::BindingState::Preparing | hub_api::BindingState::Closing
            )
        }) {
            Wait::AtMost(1)
        } else {
            Wait::AtMost(PROBE_MS)
        };
        match pile.await_(wait) {
            Ok(env::ToleAwaitReply::Source {
                source: env::ToleEventSource::Hole { token, .. },
                ..
            }) => {
                if token == dma {
                    // DMA 那一面走类型化的一问一答（回信孔由 ipc::rpc 铸、交、核）。
                    drain_dma(&mut ledger, &mut peers, &tree, &directory, token, &mut buf);
                } else if token == retire {
                    drain_retirement(&mut ledger, &mut executions, token, &mut buf);
                } else {
                    // 门牌是**单槽**：一次醒来的这一批要取干净（可能不止一位客人）。
                    let hole = Hole::from_raw(token);
                    while let Ok((len, from)) = hole.pull(&mut buf, Wait::POLL) {
                        // 设备门牌上的 DMA 认领（“哪一台”由这一枚门回答）；
                        // 其余形状仍走普通设备那一面的招待。
                        if peers
                            .as_ref()
                            .and_then(|peers| {
                                dma::claim_at_door(&mut ledger, peers, token, from, &buf[..len])
                            })
                            .is_none()
                        {
                            turn(&mut ledger, &league, plates, token, from, &buf[..len]);
                        }
                    }
                }
            }
            // **挂起过 / 期限到**：这是常态（没有客人），故它顺带就是探活那一拍。
            Ok(_) => {}
            Err(_) => return Err(Start::Dead(E_HUB)),
        }
        // 探活：主人没了的格当场空出来（读数只在真空出东西时印一行）；
        // DMA 绑定的主人没了只转收场，绝不因此变为可重绑。
        let freed = ledger.vacate(alive);
        if freed > 0 {
            debug!("hub: vacated {freed}");
        }
        if let Some(peers) = peers.as_ref() {
            dma::advance(&mut ledger, peers, &mut executions);
        }
        for class in ledger.classes().ok_or(Start::Room(E_HUB))? {
            for (_, door) in ledger.doors(class) {
                if !doors.contains(&door) {
                    pile.attach(env::ToleEventSource::Hole {
                        token: door,
                        condition: HoleCondition::Pull,
                    })
                    .map_err(|_| Start::Desk(E_HUB))?;
                    doors.push(door);
                }
            }
        }
    }
}

/// DMA 那一面上的几问：取干净这一批、一问一答。
fn drain_dma(
    ledger: &mut Ledger,
    peers: &mut Option<Peers>,
    tree: &Face,
    directory: &Directory,
    entry: PieToken,
    buffer: &mut [u8],
) {
    let receiver = RequestReceiver::<DmaCall>::from_raw(entry, DmaCall::BACK, DmaCall::back);
    loop {
        match receiver.receive(buffer, Wait::POLL) {
            Ok(incoming) => {
                if peers.is_none() && matches!(incoming.request, hub_api::DmaWire::Enroll(_)) {
                    match Peers::find(tree, Wait::AtMost(MS)) {
                        Ok(found) => *peers = Some(found),
                        Err(reason) => debug!("hub: coordinators unavailable: {reason}"),
                    }
                }
                let said = dma::turn(
                    ledger,
                    peers.as_ref(),
                    directory,
                    incoming.from,
                    incoming.request,
                );
                let _ = incoming.reply.send(said);
            }
            Err(rejected) => match rejected.fail {
                // 单槽取干了。
                rpc::Fail::Receive(_) => break,
                // 坏帧 / 读不懂：不猜、也不回话。
                _ => continue,
            },
        }
    }
}

/// Generic retirement face: only the assembler (sire) may ask; the provider maps its own
/// outstanding responsibility count to Pending/Done. No lifecycle is copied here.
///
/// A non-zero target is Control's proof that one execution domain stopped. It only
/// advances Hub resources bound to that exact task. The global target additionally waits
/// for every owner receipt Hub accepted, but never for a release reply while answering a
/// per-instance resource request.
fn drain_retirement(
    ledger: &mut Ledger,
    executions: &mut execution::Executions,
    entry: PieToken,
    buffer: &mut [u8],
) {
    let receiver = RequestReceiver::<system_api::retirement::Call>::from_raw(
        entry,
        system_api::retirement::Call::BACK,
        system_api::retirement::Call::back,
    );
    loop {
        match receiver.receive(buffer, Wait::POLL) {
            Ok(incoming) => {
                let target = incoming.request.target;
                let trusted = incoming.from == unit::sire()
                    && incoming.request.state == system_api::retirement::ASK;
                let done = if !trusted {
                    false
                } else if target == TaskId::new(0) {
                    ledger.settle();
                    executions.settle(ledger);
                    ledger.open_bindings_all().next().is_none() && executions.idle(ledger)
                } else {
                    if !executions.target(ledger, target) {
                        let _ = incoming.reply.send(
                            system_api::retirement::Retirement::pending_for(target),
                        );
                        continue;
                    }
                    let open = ledger.stop_target(target, |serial| {
                        crate::debug::put(&alloc::format!(
                            "hub: binding {serial} execution domain stopped (tid={})",
                            target.get()
                        ));
                    });
                    open == 0 && executions.target_received(target)
                };
                let reply = if done {
                    system_api::retirement::Retirement::done_for(target)
                } else {
                    system_api::retirement::Retirement::pending_for(target)
                };
                let _ = incoming.reply.send(reply);
            }
            Err(rejected) => match rejected.fail {
                rpc::Fail::Receive(_) => break,
                _ => continue,
            },
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
        // DMA 那一面的一答是它自己那一形（正常路径在 drain_dma 里，这里只为穷尽）。
        Grant::Dma => {
            let mut tx = Sender::<hub_api::DmaSaid>::from_raw(back);
            let _ = tx.send(hub_api::DmaSaid::of(status));
        }
        Grant::Retire => {
            let mut tx = Sender::<system_api::retirement::Retirement>::from_raw(back);
            let _ = tx.send(system_api::retirement::Retirement::pending());
        }
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
