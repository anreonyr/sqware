use super::*;

/// **报名**：这个类不在册上 ⇒ `Unknown`（这台机器没有这一类）；否则**代报名**——把发送者放进
/// 这一类那枚盟（盟册 `admit`，钥匙 = "你是不是立盟那位"）
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
    let Ok(Some(binding)) = league.query.resolve(from, Wait::AtMost(MS)) else {
        put_said(back, hub::DENIED);
        return;
    };
    let status = match league.organization.admit(coalition, binding.current.principal, Wait::AtMost(MS)) {
        Ok(()) if crate::service::hub::bridge::activate(from, &[coalition]).is_ok() => hub::OK,
        Ok(()) => hub::DENIED,
        Err(_) => hub::DENIED,
    };
    put_said(back, status);
}
