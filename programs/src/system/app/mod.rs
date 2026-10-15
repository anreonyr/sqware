pub(crate) mod boot;
pub(crate) mod bootstrap;
mod execute;
pub(crate) mod install;
pub(crate) mod life;
pub(crate) mod namespace;
pub(crate) mod operator;
pub(crate) mod policy;
pub(crate) mod scene;
pub(crate) mod schedule;
pub(crate) mod wait;
pub use execute::run;

mod config;
pub(crate) mod retention;
pub(crate) mod supplies;

mod waiting;

#[derive(Debug)]
pub(crate) enum Fault {
    Room,
    Dead,
    Wait,
    Idle,
    Shutdown,
}

#[derive(Debug)]
pub(crate) enum InstallError {
    Resource(::schedule::resource::AccessError),
    Capability(erra::Error<env::PieFail>),
    Configuration(&'static str),
}
impl From<::schedule::resource::AccessError> for InstallError {
    fn from(error: ::schedule::resource::AccessError) -> Self {
        Self::Resource(error)
    }
}
impl From<erra::Error<env::PieFail>> for InstallError {
    fn from(error: erra::Error<env::PieFail>) -> Self {
        Self::Capability(error)
    }
}
impl core::fmt::Display for InstallError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Resource(error) => write!(f, "resource: {:?}", error),
            Self::Capability(error) => write!(f, "capability: {}", error),
            Self::Configuration(reason) => write!(f, "configuration: {reason}"),
        }
    }
}
impl From<::schedule::resource::AccessError> for Fault {
    fn from(_: ::schedule::resource::AccessError) -> Self {
        Self::Room
    }
}
impl From<::schedule::DispatchError> for Fault {
    fn from(_: ::schedule::DispatchError) -> Self {
        Self::Room
    }
}
impl From<alloc::collections::TryReserveError> for Fault {
    fn from(_: alloc::collections::TryReserveError) -> Self {
        Self::Room
    }
}
