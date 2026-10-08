use super::sweep::alive;
use super::*;

/// **认领**：那一台由"哪一枚孔响了"回答（`door`）；主人是发送者 ＋ 它交来的那枚报活孔
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
    if !sensor_from(from, sensor) {
        put_deed(back, Deed::of(hub::DENIED));
        return;
    }
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
        match ledger.claim_with(door, owner, alive, |entry| {
            ship(entry.clone(), from, kind, access, policy).map_err(|_| hub::Fail::Denied)
        }) {
            Ok((entry, page)) => Deed::granted(entry.name, entry.line, page),
            Err(hub::Fail::Taken) => Deed::of(hub::TAKEN),
            Err(hub::Fail::Denied) => Deed::of(hub::DENIED),
            Err(_) => Deed::of(hub::UNKNOWN),
        }
    };
    put_deed(back, deed);
}

/// 接入只接受实际发件人直接转授的报活孔。
fn sensor_from(from: TaskId, sensor: PieToken) -> bool {
    ::resource::raw::alive(sensor)
        && matches!(::resource::raw::reserve(sensor), Ok((giver, owner, mark))
            if giver == from && owner == from && mark == hub::ALIVE_MARK)
}
