//! 控制器：页的主人、编排者与验收方。
//!
//! 它铸本场景那一窗页（自己的 Pole），按 α 的请求把整窗交过去（含 VEST，α 还要再交进
//! IOMMU），自己在本域视图里摆请求、核数据与哨兵，并在故障那一档真杀掉 α。
//! 固定扇区、哨兵、杀服务、反复敲门铃都是本层的事，不进生产协议。

use env::wire::Span as _;
use env::{AcquireSource, PAGE_SIZE, PieToken, Placement, TaskId, TeamId, ViewControl, Wait};
use execution::memory::{Reference, View};
use ipc::rpc::request::Receiver as RequestReceiver;
use ipc::session::establish;
use system_client::operator;

use crate::driver::shared::context::{Context, Step};
use crate::driver::shared::dma::{self, HubDma};
use crate::harness::device::layout::{self, Block};

/// 本台那一扇门与它铸的那两枚孔的记号。
pub const ENTRY: env::Mark = env::Mark::of("device-ctl/entry");
pub const ENTRY_NAME: &str = "entry";
pub const ENTRY_PATH: &str = "device-ctl/entry";
/// 命令孔：控制器铸、把读端交给 α。
pub const COMMAND: env::Mark = env::Mark::of("device-ctl/command");
/// 回信孔那一枚上的记号（报告那一面自己的）。
pub const BACK: env::Mark = env::Mark::of("device-ctl-back");
pub const REPORT: u8 = 1;
/// 等树 / 等报告 / 等硬件的上限（毫秒）。
const MS: usize = 3000;

/// α 报上来的认领结果：编号 ＋ 要不要交窗。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 48)]
pub struct Report {
    pub op: u8,
    pub serial: u64,
    pub want: u8,
    pub back: PieToken,
    pub auxiliary: TaskId,
    pub sensor: PieToken,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wire {
    Report(Report),
}

impl Wire {
    pub const LEN: usize = Report::LEN;

    pub fn take(bytes: &[u8]) -> Option<Self> {
        match *bytes.first()? {
            REPORT => {
                let (value, end) = <Report as env::wire::Span>::fetch_at(bytes, 0)?;
                (end == bytes.len()).then_some(Self::Report(value))
            }
            _ => None,
        }
    }

    pub fn store(&self, out: &mut [u8]) -> Option<usize> {
        match self {
            Self::Report(value) if value.op == REPORT => value.store_at(out, 0),
            _ => None,
        }
    }
}

impl wire::Message for Wire {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        Wire::store(self, out)
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        Wire::take(bytes)
    }
}

#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
#[frame(len = 8)]
pub struct Said {
    pub status: u8,
}

impl wire::Message for Said {
    type In = Self;
    type Buf = [u8; Self::LEN];
    const EMPTY: Self::Buf = [0; Self::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        let (value, end) = <Self as env::wire::Span>::fetch_at(bytes, 0)?;
        (end == bytes.len()).then_some(value)
    }
}

fn reply_to(wire: &Wire) -> PieToken {
    match wire {
        Wire::Report(frame) => frame.back,
    }
}

#[mold::contract(request = Wire, response = Said, mark = BACK, back = reply_to)]
pub struct Call;

/// 控制器发给 α 的命令。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    /// 窗已经交过去了，可以取用。
    Window,
    /// 可以敲门铃了。
    Ring,
    /// 收场（按那一景的理由）。
    Close,
    /// 退出。
    Exit,
    /// 提交负证请求并回报实际敲铃。
    Probe,
    /// 提交在途请求并确认实际敲铃。
    Submit,
}

impl Command {
    pub const fn code(self) -> u8 {
        match self {
            Self::Window => 1,
            Self::Ring => 2,
            Self::Close => 3,
            Self::Exit => 4,
            Self::Probe => 5,
            Self::Submit => 6,
        }
    }

