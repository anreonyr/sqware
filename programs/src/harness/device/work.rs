//! α：真实设备执行实例。
//!
//! 它自己上树、自己认领（报活孔是它自己的）、自己把控制器交来的窗再交进 IOMMU、
//! 自己按 Hub 放行时推来的坐标立队列、按控制器的命令敲门铃与收场。它不持内存、
//! 不解释 IOMMU、不写 ECAM：那些各归各域。
//!
//! 三个模式只差“什么时候收场”（场景编排，不进生产协议）：
//! - `Normal`：跑完第一轮，等控制器说“正常收场”才请求 `Close::Normal`；
//! - `Before`：只认领，不交窗；等控制器说“收场”就请求 `Close::Fault`（发布之前中止）；
//! - `After`：交窗、等放行（发布与 MASTER 都过了）之后才请求 `Close::Fault`。

use env::{HoleLimits, PieToken, Wait};
use ipc::session::establish;
use resource::dock::Dock;
use system_api::operator::Path;
use system_client::operator;
use wire::Message;

use crate::driver::iommu::api::ENTRY_PATH as IOMMU_ENTRY;
use crate::driver::pci::virtio::{Caps, Virtio};
use crate::driver::shared::dma::{self, HubDma};
use crate::driver::shared::peer::Peer;
use crate::harness::device::layout::{self, Block};

use super::control::{self, Call as CtlCall, Command};

/// 等树 / 等命令 / 等硬件的上限（毫秒）。
const MS: usize = 3000;
/// 本台那一扇门（控制器往这里推命令）与报活孔的记号。
const ENTRY: env::Mark = env::Mark::of("device-work/entry");

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Normal,
    Before,
    After,
}

