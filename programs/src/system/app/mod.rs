pub(crate) mod boot;
pub(crate) mod bootstrap;
mod execute;
pub(crate) mod install;
pub(crate) mod life;
pub(crate) mod policy;
pub(crate) mod scene;
pub(crate) mod schedule;
pub(crate) mod wait;
pub use execute::run;

pub(crate) mod assembly;
mod config;
mod supplies;

mod waiting;

#[derive(Debug)]
pub(crate) enum Fault {
    Room,
    Publication,
    Dead,
    Wait,
    Idle,
    Shutdown,
}

#[derive(Debug)]
pub(crate) enum InstallError {
    Resource(::schedule::resource::AccessError),
    Capability(erra::Error<env::PieFail>),
}
impl From<::schedule::resource::AccessError> for InstallError {
    fn from(error: ::schedule::resource::AccessError) -> Self { Self::Resource(error) }
}
impl From<erra::Error<env::PieFail>> for InstallError {
    fn from(error: erra::Error<env::PieFail>) -> Self { Self::Capability(error) }
}
impl core::fmt::Display for InstallError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Resource(error) => write!(f, "resource: {:?}", error),
            Self::Capability(error) => write!(f, "capability: {}", error),
        }
    }
}
impl From<::schedule::resource::AccessError> for Fault {
    fn from(_: ::schedule::resource::AccessError) -> Self { Self::Room }
}
impl From<::schedule::DispatchError> for Fault {
    fn from(_: ::schedule::DispatchError) -> Self { Self::Room }
}
impl From<alloc::collections::TryReserveError> for Fault {
    fn from(_: alloc::collections::TryReserveError) -> Self { Self::Room }
}
