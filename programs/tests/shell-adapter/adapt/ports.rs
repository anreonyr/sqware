use alloc::vec::Vec;

mod pump;
pub use pump::Pump;

pub enum Buffer {
    Read { bytes: Vec<u8>, at: usize },
    Write { bytes: Vec<u8> },
    Terminal(crate::pipe_client::Direction),
}