pub fn run(mode: Mode) -> Result<(), &'static str> {
    // 本台是受限构造出来的：sire 是 owner（Hub），起手引导从它那里拿 System 的号。
    let sire = env::unit::sire();
    let system = bootstrap(sire)?;
    let session = ipc::session::Session::open(system, operator::BERTH, Wait::AtMost(MS))
        .map_err(|_| "work: operator session")?;
    let tree = operator::Face::of(session);
    let hub = HubDma::find(&tree, Wait::AtMost(MS))?;
    // 报活孔：内核只允许主人自己交来的那一枚。
    let sensor = env::hole::unseal(hub_api::ALIVE_MARK, HoleLimits::default())
        .map_err(|_| "work: sensor")?;
    let sensor = resource::port::ship(
        sensor,
        hub.host(),
        env::Access::FETCH_STORE,
        env::Policy::NONE,
    )
    .map_err(|_| "work: sensor handover")?
    .seed();
    // 认领：先经 `bond` 进类（类联盟的 permit 在设备门牌上），再列册找到那一台、
    // 推一帧 DMA 认领（“哪一台”由门牌回答，本台不自报设备号）。
    let devices_entry = tree
        .resolve(system_api::namespace::DEVICE, Wait::AtMost(MS))
        .map_err(|_| "work: device namespace")?;
    let service = tree
        .resolve(system_api::namespace::SERVICE, Wait::AtMost(MS))
        .map_err(|_| "work: service namespace")?;
    let bond_face = hub_client::Face::of(
        tree.tile_from(service, Path::new("hub/bond"), Wait::AtMost(MS))
            .and_then(|tile| tile.token(Wait::AtMost(MS)))
            .map_err(|_| "work: hub bond face")?,
    )
    .map_err(|_| "work: hub bond face")?;
    bond_face
        .bond(CLASS.into(), Wait::AtMost(MS))
        .map_err(|_| "work: hub bond")?;
    let list_face = hub_client::Face::of(
        tree.tile_from(service, Path::new("hub/list"), Wait::AtMost(MS))
            .and_then(|tile| tile.token(Wait::AtMost(MS)))
            .map_err(|_| "work: hub list face")?,
    )
    .map_err(|_| "work: hub list face")?;
    let until = env::chrono::clock().saturating_add(MS as u64 * 1_000_000);
    let name = loop {
        let window = list_face
            .list(CLASS.into(), 0, Wait::AtMost(MS))
            .map_err(|_| "work: hub list")?;
        if let Some(name) = window.name(0) {
            break name.clone();
        }
        if env::chrono::clock() >= until {
            return Err("work: no device");
        }
        let _ = execution::room::park(::core::time::Duration::from_millis(1));
    };
    let road = system_api::operator::PathBuf::try_new(&alloc::format!("{CLASS}/{name}"))
        .ok_or("work: device road")?;
    let door = tree
        .tile_from(devices_entry, &road, Wait::AtMost(MS))
        .and_then(|tile| tile.token(Wait::AtMost(MS)))
        .map_err(|_| "work: device door")?;
    let claim_sender =
        ipc::rpc::request::Sender::<hub_api::DmaCall>::from_raw(door, hub_api::DmaCall::BACK)
            .map_err(|_| "work: claim face")?;
    let said = claim_sender
        .call(ipc::time::Deadline::new(Wait::AtMost(MS)), |back| {
            hub_api::DmaWire::Claim(hub_api::DmaClaim {
                op: hub_api::DMA_CLAIM,
                id: 1,
                sensor,
                back,
            })
        })
        .map_err(|_| "work: claim call")?;
    if said.status != wire::OK || said.serial == 0 {
        crate::debug::put(&alloc::format!(
            "work: claim refused status={} serial={}",
            said.status,
            said.serial
        ));
        return Err("work: claim refused");
    }
    let serial = said.serial;
    crate::debug::put(&alloc::format!("work: claimed serial={serial}"));
    // 把编号告诉控制器（往它那一扇门推一帧）。
    let ctl = Peer::<CtlCall>::find(
        &tree,
        Path::new(control::ENTRY_PATH),
        CtlCall::BACK,
        Wait::AtMost(MS),
    )?;
    let command = establish::claim(ctl.host(), control::COMMAND, Wait::AtMost(MS))
        .map_err(|_| "work: command channel")?;
    let (auxiliary, sensor) = super::auxiliary::spawn(ctl.host())?;
    report(
        &ctl,
        serial,
        u8::from(mode != Mode::Before),
        auxiliary,
        sensor,
    )?;
    if mode == Mode::Before {
        // 不交窗：发布不会发生。等控制器说“收场”。
        await_command(command, Command::Close)?;
        crate::debug::put("work: requesting Close::Fault before publish");
        hub.close(Wait::AtMost(MS), serial, hub_api::Close::Fault)?;
        return Ok(());
    }
    // 交窗：从控制器手里拿那一枚（按本次绑定的记号认），再交进 IOMMU。
    await_command(command, Command::Window)?;
    let window = find_window(serial).ok_or("work: window missing")?;
    let dock = Dock::open(window).map_err(|_| "work: window mapping")?;
    let (base, size) = (dock.view().base(), dock.view().size());
    if size != layout::SPAN as usize {
        return Err("work: window is not the scene window");
    }
    let iommu = Peer::<crate::driver::iommu::api::Call>::find(
        &tree,
        Path::new(IOMMU_ENTRY),
        crate::driver::iommu::api::Call::BACK,
        Wait::AtMost(MS),
    )?;
    // 收到的窗已量到 `layout::SPAN`；只收窄权限，范围由普通 Accord 继承。
    env::pie::accord(
        window,
        iommu.host(),
        env::Permission::FETCH.union(env::Permission::STORE),
        dma::window_mark(serial),
    )
    .map_err(|_| "work: window handover")?;
    crate::debug::put(&alloc::format!(
        "work: window handed to iommu base={base:#x} pages={}",
        layout::PAGES
    ));
    if mode == Mode::After {
        // 等放行（Hub 把坐标推来 = 发布与 MASTER 都过了），然后按控制器说的收场。
        let coords = await_coords(hub.host(), serial)?;
        crate::debug::put(&alloc::format!(
            "work: released iova={:#x} pages={}",
            coords.iova,
            coords.pages
        ));
        await_command(command, Command::Close)?;
        crate::debug::put("work: requesting Close::Fault after publish");
        hub.close(Wait::AtMost(MS), serial, hub_api::Close::Fault)?;
        return Ok(());
    }
    // 正常那一档：按坐标立队列，然后按控制器的命令敲门铃。
    let coords = await_coords(hub.host(), serial)?;
    let bar = find_bar(serial).ok_or("work: bar missing")?;
    let bar_dock = Dock::open(bar).map_err(|_| "work: bar mapping")?;
    if bar_dock.view().size() != coords.pages as usize * env::PAGE_SIZE {
        return Err("work: bar is not the exact bar");
    }
    let caps = Caps::from_parts(
        coords.common,
        coords.notify,
        coords.multiplier,
        coords.device,
        bar_dock.view().size(),
    )
    .ok_or("work: virtio caps")?;
    let mut virtio = Virtio::open(bar_dock.view().base(), &caps)?;
    let (desc, avail, used) = (
        layout::iova(coords.iova, 0),
        layout::iova(coords.iova, 1),
        layout::iova(coords.iova, 2),
    );
    let queue = virtio.setup_queue(desc, avail, used)?;
    let block = Block::at(base);
    crate::debug::put(&alloc::format!(
        "work: queue={queue} status={} iova={:#x}",
        virtio.status(),
        coords.iova
    ));
    // 一轮一轮来：控制器摆好一轮（`avail.idx` 动了）本台就敲门铃；
    // 直到控制器说收场。
    let mut seen = block.used_index();
    loop {
        if let Some(command) = poll_command(command) {
            match command {
                Command::Close => {
                    crate::debug::put("work: requesting Close::Normal");
                    hub.close(Wait::AtMost(MS), serial, hub_api::Close::Normal)?;
                    return Ok(());
                }
                Command::Exit => return Ok(()),
                Command::Ring => {
                    virtio.notify();
                    crate::debug::put("work: rang the doorbell");
                }
                Command::Probe => {
                    virtio.notify();
                    crate::debug::put("work: rang the stale iova doorbell");
                    report(&ctl, serial, 2, auxiliary, sensor)?;
                }
                Command::Submit => {
                    virtio.notify();
                    crate::debug::put("work: rang the in-flight request doorbell");
                    report(&ctl, serial, 3, auxiliary, sensor)?;
                }
                Command::Window => {}
            }
        }
        let now = block.used_index();
        if now != seen {
            seen = now;
            crate::debug::put(&alloc::format!("work: used.idx={now}"));
        }
        let _ = execution::room::park(::core::time::Duration::from_millis(1));
    }
}

