//! Scene membership and startup are selected independently from UnitFile declarations.

#[derive(Clone, Copy)]
pub struct Deployment {
    pub name: &'static str,
    pub entry: &'static str,
    pub start: &'static [&'static str],
    pub resources: &'static [&'static str],
    pub targets: &'static [Target],
    pub bootstrap: &'static [Bootstrap],
    pub sources: &'static [&'static str],
}

#[derive(Clone, Copy)]
pub struct Target {
    pub name: &'static str,
    pub members: &'static [&'static str],
}

pub const fn scene_target(members: &'static [&'static str]) -> Target {
    Target {
        name: SCENE,
        members,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bootstrap {
    Operator,
    Identity,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dependency {
    Unit(&'static str),
    Target(&'static str),
    Bootstrap(Bootstrap),
}

pub const SCENE: &str = "scene";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeploymentSelectionError {
    Missing,
    Ambiguous,
}

pub fn select(
    mut contains_image: impl FnMut(&str) -> bool,
) -> Result<&'static Deployment, DeploymentSelectionError> {
    let mut selected = None;
    for deployment in DEPLOYMENTS {
        if deployment.entry_image().is_some_and(&mut contains_image) {
            if selected.is_some() {
                return Err(DeploymentSelectionError::Ambiguous);
            }
            selected = Some(deployment);
        }
    }
    selected.ok_or(DeploymentSelectionError::Missing)
}

pub const DEPLOYMENTS: &[Deployment] = &[
    Deployment {
        name: "accept",
        entry: "accept-system",
        start: &[
            "hub",
            "pipe",
            "account",
            "terminal",
            "login",
            "probe-terminal",
            "guest",
            "passer",
            "lodger",
            "router",
            "uart",
            "rtc",
            "sleeper",
            "subject",
            "member",
            "probe-denied",
            "probe-owner",
            "probe-rule",
            "probe-rule-other",
            "probe-watch",
            "probe-watch-after",
            "probe-watch-gone",
            "probe-lease",
            "probe-bound",
            "probe-rack",
            "probe-rack-mount",
            "probe-rack-guest",
            "probe-coalition",
            "probe-control",
            "probe-operator-gate",
            "probe-operator-land",
        ],
        resources: &[],
        targets: &[scene_target(&[
            "hub",
            "pipe",
            "account",
            "terminal",
            "router",
            "uart",
            "rtc",
            "probe-terminal",
            "guest",
            "passer",
            "lodger",
            "sleeper",
            "subject",
            "member",
            "probe-denied",
            "probe-owner",
            "probe-rule",
            "probe-rule-other",
            "probe-watch",
            "probe-watch-after",
            "probe-watch-gone",
            "probe-lease",
            "probe-bound",
            "probe-rack",
            "probe-rack-mount",
            "probe-rack-guest",
            "probe-coalition",
            "probe-operator-gate",
            "probe-operator-land",
        ])],
        bootstrap: &[Bootstrap::Operator, Bootstrap::Identity],
        sources: &[],
    },
    Deployment {
        name: "product",
        entry: "system",
        start: &[
            "hub", "pipe", "account", "terminal", "login", "router", "uart", "rtc",
        ],
        resources: &["shell", "cat", "emit", "upper", "fail", "spin", "workers"],
        targets: &[scene_target(&[
            "hub", "pipe", "account", "terminal", "router", "uart", "rtc",
        ])],
        bootstrap: &[Bootstrap::Operator, Bootstrap::Identity],
        sources: &[],
    },
    Deployment {
        name: "system-fault",
        entry: "system-fault",
        start: &[
            "hub",
            "pipe",
            "account",
            "router",
            "uart",
            "rtc",
            "system-dependent",
            "system-child",
        ],
        resources: &["system-fault-unit"],
        targets: &[scene_target(&[
            "hub",
            "pipe",
            "account",
            "router",
            "uart",
            "rtc",
            "system-dependent",
            "system-child",
        ])],
        bootstrap: &[Bootstrap::Operator, Bootstrap::Identity],
        sources: &[],
    },
    Deployment {
        name: "memory-transfer",
        entry: "memory-transfer",
        start: &[],
        resources: &["memory-transfer-unit"],
        targets: &[scene_target(&[])],
        bootstrap: &[Bootstrap::Operator, Bootstrap::Identity],
        sources: &[],
    },
    Deployment {
        name: "memory-transfer-tlb",
        entry: "memory-transfer-tlb",
        start: &[],
        resources: &[],
        targets: &[scene_target(&[])],
        bootstrap: &[Bootstrap::Operator, Bootstrap::Identity],
        sources: &[],
    },
    Deployment {
        name: "rig",
        entry: "rig",
        start: &[],
        resources: &["hang"],
        targets: &[scene_target(&[])],
        bootstrap: &[Bootstrap::Operator, Bootstrap::Identity],
        sources: &[],
    },
    Deployment {
        name: "load",
        entry: "load",
        start: &[],
        resources: &["busy", "park"],
        targets: &[scene_target(&[])],
        bootstrap: &[Bootstrap::Operator, Bootstrap::Identity],
        sources: &[],
    },
    Deployment {
        name: "group",
        entry: "group",
        start: &[],
        resources: &["waiter"],
        targets: &[scene_target(&[])],
        bootstrap: &[Bootstrap::Operator, Bootstrap::Identity],
        sources: &[],
    },
    Deployment {
        name: "beat",
        entry: "beat",
        start: &[],
        resources: &[],
        targets: &[scene_target(&[])],
        bootstrap: &[Bootstrap::Operator, Bootstrap::Identity],
        sources: &[],
    },
    Deployment {
        name: "again",
        entry: "again",
        start: &[],
        resources: &["churn"],
        targets: &[scene_target(&[])],
        bootstrap: &[Bootstrap::Operator, Bootstrap::Identity],
        sources: &[],
    },
];

impl Deployment {
    pub fn find(name: &str) -> Option<&'static Self> {
        DEPLOYMENTS
            .iter()
            .find(|deployment| deployment.name == name)
    }

    pub fn entry_image(&self) -> Option<&'static str> {
        super::PROGRAMS
            .iter()
            .find(|program| program.name() == self.entry)
            .map(|program| program.loader.image)
    }
}
