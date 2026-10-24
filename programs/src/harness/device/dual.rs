use env::Wait;
use ipc::session::establish;
use system_client::operator;

use crate::driver::shared::{context::{Context, Step}, dma::HubDma};
use super::{control, endpoint::Endpoint, layout};

const MS: usize = 3000;

pub fn run(generations: usize) -> Result<(), &'static str> {
    let sire = env::unit::sire();
    let ctx = Context::open(sire, Wait::AtMost(MS)).map_err(|s| match s { Step::Tree => "dual: tree" })?;
    let tree = operator::Face::from(&ctx.session);
    let hub = HubDma::find(&tree, Wait::AtMost(MS))?;
    let launch = establish::give(hub.host(), hub_api::LAUNCH_MARK).map_err(|_| "dual: launch")?;
    let entry = env::hole::unseal(control::ENTRY, env::HoleLimits::default()).map_err(|_| "dual: entry")?;
    ctx.plate(entry, control::ENTRY_NAME, Wait::AtMost(MS));
    let mut a = Endpoint::open(sire, &hub, launch, entry, 1, 0)?;
    let mut b = Endpoint::open(sire, &hub, launch, entry, 2, 0xa5)?;
    if a.address() == b.address() || a.task == b.task { return Err("dual: windows overlap"); }
    crate::debug::put(&alloc::format!(
        "dual: windows a=0x{:x} b=0x{:x} bytes={}", a.address(), b.address(), layout::SPAN,
    ));
    let mut baseline = None;
    for generation in 1..=generations {
        if a.iova != b.iova { return Err("dual: expected equal Domain-local IOVA"); }
        crate::debug::put(&alloc::format!(
            "dual: generation {generation} active a={} b={} iova=0x{:x}",
            a.report.serial, b.report.serial, a.iova,
        ));
        a.read(0)?;
        b.read(0)?;
        a.verify(0)?;
        crate::debug::put(&alloc::format!(
            "dual: isolated a={} b={} a_preserved=true", a.report.serial, b.report.serial,
        ));
        let fault = generation % 2 == 0;
        a.close(&hub, fault)?;
        if hub.inspect(Wait::AtMost(MS), b.report.serial)?.0 != hub_api::BindingState::Active {
            return Err("dual: closing A stopped B");
        }
        b.read(1)?;
        a.verify(0)?;
        b.close(&hub, !fault)?;
        let (mut capabilities, mut launches, mut commands) = (0, 0, 0);
        for info in resource::raw::pies() {
            capabilities += 1;
            if info.alive && info.kind == env::PieKind::Hole {
                launches += usize::from(info.mark == hub_api::LAUNCH_MARK);
                commands += usize::from(info.mark == control::COMMAND);
            }
        }
        let expected = *baseline.get_or_insert(capabilities);
        if capabilities != expected || launches != 1 || commands != 2 {
            return Err("dual: generation capability growth");
        }
        crate::debug::put(&alloc::format!(
            "dual: generation {generation} settled a={} b={} capabilities={capabilities} launch={launches} command={commands} a_fault={fault} b_fault={}",
            a.report.serial, b.report.serial, !fault,
        ));
        if generation != generations {
            a.rebind(sire, &hub, launch, entry, (generation * 2 + 1) as u64)?;
            b.rebind(sire, &hub, launch, entry, (generation * 2 + 2) as u64)?;
        }
    }
    a.release()?;
    b.release()?;
    resource::raw::release(launch).map_err(|_| "dual: launch release")?;
    crate::debug::put(&alloc::format!("dual: completed generations={generations}"));
    Ok(())
}
