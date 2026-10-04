/// Serialize transfer-graph changes. Take task snapshots only after acquiring it.
pub(crate) static GRAPH: crate::lock::SpinLock<()> = crate::lock::SpinLock::new(());
mod forget;
pub(crate) use forget::{forget, same};
mod accord;
mod cull;
mod fail;
mod narrow;
mod pie;
mod release;
mod revoke;

pub(crate) use fail::GateFail;
mod snap;

#[cfg(debug_assertions)]
pub(crate) use pie::form_ok;
pub(crate) use pie::{
    AnyPie, Hole, Need, Nole, Permission, Pie, Pole, Tole, accede, allows, locate, new_pie, try_new_pie,
};

pub(crate) use accord::{accord, clear_heir};
pub(crate) use cull::{cull, doom};
#[cfg(debug_assertions)]
pub(crate) use narrow::narrow;
pub(crate) use narrow::reduce;
pub(crate) use release::release;
pub(crate) use revoke::revoke;
pub(crate) use snap::{install, snap, vestor};
