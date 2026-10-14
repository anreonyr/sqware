pub(crate) trait WaitFail: env::FailCode {
    fn busy() -> Self;
    fn oom() -> Self;
}

impl WaitFail for env::RoomFail {
    fn busy() -> Self {
        env::RoomFail::Busy
    }
    fn oom() -> Self {
        env::RoomFail::OoM
    }
}

impl WaitFail for env::UnitFail {
    fn busy() -> Self {
        env::UnitFail::Busy
    }
    fn oom() -> Self {
        env::UnitFail::OoM
    }
}

impl WaitFail for env::MailFail {
    fn busy() -> Self {
        env::MailFail::Busy
    }
    fn oom() -> Self {
        env::MailFail::OoM
    }
}
