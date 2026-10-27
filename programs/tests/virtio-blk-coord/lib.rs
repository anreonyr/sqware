#![cfg(test)]
#![allow(dead_code)]

extern crate alloc;
extern crate env as abi;
extern crate self as env;
extern crate self as ipc;
extern crate self as resource;

pub use abi::{
    Access, Frame, Mark, PAGE_SIZE, Permission, PieFail, PieInfo, PieKind, PieToken, PoleFail,
    ReleaseMode, TaskId, Wait, WireCodes, wire,
};
pub type PoleResult<T> = Result<T, PoleFail>;

pub use pci_api;
pub use iommu_api;
#[path = "../../src/driver/iommu/domain_id.rs"]
pub mod domain_id;
#[path = "../../src/driver/virtio/blk/recovery/api.rs"]
pub mod api;
#[path = "../../src/driver/virtio/blk/client/mod.rs"]
pub mod client;
#[path = "../../src/driver/virtio/blk/recovery/attach_request.rs"]
pub mod attach_request;
#[path = "../../src/driver/shared/coords.rs"]
pub mod coords;
#[path = "../../src/driver/virtio/blk/operation.rs"]
pub mod operation;
#[path = "../../src/driver/virtio/blk/exchange.rs"]
pub mod exchange;
#[path = "../../src/driver/virtio/blk/peers.rs"]
pub mod peers;
#[path = "../../src/driver/virtio/blk/polled.rs"]
pub mod polled;
pub use crate::fixtures::time;
#[path = "../../src/driver/virtio/blk/profile.rs"]
pub mod profile;
#[path = "../../src/driver/virtio/blk/stage.rs"]
pub mod stage;
#[path = "../../src/driver/virtio/blk/coord.rs"]
pub mod coord;
#[path = "../../src/driver/virtio/blk/recovery/executor.rs"]
pub mod executor;
#[path = "../../src/driver/virtio/blk/recovery/attachment.rs"]
pub mod attachment;
#[path = "../../src/driver/virtio/registers.rs"]
pub mod registers;
#[path = "../../src/driver/virtio/caps.rs"]
pub mod caps;
#[path = "../../src/driver/virtio/device.rs"]
pub mod virtio_device;

pub mod dock {
    pub type Dock = resource_impl::mapping::Dock<crate::Kernel>;
    pub use resource_impl::view::View;
}

pub mod debug { pub fn put(_: &str) {} }
pub mod chrono {
    use core::cell::Cell;
    thread_local! { static CLOCK: Cell<u64> = const { Cell::new(0) }; }
    pub fn clock() -> u64 { CLOCK.with(Cell::get) }
    pub fn set(value: u64) { CLOCK.with(|clock| clock.set(value)); }
}
pub mod unit { pub fn self_id() -> crate::TaskId { crate::TaskId::new(1) } }

pub mod pie {
    use crate::{Mark, Permission, PieFail, PieToken, TaskId};

    pub fn release(_: PieToken, _: crate::ReleaseMode) -> Result<(), ()> {
        crate::fixtures::handoff::release()
    }

    pub fn accord(
        token: PieToken,
        _: TaskId,
        subset: Permission,
        _: Mark,
    ) -> Result<PieToken, PieFail> {
        assert_eq!(subset, Permission::FETCH | Permission::STORE | Permission::ONLY);
        crate::fixtures::script::record(crate::fixtures::action::Action::CoreAttach);
        crate::fixtures::state::accord(token)
    }
}

pub mod pole {
    use crate::PieToken;

    pub fn shut(token: PieToken) -> Result<(), crate::PoleFail> {
        crate::fixtures::state::shut(token)
    }
}

pub use crate::fixtures::raw;

pub mod rpc {
    pub mod request {
        use alloc::rc::Rc;
        use core::cell::RefCell;

        use crate::fixtures::peer::Fixture;
        use crate::fixtures::script::Script;

        pub struct Pending<C: Fixture> {
            shared: Rc<RefCell<Script>>,
            request: C::Request,
        }

        impl<C: Fixture> Pending<C> {
            pub fn new(shared: Rc<RefCell<Script>>, request: C::Request) -> Self {
                Self { shared, request }
            }

            pub fn poll(&mut self) -> Result<Option<C::Response>, &'static str> {
                let held = {
                    let script = self.shared.borrow();
                    C::held(&script, &self.request)
                };
                if held {
                    return Ok(None);
                }
                C::call(&mut self.shared.borrow_mut(), self.request.clone()).map(Some)
            }
        }
    }
}

pub mod session {
    pub mod establish {
        pub fn lend(_: env::TaskId, _: env::Mark) -> Result<crate::fixtures::endpoint::Endpoint, ()> {
            Ok(crate::fixtures::endpoint::Endpoint)
        }
        pub fn give(_: env::TaskId, _: env::Mark) -> Result<env::PieToken, ()> {
            crate::fixtures::handoff::give()
        }
    }
}

pub mod driver {
    pub mod pci { pub use crate::pci_api as api; }
    pub mod iommu { pub use crate::iommu_api as api; pub use crate::domain_id; }
    pub mod virtio {
        pub use crate::caps::Caps;
        pub use crate::virtio_device::{queue_enable, reset, status};

        pub mod device {
            use super::Caps;

            pub use crate::virtio_device::{queue_enable, reset, status};

            pub fn reset_begin(bar: usize, caps: &Caps) {
                crate::fixtures::script::record(crate::fixtures::action::Action::CoreStop);
                crate::virtio_device::reset_begin(bar, caps);
            }
        }

        pub mod blk {
            pub use crate::{
                client, coord, exchange, operation, peers, polled, profile, stage,
            };

            pub mod recovery {
                pub use crate::{api, attach_request, attachment, executor};
            }
        }
    }
    pub mod shared {
        pub mod dma {
            pub use crate::coords::Coords;
            pub use hub_api::{binding_mark, coord_mark};
        }
        pub mod order {
            pub fn sync() {
                core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
            }
        }
        pub use crate::fixtures::peer;
    }
}

mod fixtures;
mod tests;
pub use crate::fixtures::Kernel;