    pub const fn of_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Window),
            2 => Some(Self::Ring),
            3 => Some(Self::Close),
            4 => Some(Self::Exit),
            5 => Some(Self::Probe),
            6 => Some(Self::Submit),
            _ => None,
        }
    }
}

/// 本景走哪一条编排。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// 正常：一轮真读 → 核数据 → 让 α 正常收场。
    Normal,
    /// 故障：一轮真读 → 摆第二轮哨兵 → 让 Hub 收场（Hub 停 α）→ 等恢复 → 核哨兵与重绑。
    Fault,
    /// 发布前中止：不交窗，让 α 直接收场。
    Before,
    /// 发布后中止：交窗、等放行，再让 α 收场。
    After,
    /// 请求保持在途，由 System Control 显式退役静态恢复驱动。
    Ruin,
}

impl Mode {
    /// 本景构造哪一张 α 镜像（受限构造按名字选，owner 恒为 Hub）。
    const fn work(self) -> &'static str {
        match self {
            Self::Normal | Self::Fault | Self::Ruin => "device-work",
            Self::Before => "device-work-before",
            Self::After => "device-work-after",
        }
    }
}

/// 用受限构造协议把执行实例建出来，**以 Hub 为 owner**：只有 Hub 能停止/核销它
/// （实例面按真实 caller 与 owner 核）；再把它的号报给 Hub，由 Hub 放行。
fn construct(sire: TaskId, hub: TaskId, unit: &str, id: u64) -> Result<TaskId, &'static str> {
    use system_api::control::construction as api;
    let me = env::unit::self_id();
    // 本台自己的主体（构造出来的那一台沿用本台的 principal）。
    let authority = system_client::identity::authority().ok_or("ctl: identity authority")?;
    let resolve = {
        let entry = establish::find(authority, system_api::identity::Grant::Resolve.mark())
            .map_err(|_| "ctl: identity face")?;
        system_client::identity::Face::direct(
            authority,
            system_api::identity::Grant::Resolve,
            entry,
        )
        .map_err(|_| "ctl: identity face")?
    };
    let subject = {
        let system_api::identity::Reply::Binding(Some(binding)) = resolve
            .call(system_api::identity::Wire::Resolve(me), Wait::AtMost(MS))
            .map_err(|_| "ctl: identity resolve")?
        else {
            return Err("ctl: identity binding");
        };
        system_api::identity::Subject::new(binding.current.principal, &[])
            .map_err(|_| "ctl: subject")?
    };
    let entry = establish::find(sire, api::ENTRY).map_err(|_| "ctl: construction entry")?;
    let sender = ipc::rpc::request::Sender::<api::Call>::from_raw(entry, api::BACK)
        .map_err(|_| "ctl: construction face")?;
    let mut args = [0u64; system_api::loader::MAX_ARGS];
    args[0] = hub.get() as u64;
    let said = sender
        .call(ipc::time::Deadline::new(Wait::AtMost(MS)), |back| {
            api::Request {
                op: api::BUILD,
                id: api::CreateId { value: id },
                input: api::InputOption(Some(api::Input::Unit(api::UnitRef {
                    name: unit.into(),
                    stack: 0,
                    count: 1,
                    args,
                }))),
                owner: hub,
                subject,
                back,
            }
        })
        .map_err(|_| "ctl: construction call")?;
    if said.status != wire::OK || said.task == TaskId::new(0) {
        return Err("ctl: construction refused");
    }
    let task = said.task;
    // 等它准备完（准备完才可放行）。
    let until = env::chrono::clock().saturating_add(MS as u64 * 1_000_000);
    loop {
        let observed = sender
            .call(ipc::time::Deadline::new(Wait::AtMost(MS)), |back| {
                api::Request {
                    op: api::INSPECT,
                    id: api::CreateId { value: id },
                    input: api::InputOption(None),
                    owner: hub,
                    subject,
                    back,
                }
            })
            .map_err(|_| "ctl: construction inspect")?;
        if observed.state == api::COMPLETED {
            break;
        }
        if observed.state == api::FAILED || observed.state == api::CANCELLED {
            return Err("ctl: construction failed");
        }
        if env::chrono::clock() >= until {
            return Err("ctl: construction pending");
        }
        let _ = execution::room::park(::core::time::Duration::from_millis(1));
    }
    // 只报告任务号；Hub 从 Control 查询真实 Team 与 owner。
    let writer = establish::give(hub, hub_api::LAUNCH_MARK).map_err(|_| "ctl: launch channel")?;
    let bytes = (task.get() as u64).to_le_bytes();
    ::resource::raw::Hole::from_raw(writer)
        .push(&bytes, Wait::AtMost(MS))
        .map_err(|_| "ctl: launch report")?;
    crate::debug::put(&alloc::format!(
        "ctl: constructed {unit} tid={} owner=hub",
        task.get()
    ));
    Ok(task)
}

