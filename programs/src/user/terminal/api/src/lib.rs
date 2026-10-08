#![no_std]

pub mod frame;
pub mod marks;
pub use interface::{INTERFACE_ID, PUBLICATIONS, REGISTRY};

#[mold::interface(id = "sqware.user.terminal.v1", metadata)]
mod interface {
    #[channels]
    pub enum Channel {
        #[channel(key = "entry", legacy = "terminal-attach", constant = ENTRY, publication = "attach")]
        Entry,
        #[channel(key = "authority", legacy = "terminal-authority", constant = AUTHORITY)]
        Authority,
        #[channel(key = "back", legacy = "terminal-back", constant = BACK)]
        Back,
        #[channel(key = "input", legacy = "terminal-input", constant = INPUT)]
        Input,
        #[channel(key = "output", legacy = "terminal-output", constant = OUTPUT)]
        Output,
        #[channel(key = "control", legacy = "terminal-control", constant = CONTROL)]
        Control,
    }
}
