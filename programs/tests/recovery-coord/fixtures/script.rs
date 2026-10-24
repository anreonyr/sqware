use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

use env::PieToken;

use super::action::Action;

pub struct Script {
    pub actions: Vec<Action>,
    pub pci: Vec<crate::pci_api::Wire>,
    pub iommu: Vec<crate::iommu_api::Wire>,
    pub place_ok: bool,
    pub place_status: u8,
    pub pages: u32,
    pub enable_ok: bool,
    pub disable_ok: bool,
    pub pci_release_ok: bool,
    pub acquire_ok: bool,
    pub publish_ok: bool,
    pub invalidate_ok: bool,
    pub quiesce_ok: bool,
    pub iommu_release_ok: bool,
    pub iommu_status: u8,
    pub publish_pending: bool,
    pub iova: u64,
    pub iovas: Vec<(u64, u64)>,
    pub hold_pci: Option<(u8, u64)>,
    pub hold_iommu: Option<(u8, u64)>,
}

impl Script {
    pub fn new() -> Self {
        Self {
            actions: Vec::new(),
            pci: Vec::new(),
            iommu: Vec::new(),
            place_ok: true,
            place_status: 0,
            pages: 1,
            enable_ok: true,
            disable_ok: true,
            pci_release_ok: true,
            acquire_ok: true,
            publish_ok: true,
            invalidate_ok: true,
            quiesce_ok: true,
            iommu_release_ok: true,
            iommu_status: 0,
            publish_pending: false,
            iova: 0x2_0000_0000,
            iovas: Vec::new(),
            hold_pci: None,
            hold_iommu: None,
        }
    }

    pub fn action(&mut self, action: Action) {
        self.actions.push(action);
    }

    pub fn bar(&self, requester: u32) -> PieToken {
        PieToken::mint(0x100 + requester as usize)
    }

    pub fn iova_for(&self, key: u64) -> u64 {
        self.iovas
            .iter()
            .find(|(domain, _)| *domain == key)
            .map(|(_, iova)| *iova)
            .unwrap_or(self.iova)
    }
}

thread_local! {
    static ACTIVE: RefCell<Option<Rc<RefCell<Script>>>> = const { RefCell::new(None) };
}

pub fn install(script: &Rc<RefCell<Script>>) {
    ACTIVE.with(|active| *active.borrow_mut() = Some(script.clone()));
}

pub fn record(action: Action) {
    ACTIVE.with(|active| {
        if let Some(script) = active.borrow().as_ref() {
            script.borrow_mut().action(action);
        }
    });
}