/// 起手引导：从 sire（本台的 owner）那里拿 System 的号。
fn bootstrap(sire: env::TaskId) -> Result<env::TaskId, &'static str> {
    let hole = establish::claim(sire, hub_api::STARTUP_MARK, Wait::AtMost(MS))
        .map_err(|_| "work: startup channel")?;
    let mut bytes = [0u8; 8];
    let (len, from) = ::resource::raw::Hole::from_raw(hole)
        .pull(&mut bytes, Wait::AtMost(MS))
        .map_err(|_| "work: startup frame")?;
    if from != sire || len != bytes.len() {
        return Err("work: startup sender");
    }
    usize::try_from(u64::from_le_bytes(bytes))
        .ok()
        .filter(|raw| *raw != 0)
        .map(env::TaskId::new)
        .ok_or("work: system task")
}

/// 场景那一台设备在树上的类（与 PCI 报名时的一致）。
const CLASS: &str = "virtio-blk";

/// 把“认领结果”推给控制器（一问一答：答码只用来核它还在）。
fn report(
    ctl: &Peer<CtlCall>,
    serial: u64,
    want: u8,
    auxiliary: env::TaskId,
    sensor: PieToken,
) -> Result<(), &'static str> {
    let said = ctl.call(Wait::AtMost(MS), |back| {
        control::Wire::Report(control::Report {
            op: control::REPORT,
            serial,
            want,
            back,
            auxiliary,
            sensor,
        })
    })?;
    (said.status == wire::OK)
        .then_some(())
        .ok_or("work: report refused")
}

/// 在本域表里按记号认一枚（窗口／BAR 都由别人交来）。
fn find_window(serial: u64) -> Option<PieToken> {
    resource::raw::pies()
        .find(|info| {
            info.alive && info.kind == env::PieKind::Pole && info.mark == dma::window_mark(serial)
        })
        .map(|info| info.token)
}

fn find_bar(serial: u64) -> Option<PieToken> {
    resource::raw::pies()
        .find(|info| {
            info.alive && info.kind == env::PieKind::Pole && info.mark == dma::binding_mark(serial)
        })
        .map(|info| info.token)
}

/// 等一条命令（有界）。
fn await_command(token: PieToken, want: Command) -> Result<(), &'static str> {
    let until = env::chrono::clock().saturating_add(MS as u64 * 1_000_000);
    loop {
        if poll_command(token) == Some(want) {
            return Ok(());
        }
        if env::chrono::clock() >= until {
            return Err("work: command timeout");
        }
        let _ = execution::room::park(::core::time::Duration::from_millis(1));
    }
}

/// 非阻塞地取一条命令（孔是单槽）。
fn poll_command(token: PieToken) -> Option<Command> {
    let mut bytes = [0u8; 2];
    match ::resource::raw::Hole::from_raw(token).pull(&mut bytes, Wait::POLL) {
        Ok((1, _)) => Command::of_code(bytes[0]),
        _ => None,
    }
}

/// 等 Hub 放行业务时推来的那一帧坐标。
fn await_coords(hub: env::TaskId, serial: u64) -> Result<dma::Coords, &'static str> {
    let token = ipc::session::establish::claim(hub, dma::coord_mark(serial), Wait::AtMost(MS))
        .map_err(|_| "work: coordinate channel")?;
    let mut bytes = dma::Coords::EMPTY;
    let (len, from) = ::resource::raw::Hole::from_raw(token)
        .pull(&mut bytes, Wait::AtMost(MS))
        .map_err(|_| "work: coordinates")?;
    if from != hub {
        return Err("work: coordinate sender");
    }
    <dma::Coords as wire::Message>::fetch(&bytes[..len]).ok_or("work: coordinate frame")
}
