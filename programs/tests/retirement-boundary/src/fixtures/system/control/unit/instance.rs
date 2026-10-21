pub struct Instance {
    pub team: Option<crate::TeamId>,
}

impl Instance {
    pub fn settled(&self) -> bool { self.team.is_none() }
}
