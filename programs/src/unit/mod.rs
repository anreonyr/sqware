//! Static module configuration combined by the composition root.
//!
//! 声明跟着 program.rs：每个 bin 自有目录，声明就在那里。

/// 装配失败号。**true = 死在这**——装配期那两步不能往里挂死。
pub type Died = env::Reason;

/// A program's module configuration.
#[derive(Clone, Copy)]
pub struct UnitFile {
    pub name: &'static str,
    pub loader: LoaderConfig,
    pub operator: OperatorConfig,
    pub identity: Identity,
    pub control: ControlConfig,
    pub supply: SupplyConfig,
}

impl UnitFile {
    /// 清单名。
    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn listed(&self) -> bool {
        self.control.after.is_some()
    }
}

/// Identity facts owned by the identity module.
#[derive(Clone, Copy)]
pub struct Identity {
    /// Names this unit may publish for principals inside its own identity subtree.
    pub aliases: &'static [&'static str],
    pub grants: &'static [system_api::identity::Grant],
}

/// Static publication and construction rights owned by Operator.
#[derive(Clone, Copy)]
pub struct OperatorConfig {
    pub namespace: &'static [Namespace],
}

/// Lifecycle configuration owned by Control.
#[derive(Clone, Copy)]
pub struct ControlConfig {
    pub after: Option<&'static [Dependency]>,
    pub ending: Option<Ending>,
    pub ready: Ready,
    pub construction: &'static [Construction],
    pub retirement: &'static [Retirement],
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ready {
    OnSpawn,
    Signal(env::Mark),
}

pub const READY_MARK: env::Mark = env::Mark::of("ready");

/// Resources required during preparation.
#[derive(Clone, Copy)]
pub struct SupplyConfig {
    pub bindings: &'static [Binding],
}

/// 谁结束它——这一台的寿命由谁定。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ending {
    /// 我收它：只有被收才走。
    Resident,
    Transient,
    /// 有人叫它走：终止词从外面来。
    Told,
}

impl Identity {
    pub const DEFAULT: Identity = Identity {
        aliases: &[],
        grants: &[],
    };
}

impl OperatorConfig {
    pub const DEFAULT: OperatorConfig = OperatorConfig { namespace: &[] };
}

impl ControlConfig {
    pub const DEFAULT: ControlConfig = ControlConfig {
        after: None,
        ending: None,
        ready: Ready::OnSpawn,
        construction: &[],
        retirement: &[],
    };
}

impl SupplyConfig {
    pub const DEFAULT: SupplyConfig = SupplyConfig { bindings: &[] };
}

mod supply;
#[allow(unused_imports)]
pub use supply::{Binding, descriptor_mark, valid_supplies};

mod deployment;
#[allow(unused_imports)]
pub use deployment::{
    Bootstrap, DEPLOYMENTS, Dependency, Deployment, SCENE, Target,
    select as select_deployment,
};
mod deployment_selection_error;
pub use deployment_selection_error::DeploymentSelectionError;
mod package;
pub use package::{PACKAGES, Package};
mod loader;
#[allow(unused_imports)]
pub use loader::LoaderConfig;

mod catalog;
pub use catalog::*;

mod order;
pub use order::*;

pub mod operator;
pub use operator::{Binding as OperatorBinding, Namespace, Scope};

mod construction;
pub use construction::{
    Arguments as ConstructionArguments, Construction, Identity as ConstructionIdentity,
    Owner as ConstructionOwner, Range as ConstructionRange, Selector as ConstructionSelector,
};

impl UnitFile {
    pub fn valid(&self) -> bool {
        valid_supplies(self.supply()) && self.control.retirement.iter().all(Retirement::valid)
    }
    /// 这一台显式声明要交的资源与启动凭据。
    pub fn supply(&self) -> &'static [Binding] {
        self.supply.bindings
    }
}

mod retirement;
pub use retirement::Retirement;

mod image;
#[allow(unused_imports)]
pub use image::{IMAGE_MARK, SupplyFrame};
