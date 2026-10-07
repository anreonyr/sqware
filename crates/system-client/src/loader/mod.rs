pub mod client;
pub mod rpc;

pub mod frame {
    pub use system_api::loader::{Ask, Claim, Said, Wire, Fail, Grant, grant_of,
        BACK, IMAGE, BUILD, CLAIM, CLAIM_MS, MAX_ARGS, MAX_IMAGE, REGISTRY, code_to_fail, fail_to_code};
    pub const DIR: &crate::operator::Path = crate::operator::Path::new(system_api::loader::DIR);
}
pub mod grant { pub use system_api::loader::Grant; pub use system_api::loader::grant_of; }
pub mod marks { pub use system_api::loader::{BACK, IMAGE}; pub const DECLARATIONS: &[env::marks::Definition] = system_api::loader::CHANNELS; }

pub const DIR: &crate::operator::Path = frame::DIR;
pub use client::{Built, Face};
pub use system_api::loader::{Fail, Grant, grant_of, REGISTRY};
