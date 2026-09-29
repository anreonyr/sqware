pub(crate) trait GateFail: env::FailCode {
    fn denied() -> Self;
    fn dead() -> Self;
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
