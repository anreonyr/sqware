//! IOMMU 那一台的常驻面：收声明供给、立保留页与目录表，然后一枚一枚答门上的问。
//!
//! 请求只接受经 Control 声明认证的设备协调 Driver；发送者由内核给出。
//! 本域管理翻译与引用，不解释设备协议。

use env::{HoleLimits, PieToken, TaskId, Wait};
use ipc::rpc::request::Receiver;
use ipc::session::establish;
use system_client::operator;

use crate::driver::iommu::acquire::AcquireRequest;
use crate::driver::iommu::api::{self, Call};
use crate::driver::iommu::bindings::{REGION, REGION_SUBSET, RETENTION};
use crate::driver::iommu::core::Core;
use crate::driver::iommu::domain_id::DomainId;
use crate::driver::shared::context::{Context, Step};
use crate::driver::shared::lookup;
use crate::driver::shared::platform;
use crate::driver::shared::retention;

/// 等板 / 等树 / 一次编程的总上限（毫秒）。
const MS: usize = 2000;

/// 本台那一台的身子。
pub fn serve() -> Result<(), &'static str> {
    let sire = env::unit::sire();
    let region = platform::take(sire, REGION, REGION_SUBSET, Wait::AtMost(MS))?;
    // 保留那一叶：先收描述（形状：一叶），再从本域表里认回它。
    let (length, description) = platform::descriptor(sire, RETENTION.slot, Wait::AtMost(MS))?;
    if usize::from(length) != 1 || description[0] != 1 {
        return Err("iommu: retention description");
    }
    let retained = retention::claim(sire, RETENTION.slot)?;
    let core = Core::start(region, retained)?;
    // 上树：开会话 ＋ 拿本台那份命名空间能力；再找设备账那一面（协调入口的号）。
    let ctx = Context::open(sire, Wait::AtMost(MS)).map_err(|s| match s {
        Step::Tree => "iommu: tree",
    })?;
    let tree = operator::Face::from(&ctx.session);
    // 铸自己那一扇门、落牌、报“答得动了”。
    let entry = env::hole::unseal(api::ENTRY, HoleLimits::default()).map_err(|_| "iommu: entry")?;
    ctx.plate(entry, api::ENTRY_NAME, Wait::AtMost(MS));
    let _ = establish::lend(sire, crate::unit::READY_MARK);
    crate::debug::put("iommu: ready");
    run(entry, &tree, core)
}

/// 常驻那一圈：一枚门、一问一答。门死了就先把硬件收干净再走。
fn run(entry: PieToken, tree: &operator::Face, mut core: Core) -> Result<(), &'static str> {
    let receiver = Receiver::<Call>::from_raw(entry, Call::BACK, Call::back);
    let mut buffer = [0u8; api::Wire::LEN];
    let mut state = None;
    let mut rejected_state = None;
    let mut driver = None;
    loop {
        lookup::reclaim(&mut rejected_state);
        let incoming = match receiver.receive(&mut buffer, Wait::Forever) {
            Ok(incoming) => incoming,
            Err(rejected) => {
                if matches!(
                    rejected.fail,
                    ipc::rpc::Fail::Receive(error)
                        if matches!(error, env::HoleFail::Dead | env::HoleFail::Denied)
                ) {
                    // 门死了：先把本域硬件收干净再走；收不干净要报出来，不吞掉。
                    match core.shutdown() {
                        Ok(()) => return Err("iommu: door dead"),
                        Err(why) => {
                            crate::debug::put(&alloc::format!("iommu: shutdown unsettled {why}"));
                            return Err("iommu: door dead, shutdown unsettled");
                        }
                    }
                }
                let _ = execution::room::park(::core::time::Duration::from_millis(1));
                continue;
            }
        };
        if state.is_none() {
            state = lookup::state(tree, env::unit::sire(), &mut rejected_state, Wait::AtMost(MS)).ok();
        }
        if driver.is_none() {
            driver = state.as_ref().and_then(|state| state.publisher(
                system_api::namespace::SERVICE, hub_api::coordination::call::ENTRY, Wait::AtMost(MS)
            ).ok());
        }
        let said = turn(&mut core, driver.unwrap_or(TaskId::new(0)), incoming.from, incoming.request);
        let _ = incoming.reply.send(said);
    }
}

/// 一问 → 一答；不是实际 Driver 发来的，一律拒。核心拿到的 caller 就是内核盖章的发送者。
fn turn(core: &mut Core, driver: TaskId, from: TaskId, wire: api::Wire) -> api::Said {
    if from != driver {
        return api::Said::of(api::fail_to_code(Some(api::Fail::Denied)));
    }
    match wire {
        api::Wire::Establish(establish) => {
            match core.establish(from, establish.requester) {
                Ok(domain) => reply(core, domain, Ok(::schedule::Progress::Done)),
                Err(fail) => api::Said::of(api::fail_to_code(Some(fail))),
            }
        }
        api::Wire::Acquire(acquire) => {
            let domain = DomainId::new(acquire.engine, acquire.key);
            let request = AcquireRequest {
                domain,
                binding: hub_api::BindingId {
                    hub: acquire.hub,
                    serial: acquire.serial,
                },
                execution: acquire.execution,
                source: acquire.source,
                access: acquire.access,
            };
            let result = core.acquire(from, request).map(|()| ::schedule::Progress::Done);
            reply(core, domain, result)
        }
        api::Wire::Publish(step) => {
            let (domain, binding) = step_parts(&step);
            let result = core.publish(from, domain, binding);
            reply(core, domain, result)
        }
        api::Wire::Invalidate(step) => {
            let (domain, binding) = step_parts(&step);
            let result = core.invalidate(from, domain, binding);
            reply(core, domain, result)
        }
        api::Wire::Quiesce(step) => {
            let (domain, binding) = step_parts(&step);
            let result = core.quiesce(from, domain, binding);
            reply(core, domain, result)
        }
        api::Wire::Release(step) => {
            let (domain, binding) = step_parts(&step);
            let result = core.release(from, domain, binding, step.reason);
            reply(core, domain, result)
        }
    }
}

fn step_parts(step: &api::Step) -> (DomainId, hub_api::BindingId) {
    (
        DomainId::new(step.engine, step.key),
        hub_api::BindingId {
            hub: step.hub,
            serial: step.serial,
        },
    )
}

/// 一步的答：完成／还要再推／失败；都带上本 Domain 的私有分配与此刻持有几页帧。
fn reply(
    core: &Core,
    domain: DomainId,
    result: Result<::schedule::Progress, api::Fail>,
) -> api::Said {
    let (iova, bytes) = core.range(domain).unwrap_or((0, 0));
    let (status, pending) = match result {
        Ok(::schedule::Progress::Done) => (wire::OK, 0),
        Ok(::schedule::Progress::Pending) => (wire::OK, 1),
        Err(fail) => (api::fail_to_code(Some(fail)), 0),
    };
    api::Said {
        status,
        pending,
        held: core.held(),
        engine: domain.engine(),
        key: domain.key(),
        iova,
        bytes,
    }
}
