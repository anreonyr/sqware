// 取用判据（gate）会答出的条件 —— **域词表的构造子**。
//
// `gate::accede` 与 `envcall::pie::usable` 被 **Pie / Mail / Tole** 三域共用，故它们泛型到
// "答得出这三枚条件的域"上，各域实现各自的词表（答不出的域——Memory / Unit / Room /
// Debug / Control——**不实现它，于是编译期就调不到这两手**）。`Fail` 那一份是过渡期的
// （还没域化的轴仍写 `Fail::X`），清尾时删。

/// 取用这条路上答得出的三枚：**没这枚 / 已封印 / 已交出去**。
pub(crate) trait GateFail: env::FailCode {
    /// 表里没有这一枚 / 权不够 / 类型不符。
    fn denied() -> Self;
    /// 那一枚已封印。
    fn dead() -> Self;
    /// 这一枚已被我交出去（交回即复原）。
    fn handed_over() -> Self;
}

impl GateFail for env::PieFail {
    fn denied() -> Self {
        env::PieFail::Denied
    }
    fn dead() -> Self {
        env::PieFail::Dead
    }
    fn handed_over() -> Self {
        env::PieFail::HandedOver
    }
}

impl GateFail for env::MailFail {
    fn denied() -> Self {
        env::MailFail::Denied
    }
    fn dead() -> Self {
        env::MailFail::Dead
    }
    fn handed_over() -> Self {
        env::MailFail::HandedOver
    }
}

impl GateFail for env::ToleFail {
    fn denied() -> Self {
        env::ToleFail::Denied
    }
    fn dead() -> Self {
        env::ToleFail::Dead
    }
    fn handed_over() -> Self {
        env::ToleFail::HandedOver
    }
}

impl GateFail for env::Fail {
    fn denied() -> Self {
        env::Fail::Denied
    }
    fn dead() -> Self {
        env::Fail::Dead
    }
    fn handed_over() -> Self {
        env::Fail::HandedOver
    }
}
