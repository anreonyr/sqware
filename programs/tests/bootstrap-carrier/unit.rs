pub fn self_id() -> crate::TaskId {
    crate::EFFECTS.with(|effects| effects.borrow().me)
}

pub fn creator() -> crate::TaskId {
    crate::EFFECTS.with(|effects| effects.borrow().creator)
}
