//! PCI 那一台的常驻面：收声明供给、枚举、报名、落牌，然后一枚一枚答门上的问。
//!
//! 落位与开启 MASTER 都只接受**协调入口**（Hub）那一台的请求：发送者由内核给出，
//! 不看报文自报。本台不解释设备协议（virtio 寄存器归设备那一侧），
//! 也不持设备协议那一侧的运行状态。

use env::{HoleLimits, PieToken, TaskId, Wait};
use ipc::rpc::request::Receiver;
use ipc::session::establish;
use resource::dock::Dock;
use system_client::operator;

use crate::driver::pci::api::{self, Call};
use crate::driver::pci::bindings::{APERTURE_BINDING, APERTURE_SUBSET, ECAM_BINDING, ECAM_SUBSET};
use crate::driver::pci::core::{self, Core, PlaceRequest};
use crate::driver::shared::context::{Context, Step};
use crate::driver::shared::dma::HubDma;
use crate::driver::shared::platform;

/// 等板 / 等树 / 一次落位的总上限（毫秒）。
const MS: usize = 2000;

/// 本台那一台的身子。
pub fn serve() -> Result<(), &'static str> {
    let sire = env::unit::sire();
    let ecam = platform::take(sire, ECAM_BINDING, ECAM_SUBSET, Wait::AtMost(MS))?;
    let aperture = platform::take(sire, APERTURE_BINDING, APERTURE_SUBSET, Wait::AtMost(MS))?;
    if aperture.size == 0 {
        return Err("pci: aperture geometry");
    }
    let dock = core::open_ecam(&ecam)?;
    let base = dock.view().base();
    let core = Core::discover(base, aperture)?;
    crate::debug::put(&alloc::format!(
        "pci: space task={} ecam={base:#x} requester={} bar={}",
        env::unit::self_id().get(),
        core.requester(),
        core.bar(),
    ));
    // 上树：开会话 ＋ 拿本台那份命名空间能力；再找设备账那一面报名。
    let ctx = Context::open(sire, Wait::AtMost(MS)).map_err(|s| match s {
        Step::Tree => "pci: tree",
    })?;
    let tree = operator::Face::from(&ctx.session);
    let hub = HubDma::find(&tree, Wait::AtMost(MS))?;
    // 铸自己那一扇门、落牌、报“答得动了”。
    let entry = env::hole::unseal(api::ENTRY, HoleLimits::default()).map_err(|_| "pci: entry")?;
    ctx.plate(entry, api::ENTRY_NAME, Wait::AtMost(MS));
    let _ = establish::lend(sire, crate::unit::READY_MARK);
    crate::debug::put(&alloc::format!("pci: ready hub={}", hub.host().get()));
    let until = env::chrono::clock().saturating_add(5_000_000_000);
    loop {
        if hub
            .enroll(
                Wait::AtMost(MS),
                core.requester() as u32,
                0,
                "virtio-blk-pci",
                "virtio-blk",
            )
            .is_ok()
        {
            break;
        }
        if env::chrono::clock() >= until {
            return Err("pci: enrollment");
        }
        let _ = execution::room::park(::core::time::Duration::from_millis(1));
    }
    run(entry, hub.host(), core, dock)
}

/// 常驻那一圈：一枚门、一问一答。
fn run(entry: PieToken, hub: TaskId, mut core: Core, _dock: Dock) -> Result<(), &'static str> {
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
                    return Err("pci: door dead");
                }
                // 坏帧 / 读不懂：不猜、也不回话（没可信的“往哪回”）。
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
        api::Wire::Place(place) => {
            let request = PlaceRequest {
                serial: place.serial,
                requester: place.requester,
                recovery: place.recovery,
                bar: place.bar,
            };
            match core.place(request) {
                Ok(placement) => {
                    let (common, notify, multiplier, device) = placement.caps.parts();
                    api::Said {
                        status: wire::OK,
                        pages: placement.pages(),
                        bar: placement.bar,
                        common,
                        notify,
                        multiplier,
                        device,
                    }
                }
                Err(fail) => api::Said::of(api::fail_to_code(Some(fail))),
            }
        }
        api::Wire::Enable(toggle) => match core.enable(toggle.serial) {
            Ok(()) => api::Said::of(wire::OK),
            Err(fail) => api::Said::of(api::fail_to_code(Some(fail))),
        },
        api::Wire::Disable(toggle) => match core.disable(toggle.serial) {
            Ok(()) => api::Said::of(wire::OK),
            Err(fail) => api::Said::of(api::fail_to_code(Some(fail))),
        },
        api::Wire::Release(toggle) => match core.release(toggle.serial) {
            Ok(()) => api::Said::of(wire::OK),
            Err(fail) => api::Said::of(api::fail_to_code(Some(fail))),
        },
    }
}
