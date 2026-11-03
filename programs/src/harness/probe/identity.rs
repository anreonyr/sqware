//! Trusted setup for the two Identity probes, not a product capability declaration.

use ::resource::port::{self, Access, Policy};
use env::Wait;
use ipc::session::establish;
use system_api::identity::Grant;

use crate::unit::UnitFile;
use ::resource::raw::{Hole, reserve};
use env::pie;

pub(crate) fn supply_to(
    authority: Option<env::TaskId>,
    program: &UnitFile,
    task: env::TaskId,
) -> Result<(), &'static str> {
    if matches!(program.name(), "probe-rule" | "probe-rule-other") {
        let mark = env::Mark::of("probe-rule-verified");
        let owner = env::unit::self_id();
        let token = match establish::find(owner, mark) {
            Ok(token) => token,
            Err(establish::DiscoveryFail::Missing) => {
                env::hole::unseal(mark, env::HoleLimits::default()).map_err(|_| "rule verification channel")?
            }
            Err(establish::DiscoveryFail::Ambiguous) => {
                return Err("rule verification channel ambiguous");
            }
        };
        let access = if program.name() == "probe-rule" {
            Access::FETCH
        } else {
            Access::STORE
        };
        port::ship(token, task, access, Policy::NONE).map_err(|_| "rule verification supply")?;
    }
    if program.name() == "system-dependent" {
        super::hierarchy::supply(task)?;
    }
    if program.name() != composition::units::probe_denied::PROBE_DENIED.name()
        && program.name() != composition::units::probe_coalition::PROBE_COALITION.name()
    {
        return Ok(());
    }
    let authority = authority.ok_or("identity fixture authority")?;
    for grant in [Grant::Bind, Grant::Unbind] {
        let token =
            establish::claim(authority, grant.mark(), Wait::POLL).map_err(
                |failure| match failure {
                    establish::DiscoveryFail::Missing => "identity fixture face",
                    establish::DiscoveryFail::Ambiguous => "identity fixture face ambiguous",
                },
            )?;
        if !matches!(reserve(token), Ok((_, owner, mark))
            if owner == authority && mark == grant.mark())
        {
            return Err("identity fixture source");
        }
        // Transfer an entry copy, not the installer's kernel sender identity.
        port::ship(token, task, Access::FETCH | Access::STORE, Policy::NONE)
            .map_err(|_| "identity fixture transfer")?;
    }
    Ok(())
}

/// A timed-out client may drop its send buffer while the kernel still holds the request.
pub fn timeout() {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicBool, Ordering};
    use system_api::identity::Wire;
    use system_api::identity::limits::MAX_FRAME;
    use system_client::identity::CallError;
    use system_client::identity::Face;

    let owner = env::unit::self_id();
    let entry = env::hole::unseal(Grant::Resolve.mark(), env::HoleLimits::default()).unwrap();
    let timed_out = Arc::new(AtomicBool::new(false));
    let release_reader = timed_out.clone();
    let raw_owner = owner.get();
    let reader = execution::unit::task::spawn(move || {
        let owner = env::TaskId::new(raw_owner);
        let entry = establish::claim(owner, Grant::Resolve.mark(), Wait::AtMost(1000)).unwrap();
        while !release_reader.load(Ordering::Acquire) {
            execution::room::park(core::time::Duration::from_millis(1)).unwrap();
        }
        let mut bytes = [0; MAX_FRAME];
        let (n, from) = Hole::from_raw(entry)
            .pull(&mut bytes, Wait::AtMost(1000))
            .unwrap();
        assert_eq!(from, owner);
        assert_eq!(
            Wire::take(&bytes[..n]).unwrap().0,
            Some(Wire::Resolve(owner))
        );
    });
    port::ship(entry, reader.id(), Access::FETCH, Policy::NONE).unwrap();
    let face = Face::direct(owner, Grant::Resolve, entry).unwrap();
    let result = face.call(Wire::Resolve(owner), Wait::AtMost(1));
    timed_out.store(true, Ordering::Release);
    assert_eq!(result, Err(CallError::Transport));
    reader.join();
    let _ = env::hole::seal(entry);
    let _ = pie::release(entry, env::ReleaseMode::Revoke);
    programs::debug::put("identity-timeout: queued request decoded after client timeout");
}

fn uart_rx() -> system_api::operator::PathBuf {
    crate::driver::uart::client::road()
        .unwrap()
        .as_path()
        .try_join("rx")
        .unwrap()
}

fn account_callback(
    assembly: &mut system_server::test_support::Fixture,
    operator: &system_client::operator::Face,
) {
    super::account::acceptance(assembly, operator);
}

/// Host wrapper: Source keeps selection and supplies explicit hooks.
pub fn acceptance() {
    system_server::test_support::identity_scenario::preflight();
    let boot = system_server::boot::take(router_api::PLIC_CLASS).expect("identity: bootstrap");
    let deployment = crate::harness::deployment(&boot.catalog).expect("identity: deployment");
    let declarations = crate::harness::declaration_slice();
    let hooks = system_server::test_support::IdentityHooks {
        supply: supply_to,
        account: account_callback,
        hub_activate: hub_client::activate,
        router_entry: router_api::ENTRY,
        rtc_entry: crate::driver::rtc::core::frame::ENTRY_PATH,
        uart_rx,
    };
    system_server::test_support::identity_scenario::acceptance(
        boot,
        declarations,
        deployment,
        hooks,
    );
}
