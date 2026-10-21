use crate::system_client::operator::Capability;

pub struct System;

impl System {
    pub fn capability(&self) -> &Capability {
        &CAPABILITY
    }
}

const CAPABILITY: Capability = Capability;
