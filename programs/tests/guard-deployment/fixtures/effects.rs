use std::{cell::RefCell, vec::Vec};

#[derive(Default)]
pub struct Effects {
    pub calls: Vec<&'static str>,
    pub debark_fail: Option<crate::UnitFail>,
    pub guard_fail: Option<crate::UnitFail>,
    pub unguard_fail: Option<crate::UnitFail>,
    pub embark_team_fail: Option<crate::UnitFail>,
    pub embark_task_fail: Option<crate::UnitFail>,
    pub oust_fail: Option<crate::UnitFail>,
    pub joined: bool,
    pub inspect: Option<Result<crate::PieInfo, ()>>,
}

thread_local! {
    pub static EFFECTS: RefCell<Effects> = RefCell::new(Effects::default());
}

pub fn reset() {
    EFFECTS.with(|effects| *effects.borrow_mut() = Effects::default());
}

pub fn calls() -> Vec<&'static str> {
    EFFECTS.with(|effects| effects.borrow().calls.clone())
}

pub(crate) fn record(call: &'static str) {
    EFFECTS.with(|effects| effects.borrow_mut().calls.push(call));
}