pub fn run(mode: Mode) -> Result<(), &'static str> {
    let sire = env::unit::sire();
    let ctx = Context::open(sire, Wait::AtMost(MS)).map_err(|s| match s {
        Step::Tree => "ctl: tree",
    })?;
    let tree = operator::Face::from(&ctx.session);
    let hub = HubDma::find(&tree, Wait::AtMost(MS))?;
    // 执行实例由本台用受限构造建出（owner = Hub），再报给 Hub 放行。
    let alpha = construct(sire, hub.host(), mode.work(), 1)?;
    // 本景那一窗页：自己的 Pole ＋ 一份引用 ＋ 本域视图。
    let pole = resource::raw::pole(layout::SPAN as usize, true).map_err(|_| "ctl: pages")?;
    let mut reference = Reference::acquire(
        AcquireSource::Source {
            token: pole,
            offset: 0,
            count: layout::PAGES,
        },
        PieToken::NONE,
    )
    .map_err(|_| "ctl: page reference")?;
    let placement = Placement::lowest(PAGE_SIZE).ok_or("ctl: page placement")?;
    let mut view: View = reference
        .attach(
            TeamId::new(0),
            0,
            layout::PAGES,
            placement,
            6,
            ViewControl::Caller,
        )
        .map_err(|_| "ctl: page view")?;
    let block = Block::at(view.address());
    block.fill(layout::DATA as usize, 0);
    // 自己那一扇门：α 往这里报认领结果。
    let entry = env::hole::unseal(ENTRY, env::HoleLimits::default()).map_err(|_| "ctl: entry")?;
    ctx.plate(entry, ENTRY_NAME, Wait::AtMost(MS));
    // 命令孔：本台铸、把读端交给 α。
    let writer = establish::give(alpha, COMMAND).map_err(|_| "ctl: command channel")?;
    // 等报告。
    let report = receive_report(entry, alpha)?;
    check_auxiliary(report, true)?;
    let (serial, want) = (report.serial, report.want);
    crate::debug::put(&alloc::format!("ctl: report serial={serial} want={want}"));
    if report.want != 0 {
        // 交窗：整窗交给 α（含 VEST：它还要再交进 IOMMU）。
        env::pie::accord_window(
            pole,
            alpha,
            env::Permission::FETCH
                .union(env::Permission::STORE)
                .union(env::Permission::VEST),
            dma::window_mark(serial),
            0,
            layout::PAGES,
        )
        .map_err(|_| "ctl: window handover")?;
        command(writer, Command::Window)?;
    }
    match mode {
        Mode::Ruin => {
            block.fill(layout::DATA, layout::SENTINEL);
            block.put32(layout::STATUS, 0, 0xFF);
            block.arm(dma::iova_base(serial).ok_or("ctl: iova")?, 0);
            command(writer, Command::Submit)?;
            if receive_report(entry, alpha)?.want != 3 {
                return Err("ctl: in-flight request not submitted");
            }
            let request = establish::give(sire, crate::harness::probe::device_ruin::REQUEST)
                .map_err(|_| "ctl: Ruin request channel")?;
            resource::raw::Hole::from_raw(request).push(&[1], Wait::AtMost(MS))
                .map_err(|_| "ctl: Ruin request")?;
            await_closed(&hub, serial, 6000)?;
            let delivered = block.delivered();
            let status = block.status();
            let (_, len) = block.used_entry();
            crate::debug::put(&alloc::format!(
                "ctl: drained read sector_ok={delivered} status={status} used={} len={len}",
                block.used_index(),
            ));
            if !delivered || status != layout::BLK_OK {
                return Err("ctl: held read did not reach retained memory");
            }
            check_auxiliary(report, false)?;
            crate::debug::put("ctl: in-flight binding settled by static Driver Ruin");
        }
        Mode::Before | Mode::After => {
            // 发布前/后中止：α 自己收场，Hub 按同一道门与同一条停链结清。
            command(writer, Command::Close)?;
            await_closed(&hub, serial, MS)?;
            check_auxiliary(report, false)?;
            crate::debug::put(&alloc::format!("ctl: binding {serial} settled after abort"));
        }
        Mode::Normal => {
            round_one(&block, writer, serial)?;
            command(writer, Command::Close)?;
            await_closed(&hub, serial, MS)?;
            check_auxiliary(report, false)?;
            crate::debug::put(&alloc::format!(
                "ctl: binding {serial} closed and reclaimed"
            ));
        }
        Mode::Fault => {
            round_one(&block, writer, serial)?;
            command(writer, Command::Exit)?;
            crate::debug::put("ctl: alpha exit requested (hub recovers owner death)");
            await_closed(&hub, serial, MS)?;
            check_auxiliary(report, false)?;
            let replacement = construct(sire, hub.host(), "device-work", 2)?;
            let writer =
                establish::give(replacement, COMMAND).map_err(|_| "ctl: rebind command")?;
            let replacement_report = receive_report(entry, replacement)?;
            check_auxiliary(replacement_report, true)?;
            let again = replacement_report.serial;
            crate::debug::put(&alloc::format!(
                "ctl: rebind probe claimed serial={again} (device free again)"
            ));
            if again <= serial {
                return Err("ctl: binding serial reused");
            }
            env::pie::accord_window(
                pole,
                replacement,
                env::Permission::FETCH | env::Permission::STORE | env::Permission::VEST,
                dma::window_mark(again),
                0,
                layout::PAGES,
            )
            .map_err(|_| "ctl: rebind window")?;
            block.fill(layout::DESC, 0);
            block.fill(layout::AVAIL, 0);
            block.fill(layout::USED, 0);
            block.fill(layout::DATA, layout::SENTINEL);
            block.fill(layout::STATUS, 0xFF);
            // Queue metadata uses the new IOVA; the submitted descriptors deliberately use the retired one.
            block.arm(dma::iova_base(serial).ok_or("ctl: stale iova")?, 0);
            command(writer, Command::Window)?;
            command(writer, Command::Probe)?;
            if receive_report(entry, replacement)?.want != 2 {
                return Err("ctl: stale request not submitted");
            }
            let until = env::chrono::clock().saturating_add(100_000_000);
            while env::chrono::clock() < until {
                let _ = execution::room::park(::core::time::Duration::from_millis(1));
            }
            let held = (0..layout::SECTOR).all(|at| block.data(at) == layout::SENTINEL);
            if !held || block.status() != 0xFF {
                return Err("ctl: retired iova reached memory");
            }
            crate::debug::put("ctl: stale iova submitted sentinel_held=true status_held=true");
            command(writer, Command::Close)?;
            await_closed(&hub, again, MS)?;
            check_auxiliary(replacement_report, false)?;
        }
    }
    // 收图与引用（本景的页）：失败保留句柄，交给任务尽头统一结清。
    view.close().map_err(|_| "ctl: view close")?;
    reference.release().map_err(|_| "ctl: reference release")?;
    crate::debug::put("ctl: pages closed");
    Ok(())
}

