#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ending {
    Resident,
}

pub fn self_id() -> crate::TaskId {
    crate::TaskId::new(1)
}

pub fn debark_team(_: crate::TeamId) -> Result<(), crate::Error> {
    crate::record("debark_team");
    match crate::EFFECTS.with(|effects| effects.borrow().debark_fail) {
        Some(source) => Err(crate::Error { source }),
        None => Ok(()),
    }
}

pub fn guard(_: crate::TeamId, _: crate::PieToken) -> Result<(), crate::Error> {
    crate::record("guard");
    match crate::EFFECTS.with(|effects| effects.borrow().guard_fail) {
        Some(source) => Err(crate::Error { source }),
        None => Ok(()),
    }
}

pub fn unguard(_: crate::TeamId, _: crate::PieToken) -> Result<(), crate::Error> {
    crate::record("unguard");
    match crate::EFFECTS.with(|effects| effects.borrow().unguard_fail) {
        Some(source) => Err(crate::Error { source }),
        None => Ok(()),
    }
}

pub fn embark_team(_: crate::TeamId) -> Result<(), crate::Error> {
    crate::record("embark_team");
    match crate::EFFECTS.with(|effects| effects.borrow().embark_team_fail) {
        Some(source) => Err(crate::Error { source }),
        None => Ok(()),
    }
}

pub fn embark_task(_: crate::TaskId) -> Result<(), crate::Error> {
    crate::record("embark_task");
    match crate::EFFECTS.with(|effects| effects.borrow().embark_task_fail) {
        Some(source) => Err(crate::Error { source }),
        None => Ok(()),
    }
}

pub fn slay_team(_: crate::TeamId) -> Result<(), crate::Error> {
    crate::record("slay_team");
    crate::EFFECTS.with(|effects| effects.borrow_mut().joined = true);
    Ok(())
}

pub fn join_task(_: crate::TaskId, _: crate::Wait) -> Result<bool, crate::Error> {
    crate::record("join_task");
    Ok(crate::EFFECTS.with(|effects| effects.borrow().joined))
}

pub fn oust(_: crate::TeamId) -> Result<(), crate::Error> {
    crate::record("oust");
    match crate::EFFECTS.with(|effects| effects.borrow().oust_fail) {
        Some(source) => Err(crate::Error { source }),
        None => Ok(()),
    }
}
