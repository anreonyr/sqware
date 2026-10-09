//! 设备能力交付；返回接收任务表中的编号。
use crate::service::hub::core::Entry;
use env::{Access, PieKind, PieToken, Policy, TaskId};
use resource::port;

pub(super) fn ship(
    entry: Entry,
    to: TaskId,
    kind: PieKind,
    access: Access,
    policy: Policy,
) -> Result<PieToken, ()> {
    let shipped = match kind {
        PieKind::Pole => port::ship(entry.page, to, access, policy),
        PieKind::Nole => port::ship(entry.page, to, access, policy),
        PieKind::Hole | PieKind::Tole => return Err(()),
    };
    shipped.map(|seat| seat.seed()).map_err(|_| ())
}
