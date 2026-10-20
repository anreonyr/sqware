//! 设备恢复那一台的常驻面：上树落牌，然后一枚一枚答门上的问。
//!
//! 三手都只接受**协调入口**（Hub）那一台的请求；BAR 窗的来源（PCI 那一台）在起手时
//! 从树上认一次。本台不进 α 的 Team 子树：一笔责任结清不要求本台退出。

use env::{HoleLimits, PieToken, TaskId, Wait};
use ipc::rpc::request::Receiver;
use ipc::session::establish;
use system_api::operator::Path;
use system_client::operator;

use crate::driver::recovery::api::{self, Call};
use crate::driver::recovery::core::{AttachRequest, Core};
use crate::driver::shared::context::{Context, Step};
use crate::driver::shared::dma::HubDma;
use crate::driver::shared::peer::Peer;

/// 等树 / 认一次门面的上限（毫秒）。
const MS: usize = 2000;

/// 本台那一台的身子。
pub fn serve() -> Result<(), &'static str> {
    let sire = env::unit::sire();
    let ctx = Context::open(sire, Wait::AtMost(MS)).map_err(|s| match s {
        Step::Tree => "recovery: tree",
    })?;
    let tree = operator::Face::from(&ctx.session);
    let hub = HubDma::find(&tree, Wait::AtMost(MS))?;
    // BAR 窗的来源：PCI 那一台那一扇门。
    let pci_road = Path::new("pci/entry");
    let pci = Peer::<crate::driver::pci::api::Call>::find(
        &tree,
        pci_road,
        crate::driver::pci::api::Call::BACK,
        Wait::AtMost(MS),
    )?;
    let core = Core::new(pci.host());
    let entry =
        env::hole::unseal(api::ENTRY, HoleLimits::default()).map_err(|_| "recovery: entry")?;
    ctx.plate(entry, api::ENTRY_NAME, Wait::AtMost(MS));
    let _ = establish::lend(sire, crate::unit::READY_MARK);
    crate::debug::put(&alloc::format!(
        "recovery: ready hub={} pci={}",
        hub.host().get(),
        pci.host().get(),
    ));
    run(entry, hub.host(), core)
}

/// 常驻那一圈：一枚门、一问一答。
fn run(entry: PieToken, hub: TaskId, mut core: Core) -> Result<(), &'static str> {
    let receiver = Receiver::<Call>::from_raw(entry, Call::BACK, Call::back);
    let mut buffer = [0u8; api::Wire::LEN];
    loop {
        let incoming = match receiver.receive(&mut buffer, Wait::Forever) {
            Ok(incoming) => incoming,
            Err(rejected) => {
                if matches!(
                    rejected.fail,
                    ipc::rpc::Fail::Receive(error)
                        if matches!(error, env::HoleFail::Dead | env::HoleFail::Denied)
                ) {
                    return Err("recovery: door dead");
                }
                let _ = execution::room::park(::core::time::Duration::from_millis(1));
                continue;
            }
        };
        let said = turn(&mut core, hub, incoming.from, incoming.request);
        let _ = incoming.reply.send(said);
    }
}

/// 一问 → 一答；不是协调入口发来的，一律拒。
fn turn(core: &mut Core, hub: TaskId, from: TaskId, wire: api::Wire) -> api::Said {
    if from != hub {
        return api::Said::of(api::fail_to_code(Some(api::Fail::Denied)));
    }
    match wire {
        api::Wire::Attach(attach) => {
            let request = AttachRequest {
                serial: attach.serial,
                execution: attach.execution,
                bar: attach.bar,
                pages: attach.pages,
                common: attach.common,
                notify: attach.notify,
                multiplier: attach.multiplier,
                device: attach.device,
            };
            match core.attach(request) {
                Ok(()) => api::Said::of(wire::OK),
                Err(fail) => api::Said::of(api::fail_to_code(Some(fail))),
            }
        }
        api::Wire::Stop(step) => match core.stop(step.serial) {
            Ok(device) => api::Said {
                status: wire::OK,
                device,
            },
            Err(fail) => api::Said::of(api::fail_to_code(Some(fail))),
        },
        api::Wire::Detach(step) => match core.detach(step.serial) {
            Ok(()) => api::Said::of(wire::OK),
            Err(fail) => api::Said::of(api::fail_to_code(Some(fail))),
        },
    }
}
