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
pub use super::command::Command;
pub use super::report::Report;

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
    /// 请求保持在途，持续存活直到 System 的空闲超时触发全局退场。
    Forced,
    /// 请求保持在途，保持全部资源，等待测试用 Root 在真实 Guard 保护下杀掉目标。
    Guard,
    /// 多代真实执行实例，交替正常关闭与主人死亡。
    Soak { generations: usize },
    /// 指定收尾操作无法取得确认，保留责任后恢复传输并重试。
    Confirmation,
}

impl Mode {
    /// 本景构造哪一张 α 镜像（受限构造按名字选，owner 恒为 Hub）。
    const fn work(self) -> &'static str {
        match self {
            Self::Normal
            | Self::Fault
            | Self::Ruin
            | Self::Forced
            | Self::Guard
            | Self::Soak { .. }
            | Self::Confirmation => "device-work",
            Self::Before => "device-work-before",
            Self::After => "device-work-after",
        }
    }
}

/// 用受限构造协议把执行实例建出来，**以 Hub 为 owner**：只有 Hub 能停止/核销它
/// （实例面按真实 caller 与 owner 核）；再把它的号报给 Hub，由 Hub 放行。
pub(super) fn construct(
    sire: TaskId, hub: TaskId, launch: PieToken, unit: &str, id: u64,
) -> Result<TaskId, &'static str> {
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
    let bytes = (task.get() as u64).to_le_bytes();
    ::resource::raw::Hole::from_raw(launch)
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
    let launch = establish::give(hub.host(), hub_api::LAUNCH_MARK)
        .map_err(|_| "ctl: launch channel")?;
    let alpha = construct(sire, hub.host(), launch, mode.work(), 1)?;
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
    let mut writer = establish::give(alpha, COMMAND).map_err(|_| "ctl: command channel")?;
    // 等报告。
    let report = receive_report(entry, alpha)?;
    check_auxiliary(report, true)?;
    let (serial, want) = (report.serial, report.want);
    crate::debug::put(&alloc::format!("ctl: report serial={serial} want={want}"));
    if report.want != 0 {
        // 交窗：这根 Pole 正好是整窗，普通 Accord 继承同一范围；含 VEST 供 α 再交 IOMMU。
        let window = env::pie::accord(
            pole,
            alpha,
            env::Permission::FETCH
                .union(env::Permission::STORE)
                .union(env::Permission::VEST),
            dma::window_mark(serial),
        )
        .map_err(|_| "ctl: window handover")?;
        command(writer, Command::Window(window))?;
    }
    let iova = if want != 0 { activation(entry, alpha, serial)?.iova } else { 0 };
    match mode {
        Mode::Ruin | Mode::Forced | Mode::Guard => {
            block.fill(layout::DATA, layout::SENTINEL);
            block.put32(layout::STATUS, 0, 0xFF);
            block.arm(iova, 0);
            command(writer, Command::Submit)?;
            if receive_report(entry, alpha)?.want != 3 {
                return Err("ctl: in-flight request not submitted");
            }
            if mode == Mode::Ruin {
                let request = establish::give(sire, crate::harness::probe::device_ruin::REQUEST)
                    .map_err(|_| "ctl: Ruin request channel")?;
                resource::raw::Hole::from_raw(request).push(&[1], Wait::AtMost(MS))
                    .map_err(|_| "ctl: Ruin request")?;
            } else if mode == Mode::Guard {
                let request = establish::give(sire, crate::harness::probe::device_guard::REQUEST)
                    .map_err(|_| "ctl: Guard request channel")?;
                resource::raw::Hole::from_raw(request).push(&[1], Wait::AtMost(MS))
                    .map_err(|_| "ctl: Guard request")?;
                crate::debug::put(
                    "ctl: in-flight request submitted; retaining block resources for guard death",
                );
                loop {
                    let _ = execution::room::park(::core::time::Duration::from_millis(100));
                }
            } else {
                crate::debug::put("ctl: awaiting forced global shutdown with in-flight read");
            }
            await_closed(&hub, serial, if mode == Mode::Forced { 30000 } else { 6000 })?;
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
            if mode == Mode::Forced {
                crate::debug::put("ctl: drained read verified; awaiting global retirement");
                loop {
                    execution::room::park(::core::time::Duration::from_millis(100))
                        .map_err(|_| "ctl: forced retirement wait")?;
                }
            }
            crate::debug::put("ctl: in-flight binding settled by static Driver Ruin");
        }
        Mode::Soak { generations } => {
            let started = env::chrono::clock();
            let mut current = report;
            let mut current_iova = iova;
            let mut baseline = None;
            for generation in 1..=generations {
                round_one(&block, writer, current_iova)?;
                let fault = generation % 2 == 0;
                command(writer, if fault { Command::Exit } else { Command::Close })?;
                await_closed(&hub, current.serial, MS)?;
                check_auxiliary(current, false)?;
                let (mut capabilities, mut launches, mut commands) = (0, 0, 0);
                for info in resource::raw::pies() {
                    capabilities += 1;
                    if info.alive && info.kind == env::PieKind::Hole {
                        launches += usize::from(info.mark == hub_api::LAUNCH_MARK);
                        commands += usize::from(info.mark == COMMAND);
                    }
                }
                let expected = *baseline.get_or_insert(capabilities);
                if capabilities != expected || launches != 1 || commands != 1 {
                    return Err("ctl: generation capability growth");
                }
                crate::debug::put(&alloc::format!(
                    "ctl: generation {generation} capabilities={capabilities} launch={launches} command={commands}",
                ));
                crate::debug::put(&alloc::format!(
                    "ctl: generation {generation} serial={} settled fault={fault}",
                    current.serial,
                ));
                if generation == generations {
                    break;
                }
                resource::raw::release(writer).map_err(|_| "ctl: generation command release")?;
                let replacement = construct(
                    sire, hub.host(), launch, mode.work(), generation as u64 + 1,
                )?;
                writer = establish::give(replacement, COMMAND)
                    .map_err(|_| "ctl: generation command")?;
                let next = receive_report(entry, replacement)?;
                check_auxiliary(next, true)?;
                if next.serial <= current.serial || next.want != 1 {
                    return Err("ctl: generation did not receive a fresh binding");
                }
                for page in 0..layout::PAGES {
                    block.fill(page, 0);
                }
                let window = env::pie::accord(
                    pole,
                    replacement,
                    env::Permission::FETCH | env::Permission::STORE | env::Permission::VEST,
                    dma::window_mark(next.serial),
                )
                .map_err(|_| "ctl: generation window")?;
                command(writer, Command::Window(window))?;
                current_iova = activation(entry, replacement, next.serial)?.iova;
                current = next;
            }
            crate::debug::put(&alloc::format!(
                "ctl: soak generations={generations} elapsed_ms={}",
                env::chrono::clock().saturating_sub(started) / 1_000_000,
            ));
        }
        Mode::Before | Mode::After => {
            // 发布前/后中止：α 自己收场，Hub 按同一道门与同一条停链结清。
            command(writer, Command::Close)?;
            await_closed(&hub, serial, MS)?;
            check_auxiliary(report, false)?;
            crate::debug::put(&alloc::format!("ctl: binding {serial} settled after abort"));
        }
        Mode::Normal => {
            round_one(&block, writer, iova)?;
            command(writer, Command::Close)?;
            await_closed(&hub, serial, MS)?;
            check_auxiliary(report, false)?;
            crate::debug::put(&alloc::format!(
                "ctl: binding {serial} closed and reclaimed"
            ));
        }
        Mode::Confirmation => {
            round_one(&block, writer, iova)?;
            let request = establish::give(sire, crate::harness::probe::device_confirmation::REQUEST)
                .map_err(|_| "ctl: confirmation request")?;
            resource::raw::Hole::from_raw(request).push(&[1], Wait::AtMost(MS))
                .map_err(|_| "ctl: confirmation arm")?;
            let ack = establish::claim(
                sire, crate::harness::probe::device_confirmation::ACK, Wait::AtMost(MS),
            ).map_err(|_| "ctl: confirmation acknowledgement")?;
            let mut expected = [0u8; 3];
            if resource::raw::Hole::from_raw(ack).pull(&mut expected, Wait::AtMost(MS))
                .map_err(|_| "ctl: confirmation armed")? != (3, sire)
            {
                return Err("ctl: confirmation sender");
            }
            command(writer, Command::Exit)?;
            let until = env::chrono::clock().saturating_add(5_000_000_000);
            loop {
                if let Ok((state, Some(failure))) = hub.inspect(Wait::AtMost(MS), serial) {
                    if state != hub_api::BindingState::Closing
                        || (failure.party, failure.op, failure.code)
                            != (expected[0], expected[1], expected[2])
                    {
                        return Err("ctl: wrong confirmation failure");
                    }
                    break;
                }
                if env::chrono::clock() >= until {
                    return Err("ctl: confirmation failure was not observed");
                }
                execution::room::park(::core::time::Duration::from_millis(1))
                    .map_err(|_| "ctl: confirmation wait")?;
            }
            check_auxiliary(report, false)?;
            crate::debug::put(&alloc::format!(
                "ctl: confirmation failed party={} op={} code={} state=Closing",
                expected[0], expected[1], expected[2],
            ));
            resource::raw::Hole::from_raw(request).push(&[2], Wait::AtMost(MS))
                .map_err(|_| "ctl: confirmation resume")?;
            let mut observed = [0u8; 3];
            if resource::raw::Hole::from_raw(ack).pull(&mut observed, Wait::AtMost(MS))
                .map_err(|_| "ctl: confirmation resumed")? != (3, sire)
                || observed != expected
            {
                return Err("ctl: confirmation resume sender");
            }
            await_closed(&hub, serial, MS)?;
            let (state, failure) = hub.inspect(Wait::AtMost(MS), serial)?;
            if state != hub_api::BindingState::Closed
                || failure.map(|failure| (failure.party, failure.op, failure.code))
                    != Some((expected[0], expected[1], expected[2]))
            {
                return Err("ctl: original failure lost after retry");
            }
            crate::debug::put("ctl: confirmation retry settled; original failure preserved");
        }
        Mode::Fault => {
            round_one(&block, writer, iova)?;
            command(writer, Command::Exit)?;
            crate::debug::put("ctl: alpha exit requested (hub recovers owner death)");
            await_closed(&hub, serial, MS)?;
            check_auxiliary(report, false)?;
            if let (_, Some(failure)) = hub.inspect(Wait::AtMost(MS), serial)? {
                crate::debug::put(&alloc::format!(
                    "ctl: closed binding {serial} failure party={} op={} code={}",
                    failure.party, failure.op, failure.code,
                ));
            }
            resource::raw::release(writer).map_err(|_| "ctl: command release")?;
            let replacement = construct(sire, hub.host(), launch, "device-work", 2)?;
            writer =
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
            let window = env::pie::accord(
                pole,
                replacement,
                env::Permission::FETCH | env::Permission::STORE | env::Permission::VEST,
                dma::window_mark(again),
            )
            .map_err(|_| "ctl: rebind window")?;
            block.fill(layout::DESC, 0);
            block.fill(layout::AVAIL, 0);
            block.fill(layout::USED, 0);
            block.fill(layout::DATA, layout::SENTINEL);
            block.fill(layout::STATUS, 0xFF);
            // Queue metadata uses the new IOVA; the submitted descriptors deliberately use the retired one.
            block.arm(iova, 0);
            command(writer, Command::Window(window))?;
            let next = activation(entry, replacement, again)?;
            if next.iova == iova { return Err("ctl: retired range reused in the same domain"); }
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
    resource::raw::release(writer).map_err(|_| "ctl: command release")?;
    resource::raw::release(launch).map_err(|_| "ctl: launch release")?;
    // 收图与引用（本景的页）：失败保留句柄，交给任务尽头统一结清。
    view.close().map_err(|_| "ctl: view close")?;
    reference.release().map_err(|_| "ctl: reference release")?;
    crate::debug::put("ctl: pages closed");
    Ok(())
}

/// 第一轮：摆请求 → 让 α 敲门铃 → 等设备写回。
pub(super) fn check_auxiliary(report: Report, living: bool) -> Result<(), &'static str> {
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

pub(super) fn receive_report(entry: PieToken, expected: TaskId) -> Result<Report, &'static str> {
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

pub(super) fn activation(entry: PieToken, execution: TaskId, serial: u64) -> Result<Report, &'static str> {
    let report = receive_report(entry, execution)?;
    if report.want != 4 || report.serial != serial || report.iova == 0
        || report.iova % PAGE_SIZE as u64 != 0 || report.bytes != layout::SPAN
    {
        return Err("ctl: activation range");
    }
    Ok(report)
}

fn round_one(block: &Block, writer: PieToken, iova: u64) -> Result<(), &'static str> {
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
pub(super) fn command(writer: PieToken, command: Command) -> Result<(), &'static str> {
    let mut bytes = [0; Command::LEN];
    let len = command.store(&mut bytes);
    ::resource::raw::Hole::from_raw(writer)
        .push(&bytes[..len], Wait::AtMost(MS))
        .map_err(|_| "ctl: command push")
}

/// 等这一号绑定结清（inspect 到 Closed）。
pub(super) fn await_closed(hub: &HubDma, serial: u64, ms: usize) -> Result<(), &'static str> {
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
