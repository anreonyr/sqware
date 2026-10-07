//! Provider-owned channel marks and their registry.
use env::{Mark, marks::Definition};

macro_rules! channels {
    ($($symbol:ident => $name:expr;)+) => {
        $(pub const $symbol: Mark = Mark::of($name);)+
        pub const DECLARATIONS: &[Definition] = &[
            $(Definition { name: $name, mark: $symbol },)+
        ];
    };
}
channels! {
    ENTRY => "terminal-attach";
    AUTHORITY => "terminal-authority";
    BACK => "terminal-back";
    INPUT => "terminal-input";
    OUTPUT => "terminal-output";
    CONTROL => "terminal-control";
}
const _: () = assert!(env::marks::conflict(&[DECLARATIONS]).is_none());
