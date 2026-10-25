use ::schedule::{Progress, Res, ResMut};
use system_api::operator::{self as api, NamespaceKey};

/// The System-owned Operator authority, retained while product layout is constructed.
pub(crate) type Layout = Option<crate::system::app::namespace::System>;

#[derive(Default)]
pub(crate) struct BootstrapReceipt {
    operation_start: Option<api::OperationId>,
    pending: Option<system_client::operator::PartResult>,
    cursor: usize,
    declarations: Option<crate::system::app::Declarations>,
}

impl BootstrapReceipt {
    pub(crate) fn new(declarations: crate::system::app::Declarations) -> Self {
        Self {
            declarations: Some(declarations),
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy)]
struct RootPart {
    key: NamespaceKey,
    name: &'static str,
}

// The product chooses this layout. System itself only exposes generic part/associate/resolve.
const ROOT_PARTS: &[RootPart] = &[
    RootPart {
        key: system_api::namespace::SERVICE,
        name: "service",
    },
    RootPart {
        key: system_api::namespace::DEVICE,
        name: "device",
    },
    RootPart {
        key: system_api::namespace::IDENTITY,
        name: "identity",
    },
    RootPart {
        key: system_api::namespace::UNIT,
        name: "unit",
    },
];

const WAIT: env::Wait = env::Wait::AtMost(crate::support::timing::BOOT_MS);

pub(crate) fn bootstrap(
    tree: Res<crate::system::operator::management::Tree>,
    mut layout: ResMut<Layout>,
    mut receipt: ResMut<BootstrapReceipt>,
) -> Result<Progress, &'static str> {
    if layout.is_none() {
        let info = tree.root_entry()?;
        *layout = Some(
            crate::system::app::namespace::System::new(info)
                .map_err(|_| "operator root namespace")?,
        );
    }
    let system = layout.as_ref().ok_or("operator root namespace")?;
    if receipt.operation_start.is_none() {
        receipt.operation_start = Some(
            system
                .reserve_operations(ROOT_PARTS.len())
                .map_err(|_| "namespace operations")?,
        );
    }
    let start = receipt.operation_start.ok_or("namespace operations")?;
    while let Some(part) = ROOT_PARTS.get(receipt.cursor).copied() {
        if receipt.pending.is_none() {
            let operation = api::OperationId(start.0 + receipt.cursor as u64);
            let outcome = match system.part(crate::system::app::namespace::PartRequest {
                parent: system.root(),
                name: part.name,
                operation,
                wait: WAIT,
            }) {
                Ok(outcome) => outcome,
                Err(api::Fail::Unjudged) => return Ok(Progress::Pending),
                Err(_) => return Err("operator root namespace"),
            };
            receipt.pending = Some(outcome);
        }
        let pending = receipt.pending.ok_or("namespace part receipt")?;
        match system.associate(crate::system::app::namespace::AssociateRequest {
            key: part.key,
            entry: pending.entry,
            wait: WAIT,
        }) {
            Ok(()) => {}
            Err(api::Fail::Unjudged) => return Ok(Progress::Pending),
            Err(_) => return Err("operator root association"),
        }
        receipt.pending = None;
        receipt.cursor += 1;
    }
    for program in receipt
        .declarations
        .ok_or("declaration set")?
        .list()
        .iter()
        .copied()
    {
        for declaration in program.operator.namespace {
            match system.resolve(declaration.key, WAIT) {
                Ok(_) => {}
                Err(api::Fail::Unjudged) => return Ok(Progress::Pending),
                Err(_) => return Err("unknown namespace key"),
            }
        }
    }
    Ok(Progress::Done)
}
