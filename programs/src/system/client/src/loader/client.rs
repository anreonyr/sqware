use env::{Access, PieKind, PieToken, Wait};
use ipc::rpc;
use resource::raw::inspect;
use system_api::loader::{self as frame, Built, Call, Wire};

pub struct Face {
    rpc: rpc::request::Sender<Call>,
}

impl Face {
    pub fn of(entry: PieToken) -> Result<Self, frame::Fail> {
        let info = inspect(entry).map_err(|_| frame::Fail::Bad)?;
        if !info.alive
            || info.owner.get() == 0
            || info.owner == env::unit::self_id()
            || info.vestor != info.owner
            || info.kind != PieKind::Hole
            || info.mark != frame::Grant::Claim.mark()
            || info.permission != Access::STORE.bits()
        {
            return Err(frame::Fail::Denied);
        }
        let rpc =
            rpc::request::Sender::from_raw(entry, Call::BACK).map_err(|_| frame::Fail::Bad)?;
        if rpc.peer() != info.owner {
            return Err(frame::Fail::Denied);
        }
        Ok(Self { rpc })
    }

    pub fn claim(&self, task: env::TaskId, wait: Wait) -> Result<Built, frame::Fail> {
        let said = self
            .rpc
            .call(ipc::time::Deadline::new(wait), |back| {
                Wire::Claim(frame::Claim {
                    op: frame::CLAIM,
                    task,
                    back,
                })
            })
            .map_err(|_| frame::Fail::Unknown)?;
        if said.status != wire::OK || said.task != task || said.team == 0 {
            return Err(frame::Fail::NotReady);
        }
        Ok(Built {
            team: env::TeamId::new(said.team as usize),
            task,
        })
    }
}
