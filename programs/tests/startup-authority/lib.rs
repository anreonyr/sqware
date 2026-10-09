#![allow(dead_code)]
extern crate alloc;
extern crate env as abi;
extern crate self as env;
extern crate self as resource;
pub use abi::wire;
pub use abi::{UnsealArgs, Access, Frame, Mark, PieToken, Policy, TaskId, Wait};
#[path = "../../src/unit/image.rs"]
mod image;
#[path = "../../src/unit/supply.rs"]
mod supply;
use std::cell::RefCell;
#[derive(Default)]
struct Effects {
    fail: bool,
    ships: Vec<(PieToken, TaskId, Access, Policy)>,
}
thread_local! { static EFFECTS: RefCell<Effects> = RefCell::new(Effects::default()); }
pub mod pie {
    pub fn unseal(args: crate::UnsealArgs) -> Result<crate::PieToken, ()> {
        match args { crate::UnsealArgs::Hole { mark, .. } => hole_token(mark), _ => unreachable!() }
    }

    fn hole_token(_: crate::Mark) -> Result<crate::PieToken, ()> {
        Ok(crate::PieToken::mint(1))
    }
}
pub mod port {
    pub fn ship(
        entry: crate::PieToken,
        task: crate::TaskId,
        access: crate::Access,
        policy: crate::Policy,
    ) -> Result<(), ()> {
        crate::EFFECTS.with(|e| {
            let mut e = e.borrow_mut();
            e.ships.push((entry, task, access, policy));
            if e.fail { Err(()) } else { Ok(()) }
        })
    }
}
pub mod system {
    pub mod app {
        pub type Fault = &'static str;
    }
    pub mod loader {
        pub fn release_image(_: &system_api::loader::Ask, _: crate::TaskId) {}
    }
    pub mod control {
        pub mod unit {
            pub struct Control;
            impl Control {
                pub fn live(&self, _: crate::TaskId) -> bool {
                    true
                }
                pub fn owns_team_instance(&self, _: crate::TaskId) -> bool {
                    false
                }
            }
        }
        pub mod identity {
            pub struct Roster;
            impl Roster {
                pub fn allow_subject(
                    &self,
                    _: crate::TaskId,
                    _: system_api::identity::Subject,
                ) -> Result<(), ()> {
                    Ok(())
                }
            }
        }
    }
}
#[path = "../../src/system/control/endpoint/construction.rs"]
mod construction;
#[cfg(test)]
mod tests {
    use super::*;
    fn reset() {
        EFFECTS.with(|e| *e.borrow_mut() = Effects::default());
    }
    #[test]
    fn successful_grant_registers_only_after_shipping_and_is_idempotent() {
        reset();
        let mut inlet = construction::Construction::new().unwrap();
        let task = TaskId::new(7);
        inlet.grant(task).unwrap();
        inlet.grant(task).unwrap();
        assert_eq!(inlet.creators, [task]);
        EFFECTS.with(|e| {
            let e = e.borrow();
            assert_eq!(e.ships.len(), 1);
            assert_eq!(e.ships[0], (inlet.entry, task, Access::STORE, Policy::NONE));
        });
    }
    #[test]
    fn failed_ship_does_not_register_creator() {
        reset();
        EFFECTS.with(|e| e.borrow_mut().fail = true);
        let mut inlet = construction::Construction::new().unwrap();
        assert!(inlet.grant(TaskId::new(7)).is_err());
        assert!(inlet.creators.is_empty());
    }
}

#[cfg(test)]
mod image_tests {
    use super::*;
    use ::wire::Message;
    #[test]
    fn image_frame_has_fixed_recipient_seed_and_length_layout() {
        let frame = image::ImageSupplyFrame {
            seed: PieToken::mint(0x102),
            length: 0x304,
        };
        let mut bytes = image::ImageSupplyFrame::EMPTY;
        assert_eq!(frame.store(&mut bytes), Some(16));
        assert_eq!(bytes, [2, 1, 0, 0, 0, 0, 0, 0, 4, 3, 0, 0, 0, 0, 0, 0]);
        let decoded = image::ImageSupplyFrame::fetch(&bytes).unwrap();
        assert_eq!(decoded.seed, frame.seed);
        assert_eq!(decoded.length, frame.length);
        assert!(image::ImageSupplyFrame::fetch(&bytes[..15]).is_none());
        let mut long = bytes.to_vec();
        long.push(0);
        assert!(image::ImageSupplyFrame::fetch(&long).is_none());
    }
}

#[cfg(test)]
mod supply_tests {
    use super::supply::{Setup, valid_supplies};
    #[test]
    fn image_requires_explicit_consumption_readiness() {
        assert!(!valid_supplies(&[Setup::Image {
            name: "payload",
            load: "load"
        }]));
        assert!(valid_supplies(&[
            Setup::Image {
                name: "payload",
                load: "load"
            },
            Setup::Ready
        ]));
    }
    #[test]
    fn duplicate_and_empty_load_channels_are_rejected() {
        assert!(!valid_supplies(&[
            Setup::Image {
                name: "one",
                load: "ready"
            },
            Setup::Ready
        ]));
        assert!(!valid_supplies(&[
            Setup::Image {
                name: "one",
                load: "load"
            },
            Setup::Image {
                name: "two",
                load: "load"
            },
            Setup::Ready
        ]));
        assert!(!valid_supplies(&[Setup::Machine {
            load: "",
            ready: "ok"
        }]));
        assert!(!valid_supplies(&[Setup::Machine {
            load: "same",
            ready: "same"
        }]));
    }
}
