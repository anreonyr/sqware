pub(crate) mod boot;
mod activity;
pub(crate) mod bootstrap;
pub(crate) mod declarations;
mod execute;
pub(crate) mod install;
pub(crate) mod life;
pub(crate) mod namespace;
pub(crate) mod operator;
pub(crate) mod policy;
pub(crate) mod scene;
pub(crate) mod schedule;
pub(crate) mod wait;
pub(crate) use declarations::Declarations;
pub(crate) use execute::supervise;

/// Executable-system entry: take boot once, validate the catalog selection and
/// hand the borrowed declaration set plus chosen deployment to the server body.
pub fn run(
    declarations: &'static [&'static unit::UnitFile],
    deployments: &'static [unit::Deployment],
) -> Result<(), env::Reason> {
    let boot = bootstrap::take().map_err(|error| error.code())?;
    let deployment = unit::select_deployment(
        deployments,
        declarations,
        boot.catalog.deployment(),
        boot.catalog.entry(),
    )
    .map_err(|why| {
        programs::debug!(
            "system: invalid deployment {}: {why:?}",
            boot.catalog.deployment()
        );
        crate::system::control::unit::start::E_PROGRAM
    })?;
    execute::run(boot, Declarations(declarations), deployment)
}

mod config;
mod guard;
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
    Notification(erra::Error<env::NoleFail>),
    Waiting(erra::Error<env::ToleFail>),
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
impl From<erra::Error<env::NoleFail>> for InstallError {
    fn from(error: erra::Error<env::NoleFail>) -> Self {
        Self::Notification(error)
    }
}
impl From<erra::Error<env::ToleFail>> for InstallError {
    fn from(error: erra::Error<env::ToleFail>) -> Self {
        Self::Waiting(error)
    }
}
impl core::fmt::Display for InstallError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Resource(error) => write!(f, "resource: {:?}", error),
            Self::Capability(error) => write!(f, "capability: {}", error),
            Self::Notification(error) => write!(f, "notification: {}", error),
            Self::Waiting(error) => write!(f, "waiting: {}", error),
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
