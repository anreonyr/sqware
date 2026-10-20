//! Scene membership and startup are selected independently from UnitFile declarations.

use super::DeploymentSelectionError;

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

pub fn select(
    name: &str,
    entry: env::ledger::manifest::Entry<'_>,
) -> Result<&'static Deployment, DeploymentSelectionError> {
    let deployment = Deployment::find(name).ok_or(DeploymentSelectionError::Unknown)?;
    let program = super::PROGRAMS.iter()
        .find(|program| program.name() == deployment.entry)
        .ok_or(DeploymentSelectionError::EntryMismatch)?;
    if entry.name != program.loader.image || entry.kind != program.loader.space {
        return Err(DeploymentSelectionError::EntryMismatch);
    }
    Ok(deployment)
}

pub const DEPLOYMENTS: &[Deployment] = &[
    Deployment {
        name: "iommu-ruin",
        entry: "iommu-ruin",
        start: &["hub", "pci", "iommu", "recovery", "device-ctl-ruin"],
        resources: &[],
        targets: &[scene_target(&["hub", "pci", "iommu", "recovery", "device-ctl-ruin"])],
        bootstrap: &[Bootstrap::Operator, Bootstrap::Identity],
        sources: &[],
    },
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
        name: "iommu-product",
        entry: "system",
        start: &[
            "hub",
            "pipe",
            "account",
            "terminal",
            "login",
            "router",
            "uart",
            "rtc",
            "pci",
            "iommu",
            "recovery",
            "device-ctl",
        ],
        resources: &["shell", "cat", "emit", "upper", "fail", "spin", "workers"],
        targets: &[scene_target(&[
            "hub",
            "pipe",
            "account",
            "terminal",
            "router",
            "uart",
            "rtc",
            "pci",
            "iommu",
            "recovery",
            "device-ctl",
        ])],
        bootstrap: &[Bootstrap::Operator, Bootstrap::Identity],
        sources: &[],
    },
    Deployment {
        name: "iommu-product-fault",
        entry: "system",
        start: &[
            "hub",
            "pipe",
            "account",
            "terminal",
            "login",
            "router",
            "uart",
            "rtc",
            "pci",
            "iommu",
            "recovery",
            "device-ctl-fault",
        ],
        resources: &["shell", "cat", "emit", "upper", "fail", "spin", "workers"],
        targets: &[scene_target(&[
            "hub",
            "pipe",
            "account",
            "terminal",
            "router",
            "uart",
            "rtc",
            "pci",
            "iommu",
            "recovery",
            "device-ctl-fault",
        ])],
        bootstrap: &[Bootstrap::Operator, Bootstrap::Identity],
        sources: &[],
    },
    Deployment {
        name: "iommu-product-before",
        entry: "system",
        start: &[
            "hub",
            "pipe",
            "account",
            "terminal",
            "login",
            "router",
            "uart",
            "rtc",
            "pci",
            "iommu",
            "recovery",
            "device-ctl-before",
        ],
        resources: &["shell", "cat", "emit", "upper", "fail", "spin", "workers"],
        targets: &[scene_target(&[
            "hub",
            "pipe",
            "account",
            "terminal",
            "router",
            "uart",
            "rtc",
            "pci",
            "iommu",
            "recovery",
            "device-ctl-before",
        ])],
        bootstrap: &[Bootstrap::Operator, Bootstrap::Identity],
        sources: &[],
    },
    Deployment {
        name: "iommu-product-after",
        entry: "system",
        start: &[
            "hub",
            "pipe",
            "account",
            "terminal",
            "login",
            "router",
            "uart",
            "rtc",
            "pci",
            "iommu",
            "recovery",
            "device-ctl-after",
        ],
        resources: &["shell", "cat", "emit", "upper", "fail", "spin", "workers"],
        targets: &[scene_target(&[
            "hub",
            "pipe",
            "account",
            "terminal",
            "router",
            "uart",
            "rtc",
            "pci",
            "iommu",
            "recovery",
            "device-ctl-after",
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
        name: "memory-transfer-fresh",
        entry: "memory-transfer-fresh",
        start: &[],
        resources: &[],
        targets: &[scene_target(&[])],
        bootstrap: &[Bootstrap::Operator, Bootstrap::Identity],
        sources: &[],
    },
    Deployment {
        name: "memory-transfer-race",
        entry: "memory-transfer-race",
        start: &[],
        resources: &["memory-transfer-race-unit"],
        targets: &[scene_target(&[])],
        bootstrap: &[Bootstrap::Operator, Bootstrap::Identity],
        sources: &[],
    },
    Deployment {
        name: "iommu-probe",
        entry: "iommu-probe",
        start: &[],
        resources: &[],
        targets: &[scene_target(&[])],
        bootstrap: &[Bootstrap::Operator, Bootstrap::Identity],
        sources: &[],
    },
    Deployment {
        name: "iommu-dma",
        entry: "iommu-dma",
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
