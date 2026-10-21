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

impl WaitFail for env::HoleFail {
    fn busy() -> Self {
        env::HoleFail::Busy
    }
    fn oom() -> Self {
        env::HoleFail::OoM
    }
}

impl WaitFail for env::NoleFail {
    fn busy() -> Self {
        env::NoleFail::Busy
    }
    fn oom() -> Self {
        env::NoleFail::OoM
    }
}

impl WaitFail for env::PoleFail {
    fn busy() -> Self {
        env::PoleFail::Busy
    }
    fn oom() -> Self {
        env::PoleFail::OoM
    }
}

impl WaitFail for env::ToleFail {
    fn busy() -> Self {
        env::ToleFail::Busy
    }
    fn oom() -> Self {
        env::ToleFail::OoM
    }
}
