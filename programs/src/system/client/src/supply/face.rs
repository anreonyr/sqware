use env::{Mark, Permission, PieToken, TaskId, Wait};
use ipc::{rpc, time::Deadline};
use system_api::supply::{self, Call};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallError {
    /// Delivery is unknown; the caller must inspect using the same LeaseId.
    Unknown,
    Rejected(supply::Fail),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observation {
    pub state: supply::State,
    pub delivery: Option<supply::Delivery>,
    pub failure: Option<supply::Fail>,
}

pub struct Face {
    rpc: rpc::request::Sender<Call>,
}

impl Face {
    pub fn of(entry: PieToken) -> Result<Self, CallError> {
        Ok(Self {
            rpc: rpc::request::Sender::from_raw(entry, Call::BACK)
                .map_err(|_| CallError::Unknown)?,
        })
    }

    pub fn acquire(
        &self,
        lease: supply::LeaseId,
        target: TaskId,
        source: supply::Key,
        resource: supply::Key,
        slot: Mark,
        permission: Permission,
        wait: Wait,
    ) -> Result<supply::Delivery, CallError> {
        let response = self.request(
            supply::ACQUIRE,
            lease,
            target,
            source,
            resource,
            slot,
            permission,
            wait,
        )?;
        if response.status != wire::OK {
            return Err(CallError::Rejected(supply::fail_from_code(response.status)));
        }
        if supply::State::from_code(response.state) != supply::State::Granted
            || response.seed == PieToken::NONE
            || response.len as usize > supply::MAX_DESCRIPTION
        {
            return Err(CallError::Unknown);
        }
        Ok(decode_delivery(response))
    }

    pub fn inspect(&self, lease: supply::LeaseId, wait: Wait) -> Result<Observation, CallError> {
        let response = self.request(
            supply::INSPECT,
            lease,
            TaskId::new(0),
            supply::Key::from_mark(Mark::NONE),
            supply::Key::from_mark(Mark::NONE),
            Mark::NONE,
            Permission::empty(),
            wait,
        )?;
        let state = supply::State::from_code(response.state);
        let delivery = (state == supply::State::Granted && response.seed != PieToken::NONE)
            .then(|| decode_delivery(response));
        let failure =
            (response.status != wire::OK).then(|| supply::fail_from_code(response.status));
        Ok(Observation {
            state,
            delivery,
            failure,
        })
    }

    pub fn release(&self, lease: supply::LeaseId, wait: Wait) -> Result<supply::State, CallError> {
        self.change_state(supply::RELEASE, lease, wait)
    }

    pub fn cancel(&self, lease: supply::LeaseId, wait: Wait) -> Result<supply::State, CallError> {
        self.change_state(supply::CANCEL, lease, wait)
    }

    fn change_state(
        &self,
        op: u8,
        lease: supply::LeaseId,
        wait: Wait,
    ) -> Result<supply::State, CallError> {
        let response = self.request(
            op,
            lease,
            TaskId::new(0),
            supply::Key::from_mark(Mark::NONE),
            supply::Key::from_mark(Mark::NONE),
            Mark::NONE,
            Permission::empty(),
            wait,
        )?;
        if response.status != wire::OK {
            return Err(CallError::Rejected(supply::fail_from_code(response.status)));
        }
        Ok(supply::State::from_code(response.state))
    }

    #[allow(clippy::too_many_arguments)]
    fn request(
        &self,
        op: u8,
        lease: supply::LeaseId,
        target: TaskId,
        source: supply::Key,
        resource: supply::Key,
        slot: Mark,
        permission: Permission,
        wait: Wait,
    ) -> Result<supply::Response, CallError> {
        let deadline = Deadline::new(wait);
        self.rpc
            .call(deadline, |back| supply::Request {
                op,
                lease: lease.0,
                target,
                source: source.mark().get(),
                resource: resource.mark().get(),
                slot: slot.get(),
                permission: permission.bits(),
                back,
            })
            .map_err(|_| CallError::Unknown)
    }
}

fn decode_delivery(response: supply::Response) -> supply::Delivery {
    let mut delivery = supply::Delivery::EMPTY;
    delivery.seed = response.seed;
    delivery.len = response.len;
    delivery.description[..response.len as usize]
        .copy_from_slice(&response.description[..response.len as usize]);
    delivery
}
