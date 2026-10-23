use env::{PieToken, ToleEventSource};

pub struct Port;

impl Port {
    pub fn token(&self) -> PieToken {
        PieToken::NONE
    }

    pub fn prepare_wait(&self) -> Result<bool, pipe_api::Fail> {
        Ok(false)
    }

    pub fn source(&self) -> ToleEventSource {
        ToleEventSource::Pole {
            token: PieToken::NONE,
            bit: pipe_api::DEMAND_BIT,
        }
    }
}