/// 第一轮：摆请求 → 让 α 敲门铃 → 等设备写回。
fn check_auxiliary(report: Report, living: bool) -> Result<(), &'static str> {
    let alive = resource::raw::reserve(report.sensor).is_ok_and(|(from, owner, mark)| {
        from == report.auxiliary && owner == report.auxiliary && mark == hub_api::ALIVE_MARK
    });
    if report.auxiliary == TaskId::new(0) || alive != living {
        return Err("ctl: auxiliary lifetime");
    }
    crate::debug::put(&alloc::format!(
        "ctl: binding {} auxiliary={} alive={alive}",
        report.serial,
        report.auxiliary.get(),
    ));
    Ok(())
}

fn receive_report(entry: PieToken, expected: TaskId) -> Result<Report, &'static str> {
    let receiver = RequestReceiver::<Call>::from_raw(entry, Call::BACK, Call::back);
    let mut buffer = [0u8; Wire::LEN];
    let incoming = receiver
        .receive(&mut buffer, Wait::AtMost(MS))
        .map_err(|_| "ctl: report")?;
    if incoming.from != expected {
        let _ = incoming.reply.send(Said {
            status: hub_api::DENIED,
        });
        return Err("ctl: report sender");
    }
    let Wire::Report(frame) = incoming.request;
    let _ = incoming.reply.send(Said { status: wire::OK });
    Ok(frame)
}

