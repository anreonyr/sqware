use super::sweep::alive;
use super::*;

/// **认领**：那一台由"哪一枚孔响了"回答（`door`）；主人是发送者 ＋ 它交来的那枚报活孔。
pub(super) fn claim(
    ledger: &mut Ledger,
    door: PieToken,
    from: TaskId,
    sensor: PieToken,
    kind: u8,
    access: u32,
    policy: u32,
    back: PieToken,
) {
    let deed = {
        let (Some(kind), Some(access), Some(policy)) = (
            PieKind::of(kind),
            Access::from_bits(access),
            Policy::from_bits(policy),
        ) else {
            put_deed(back, Deed::of(hub::BAD));
            return;
        };
        let owner = Owner { task: from, sensor };
        match ledger.claim(door, owner, alive) {
            Ok(entry) => match ship(entry.clone(), from, kind, access, policy) {
                Ok(page) => Deed::granted(entry.name, entry.line, page),
                Err(_) => Deed::of(hub::DENIED),
            },
            Err(hub::Fail::Taken) => Deed::of(hub::TAKEN),
            Err(_) => Deed::of(hub::UNKNOWN),
        }
    };
    put_deed(back, deed);
}
