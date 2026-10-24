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
        let (status, pages, bar) = match request {
            Wire::Place(frame) => (script.place_status, script.pages, script.bar(frame.requester)),
            _ => (0, 0, env::PieToken::NONE),
        };
        Ok(Said {
            status,
            pages,
            bar,
            ..Said::NONE
        })
    }

    fn held(script: &Script, request: &Wire) -> bool {
        match request {
            Wire::Place(frame) => script.hold_pci == Some((crate::pci_api::PLACE, frame.serial)),
            Wire::Enable(frame) => script.hold_pci == Some((crate::pci_api::ENABLE, frame.serial)),
            Wire::Disable(frame) => script.hold_pci == Some((crate::pci_api::DISABLE, frame.serial)),
            Wire::Release(frame) => script.hold_pci == Some((crate::pci_api::RELEASE, frame.serial)),
        }
    }
}