fn round_one(block: &Block, writer: PieToken, serial: u64) -> Result<(), &'static str> {
    let iova = dma::iova_base(serial).ok_or("ctl: iova")?;
    block.fill(layout::DATA as usize, 0);
    block.put32(layout::STATUS as usize, 0, 0xFF);
    block.arm(iova, 0);
    crate::debug::put(&alloc::format!("ctl: armed round one iova={iova:#x}"));
    command(writer, Command::Ring)?;
    let until = env::chrono::clock().saturating_add(MS as u64 * 1_000_000);
    while block.used_index() < 1 {
        if env::chrono::clock() >= until {
            return Err("ctl: round one not served");
        }
        let _ = execution::room::park(::core::time::Duration::from_millis(1));
    }
    let (_, len) = block.used_entry();
    let delivered = block.delivered();
    let status = block.status();
    crate::debug::put(&alloc::format!(
        "ctl: served used={} len={len} written_ok={} sector_ok={} status={status}",
        block.used_index(),
        len == layout::WRITTEN_BYTES,
        delivered,
    ));
    if !delivered || status != layout::BLK_OK || len != layout::WRITTEN_BYTES {
        return Err("ctl: first round not delivered");
    }
    Ok(())
}

/// 发一条命令（单槽：推得进就行）。
fn command(writer: PieToken, command: Command) -> Result<(), &'static str> {
    let bytes = [command.code()];
    ::resource::raw::Hole::from_raw(writer)
        .push(&bytes, Wait::AtMost(MS))
        .map_err(|_| "ctl: command push")
}

/// 等这一号绑定结清（inspect 到 Closed）。
fn await_closed(hub: &HubDma, serial: u64, ms: usize) -> Result<(), &'static str> {
    let until = env::chrono::clock().saturating_add(ms as u64 * 1_000_000);
    loop {
        match hub.inspect(Wait::AtMost(MS), serial) {
            Ok((hub_api::BindingState::Closed, _)) => return Ok(()),
            Ok(_) => {}
            Err(_) => {}
        }
        if env::chrono::clock() >= until {
            return Err("ctl: binding did not settle");
        }
        let _ = execution::room::park(::core::time::Duration::from_millis(1));
    }
}
