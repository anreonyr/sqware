#![no_std]
//! Pure UnitFile and Deployment declarations with explicit declaration slices.

pub mod construction;
pub mod deployment;
pub mod operator;

mod control_config;
mod ending;
mod identity;
mod image_build;
mod loader;
mod operator_config;
mod ready;
mod supply;
mod supply_config;
mod unit_file;

pub use control_config::ControlConfig;
pub use ending::Ending;
pub use guard_selection_error::GuardSelectionError;
pub use identity::Identity;
pub use image_build::ImageBuild;
pub use loader::LoaderConfig;
pub use operator_config::OperatorConfig;
pub use ready::{READY_MARK, Ready};
pub use supply::valid_supplies;
pub use supply_config::SupplyConfig;
pub use unit_file::UnitFile;

pub use construction::{
    Arguments as ConstructionArguments, Construction, Identity as ConstructionIdentity,
    Owner as ConstructionOwner, Range as ConstructionRange, Selector as ConstructionSelector,
};

pub use deployment::{
    Bootstrap, Dependency, Deployment, SCENE, Target, scene_target, select as select_deployment,
};
pub use deployment_selection_error::DeploymentSelectionError;
pub use order::{DepsFail, order_scene};
pub use retirement::Retirement;

/// Assembly failure number. `true = died here`; assembly's two steps cannot hang here.
pub type Died = env::Reason;

/// Provider-owned logical binding type; UnitFile never restates its layout.
pub use system_api::supply::Binding;

mod deployment_selection_error;
mod guard_selection;
mod guard_selection_error;
mod order;
mod retirement;
