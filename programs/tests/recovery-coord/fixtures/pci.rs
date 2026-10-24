use crate::pci_api::{Call, Said, Wire};
use super::{action::Action, peer::Fixture, script::Script};

impl Fixture for Call {
    type Request = Wire;
    type Response = Said;
    fn call(script: &mut Script, request: Wire) -> Result<Said, &'static str> {
        script.pci.push(request);
        let (action, okay) = match request {
            Wire::Place(_) => (Action::PciPlace, script.place_ok),
            Wire::Enable(_) => (Action::PciEnable, script.enable_ok),
            Wire::Disable(_) => (Action::PciDisable, script.disable_ok),
            Wire::Release(_) => (Action::PciRelease, script.pci_release_ok),
        };
        script.action(action);
        if !okay { return Err("injected PCI transport error after effect"); }
        Ok(Said {
            status: if matches!(request, Wire::Place(_)) { script.place_status } else { 0 },
            pages: 1, bar: env::PieToken::mint(0x44),
            ..Said::NONE
        })
    }
}
