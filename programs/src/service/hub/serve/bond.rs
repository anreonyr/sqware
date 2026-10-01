use super::*;

/// **报名**：这个类不在册上 ⇒ `Unknown`（这台机器没有这一类）；否则**代报名**——把发送者放进
/// 这一类那枚盟（盟册 `admit`，钥匙 = "你是不是立盟那位"）。
pub(super) fn bond(
    ledger: &mut Ledger,
    league: &League,
    class: String,
    from: TaskId,
    back: PieToken,
) {
    let Some(coalition) = ledger.coalition_of(class) else {
        put_said(back, hub::UNKNOWN);
        return;
    };
    let status = match league.coalition(coalition).admit(from, Wait::AtMost(MS)) {
        Ok(()) => hub::OK,
        Err(_) => hub::DENIED,
    };
    put_said(back, status);
}
