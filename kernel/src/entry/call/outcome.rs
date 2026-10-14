use crate::machine::switcher::context::{Gprs, TrapContext};

pub(super) enum Outcome {
    Resume,
    Switch(*mut TrapContext),
    Exit,
}

impl Outcome {
    pub(super) fn fail<E: env::FailCode>(frame: &mut TrapContext, e: E) -> Self {
        ret_err(frame, e);
        Self::Resume
    }

    pub(super) fn next(self, frame: *mut TrapContext) -> Option<*mut TrapContext> {
        match self {
            Self::Resume => Some(frame),
            Self::Switch(pa) => {
                debug_assert!(!pa.is_null(), "switched trap frame must be non-null");
                Some(pa)
            }
            Self::Exit => None,
        }
    }
}

pub(super) fn ret_err<E: env::FailCode>(frame: &mut TrapContext, e: E) {
    frame.gpr.set_x(Gprs::A0, e.code() as usize);
}

pub(super) fn answer<F: env::FailCode>(frame: &mut TrapContext, result: Result<usize, F>) {
    match result {
        Ok(value) => frame.gpr.set_x(Gprs::A0, value),
        Err(e) => ret_err(frame, e),
    }
}

pub(super) fn answer_pair<F: env::FailCode>(frame: &mut TrapContext, result: Result<(usize, usize), F>) {
    match result {
        Ok((first, second)) => {
            frame.gpr.set_x(Gprs::A0, first);
            frame.gpr.set_x(Gprs::A1, second);
        }
        Err(e) => ret_err(frame, e),
    }
}
