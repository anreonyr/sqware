use alloc::collections::VecDeque;
use alloc::vec::Vec;

use crate::native::objects::LocalPort;

pub enum Buffer {
    Read { bytes: Vec<u8>, at: usize },
    Write { bytes: Vec<u8> },
    Terminal(crate::pipe_client::Direction),
}

pub struct Pump {
    pub local: u64,
    pub port: crate::pipe_client::Port,
    pub done: bool,
    pub pending: Vec<u8>,
    pub at: usize,
}

impl Pump {
    pub fn source(&self) -> bool {
        false
    }

    pub fn demand(&self) -> bool {
        false
    }

    pub fn hush_demand(&mut self) {}

    pub fn source_step(
        &mut self,
        _: &mut LocalPort,
        _: &mut VecDeque<u8>,
        _: bool,
    ) -> Result<(), alloc::string::String> {
        Ok(())
    }

    pub fn sink_step(
        &mut self,
        _: &mut LocalPort,
        _: &mut VecDeque<u8>,
        _: usize,
    ) -> Result<(), alloc::string::String> {
        crate::backend::pump_step();
        Ok(())
    }
}
