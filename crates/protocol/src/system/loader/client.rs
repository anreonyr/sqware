use super::frame::{self, Ask, Fail, Said, Wire};
use env::{Permission, PieToken, TaskId, TeamId, Wait};
use resource::raw::Loan;
use ipc::{rpc, time::Deadline};

#[derive(Clone, Copy, Debug)]
pub struct Built {
    pub team: TeamId,
    pub task: TaskId,
}

pub struct Face {
    rpc: rpc::Client,
}
impl Face {
    pub fn of(entry: PieToken) -> Result<Self, Fail> {
        Ok(Self {
            rpc: rpc::Client::from_raw(entry, frame::BACK).map_err(|_| Fail::Bad)?,
        })
    }

    pub fn build(
        &self,
        image: PieToken,
        offset: usize,
        len: usize,
        args: &[usize],
        stack: usize,
        wait: Wait,
    ) -> Result<Built, Fail> {
        if args.len() > frame::MAX_ARGS
            || len == 0
            || len > frame::MAX_IMAGE
            || offset.checked_add(len).is_none()
        {
            return Err(Fail::Bad);
        }
        // Build and Claim share one absolute budget, including image capability preparation.
        let deadline = Deadline::new(wait);
        let image_loan = Loan::accord(&image, self.rpc.peer(), Permission::FETCH, frame::IMAGE)
            .map_err(|_| Fail::Denied)?;

        let said = self.rpc.call::<Wire, Said>(&deadline, |back| {
            let mut words = [0; frame::MAX_ARGS];
            for (to, from) in words.iter_mut().zip(args) {
                *to = *from as u64;
            }
            Wire::Build(Ask {
                op: frame::BUILD,
                image: image_loan.remote(),
                offset: offset as u64,
                len: len as u64,
                stack: stack as u64,
                count: args.len() as u8,
                args: words,
                back,
            })
        }).map_err(|_| Fail::Bad)?;
        if said.status != crate::wire::OK {
            return Err(frame::code_to_fail(said.status).unwrap_or(Fail::Bad));
        }
        if said.team == 0 || said.task.get() == 0 {
            return Err(Fail::Bad);
        }

        let confirmed = self.rpc.call::<Wire, Said>(&deadline, |back| {
            Wire::Claim(frame::Claim {
                op: frame::CLAIM,
                task: said.task,
                back,
            })
        }).map_err(|_| Fail::Bad)?;
        if confirmed.status != crate::wire::OK {
            return Err(frame::code_to_fail(confirmed.status).unwrap_or(Fail::Bad));
        }
        if confirmed.task != said.task || confirmed.team != said.team {
            return Err(Fail::Bad);
        }
        Ok(Built {
            team: TeamId::new(said.team as usize),
            task: said.task,
        })
    }
}
